"""`dev static <name>`: one pinned check, or a fixed group, on the host or in a sandbox."""

import pytest
from conftest import Workflow

CLIPPY = {
    "product-core-clippy": "clippy --workspace --exclude web --exclude client "
    "--exclude csr --all-targets -- -D warnings",
    "web-server-clippy": "clippy -p web --features server --all-targets -- -D warnings",
    "web-no-server-clippy": "clippy -p web --no-default-features --all-targets "
    "-- -D warnings",
    "client-host-clippy": "clippy -p client --features csr --all-targets -- -D warnings",
    "wasm-clippy": "clippy -p web -p client -p csr --features csr "
    "--target wasm32-unknown-unknown -- -D warnings",
    "tools-clippy": "clippy --manifest-path tools/Cargo.toml --workspace "
    "--all-targets --features coverage/host -- -D warnings",
}
# Each Cargo leaf's sandbox argv after `cargo --offline`, and its home.
SANDBOX_CARGO = {
    "fmt": ("fmt --check", "product"),
    "tools-fmt": ("fmt --manifest-path tools/Cargo.toml --all --check", "tools"),
    "cargo-deny": ("deny check bans licenses sources", "product"),
    **{
        name: (line, "tools" if name.startswith("tools-") else "product")
        for name, line in CLIPPY.items()
    },
}
HOMES = {
    "product": "JAUNDER_PRODUCT_CARGO_HOME",
    "tools": "JAUNDER_TOOLS_CARGO_HOME",
}


def argv(line: str) -> list[str]:
    return line.split(" ")


@pytest.mark.parametrize(("leaf", "expected"), SANDBOX_CARGO.items())
def test_sandbox_cargo_leaves_run_offline_against_their_admitted_home(
    workflow: Workflow, leaf: str, expected: tuple[str, str]
) -> None:
    line, home = expected
    other = "tools" if home == "product" else "product"
    unused = {HOMES[other]: "unused"}
    sub = workflow.root / "sub"

    refused = workflow.dev(
        "static", leaf, "--sandbox", cwd=sub, **unused, **{HOMES[home]: ""}
    )
    assert refused.returncode == 1
    assert b"sandbox Cargo home is missing" in refused.stderr
    assert workflow.commands() == []
    assert not (sub / ".xtask").exists()

    admitted = workflow.dev(
        "static",
        leaf,
        "--sandbox",
        cwd=sub,
        **unused,
        **{HOMES[home]: "home"},
        RUSTC_WRAPPER="caller",
        CARGO_INCREMENTAL="caller",
        KACHE_CACHE_EXECUTABLES="caller",
    )
    assert admitted.returncode == 0, admitted
    assert workflow.commands() == [["cargo", "--offline", *argv(line)]]
    assert (sub / "cargo-env").read_text() == "home\ntrue\n"
    # The sandbox has no compile cache: the caller's settings reach Cargo as given.
    assert (sub / "cache-env").read_text() == "caller\ncaller\ncaller\n"

    failing = workflow.dev(
        "static",
        leaf,
        "--sandbox",
        cwd=sub,
        failure=f"cargo --offline {line}",
        **unused,
        **{HOMES[home]: "home"},
    )
    assert failing.returncode == 17


def test_host_fmt_keeps_the_callers_cargo_environment(workflow: Workflow) -> None:
    result = workflow.dev(
        "static",
        "fmt",
        cwd=workflow.root / "sub",
        CARGO_NET_OFFLINE="host-policy",
        JAUNDER_PRODUCT_CARGO_HOME="sandbox-home",
    )
    assert result.returncode == 0, result
    assert workflow.commands() == [["cargo", "fmt", "--check"]]
    assert (workflow.root / "cargo-env").read_text() == "unset\nhost-policy\n"
    assert (workflow.root / "cache-env").read_text() == "kache\n0\n0\n"


def test_sandbox_fix_drops_the_check_flag(workflow: Workflow) -> None:
    result = workflow.dev(
        "static",
        "fmt",
        "--fix",
        "--sandbox",
        JAUNDER_PRODUCT_CARGO_HOME="sandbox-home",
    )
    assert result.returncode == 0, result
    assert workflow.commands() == [["cargo", "--offline", "fmt"]]


# The sandbox prepares nothing: its derivation already has.
GROUPS = {
    "rust-format": [
        "cargo --offline fmt",
        "scoped-leptosfmt fix",
        "cargo --offline fmt --manifest-path tools/Cargo.toml --all",
    ],
}


@pytest.mark.parametrize(("group", "expected"), GROUPS.items())
@pytest.mark.parametrize(
    ("product", "tools"), [("", "tools-home"), ("product-home", ""), (None, None)]
)
def test_sandbox_groups_admit_every_home_before_work_and_keep_their_order(
    workflow: Workflow,
    group: str,
    expected: list[str],
    product: str | None,
    tools: str | None,
) -> None:
    result = workflow.dev(
        "static",
        group,
        "--fix",
        "--sandbox",
        JAUNDER_PRODUCT_CARGO_HOME="product-home" if product is None else product,
        JAUNDER_TOOLS_CARGO_HOME="tools-home" if tools is None else tools,
    )
    if product is None:
        assert result.returncode == 0, result
        assert workflow.commands() == [argv(line) for line in expected]
    else:
        assert result.returncode == 1
        assert workflow.commands() == []


