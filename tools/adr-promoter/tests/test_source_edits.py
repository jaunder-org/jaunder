"""Our destination byte patches and normalized-character-to-original-byte mapping."""

from adr_promoter.promotion import rewrite_relative_links


def test_byte_patch_preserves_unicode_and_destination_spelling():
    source = "é 😀 [λ](../a\\(b\\).md) ![é](<../猫.png> 'title') fin\r\n".encode()
    expected = "é 😀 [λ](a\\(b\\).md) ![é](<猫.png> 'title') fin\r\n".encode()
    assert rewrite_relative_links(source) == expected
    assert rewrite_relative_links(b"[x](../a\\(b\\).md) [x](../a&amp;b.md)") == (
        b"[x](a\\(b\\).md) [x](a&amp;b.md)"
    )


def test_one_leading_parent_only():
    source = b"[a](..) [b](../) [c](plain.md) [d](a/../b.md) [e](#../a) [f](https://x/../a) [g](http://x/../a) [h](mailto:x) [i](../../a.md)"
    expected = b"[a](..) [b](../) [c](plain.md) [d](a/../b.md) [e](#../a) [f](https://x/../a) [g](http://x/../a) [h](mailto:x) [i](../a.md)"
    assert rewrite_relative_links(source) == expected


def test_original_bytes_survive_offset_conversion():
    # Test our offset conversion and patch output, not Marko's normalized AST.
    source = b"\xc3\xa9\r\n[x](../a.md)\r![y](../b.png)\f[z](../c.md)\x00\n"
    expected = b"\xc3\xa9\r\n[x](a.md)\r![y](b.png)\f[z](c.md)\x00\n"
    assert rewrite_relative_links(source) == expected


def test_nul_inside_destination_remains_original_byte():
    assert rewrite_relative_links(b"[x](../a\x00b.md)") == b"[x](a\x00b.md)"
