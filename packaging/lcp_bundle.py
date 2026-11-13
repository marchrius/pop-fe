"""Download and verify the official standalone LibCrypt patcher."""

import hashlib
from pathlib import Path
import platform
import shutil
import subprocess
import urllib.request
from zipfile import ZipFile

ROOT = Path(__file__).resolve().parents[1]
VERSION = "1.1.0"
SOURCE_COMMIT = "f1e70da5666a888ca82d8e1c0ba4d04b994ac426"
LIB_ENIGMA_COMMIT = "7c4780c7031ba0d1236cb813aa3159931251ac84"
SOURCE_REPOSITORY = "https://github.com/alex-free/libcrypt-patcher.git"
RELEASE = f"https://github.com/alex-free/libcrypt-patcher/releases/download/v{VERSION}"
ASSETS = {
    "x86_64": (
        "libcrypt-patcher-v1.1.0-linux-x86_64-static.zip",
        "b3caf8bc6dfc4a70f099fbbc51e3f362342e4d66d72428b0fbc31423e93c3c3f",
        "libcrypt-patcher-v1.1.0-linux-x86_64-static/lcp",
        "4705eaf1b0a62a928739b2c9009c9763b2245c56243b84c266ee7fe4fd25fc5c",
    ),
}
PORTABLE_ASSETS = {
    "macos-x86_64": (
        "libcrypt-patcher-v1.1.0-mac-os-x86_64.zip",
        "97207aee825b82f3d13292e2817ebffd78bf5062762c2f5e8f5b3e4c561e65dc",
        "lcp",
    ),
    "windows-x86_64": (
        "libcrypt-patcher-v1.1.0-windows-x86_64-static.zip",
        "ed8fe84507b770311ac435cdf5bee815ec80b4b344708ef7f4fa3f1f33172c30",
        "lcp.exe",
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


def _git_revision(repository: Path, path: str = "") -> str:
    command = ["git", "-C", str(repository)]
    if path:
        command.extend(["-C", path])
    command.extend(["rev-parse", "HEAD"])
    return subprocess.check_output(command, text=True).strip()


def _ensure_macos_arm_bundle(offline: bool) -> Path:
    if platform.system() != "Darwin" or platform.machine().lower() not in ("arm64", "aarch64"):
        raise RuntimeError("building the native LibCrypt patcher requires an Apple Silicon Mac")

    source_key = f"{SOURCE_COMMIT[:12]}-{LIB_ENIGMA_COMMIT[:12]}"
    cache = ROOT / "target/package-tools/libcrypt-patcher" / VERSION / f"macos-aarch64-{source_key}"
    cache.mkdir(parents=True, exist_ok=True)
    source = cache / "source"
    executable = cache / "lcp"

    def source_is_pinned() -> bool:
        try:
            return (
                _git_revision(source) == SOURCE_COMMIT
                and _git_revision(source, "lib-enigma") == LIB_ENIGMA_COMMIT
                and not subprocess.check_output(
                    [
                        "git", "-C", str(source), "status", "--porcelain",
                        "--untracked-files=no", "--ignore-submodules=all",
                    ],
                    text=True,
                ).strip()
                and not subprocess.check_output(
                    [
                        "git", "-C", str(source), "-C", "lib-enigma", "status", "--porcelain",
                        "--untracked-files=no",
                    ],
                    text=True,
                ).strip()
            )
        except (OSError, subprocess.CalledProcessError):
            return False

    if not source_is_pinned():
        if offline:
            raise RuntimeError(
                f"LibCrypt patcher {VERSION} source (including lib-enigma) is not cached; "
                "run the package builder once without --offline"
            )
        temporary = cache / "source.download"
        if temporary.exists():
            shutil.rmtree(temporary)
        subprocess.run(
            [
                "git",
                "clone",
                "--depth",
                "1",
                "--branch",
                f"v{VERSION}",
                "--recurse-submodules",
                SOURCE_REPOSITORY,
                str(temporary),
            ],
            check=True,
        )
        if not (
            _git_revision(temporary) == SOURCE_COMMIT
            and _git_revision(temporary, "lib-enigma") == LIB_ENIGMA_COMMIT
        ):
            shutil.rmtree(temporary)
            raise RuntimeError(f"LibCrypt patcher {VERSION} source revision did not match the pinned commits")
        if source.exists():
            shutil.rmtree(source)
        temporary.replace(source)

    def is_arm64(path: Path) -> bool:
        result = subprocess.run(["/usr/bin/file", str(path)], text=True, capture_output=True, check=False)
        return result.returncode == 0 and "arm64" in result.stdout.lower()

    if not executable.is_file() or not is_arm64(executable):
        subprocess.run(["make", "mac-os"], cwd=source, check=True)
        built = source / "build" / f"lcp.{platform.machine()}"
        if not built.is_file():
            raise RuntimeError(f"native LibCrypt build did not produce {built.name}")
        if not is_arm64(built):
            details = subprocess.run(
                ["/usr/bin/file", str(built)], text=True, capture_output=True, check=False
            )
            raise RuntimeError(
                "native LibCrypt build did not produce an arm64 executable: "
                + details.stdout.strip()
            )
        temporary = executable.with_suffix(".tmp")
        shutil.copy2(built, temporary)
        temporary.chmod(0o755)
        temporary.replace(executable)
    executable.chmod(0o755)
    return executable


def ensure_bundle(architecture: str | None = None, offline: bool = False) -> Path | None:
    """Return the verified lcp executable, or None when upstream has no asset."""
    architecture = architecture or platform.machine()
    architecture = {"amd64": "x86_64", "arm64": "aarch64"}.get(architecture, architecture)
    if architecture == "macos-aarch64":
        return _ensure_macos_arm_bundle(offline)
    portable_asset = PORTABLE_ASSETS.get(architecture)
    if portable_asset:
        archive_name, expected_hash, executable_name = portable_asset
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
