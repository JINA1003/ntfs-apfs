"""APFS 컨테이너 이미지를 GPT 디스크로 감싸 고정 크기 VHD로 만든다 (Windows가 실제 디스크처럼 연결)."""
import struct, sys, uuid, zlib, time

src, out = sys.argv[1], sys.argv[2]
S = 512
APFS_TYPE = uuid.UUID("7C3457EF-0000-11AA-AA11-00306543ECAC").bytes_le
data = open(src, "rb").read()
first = 2048
last = first + len(data) // S - 1
total = last + 1 + 2048  # 뒤쪽 여유 + 백업 GPT
entries = bytearray(128 * 128)
entries[0:128] = APFS_TYPE + uuid.uuid4().bytes_le + struct.pack("<QQQ", first, last, 0) + "Mac SSD".encode("utf-16-le").ljust(72, b"\0")
ecrc = zlib.crc32(entries)
disk_guid = uuid.uuid4().bytes_le

def header(cur, backup, entries_lba):
    h = bytearray(struct.pack("<8sIIIIQQQQ16sQIII", b"EFI PART", 0x10000, 92, 0, 0, cur, backup, 34, total - 34, disk_guid, entries_lba, 128, 128, ecrc))
    struct.pack_into("<I", h, 16, zlib.crc32(h))
    return bytes(h).ljust(S, b"\0")

mbr = bytearray(S)
mbr[446:462] = struct.pack("<B3sB3sII", 0, b"\0\x02\0", 0xEE, b"\xff\xff\xff", 1, min(total - 1, 0xFFFFFFFF))
mbr[510:512] = b"\x55\xaa"
with open(out, "wb") as f:
    f.write(mbr + header(1, total - 1, 2) + entries)
    f.seek(first * S); f.write(data)
    f.seek((total - 33) * S); f.write(entries + header(total - 1, 1, total - 33))
    size = total * S
    # VHD 고정 디스크 꼬리표
    cyl = min(size // S // (16 * 63), 65535)
    foot = bytearray(struct.pack(">8sIIQI4sI4sQQHBBII16sB", b"conectix", 2, 0x10000, 0xFFFFFFFFFFFFFFFF,
                                 int(time.time()) - 946684800, b"rea ", 0x10000, b"Wi2k", size, size, cyl, 16, 63, 2, 0,
                                 uuid.uuid4().bytes, 0)).ljust(512, b"\0")
    struct.pack_into(">I", foot, 64, (~sum(foot)) & 0xFFFFFFFF)
    f.seek(size); f.write(foot)
print(out, size + 512)
