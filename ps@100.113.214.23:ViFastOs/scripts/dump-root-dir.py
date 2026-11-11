#!/usr/bin/env python3
"""r259: đọc trực tiếp block thư mục gốc (lba 30) từ ảnh đĩa ViFastOs."""
import sys

BS = 512
DIR_ENTRY = 32


def dump(path, lba=30):
    with open(path, "rb") as handle:
        handle.seek(lba * BS)
        block = handle.read(BS)
    entries = []
    for slot in range(BS // DIR_ENTRY):
        off = slot * DIR_ENTRY
        ino = int.from_bytes(block[off:off + 2], "little")
        length = block[off + 2]
        name = block[off + 3:off + 3 + min(length, 28)]
        entries.append((slot, ino, length, name))
    print(f"== {path} lba={lba}")
    for slot, ino, length, name in entries:
        if ino == 0:
            continue
        print(f"  slot={slot} ino={ino} len={length} name={name!r}")
    return entries


if __name__ == "__main__":
    for image in sys.argv[1:]:
        dump(image)
