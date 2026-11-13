"""Download and verify the official standalone LibCrypt patcher."""

import hashlib
from pathlib import Path
import platform
import urllib.request
from zipfile import ZipFile

ROOT = Path(__file__).resolve().parents[1]
VERSION = "1.1.0"
RELEASE = f"https://github.com/alex-free/libcrypt-patcher/releases/download/v{VERSION}"
ASSETS = {
    "x86_64": (
        "libcrypt-patcher-v1.1.0-linux-x86_64-static.zip",
        "b3caf8bc6dfc4a70f099fbbc51e3f362342e4d66d72428b0fbc31423e93c3c3f",
        "libcrypt-patcher-v1.1.0-linux-x86_64-static/lcp",
        "4705eaf1b0a62a928739b2c9009c9763b2245c56243b84c266ee7fe4fd25fc5c",
    ),
}


def _download(url: str, target: Path) -> None:
    temporary = target.with_suffix(target.suffix + ".download")
    request = urllib.request.Request(url, headers={"User-Agent": "POP-FE-PSP packaging"})
    try:
        with urllib.request.urlopen(request, timeout=90) as response, temporary.open("wb") as output:
            while chunk := response.read(1024 * 1024):
                output.write(chunk)
        temporary.replace(target)
    finally:
        temporary.unlink(missing_ok=True)


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def ensure_bundle(architecture: str | None = None, offline: bool = False) -> Path | None:
    """Return the verified lcp executable, or None when upstream has no asset."""
    architecture = architecture or platform.machine()
    architecture = {"amd64": "x86_64", "arm64": "aarch64"}.get(architecture, architecture)
    asset = ASSETS.get(architecture)
    if asset is None:
        return None
    archive_name, archive_hash, member, executable_hash = asset
    cache = ROOT / "target/package-tools/libcrypt-patcher" / VERSION / architecture
    cache.mkdir(parents=True, exist_ok=True)
    archive = cache / archive_name
    executable = cache / "lcp"

    if executable.is_file() and _sha256(executable) == executable_hash:
        executable.chmod(0o755)
        return executable

    if not archive.is_file() or _sha256(archive) != archive_hash:
        if offline:
            raise RuntimeError(
                f"standalone lcp {VERSION} ({architecture}) is not cached; "
                "run the package builder once without --offline"
            )
        _download(f"{RELEASE}/{archive_name}", archive)
        actual = _sha256(archive)
        if actual != archive_hash:
            archive.unlink(missing_ok=True)
            raise RuntimeError(f"lcp archive SHA-256 mismatch: {actual}")

    if not executable.is_file() or _sha256(executable) != executable_hash:
        with ZipFile(archive) as bundle:
            try:
                contents = bundle.read(member)
            except KeyError as error:
                raise RuntimeError(f"lcp archive is missing {member}") from error
        actual = hashlib.sha256(contents).hexdigest()
        if actual != executable_hash:
            raise RuntimeError(f"lcp executable SHA-256 mismatch: {actual}")
        temporary = executable.with_suffix(".tmp")
        temporary.write_bytes(contents)
        temporary.chmod(0o755)
        temporary.replace(executable)
    else:
        executable.chmod(0o755)
    return executable


if __name__ == "__main__":
    print(ensure_bundle())
