#!/usr/bin/env python3
"""TEST-SIDE ONLY: add consumption/census observations to pinned libde265.

No recovery behavior is changed and no resulting decoder is used by the
syntax-only validator. Exact-match edits fail if upstream source/pin drifts.
The decoder reconstructs pixels only as an independent test-side consumer.
"""
import sys
from pathlib import Path


def replace(path, old, new):
    data = path.read_text()
    if data.count(old) != 1:
        raise RuntimeError("pinned instrumentation seam mismatch: " + str(path))
    path.write_text(data.replace(old, new))


def main():
    root = Path(sys.argv[1])
    replace(root / "libde265/cabac.h", "  uint8_t* bitstream_end = nullptr;",
            "  uint8_t* bitstream_end = nullptr;\n"
            "  // Test observation: byte prefetch is NOT semantic consumption.\n"
            "  int syntax_bits() const { return (bitstream_curr-bitstream_start)*8 + bits_needed + 1; }\n")
    replace(root / "libde265/slice.cc",
            "    int end_of_slice_segment_flag = tctx->cabac_decoder.decode_term_bit();",
            "    int end_of_slice_segment_flag = tctx->cabac_decoder.decode_term_bit();\n"
            "    printf(\"HEVC_CTU %d %d %d %td\\n\", tctx->CtbAddrInRS, end_of_slice_segment_flag, "
            "tctx->cabac_decoder.syntax_bits(), tctx->cabac_decoder.bitstream_curr-tctx->cabac_decoder.bitstream_start);\n")
    replace(root / "libde265/slice.cc",
            "  // ----- decode coefficients -----\n\n  tctx->nCoeff[cIdx] = 0;\n\n\n"
            "  // i - subblock index",
            "  // ----- decode coefficients -----\n\n  tctx->nCoeff[cIdx] = 0;\n"
            "  printf(\"HEVC_RESIDUAL %d %d %d %d\\n\", x0, y0, log2TrafoSize, cIdx);\n\n\n"
            "  // i - subblock index")
    replace(root / "libde265/slice.cc",
            "        yC = (S.y << 2) + ScanOrderPos[p].y;\n\n"
            "        tctx->coeffList[cIdx][tctx->nCoeff[cIdx]] = Clip3(-32768, 32767, currCoeff);",
            "        yC = (S.y << 2) + ScanOrderPos[p].y;\n"
            "        printf(\"HEVC_COEFF %d %d %d %d %d %d %d\\n\", cIdx, x0, y0, log2TrafoSize, xC, yC, currCoeff);\n\n"
            "        tctx->coeffList[cIdx][tctx->nCoeff[cIdx]] = Clip3(-32768, 32767, currCoeff);")
    replace(root / "libde265/slice.cc",
            "      // CuQpDeltaVal shall be in [-(26 + QpBdOffsetY/2), 25 + QpBdOffsetY/2] (Sec. 7.4.9.10)",
            "      printf(\"HEVC_QP %d %d %d %d %d\\n\", x0, y0, cu_qp_delta_abs, cu_qp_delta_sign, "
            "cu_qp_delta_abs*(1-2*cu_qp_delta_sign));\n"
            "      // CuQpDeltaVal shall be in [-(26 + QpBdOffsetY/2), 25 + QpBdOffsetY/2] (Sec. 7.4.9.10)")
    replace(root / "libde265/slice.cc",
            "  int nCbS = 1 << log2CbSize; // number of coding block samples",
            "  int nCbS = 1 << log2CbSize; // number of coding block samples\n"
            "  printf(\"HEVC_CU %d %d %d\\n\", x0, y0, log2CbSize);\n")


if __name__ == "__main__":
    main()
