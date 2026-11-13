"""Download and verify the official standalone LibCrypt patcher."""

import hashlib
import json
from pathlib import Path
import platform
import urllib.request
from zipfile import ZipFile

ROOT = Path(__file__).resolve().parents[1]
VERSION = "1.1.0"
RELEASE = f"https://github.com/alex-free/libcrypt-patcher/releases/download/v{VERSION}"
GITHUB_RELEASE_API = f"https://api.github.com/repos/alex-free/libcrypt-patcher/releases/tags/v{VERSION}"
ASSETS = {
    "x86_64": (
        "libcrypt-patcher-v1.1.0-linux-x86_64-static.zip",
        "b3caf8bc6dfc4a70f099fbbc51e3f362342e4d66d72428b0fbc31423e93c3c3f",
        "libcrypt-patcher-v1.1.0-linux-x86_64-static/lcp",
        "4705eaf1b0a62a928739b2c9009c9763b2245c56243b84c266ee7fe4fd25fc5c",
    ),
}
PORTABLE_ASSETS = {
    "macos-x86_64": ("libcrypt-patcher-v1.1.0-mac-os-x86_64.zip", "lcp"),
    "windows-x86_64": ("libcrypt-patcher-v1.1.0-windows-x86_64-static.zip", "lcp.exe"),
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


def _portable_asset_digest(asset_name: str) -> str:
    request = urllib.request.Request(
        GITHUB_RELEASE_API,
        headers={"Accept": "application/vnd.github+json", "User-Agent": "POP-FE-PSP packaging"},
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        release = json.load(response)
    assets = [asset for asset in release.get("assets", []) if asset.get("name") == asset_name]
    if len(assets) != 1:
        raise RuntimeError(f"GitHub release v{VERSION} does not contain exactly one {asset_name}")
    digest = assets[0].get("digest", "")
    if not digest.startswith("sha256:") or len(digest) != 71:
        raise RuntimeError(f"GitHub release has no valid SHA-256 digest for {asset_name}")
    return digest.removeprefix("sha256:")


def ensure_bundle(architecture: str | None = None, offline: bool = False) -> Path | None:
    """Return the verified lcp executable, or None when upstream has no asset."""
    architecture = architecture or platform.machine()
    architecture = {"amd64": "x86_64", "arm64": "aarch64"}.get(architecture, architecture)
    portable_asset = PORTABLE_ASSETS.get(architecture)
    if portable_asset:
        archive_name, executable_name = portable_asset
        cache = ROOT / "target/package-tools/libcrypt-patcher" / VERSION / architecture
        cache.mkdir(parents=True, exist_ok=True)
        archive = cache / archive_name
        executable = cache / executable_name
        digest_file = cache / f"{archive_name}.sha256"
        if offline:
            if not digest_file.is_file():
                raise RuntimeError(
                    f"standalone lcp {VERSION} digest ({architecture}) is not cached; "
                    "run the package builder once without --offline"
                )
            expected_hash = digest_file.read_text(encoding="ascii").strip()
            if len(expected_hash) != 64 or any(character not in "0123456789abcdef" for character in expected_hash):
                raise RuntimeError(f"cached lcp digest is invalid for {architecture}")
        else:
            expected_hash = _portable_asset_digest(archive_name)
            digest_file.write_text(expected_hash, encoding="ascii")
        if not archive.is_file() or _sha256(archive) != expected_hash:
            if offline:
                raise RuntimeError(
                    f"standalone lcp {VERSION} ({architecture}) is not cached; "
                    "run the package builder once without --offline"
                )
            _download(f"{RELEASE}/{archive_name}", archive)
            actual = _sha256(archive)
            if actual != expected_hash:
                archive.unlink(missing_ok=True)
                raise RuntimeError(f"lcp archive SHA-256 mismatch: {actual}")
        with ZipFile(archive) as bundle:
            matches = [info for info in bundle.infolist() if Path(info.filename).name == executable_name]
            if len(matches) != 1 or matches[0].is_dir():
                raise RuntimeError(f"lcp archive does not contain exactly one {executable_name}")
            contents = bundle.read(matches[0])
        if not executable.is_file() or executable.read_bytes() != contents:
            temporary = executable.with_suffix(executable.suffix + ".tmp")
            temporary.write_bytes(contents)
            temporary.chmod(0o755)
            temporary.replace(executable)
        else:
            executable.chmod(0o755)
        return executable

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
