#!/usr/bin/env python3
"""Curve extents exclude the separate ICC word-alignment padding bytes."""
import importlib.util
import struct
import sys

spec = importlib.util.spec_from_file_location("icc_rewriter", sys.argv[1])
rewriter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rewriter)


def curve(samples):
    return b"curv" + b"\0" * 4 + struct.pack(">I", len(samples)) + b"".join(
        struct.pack(">H", value) for value in samples
    )


def accepted(payload):
    # Exercise the actual canonicalizer's admitted-type body parser.
    rewriter.body(payload, 2, b"rTRC")


def rejected(payload, expected):
    try:
        accepted(payload)
    except ValueError as error:
        if str(error) != expected:
            raise AssertionError(f"expected {expected!r}, got {str(error)!r}") from error
    else:
        raise AssertionError(f"malformed curve accepted: {payload.hex()}")


def main():
    identity = curve([])
    gamma = curve([0x0233])
    sampled = curve([0, 0x8000, 0xFFFF])
    for payload in (identity, gamma, sampled):
        accepted(payload)
    rejected(gamma + b"\0\0", "invalid ICC curve body")
    rejected(sampled + b"\0\0", "invalid ICC curve body")
    rejected(gamma[:-1], "invalid ICC curve body")
    rejected(gamma + b"\0", "invalid ICC curve body")
    rejected(gamma + b"\0\1", "invalid ICC curve body")
    rejected(gamma + b"\0" * 4, "invalid ICC curve body")
    rejected(identity + b"\0" * 4, "invalid ICC curve body")
    rejected(b"curv" + b"\0" * 4 + struct.pack(">I", 0xFFFFFFFF), "invalid ICC curve body")
    print("ICC curve controls passed: exact body; alignment padding is external")


if __name__ == "__main__":
    main()
