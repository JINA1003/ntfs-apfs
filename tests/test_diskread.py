import gzip
import struct
from pathlib import Path

import pytest

import diskread

DATA = Path(__file__).parent / "data"
NTFS = gzip.decompress((DATA / "ntfs.bin.gz").read_bytes())
APFS = gzip.decompress((DATA / "apfs.bin.gz").read_bytes())


def write(tmp_path, data):
    p = tmp_path / "disk.img"
    p.write_bytes(data)
    return str(p)


def gpt_disk(*volumes):
    # 보호 MBR(LBA0) + GPT 헤더(LBA1) + 엔트리 배열(LBA2~) + 1MiB 정렬된 볼륨들
    entries, body, lba = b"", b"", 2048
    for vol in volumes:
        last = lba + len(vol) // 512 - 1
        entries += b"\x01" * 16 + b"\x00" * 16 + struct.pack("<QQ", lba, last) + b"\x00" * 80
        body += vol
        lba = last + 1
    mbr = bytearray(512)
    mbr[446 + 4] = 0xEE
    mbr[510:512] = b"\x55\xaa"
    header = b"EFI PART".ljust(72, b"\x00") + struct.pack("<QII", 2, len(volumes), 128)
    return (bytes(mbr) + header.ljust(512, b"\x00") + entries).ljust(2048 * 512, b"\x00") + body


def mbr_disk(vol):
    mbr = bytearray(512)
    mbr[446:462] = struct.pack("<B3sB3sII", 0, b"", 0x07, b"", 2048, len(vol) // 512)
    mbr[510:512] = b"\x55\xaa"
    return bytes(mbr).ljust(2048 * 512, b"\x00") + vol


def test_partitions_raw_volume(tmp_path):
    with open(write(tmp_path, NTFS), "rb") as fh:
        assert diskread.partitions(fh) == [(0, None, "ntfs")]


def test_partitions_gpt(tmp_path):
    with open(write(tmp_path, gpt_disk(NTFS, APFS)), "rb") as fh:
        assert diskread.partitions(fh) == [
            (2048 * 512, len(NTFS), "ntfs"),
            (2048 * 512 + len(NTFS), len(APFS), "apfs"),
        ]


def test_partitions_mbr(tmp_path):
    with open(write(tmp_path, mbr_disk(NTFS)), "rb") as fh:
        assert diskread.partitions(fh) == [(2048 * 512, len(NTFS), "ntfs")]


def test_ntfs_ls_and_read(tmp_path):
    with open(write(tmp_path, NTFS), "rb") as fh:
        fs = diskread.open_fs(fh)
        names = {name: is_dir for name, is_dir, _ in fs.ls("/")}
        assert names["Directory"] is True and names["File.txt"] is False
        assert [n for n, _, _ in fs.ls("Directory")] == ["File 1.txt", "File 2.txt"]
        assert fs.open("/File.txt").read() == b"Contents here"


def test_apfs_ls_and_read_inside_gpt(tmp_path):
    with open(write(tmp_path, gpt_disk(NTFS, APFS)), "rb") as fh:
        fs = diskread.open_fs(fh, part=1)
        assert ("dir", True, 0) in [(n, d, 0) for n, d, _ in fs.ls("/")]
        assert fs.open("dir/file").read() == b"\xef\xa3\xbf File System\n"


def test_extract_directory(tmp_path):
    with open(write(tmp_path, NTFS), "rb") as fh:
        fs = diskread.open_fs(fh)
        diskread.extract(fs, "Directory", tmp_path / "out")
    assert sorted(p.name for p in (tmp_path / "out" / "Directory").iterdir()) == ["File 1.txt", "File 2.txt"]


@pytest.mark.parametrize("bad", ["..", ".", "", "a/b", "a\\b"])
def test_safe_name_rejects_traversal(bad):
    with pytest.raises(ValueError):
        diskread.safe_name(bad)


def test_safe_name_replaces_windows_chars():
    assert diskread.safe_name('a:b?"c') == "a_b__c"
