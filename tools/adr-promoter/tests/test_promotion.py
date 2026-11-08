"""Complete mutation proof in disposable tracked repositories; never the checkout."""

import os
import subprocess
import pytest

from adr_promoter.promotion import promote, sync_readme


GIT_REDIRECTS = (
    "GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_OBJECT_DIRECTORY",
    "GIT_COMMON_DIR", "GIT_NAMESPACE",
)


def git(root, *args):
    environment = os.environ.copy()
    for name in GIT_REDIRECTS:
        environment.pop(name, None)
    return subprocess.run(
        ["git", "-C", str(root), *args], env=environment,
        capture_output=True, check=True,
    ).stdout


def write(root, path, body):
    target = root / path
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(body)


def tracked_repo(root, files):
    git(root, "init", "-q")
    for path, body in files.items():
        write(root, path, body)
    git(root, "add", ".")
    git(root, "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
        "-c", "core.hooksPath=/dev/null", "commit", "-qm", "fixture")


BEGIN = b"<!-- adr-table:begin -->"
END = b"<!-- adr-table:end -->"
OLD = b"# ADR-0001: Old heading\n\n- Status: accepted\n"
README = (b"# Records\r\n" + BEGIN + b"\r\n\r\n"
          b"| [0001](adr/wrong.md) | Curated old | rejected |\r\n" + END + b"\r\nFooter\r\n")
ALPHA = ("# ADR-DRAFT: Alpha é\r\n\r\n- Status: proposed\r\n\r\n"
         "See [beta](../drafts/beta.md) and ![old](<../0001-old.md> \"title\").\r\n"
         "Literal ADR-DRAFT proposed; docs/adr/drafts/beta.md#part\r\n"
         "\r\n    [code](../code.md)\r\n\r\n"
         "`multi\r\n[code](../code.md)`\r\nprose ](../code.md)\r\n").encode()
ALPHA_EXPECTED = ("# ADR-0002: Alpha é\r\n\r\n- Status: accepted\r\n\r\n"
                  "See [beta](0003-beta.md) and ![old](<0001-old.md> \"title\").\r\n"
                  "Literal ADR-DRAFT proposed; docs/adr/0003-beta.md#part\r\n"
                  "\r\n    [code](../code.md)\r\n\r\n"
                  "`multi\r\n[code](../code.md)`\r\nprose ](../code.md)\r\n").encode()
BETA = b"# ADR-DRAFT: Beta\n\n- Status: proposed\n\n| Link |\n| --- |\n| [alpha](../drafts/alpha.md) |\n"
BETA_EXPECTED = b"# ADR-0003: Beta\n\n- Status: accepted\n\n| Link |\n| --- |\n| [alpha](0002-alpha.md) |\n"
README_EXPECTED = (b"# Records\r\n" + BEGIN + b"\n\n"
                   b"| #   | Title | Status |\n| --- | ----- | ------ |\n"
                   b"| [0001](adr/0001-old.md) | Curated old | accepted |\n"
                   b"| [0002](adr/0002-alpha.md) | Alpha \xc3\xa9 | accepted |\n"
                   b"| [0003](adr/0003-beta.md) | Beta | accepted |\n\n" + END + b"\r\nFooter\r\n")


def promotion_fixture(root):
    # The old read-only Rust gate recognizes the three excluded pseudo/code links;
    # a tracked target lets the fixture prove parser eligibility without gate noise.
    files = {
        "docs/adr/0001-old.md": OLD,
        "docs/adr/drafts/beta.md": BETA,
        "docs/adr/drafts/alpha.md": ALPHA,
        "docs/README.md": README,
        "docs/ARCHITECTURE.md": b"adr/0001-old.md adr/drafts/alpha.md adr/drafts/beta.md\n",
        "citation.txt": b"docs/adr/drafts/alpha.md drafts/beta docs/adr/drafts/alpha.md\r\n",
        "citation.bin": b"\xff\x00drafts/beta\x00",
        "docs/code.md": b"sample\n",
        "docs/adr/drafts/README.md": b"Author guide\n",
    }
    tracked_repo(root, files)
    return files