LEAVES = {
    "elisp-fmt": (
        "emacs --batch -Q -l elisp/scripts/format.el -f jaunder-fmt-check",
        "emacs --batch -Q -l elisp/scripts/format.el -f jaunder-fmt-fix",
    ),
    "byte-compile": ("emacs --batch -Q -l elisp/scripts/byte-compile.el",) * 2,
    "ert": ("emacs --batch -Q -l elisp/scripts/run-tests.el",) * 2,
    "ast-grep-tests": ("ast-grep test --config sgconfig.yml",) * 2,
    "no-full-reload": (
        (
            "ast-grep scan --config sgconfig.yml --filter ^no-full-reload$ "
            "web/src client/src"
        ),
    )
    * 2,
    "leptosfmt": ("scoped-leptosfmt verify", "scoped-leptosfmt fix"),
    "tsc": ("tsc --noEmit -p end2end/tsconfig.json",) * 2,
    "leptosfmt-cli": (
        "leptosfmt -x .direnv -x .git -x target --check web/src csr/src",
        "leptosfmt -x .direnv -x .git -x target web/src csr/src",
    ),
    "cargo-deny": ("cargo deny check",) * 2,
    "fmt": ("cargo fmt --check", "cargo fmt"),
    "tools-fmt": (
        "cargo fmt --manifest-path tools/Cargo.toml --all --check",
        "cargo fmt --manifest-path tools/Cargo.toml --all",
    ),
    **{name: (f"cargo {line}",) * 2 for name, line in CLIPPY.items()},
    # The globs reach Prettier unexpanded, though matching files exist.
    "prettier-markdown": ("prettier --check **/*.md", "prettier -w **/*.md"),
    "prettier-end2end": ("prettier --check end2end", "prettier -w end2end"),
    "prettier-css": ("prettier --check **/*.css", "prettier -w **/*.css"),
}


# What the host runs before a leaf, from this checkout.
PREPARATION = {
    "tsc": ["setup-e2e-node-modules  "],
    # The scoped adapter's package build: its evaluation, then the build.
    "leptosfmt": [
        "nix path-info --derivation --accept-flake-config .#repository-policy",
        (
            "nix build -L --accept-flake-config --print-out-paths --no-link "
            ".#repository-policy"
        ),
    ],
}


@pytest.mark.parametrize(("leaf", "lines"), LEAVES.items())
@pytest.mark.parametrize("fix", [False, True])
def test_host_leaves_prepare_then_run_their_check_from_the_checkout_root(
    workflow: Workflow, leaf: str, lines: tuple[str, str], fix: bool
) -> None:
    (workflow.root / "nested").mkdir()
    (workflow.root / "nested" / "guide.md").write_text("# Guide\n")
    (workflow.root / "nested" / "style.css").write_text("a {}\n")
    line = lines[fix]
    args = ["--fix"] if fix else []

    result = workflow.dev("static", leaf, *args, cwd=workflow.root / "sub")
    assert result.returncode == 0, result
    assert workflow.commands() == [
        argv(prepared) for prepared in [*PREPARATION.get(leaf, []), line]
    ]
    assert not (workflow.root / "sub" / "commands").exists()

    failed = workflow.dev("static", leaf, *args, failure=line)
    assert failed.returncode == 17
    assert failed.stderr.endswith(f"{line.split()[0]} failed\n".encode())


def test_a_failed_preparation_stops_before_its_check(workflow: Workflow) -> None:
    evaluate, build = PREPARATION["leptosfmt"]
    result = workflow.dev("static", "rust-format", failure=build)
    assert result.returncode == 17
    assert workflow.commands() == [
        argv("cargo fmt --check"),
        argv(evaluate),
        argv(build),
    ]


def test_a_leaf_keeps_its_streams(workflow: Workflow) -> None:
    workflow.checkout.tool(
        "emacs",
        'print("workload stdout"); print("workload stderr", file=sys.stderr)\n'
        "sys.exit(23)",
    )
    result = workflow.dev("static", "ert")
    assert result.returncode == 23
    assert result.stdout == b"workload stdout\n"
    assert result.stderr == b"workload stderr\n"
    assert (workflow.root / "order").read_text() == (
        "emacs --batch -Q -l elisp/scripts/run-tests.el\n"
    )


@pytest.mark.parametrize(
    "args",
    [
        ["static", "bogus"],
        ["static", "all"],
        ["static", "fmt", "verify"],
        ["static", "fmt", "--check"],
    ],
)
def test_unknown_static_arguments_are_refused_before_any_work(
    workflow: Workflow, args: list[str]
) -> None:
    result = workflow.dev(*args)
    assert result.returncode == 2
    assert workflow.commands() == []
    assert workflow.checkout.runs() == []
