#!/usr/bin/env python3
"""Positive alias-preservation control for the owned RGB profile fixtures."""
import struct
import subprocess
import sys
from pathlib import Path

root = Path(sys.argv[1])
rewriter = sys.argv[2]
work = Path(sys.argv[3]) if len(sys.argv) == 4 else root / "ordinary"
work.mkdir(parents=True, exist_ok=True)
source = root / "ordinary/srgb-v4.icc"
shared = work / "control-shared-trc.icc"
output = work / "control-shared-trc.scrubbed.icc"
shared.write_bytes(source.read_bytes())


def locations(data):
    count = int.from_bytes(data[128:132], "big")
    return {signature: (offset, size)
            for index in range(count)
            for signature, offset, size in [struct.unpack_from(">4sII", data, 132 + 12 * index)]}


def assert_shared(data):
    tags = locations(data)
    if len({tags[signature] for signature in (b"rTRC", b"gTRC", b"bTRC")}) != 1:
        raise AssertionError("fixture RGB TRC extents are not identically shared")


assert_shared(shared.read_bytes())
subprocess.run([sys.executable, "-B", rewriter, shared, output], check=True)
assert_shared(output.read_bytes())
print("ordinary RGB shared-TRC control passed: source and rewritten aliases verified")