def assert_complete_promotion(tmp_path, before, summary):
    expected = {**before}
    del expected["docs/adr/drafts/alpha.md"]
    del expected["docs/adr/drafts/beta.md"]
    expected.update({
        "docs/adr/0002-alpha.md": ALPHA_EXPECTED,
        "docs/adr/0003-beta.md": BETA_EXPECTED,
        "docs/README.md": README_EXPECTED,
        "docs/ARCHITECTURE.md": b"adr/0001-old.md adr/0002-alpha.md adr/0003-beta.md\n",
        "citation.txt": b"docs/adr/0002-alpha.md 0003-beta docs/adr/0002-alpha.md\r\n",
        "citation.bin": b"\xff\x000003-beta\x00",
    })
    paths = git(tmp_path, "ls-files", "-z").decode().strip("\x00").split("\x00")
    assert set(paths) == set(expected)
    assert {path: (tmp_path / path).read_bytes() for path in paths} == expected
    assert not (tmp_path / "docs/adr/drafts/alpha.md").exists()
    assert not (tmp_path / "docs/adr/drafts/beta.md").exists()
    assert git(tmp_path, "diff", "--name-only") == b""
    assert set(git(tmp_path, "diff", "--cached", "--name-only", "--no-renames").decode().splitlines()) == {
        "citation.bin", "citation.txt", "docs/ARCHITECTURE.md", "docs/README.md",
        "docs/adr/drafts/alpha.md", "docs/adr/drafts/beta.md",
        "docs/adr/0002-alpha.md", "docs/adr/0003-beta.md",
    }
    assert summary == (
        "docs/adr/drafts/alpha.md -> docs/adr/0002-alpha.md (status: proposed -> accepted); "
        "docs/adr/drafts/beta.md -> docs/adr/0003-beta.md (status: proposed -> accepted)"
        " — README table synced (3 rows (2 added, 0 removed)); staged"
    )
    assert sync_readme(tmp_path) == "3 rows, already in sync"
    assert promote(tmp_path) == "no ADR drafts to promote"


def test_complete_two_draft_promotion(tmp_path):
    before = promotion_fixture(tmp_path)
    summary = promote(tmp_path)
    assert_complete_promotion(tmp_path, before, summary)


@pytest.mark.parametrize("bad", [b"# Wrong: Title\n", b"# ADR-DRAFT:   \r\n"])
def test_all_headings_admitted_before_any_move(tmp_path, bad):
    files = promotion_fixture(tmp_path)
    write(tmp_path, "docs/adr/drafts/beta.md", bad)
    before_index = git(tmp_path, "write-tree")
    with pytest.raises(ValueError, match="non-empty title"):
        promote(tmp_path)
    assert git(tmp_path, "write-tree") == before_index
    assert (tmp_path / "docs/adr/drafts/alpha.md").read_bytes() == files["docs/adr/drafts/alpha.md"]
    assert (tmp_path / "docs/adr/drafts/beta.md").read_bytes() == bad
    assert not (tmp_path / "docs/adr/0002-alpha.md").exists()


def test_whole_set_tracked_admission_has_no_partial_mutation(tmp_path):
    promotion_fixture(tmp_path)
    write(tmp_path, "docs/adr/drafts/zeta.md", b"# ADR-DRAFT: Zeta\n")
    before_index = git(tmp_path, "write-tree")
    before = {path: path.read_bytes() for path in (tmp_path / "docs").rglob("*") if path.is_file()}
    with pytest.raises(ValueError, match="zeta.md must be tracked"):
        promote(tmp_path)
    assert git(tmp_path, "write-tree") == before_index
    assert {path: path.read_bytes() for path in (tmp_path / "docs").rglob("*") if path.is_file()} == before


