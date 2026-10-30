"""NTFS / APFS 읽기 전용 도구.

디스크 이미지 또는 물리 디스크(Windows: \\\\.\\PhysicalDrive2, Linux: /dev/sdb)를 열어
파티션을 찾고, 목록을 보고, 파일을 꺼낸다. 원본에는 절대 쓰지 않는다.
"""

import argparse
import re
import struct
import sys
from pathlib import Path

from dissect.apfs import APFS
from dissect.ntfs import NTFS
from dissect.util.stream import RangeStream


def detect(head):
    if head[3:11] == b"NTFS    ":
        return "ntfs"
    if head[32:36] == b"NXSB":
        return "apfs"
    return None


def read_at(fh, offset, length):
    fh.seek(offset)
    return fh.read(length)


def partitions(fh):
    """(offset, size, fs) 목록. 파티션 테이블 없이 볼륨 자체면 (0, None, fs)."""
    # 물리 디스크는 섹터 정렬 읽기만 허용하므로 4096 배수로 읽는다
    head = read_at(fh, 0, 8192)
    if fs := detect(head):
        return [(0, None, fs)]

    ranges = []
    for sector in (512, 4096):  # 512e / 4Kn 디스크
        if head[sector : sector + 8] == b"EFI PART":
            entries_lba, count, entry_size = struct.unpack_from("<QII", head, sector + 72)
            length = -(-count * entry_size // 4096) * 4096
            table = read_at(fh, entries_lba * sector, length)
            for i in range(count):
                e = table[i * entry_size : (i + 1) * entry_size]
                first, last = struct.unpack_from("<QQ", e, 32)
                if e[:16] != bytes(16):
                    ranges.append((first * sector, (last - first + 1) * sector))
            break
    else:
        if head[510:512] == b"\x55\xaa":
            for i in range(4):
                ptype, start, num = struct.unpack_from("<4xB3xII", head, 446 + i * 16)
                if ptype:
                    ranges.append((start * 512, num * 512))

    result = []
    for offset, size in ranges:
        if fs := detect(read_at(fh, offset, 4096)):
            result.append((offset, size, fs))
    return result


class NtfsFS:
    def __init__(self, stream):
        self.fs = NTFS(stream)

    def _get(self, path):
        return self.fs.mft.get(path.strip("/").replace("/", "\\"))

    def is_dir(self, path):
        return self._get(path).is_dir()

    def ls(self, path):
        out = []
        for name, entry in self._get(path).listdir().items():
            if name == ".":
                continue
            # ponytail: 인덱스의 file_size는 갱신이 늦을 수 있음, 정확한 값이 필요하면 dereference().size()
            fn = entry.attribute
            out.append((name, fn.is_dir(), 0 if fn.is_dir() else fn.file_size))
        return out

    def open(self, path):
        return self._get(path).open()


class ApfsFS:
    def __init__(self, stream, volume=0):
        self.fs = APFS(stream).volumes[volume]

    def _get(self, path):
        return self.fs.get(path.strip("/") or "/")

    def is_dir(self, path):
        return self._get(path).is_dir()

    def ls(self, path):
        out = []
        for name, entry in self._get(path).listdir().items():
            if entry.is_dir():
                out.append((name, True, 0))
            elif entry.is_file():
                out.append((name, False, self.fs.get(entry).size))
            # ponytail: 심볼릭 링크·FIFO 등 특수 파일은 건너뜀, 필요해지면 readlink로 추가
        return out

    def open(self, path):
        return self._get(path).open()


def open_fs(fh, part=None, volume=0):
    parts = partitions(fh)
    if not parts:
        raise SystemExit("NTFS/APFS 파티션을 찾지 못했습니다")
    offset, size, kind = parts[part or 0]
    stream = RangeStream(fh, offset, size)
    return NtfsFS(stream) if kind == "ntfs" else ApfsFS(stream, volume)


def safe_name(name):
    """디스크에서 읽은 이름을 로컬 파일명으로 쓸 수 있게 정리. 경로 탈출은 거부."""
    if name in ("", ".", "..") or "/" in name or "\\" in name:
        raise ValueError(f"안전하지 않은 파일명: {name!r}")
    return re.sub(r'[<>:"|?*\x00-\x1f]', "_", name)


def extract(fs, path, dest):
    target = Path(dest) / safe_name(path.strip("/").split("/")[-1] or "root")
    if fs.is_dir(path):
        target.mkdir(parents=True, exist_ok=True)
        for name, _, _ in fs.ls(path):
            extract(fs, f"{path.rstrip('/')}/{name}", target)
    else:
        target.parent.mkdir(parents=True, exist_ok=True)
        with fs.open(path) as src, open(target, "wb") as out:
            while chunk := src.read(1 << 20):
                out.write(chunk)
        print(target)


def main(argv=None):
    ap = argparse.ArgumentParser(description="NTFS/APFS 읽기 전용 도구")
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("parts", help="파티션 목록").add_argument("source")
    for name in ("ls", "get"):
        p = sub.add_parser(name, help="목록 보기" if name == "ls" else "파일/폴더 꺼내기")
        p.add_argument("source", help=r"이미지 파일 또는 \\.\PhysicalDriveN, /dev/sdX")
        p.add_argument("path", nargs="?" if name == "ls" else None, default="/")
        if name == "get":
            p.add_argument("dest")
        p.add_argument("-p", "--part", type=int, help="parts 목록의 번호 (기본: 첫 번째)")
        p.add_argument("-v", "--volume", type=int, default=0, help="APFS 볼륨 번호")
    args = ap.parse_args(argv)

    # buffering=0: 물리 디스크에 정렬되지 않은 읽기가 가지 않도록 RangeStream에 정렬을 맡긴다
    with open(args.source, "rb", buffering=0) as fh:
        if args.cmd == "parts":
            for i, (offset, size, kind) in enumerate(partitions(fh)):
                print(f"[{i}] {kind}  offset={offset}  size={size or '?'}")
                if kind == "apfs":
                    for j, vol in enumerate(APFS(RangeStream(fh, offset, size)).volumes):
                        print(f"     -v {j}: {vol.name}")
            return
        fs = open_fs(fh, args.part, args.volume)
        if args.cmd == "ls":
            for name, is_dir, size in fs.ls(args.path):
                print(f"{'<DIR>' if is_dir else size:>12}  {name}")
        else:
            extract(fs, args.path, args.dest)


if __name__ == "__main__":
    sys.exit(main())
