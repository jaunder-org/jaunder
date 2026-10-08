#!/usr/bin/env python3
"""Real syntax/ownership validator for the bounded still-IDR proof lane.

CLI input is a private test adapter JSON containing four NAL hex strings,
not image content detection. Container rewriting calls validate_nals directly.
Only typed HEVC-domain failures map to exit2; tooling/I/O/import/programming
failures propagate. No result file is created until complete ownership passes.
"""
import json
import sys
from pathlib import Path
from hevc_headers import HEVCDomainError, require, vps, sps, pps, slice_header
from hevc_intra_syntax import IntraSyntax


def validate_nals(video_nal, sequence_nal, picture_nal, coded_nal):
    video = vps(video_nal)
    sequence = sps(sequence_nal, video)
    picture = pps(picture_nal, sequence)
    header = slice_header(coded_nal, sequence, picture)
    return {"vps_id": video["id"], "sps_id": sequence["id"], "pps_id": picture["id"],
            "cabac_start": header["cabac_start"], "qp": header["qp"],
            **IntraSyntax(sequence, picture, header).validate()}


def main():
    source, destination = Path(sys.argv[1]), Path(sys.argv[2])
    require(source.resolve() != destination.resolve(), "input/output alias")
    require(not destination.exists(), "output already exists")
    with source.open("rb") as stream:
        data = stream.read(32 * 1024 * 1024 + 1)
    require(len(data) <= 32 * 1024 * 1024, "adapter file budget")
    record = json.loads(data)
    require(set(record) == {"vps", "sps", "pps", "coded"}, "adapter keys")
    nals = [bytes.fromhex(record[name]) for name in ("vps", "sps", "pps", "coded")]
    result = validate_nals(*nals)
    destination.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    try:
        main()
    except HEVCDomainError as error:
        print("HEVC_DOMAIN: " + str(error), file=sys.stderr)
        sys.exit(2)
