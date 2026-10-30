"""ops.ps1이 남긴 기대값을 winapfs와 별개인 APFS 구현(dissect.apfs)으로 이미지에서 직접 확인한다.

사용: verify_apfs.py <이미지> <expected.txt> [파티션 바이트 오프셋]
"""
import hashlib
import sys

from dissect.apfs import APFS
from dissect.util.stream import RangeStream

image, expected, offset = sys.argv[1], sys.argv[2], int(sys.argv[3]) if len(sys.argv) > 3 else 0
fh = open(image, "rb")
vol = APFS(RangeStream(fh, offset, None) if offset else fh).volumes[0]
fails = 0
for line in open(expected, encoding="utf-8-sig").read().splitlines():
    kind, rest = line.split(" ", 1)
    if kind == "absent":
        try:
            vol.get(rest)
            ok = False
        except FileNotFoundError:
            ok = True
    elif kind == "text":
        path, text = rest.split(" ", 1)
        ok = vol.get(path).open().read().decode("utf-8-sig") == text
    else:
        h = hashlib.sha256()
        with vol.get(rest).open() as f:
            while chunk := f.read(1 << 22):
                h.update(chunk)
        ok = h.hexdigest() == kind
    print(("ok    " if ok else "FAIL  ") + line)
    fails += not ok
sys.exit(1 if fails else 0)
