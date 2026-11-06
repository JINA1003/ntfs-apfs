"""이미지의 모든 스냅샷에 든 모든 파일의 SHA-256을 winapfs와 별개인 구현(dissect.apfs)으로 출력한다.

쓰기 전과 후의 출력이 같으면 쓰기가 스냅샷을 건드리지 않은 것이다.
사용: snapshot_hashes.py <이미지> [N]   N: 스냅샷 N개마다 하나와 마지막 것만 (기본 1 = 전부)
"""
import hashlib
import sys

from dissect.apfs import APFS


def walk(fs, path):
    for name in sorted(fs.get(path).listdir()):
        child = f"{path.rstrip('/')}/{name}"
        node = fs.get(child)
        if node.is_dir():
            yield from walk(fs, child)
        elif node.is_file():
            yield child, hashlib.sha256(node.open().read()).hexdigest()


step = int(sys.argv[2]) if len(sys.argv) > 2 else 1
for vol in APFS(open(sys.argv[1], "rb")).volumes:
    snaps = list(vol.snapshots)
    last = [snaps[-1]] if snaps and (len(snaps) - 1) % step else []
    for snap in snaps[::step] + last:
        for path, digest in walk(snap.open(), "/"):
            print(f"{vol.name}@{snap.name} {digest} {path}")
