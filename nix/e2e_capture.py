"""Archive a quiesced NixOS E2E capture from its mapped-file host exchange.

The guest's independent inventory remains authoritative. Only the transport is
host-local; the reopened archive must reconcile the entire logical population.
"""

import hashlib
import os
from pathlib import Path, PurePosixPath
import stat
import tarfile


def _is_backend_metadata(name):
    return (".virtfs_metadata" in PurePosixPath(name).parts
            or name == "capture/.virtfs_metadata_root")


def _require_population(hashes, sizes):
    if not hashes or hashes.keys() != sizes.keys():
        raise ValueError("source populations differ or empty")
    for name in hashes:
        path = PurePosixPath(name)
        if (path.is_absolute() or not name.startswith("capture/")
                or ".." in path.parts
                or _is_backend_metadata(name)):
            raise ValueError("unexpected capture member")


def verify_capture_archive(destination, source_hashes, source_sizes):
    """Independently reread all archived bytes; never infer success from tar exit."""
    _require_population(source_hashes, source_sizes)
    actual_hashes = {}
    actual_sizes = {}
    with tarfile.open(destination, "r|gz") as archive:
        for member in archive:
            if _is_backend_metadata(member.name):
                raise ValueError("backend metadata exposed")
            if member.isdir():
                continue
            if not member.isfile() or member.name in actual_hashes:
                raise ValueError("nonregular or duplicate capture member")
            data = archive.extractfile(member)
            if data is None:
                raise ValueError("missing regular capture member")
            sha = hashlib.sha256()
            count = 0
            with data:
                while chunk := data.read(65536):
                    sha.update(chunk)
                    count += len(chunk)
            actual_hashes[member.name] = sha.hexdigest()
            actual_sizes[member.name] = count
            if count != member.size:
                raise ValueError("archive declared size mismatch")
    if actual_hashes != source_hashes or actual_sizes != source_sizes:
        raise ValueError("source/lift member/byte/SHA mismatch")
    return actual_hashes, actual_sizes


def archive_capture(backing, destination):
    """Preserve every logical file, excluding only mapped-file backend metadata.

    `backing` is machine.state_dir / 'xchg', not the driver's shared-xchg.
    Writers must already be drained by the caller. Private canonical tar headers
    avoid disclosing host ownership or the backend's guest-ownership encoding.
    """
    if not stat.S_ISDIR(backing.lstat().st_mode):
        raise ValueError("capture backing is not a directory")
    files = {}
    directories = ["capture"]

    def walk_error(error):
        raise error

    for root, dirs, names in os.walk(backing, onerror=walk_error, followlinks=False):
        for name in list(dirs):
            path = Path(root) / name
            if (not stat.S_ISDIR(path.lstat().st_mode)
                    or path.relative_to(backing).as_posix() == ".virtfs_metadata_root"):
                raise ValueError("nonregular capture directory")
            if name == ".virtfs_metadata":
                dirs.remove(name)
            else:
                directories.append("capture/" + path.relative_to(backing).as_posix())
        for name in names:
            path = Path(root) / name
            if not stat.S_ISREG(path.lstat().st_mode) or name == ".virtfs_metadata":
                raise ValueError("nonregular capture file")
            # QEMU mapped-file stores export-root attributes outside its metadata directories.
            if path.relative_to(backing).as_posix() == ".virtfs_metadata_root":
                continue
            files["capture/" + path.relative_to(backing).as_posix()] = path
    # Exclusive, private output preserves any earlier evidence on failure.
    with os.fdopen(os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "wb") as output:
        with tarfile.open(fileobj=output, mode="w:gz") as archive:
            for name in sorted(directories):
                info = tarfile.TarInfo(name)
                info.type = tarfile.DIRTYPE
                info.mode = 0o700
                archive.addfile(info)
            for name, path in sorted(files.items()):
                info = tarfile.TarInfo(name)
                info.mode = 0o600
                info.size = path.stat().st_size
                with path.open("rb") as data:
                    archive.addfile(info, data)