@pytest.mark.parametrize(("status", "expected"), [
    (b"  Status: \tproposed \r\n", b"  Status: \taccepted \r\n"),
    (b"- Status: rejected\n", b"- Status: rejected\n"),
    (b"- Status: proposed (pending)\n", b"- Status: proposed (pending)\n"),
    (b"- Status:\nStatus: proposed\n", b"- Status:\nStatus: proposed\n"),
    (b"proposed prose\n", b"proposed prose\n"),
    (b"- Status: proposed", b"- Status: accepted"),
    (b"- Status: proposed\rStatus: proposed\n", b"- Status: proposed\rStatus: proposed\n"),
])
def test_heading_status_byte_contract(tmp_path, status, expected):
    tracked_repo(tmp_path, {"docs/adr/drafts/one.md": b"# ADR-DRAFT: Title\r\n" + status})
    promote(tmp_path)
    assert (tmp_path / "docs/adr/0000-one.md").read_bytes() == b"# ADR-0000: Title\r\n" + expected


def test_missing_links_warn_after_mutation_and_staging(tmp_path):
    tracked_repo(tmp_path, {"docs/adr/drafts/one.md": b"# ADR-DRAFT: Title\n\n[x](../gone.md#anchor) [dir](../missing/) [escape](../../../oops)\n"})
    summary = promote(tmp_path)
    assert summary.endswith("; warning: unresolved link(s) — docs/adr/0000-one.md: gone.md#anchor, missing/, ../../oops")
    assert (tmp_path / "docs/adr/0000-one.md").read_bytes() == b"# ADR-0000: Title\n\n[x](gone.md#anchor) [dir](missing/) [escape](../../oops)\n"
    assert git(tmp_path, "diff", "--name-only") == b""


def test_standalone_sync_preserves_titles_and_padded_semantic_noop(tmp_path):
    tracked_repo(tmp_path, {"docs/adr/0001-old.md": OLD, "docs/README.md": README})
    assert sync_readme(tmp_path) == "1 rows (0 added, 0 removed)"
    expected = (b"# Records\r\n" + BEGIN + b"\n\n| #   | Title | Status |\n| --- | ----- | ------ |\n"
                b"| [0001](adr/0001-old.md) | Curated old | accepted |\n\n" + END + b"\r\nFooter\r\n")
    assert (tmp_path / "docs/README.md").read_bytes() == expected
    padded = expected.replace(b"| Curated old |", b"|   Curated old    |")
    write(tmp_path, "docs/README.md", padded)
    stat = (tmp_path / "docs/README.md").stat()
    assert sync_readme(tmp_path) == "1 rows, already in sync"
    assert (tmp_path / "docs/README.md").read_bytes() == padded
    assert (tmp_path / "docs/README.md").stat().st_mtime_ns == stat.st_mtime_ns
    assert git(tmp_path, "diff", "--cached", "--name-only") == b""


@pytest.mark.parametrize("readme", [None, b"markerless\r\n", BEGIN, END])
def test_promotion_skips_missing_or_markerless_readme_but_sync_errors(tmp_path, readme):
    files = {"docs/adr/drafts/one.md": b"# ADR-DRAFT: One\n"}
    if readme is not None:
        files["docs/README.md"] = readme
    tracked_repo(tmp_path, files)
    assert "README table not synced (no adr-table markers)" in promote(tmp_path)
    with pytest.raises((OSError, ValueError)):
        sync_readme(tmp_path)
    if readme is not None:
        assert (tmp_path / "docs/README.md").read_bytes() == readme


def test_sync_rejects_out_of_order_markers_without_write(tmp_path):
    body = END + b"\n" + BEGIN
    tracked_repo(tmp_path, {"docs/adr/0001-old.md": OLD, "docs/README.md": body})
    with pytest.raises(ValueError, match="out of order"):
        sync_readme(tmp_path)
    assert (tmp_path / "docs/README.md").read_bytes() == body


