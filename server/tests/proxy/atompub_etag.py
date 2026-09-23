#!/usr/bin/env python3
"""Exercise real Jaunder conditional Post writes through a real Caddy encoder.

Run locally with --jaunder /absolute/path/to/jaunder --caddy /absolute/path/to/caddy.
This is an opt-in system regression: it does not require an operator checkout or
network access, and all server state, ports, and proxy configurations are temporary.
"""

import argparse
import base64
import json
import os
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class Scenario:
    name: str
    exclude_atompub: bool
    strip_cache_control: bool
    expect_false_412: bool
    encodings: tuple[str, ...]
    run_emacs: bool


SCENARIOS = (
    Scenario(name="before", exclude_atompub=False, strip_cache_control=True,
             expect_false_412=True, encodings=("zstd",), run_emacs=False),
    Scenario(name="server", exclude_atompub=False, strip_cache_control=False,
             expect_false_412=False, encodings=("zstd",), run_emacs=False),
    Scenario(name="operator", exclude_atompub=True, strip_cache_control=True,
             expect_false_412=False, encodings=("zstd",), run_emacs=False),
    Scenario(name="combined", exclude_atompub=True, strip_cache_control=False,
             expect_false_412=False, encodings=("zstd", "identity"), run_emacs=True),
)


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def wait_for(probe, label):
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        try:
            result = probe()
            if result:
                return result
        except (OSError, ValueError, urllib.error.URLError):
            pass
        time.sleep(0.1)
    raise AssertionError(f"timed out waiting for {label}")


def bound_runtime(path):
    runtime = json.loads(path.read_text())
    return runtime if runtime.get("port", 0) > 0 else None


