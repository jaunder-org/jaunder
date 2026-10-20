"""Behavioral proof for the driver's complete host-backed capture transport."""

import hashlib
import io
import os
from pathlib import Path
import tarfile
import tempfile
import unittest

from e2e_capture import archive_capture, verify_capture_archive


class CaptureTransportTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.backing = self.root / "vm-state-machine" / "xchg"
        self.backing.mkdir(parents=True)
        self.payload = b"abc"
        (self.backing / "otel-traces.jsonl").write_bytes(self.payload)
        # SHA256('abc') is an independent known vector, not the archiver's output.
        self.hashes = {
            "capture/otel-traces.jsonl":
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        }
        self.sizes = {"capture/otel-traces.jsonl": 3}
        self.output = self.root / "capture-test.tar.gz"

    def archive(self):
        archive_capture(self.backing, self.output)
        return verify_capture_archive(self.output, self.hashes, self.sizes)

    def test_complete_payload_excludes_only_mapped_file_backend_metadata(self):
        root_metadata = self.backing / ".virtfs_metadata_root"
        ownership = b"virtfs.uid=1000\nvirtfs.gid=100\nvirtfs.mode=16832\n"
        root_metadata.write_bytes(ownership)
        metadata = self.backing / ".virtfs_metadata"
        metadata.mkdir()
        (metadata / "otel-traces.jsonl").write_bytes(b"guest ownership metadata")
        nested = self.backing / "nested"
        nested.mkdir()
        (nested / ".virtfs_metadata").mkdir()
        (nested / ".virtfs_metadata" / "extra.jsonl").write_bytes(b"metadata")
        (nested / "extra.jsonl").write_bytes(b"complete additional stream")
        self.hashes["capture/nested/extra.jsonl"] = hashlib.sha256(
            b"complete additional stream"
        ).hexdigest()
        self.sizes["capture/nested/extra.jsonl"] = 26
        actual_hashes, actual_sizes = self.archive()
        self.assertEqual(actual_hashes, self.hashes)
        self.assertEqual(actual_sizes, self.sizes)
        self.assertEqual(self.output.stat().st_mode & 0o777, 0o600)
        with tarfile.open(self.output) as archive:
            members = archive.getmembers()
            self.assertFalse(any(".virtfs_metadata" in m.name for m in members))
            for member in members:
                self.assertEqual((member.uid, member.gid), (0, 0))
                self.assertEqual((member.uname, member.gname), ("", ""))
                self.assertEqual(member.mode, 0o700 if member.isdir() else 0o600)
            files = {m.name: archive.extractfile(m).read()
                     for m in members if m.isfile()}
            self.assertEqual(files, {
                "capture/otel-traces.jsonl": b"abc",
                "capture/nested/extra.jsonl": b"complete additional stream",
            })
        self.assertEqual((self.backing / "otel-traces.jsonl").read_bytes(), b"abc")
        self.assertEqual(root_metadata.read_bytes(), ownership)
        self.assertEqual((metadata / "otel-traces.jsonl").read_bytes(), b"guest ownership metadata")
        self.assertEqual((nested / ".virtfs_metadata" / "extra.jsonl").read_bytes(), b"metadata")

    def test_multiple_read_chunks_preserve_the_complete_stream(self):
        payload = bytes(range(256)) * 769 + b"tail"
        (self.backing / "otel-traces.jsonl").write_bytes(payload)
        self.hashes["capture/otel-traces.jsonl"] = hashlib.sha256(payload).hexdigest()
        self.sizes["capture/otel-traces.jsonl"] = len(payload)
        self.archive()
        with tarfile.open(self.output) as archive:
            self.assertEqual(archive.extractfile("capture/otel-traces.jsonl").read(), payload)

    def test_incomplete_archive_cannot_verify(self):
        self.archive()
        encoded = self.output.read_bytes()
        self.output.write_bytes(encoded[:len(encoded) // 2])
        with self.assertRaises((tarfile.TarError, EOFError)):
            verify_capture_archive(self.output, self.hashes, self.sizes)

    def test_missing_file_after_guest_inventory_fails(self):
        (self.backing / "otel-traces.jsonl").unlink()
        with self.assertRaises(ValueError):
            self.archive()

    def test_truncated_file_after_guest_inventory_fails(self):
        (self.backing / "otel-traces.jsonl").write_bytes(b"ab")
        with self.assertRaises(ValueError):
            self.archive()

    def test_same_size_changed_bytes_after_guest_inventory_fail(self):
        (self.backing / "otel-traces.jsonl").write_bytes(b"xyz")
        with self.assertRaises(ValueError):
            self.archive()

    def test_unexpected_regular_file_is_not_silently_dropped(self):
        (self.backing / "extra.jsonl").write_bytes(b"unlisted data")
        with self.assertRaises(ValueError):
            self.archive()

    def test_backend_metadata_prefix_is_not_a_general_exclusion(self):
        (self.backing / ".virtfs_metadata-other").write_bytes(b"unlisted data")
        with self.assertRaises(ValueError):
            self.archive()

    def test_metadata_lookalikes_remain_in_the_complete_guest_population(self):
        names = [".virtfs_metadata-other", ".virtfs_metadata_root-extra",
                 "nested/.virtfs_metadata_root", ".virtfs_metadata-dir/stream.jsonl"]
        for name in names:
            path = self.backing / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"abc")
            self.hashes["capture/" + name] = self.hashes["capture/otel-traces.jsonl"]
            self.sizes["capture/" + name] = 3
        self.archive()
        with tarfile.open(self.output) as archive:
            self.assertEqual({m.name for m in archive if m.isfile()}, set(self.hashes))
            for name in self.hashes:
                self.assertEqual(archive.extractfile(name).read(), b"abc")

    def test_backend_metadata_wrong_types_fail_before_archive_creation(self):
        for name, kind in [(".virtfs_metadata_root", "directory"),
                           (".virtfs_metadata_root", "symlink"),
                           (".virtfs_metadata_root", "fifo"),
                           (".virtfs_metadata", "file"),
                           (".virtfs_metadata", "symlink")]:
            with self.subTest(name=name, kind=kind):
                backing = self.root / (name + "-" + kind)
                backing.mkdir()
                self.output = self.root / (name + "-" + kind + ".tar.gz")
                path = backing / name
                if kind == "directory":
                    path.mkdir()
                elif kind == "symlink":
                    path.symlink_to(self.backing / "otel-traces.jsonl")
                elif kind == "fifo":
                    os.mkfifo(path)
                else:
                    path.write_bytes(b"unknown data")
                with self.assertRaises(ValueError):
                    archive_capture(backing, self.output)
                self.assertFalse(self.output.exists())

    def test_nonregular_backing_member_fails(self):
        (self.backing / "link").symlink_to("otel-traces.jsonl")
        with self.assertRaises(ValueError):
            self.archive()

    def test_archive_creation_error_is_not_reported_as_complete(self):
        self.output.write_bytes(b"existing evidence")
        with self.assertRaises(FileExistsError):
            self.archive()
        self.assertEqual(self.output.read_bytes(), b"existing evidence")

    def test_empty_or_discrepant_guest_population_fails(self):
        for hashes, sizes in [({}, {}), (self.hashes, {})]:
            with self.subTest(hashes=hashes, sizes=sizes):
                with self.assertRaises(ValueError):
                    verify_capture_archive(self.output, hashes, sizes)

    def test_archived_backend_metadata_is_rejected_even_without_payload_bytes(self):
        for index, name in enumerate(["capture/.virtfs_metadata/secret",
                                      "capture/nested/.virtfs_metadata/secret",
                                      "capture/.virtfs_metadata_root"]):
            with self.subTest(name=name):
                output = self.root / ("metadata-%d.tar.gz" % index)
                with tarfile.open(output, "w:gz") as archive:
                    archive.addfile(tarfile.TarInfo(name))
                with self.assertRaisesRegex(ValueError, "backend metadata exposed"):
                    verify_capture_archive(output, self.hashes, self.sizes)
                with self.assertRaisesRegex(ValueError, "unexpected capture member"):
                    verify_capture_archive(output, {name: self.hashes["capture/otel-traces.jsonl"]},
                                           {name: 3})

    def test_archive_verification_rejects_duplicate_and_nonregular_members(self):
        for kind in ("duplicate", "symlink", "wrong-bytes", "metadata"):
            with self.subTest(kind=kind):
                output = self.root / (kind + ".tar.gz")
                with tarfile.open(output, "w:gz") as archive:
                    info = tarfile.TarInfo("capture/otel-traces.jsonl")
                    info.size = 3
                    if kind == "symlink":
                        info.type = tarfile.SYMTYPE
                        info.linkname = "outside"
                        archive.addfile(info)
                    else:
                        data = b"xyz" if kind == "wrong-bytes" else b"abc"
                        archive.addfile(info, io.BytesIO(data))
                        if kind == "duplicate":
                            archive.addfile(info, io.BytesIO(data))
                        if kind == "metadata":
                            archive.addfile(tarfile.TarInfo("capture/.virtfs_metadata/secret"))
                with self.assertRaises(ValueError):
                    verify_capture_archive(output, self.hashes, self.sizes)


if __name__ == "__main__":
    unittest.main()