def test_number_allocation_uses_regular_files_and_never_reuses_gaps(tmp_path):
    tracked_repo(tmp_path, {
        "docs/adr/0001-old.md": OLD,
        "docs/adr/0012-reserved.txt": b"reserved\n",
        "docs/adr/drafts/z.md": b"# ADR-DRAFT: Z\n",
        "docs/adr/drafts/a.md": b"# ADR-DRAFT: A\n",
        "docs/adr/drafts/README.md": b"Author guide\n",
        "docs/adr/drafts/ignore.txt": b"not a draft\n",
        "docs/adr/drafts/nested/x.md": b"not an immediate draft\n",
    })
    (tmp_path / "docs/adr/9999-symlink.md").symlink_to("0001-old.md")
    write(tmp_path, "untracked-citation.txt", b"drafts/a\n")
    promote(tmp_path)
    assert (tmp_path / "docs/adr/0013-a.md").read_bytes() == b"# ADR-0013: A\n"
    assert (tmp_path / "docs/adr/0014-z.md").read_bytes() == b"# ADR-0014: Z\n"
    assert (tmp_path / "untracked-citation.txt").read_bytes() == b"drafts/a\n"
    assert (tmp_path / "docs/adr/drafts/nested/x.md").read_bytes() == b"not an immediate draft\n"


def test_sync_removes_orphans_sorts_and_seeds_legacy_heading(tmp_path):
    tracked_repo(tmp_path, {
        "docs/adr/0003-three.md": b"# 0003. Three heading\n\nStatus: deprecated extra\n",
        "docs/adr/0001-old.md": OLD,
        "docs/README.md": BEGIN + b"\n| [0009](adr/gone.md) | Gone | accepted |\n"
        b"| [0001](adr/0001-old.md) | Curated | accepted |\n" + END,
    })
    assert sync_readme(tmp_path) == "2 rows (1 added, 1 removed)"
    assert (tmp_path / "docs/README.md").read_bytes() == (
        BEGIN + b"\n\n| #   | Title | Status |\n| --- | ----- | ------ |\n"
        b"| [0001](adr/0001-old.md) | Curated | accepted |\n"
        b"| [0003](adr/0003-three.md) | Three heading | deprecated |\n\n" + END
    )


def test_git_move_failure_is_not_reported_as_success(tmp_path, monkeypatch):
    tracked_repo(tmp_path, {"docs/adr/drafts/one.md": b"# ADR-DRAFT: One\n"})
    original = subprocess.run

    def failing_move(arguments, **kwargs):
        if arguments[3] == "mv":
            return subprocess.CompletedProcess(arguments, 37, b"", b"fixture move refused")
        return original(arguments, **kwargs)

    monkeypatch.setattr(subprocess, "run", failing_move)
    with pytest.raises(RuntimeError, match="fixture move refused"):
        promote(tmp_path)
    assert (tmp_path / "docs/adr/drafts/one.md").read_bytes() == b"# ADR-DRAFT: One\n"


def test_final_inspection_error_is_advisory_after_staging(tmp_path, monkeypatch):
    tracked_repo(tmp_path, {"docs/adr/drafts/one.md": b"# ADR-DRAFT: One\n"})
    original = subprocess.run
    queries = 0

    def failing_final_inspection(arguments, **kwargs):
        nonlocal queries
        if arguments[3] == "ls-files":
            queries += 1
            if queries == 2:
                return subprocess.CompletedProcess(arguments, 1, b"", b"fixture inspection refused")
        return original(arguments, **kwargs)

    monkeypatch.setattr(subprocess, "run", failing_final_inspection)
    summary = promote(tmp_path)
    assert "; warning: unresolved link(s) — docs/adr/0000-one.md: unreadable (" in summary
    assert "fixture inspection refused" in summary
    assert git(tmp_path, "diff", "--name-only") == b""