def call(binary, *args):
    result = subprocess.run([str(binary), *args], capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError(f"{binary.name} {args[0]} failed: {result.stderr[-1000:]}")
    return result.stdout.strip()


def request(url, token, method="GET", body=None, etag=None, encoding="zstd"):
    headers = {
        "Authorization": "Basic " + base64.b64encode(f"alice:{token}".encode()).decode(),
        "Accept-Encoding": encoding,
    }
    if body is not None:
        headers["Content-Type"] = "application/atom+xml"
    if etag is not None:
        headers["If-Match"] = etag
    req = urllib.request.Request(url, body, headers, method=method)
    try:
        response = urllib.request.urlopen(req, timeout=5)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        return response.status, response.headers, response.read()


def entry(body):
    return (
        '<?xml version="1.0"?><entry xmlns="http://www.w3.org/2005/Atom">'
        f'<title>Proxy fixture</title><content type="text">{body}</content></entry>'
    ).encode()


def plain_response(result, status, canonical=None, cache=True):
    code, headers, body = result
    assert code == status, (status, code, body[:200])
    assert headers.get("Content-Encoding") is None, headers
    if cache:
        assert "no-transform" in headers.get("Cache-Control", ""), headers
    if canonical is not None:
        assert headers.get("ETag") == canonical, (headers.get("ETag"), canonical)
    return headers, body


def start_proxy(caddy, directory, port, upstream, scenario):
    policy = "encode zstd gzip"
    if scenario.exclude_atompub:
        policy = "@nonAtomPub not path /atompub/*\n encode @nonAtomPub zstd gzip"
    # Removing the header at the upstream boundary simulates the pre-fix
    # server; it proves the path exception independently of no-transform.
    header_down = "header_down -Cache-Control" if scenario.strip_cache_control else ""
    config = directory / "Caddyfile"
    config.write_text(
        f"{{\n admin off\n auto_https off\n}}\nhttp://127.0.0.1:{port} {{\n"
        f" {policy}\n reverse_proxy 127.0.0.1:{upstream} {{\n"
        f"  {header_down}\n }}\n}}\n"
    )
    log = (directory / f"caddy-{scenario.name}.log").open("w+")
    process = subprocess.Popen(
        [str(caddy), "run", "--config", str(config), "--adapter", "caddyfile"],
        stdout=log, stderr=log,
    )
    try:
        wait_for(
            lambda: process.poll() is None and request(f"http://127.0.0.1:{port}/", "unused")[0],
            f"Caddy {scenario.name}",
        )
    except Exception:
        process.terminate()
        process.wait(timeout=5)
        log.seek(0)
        raise AssertionError(f"Caddy {scenario.name} did not start: {log.read()[-1500:]}") from None
    return process, log


def exercise(base, direct, token, scenario, encoding):
    body = "A" * 3000
    created = request(base + "/atompub/alice/posts", token, "POST", entry(body), encoding=encoding)
    assert created[0] == 201, (scenario.name, created[0], created[2][:200])
    location = created[1]["Location"]
    assert location.startswith(base + "/atompub/alice/posts/"), location
    path = location.removeprefix(base)
    current = request(location, token, encoding=encoding)
    canonical = request(direct + path, token, encoding="identity")
    canonical_etag = canonical[1]["ETag"]
    assert canonical[0] == 200 and canonical[1].get("Content-Encoding") is None
    if scenario.expect_false_412:
        assert current[1].get("Content-Encoding") == "zstd", current[1]
        assert current[1]["ETag"].endswith('-zstd"'), current[1]["ETag"]
        assert current[1]["ETag"] != canonical_etag
        failed = request(location, token, "PUT", entry("B" * 3000), current[1]["ETag"])
        assert failed[0] == 412, failed[0]
        assert request(direct + path, token, encoding="identity")[1]["ETag"] == canonical_etag
        print("before: zstd, suffixed ETag, false 412, Post unchanged")
        return

    cache = not scenario.strip_cache_control
    created_headers, created_body = plain_response(created, 201, canonical_etag, cache=cache)
    assert body in created_body.decode("utf-8"), "create response was not identity Atom XML"
    headers, representation = plain_response(current, 200, canonical_etag, cache=cache)
    assert representation == canonical[2], "AtomPub bytes changed in transit"
    if scenario.exclude_atompub and encoding == "zstd":
        outside = request(base + "/", token)
        assert outside[0] == 200 and outside[1].get("Content-Encoding") == "zstd", outside[:2]
    updated = request(location, token, "PUT", entry("B" * 3000), created_headers["ETag"], encoding)
    updated_headers, _ = plain_response(updated, 200, cache=cache)
    new_etag = updated_headers["ETag"]
    assert new_etag != canonical_etag
    stale = request(location, token, "PUT", entry("C" * 3000), canonical_etag, encoding)
    assert stale[0] == 412
    assert request(direct + path, token, encoding="identity")[1]["ETag"] == new_etag
    fetched_update = request(location, token, encoding=encoding)
    plain_response(fetched_update, 200, new_etag, cache=cache)
    deleted = request(location, token, "DELETE", etag=fetched_update[1]["ETag"], encoding=encoding)
    plain_response(deleted, 204, cache=cache)
    print(f"{scenario.name}/{encoding}: identity create/GET, canonical PUT/DELETE, stale 412 unchanged")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jaunder", type=Path, required=True)
    parser.add_argument("--caddy", type=Path, required=True)
    parser.add_argument("--emacs", type=Path, required=True)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="jaunder-atompub-caddy-") as tmp:
        directory = Path(tmp)
        storage = directory / "storage"
        db = "sqlite:" + str(directory / "db.sqlite")
        common = ("--db", db, "--storage-path", str(storage))
        call(args.jaunder, "init", *common)
        call(args.jaunder, "user-create", *common, "--username", "alice", "--password", "password123")
        token = call(
            args.jaunder, "app-password-create", *common, "--username", "alice", "--label", "proxy-test"
        )
        log = (directory / "server.log").open("w+")
        server = subprocess.Popen(
            [str(args.jaunder), "serve", "--bind", "127.0.0.1:0", *common, "--environment", "dev"],
            stdout=log, stderr=log,
        )
        try:
            runtime = wait_for(
                lambda: bound_runtime(storage / "runtime.json"), "Jaunder runtime"
            )
            upstream = runtime["port"]
            direct = f"http://127.0.0.1:{upstream}"
            wait_for(lambda: request(direct + "/", token)[0], "Jaunder readiness")
            port = free_port()
            base = f"http://127.0.0.1:{port}"
            call(args.jaunder, "site-config", "set", *common, "site.base_url", base)
            for scenario in SCENARIOS:
                proxy, proxy_log = start_proxy(args.caddy, directory, port, upstream, scenario)
                try:
                    for encoding in scenario.encodings:
                        exercise(base, direct, token, scenario, encoding)
                    if scenario.run_emacs:
                        local_root = directory / "emacs-posts"
                        local_root.mkdir()
                        elisp = Path(__file__).resolve().parents[3] / "elisp/test/proxy/atompub-local-ahead.el"
                        emacs = subprocess.run(
                            [str(args.emacs), "--batch", "-Q", "-l", str(elisp)],
                            env={**os.environ, "JAUNDER_PROXY_ROOT": str(local_root),
                                 "JAUNDER_PROXY_BASE": base, "JAUNDER_PROXY_TOKEN": token},
                            capture_output=True, text=True,
                        )
                        if emacs.returncode or "Emacs local-ahead: identity validator" not in emacs.stderr:
                            raise AssertionError(f"Emacs proxy proof failed: {emacs.stderr[-2000:]}")
                        print("Emacs local-ahead: create/read markers replayed, push succeeded")
                finally:
                    proxy.terminate()
                    proxy.wait(timeout=5)
                    proxy_log.close()
        finally:
            server.terminate()
            server.wait(timeout=5)
            log.close()


if __name__ == "__main__":
    main()
