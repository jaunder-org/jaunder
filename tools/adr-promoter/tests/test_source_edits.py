"""Byte-level promotion link contract, using fake Markdown only."""

import pytest

from adr_promoter.promotion import rewrite_relative_links


def test_first_link_tracer():
    assert rewrite_relative_links(b"before [x](../drafts/b.md) after\n") == (
        b"before [x](drafts/b.md) after\n"
    )


@pytest.mark.parametrize(
    ("source", "expected"),
    [
        pytest.param(
            b"| A | [x](../a.md) | ![y](../b.png) |\n| --- | --- | --- |\n",
            b"| A | [x](a.md) | ![y](b.png) |\n| --- | --- | --- |\n",
            id="gfm-table-links-and-images",
        ),
        pytest.param(
            b'[x](../a.md "keep title") ![y](<../b b.png> \'image title\')\n',
            b'[x](a.md "keep title") ![y](<../b b.png> \'image title\')\n',
            id="titles-and-angle-destinations",
        ),
        pytest.param(
            "é 😀 [λ](../日本.md) ![é](../猫.png) fin\n".encode(),
            "é 😀 [λ](日本.md) ![é](猫.png) fin\n".encode(),
            id="unicode-labels-and-destinations",
        ),
        pytest.param(
            b"[x][id] [id] ![id]\n\n[id]: ../keep.md \"title\"\n[x](../edit.md)\n",
            b"[x][id] [id] ![id]\n\n[id]: ../keep.md \"title\"\n[x](edit.md)\n",
            id="reference-definitions-unchanged",
        ),
        pytest.param(
            b"```md\n[x](../keep.md)\n```\n~~~\n![x](../keep.png)\n~~~\n`[x](../keep.md)` [x](../edit.md)\n",
            b"```md\n[x](../keep.md)\n```\n~~~\n![x](../keep.png)\n~~~\n`[x](../keep.md)` [x](edit.md)\n",
            id="fenced-and-single-line-code",
        ),
        pytest.param(
            b"prose ](../orphan.md) [x](../edit.md)",
            b"prose ](../orphan.md) [x](edit.md)",
            id="approved-orphan-pseudo-link-exclusion",
        ),
        pytest.param(
            b"    [x](../indented.md)\n\n[x](../edit.md)\n",
            b"    [x](../indented.md)\n\n[x](edit.md)\n",
            id="approved-indented-code-exclusion",
        ),
        pytest.param(
            b"`code\n[x](../multiline.md)` [x](../edit.md)\n",
            b"`code\n[x](../multiline.md)` [x](edit.md)\n",
            id="approved-multiline-code-exclusion",
        ),
        pytest.param(
            b"[a](..) [b](../) [c](plain.md) [d](a/../b.md) [e](#../a) [f](https://x/../a) [g](http://x/../a) [h](mailto:x) [i](../../a.md)",
            b"[a](..) [b](../) [c](plain.md) [d](a/../b.md) [e](#../a) [f](https://x/../a) [g](http://x/../a) [h](mailto:x) [i](../a.md)",
            id="one-leading-parent-only",
        ),
        pytest.param(
            b"> - [**nested**](../a.md)\n>   ![image](../b.png)\n",
            b"> - [**nested**](a.md)\n>   ![image](b.png)\n",
            id="nested-block-source-positions",
        ),
        pytest.param(
            b"[x](../a\\(b\\).md) [x](../a&amp;b.md)",
            b"[x](a\\(b\\).md) [x](a&amp;b.md)",
            id="source-escapes-not-reserialized",
        ),
    ],
)
def test_source_span_fixtures(source, expected):
    assert rewrite_relative_links(source) == expected


@pytest.mark.parametrize("separator", [b"\r\n", b"\r", b"\f", b"\x00"])
def test_normalized_characters_preserve_original_bytes(separator):
    source = b"\xc3\xa9" + separator + b"[x](../a.md)" + separator + b"![y](../b.png)\n"
    expected = b"\xc3\xa9" + separator + b"[x](a.md)" + separator + b"![y](b.png)\n"
    assert rewrite_relative_links(source) == expected


def test_nul_inside_destination_remains_original_byte():
    assert rewrite_relative_links(b"[x](../a\x00b.md)") == b"[x](a\x00b.md)"
