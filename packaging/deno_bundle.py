"""Download and verify the official Deno runtime used by yt-dlp's YouTube EJS."""

import hashlib
from pathlib import Path
import subprocess
import tempfile
from urllib.request import Request, urlopen
from zipfile import ZipFile

ROOT = Path(__file__).resolve().parents[1]
VERSION = "2.9.8"
RELEASE = f"https://github.com/denoland/deno/releases/download/v{VERSION}"
LICENSE_URL = f"https://raw.githubusercontent.com/denoland/deno/v{VERSION}/LICENSE.md"
LICENSE_SHA256 = "f62497fffecc0852960c8d3e6934b9db86d16396e9b604072e923892cae3a588"
ASSETS = {
    "x86_64": (
        "deno-x86_64-unknown-linux-gnu.zip",
        "e890975bc2549807bbcc0b584ffafda72cc7c93349dd99c80483dd784f6c4609",
    ),
    "aarch64": (
        "deno-aarch64-unknown-linux-gnu.zip",
        "d49596c20b9788cf8f5d8ddf644103d4bde56ed239753f0bdb92ba9361d51905",
    ),
    "macos-x86_64": (
        "deno-x86_64-apple-darwin.zip",
        "4b2f5edd2639b7470f034238b321d7002d8dfb4726d8558f52c17afac370eb2c",
    ),
    "macos-aarch64": (
        "deno-aarch64-apple-darwin.zip",
        "9f0bf660393da2eaff044cdc7444cb08c73f235849b61d224d825a194ce7c98e",
    ),
    "windows-x86_64": (
        "deno-x86_64-pc-windows-msvc.zip",
        "cba84b26e0df5ae7f74fc66b86a95b866872a86292ebd84fa1e959927edd93bf",
    ),
}
BINARY_SHA256 = {
    "x86_64": "368bca08999e02c6911b8f248167b582276b1f547db5712b6e736d7eda1dbda7",
}
ARCH_ALIASES = {"amd64": "x86_64", "arm64": "aarch64", "x86_64-apple-darwin": "macos-x86_64", "aarch64-apple-darwin": "macos-aarch64", "x86_64-pc-windows-msvc": "windows-x86_64"}


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def _download(url: str, target: Path) -> None:
    temporary = target.with_suffix(target.suffix + ".download")
    request = Request(url, headers={"User-Agent": "POP-FE-PSP packaging"})
    try:
        with urlopen(request, timeout=120) as response, temporary.open("wb") as output:
            while chunk := response.read(1024 * 1024):
                output.write(chunk)
        temporary.replace(target)
    finally:
        temporary.unlink(missing_ok=True)


def ensure_bundle(architecture: str, offline: bool = False) -> tuple[Path, Path]:
    architecture = ARCH_ALIASES.get(architecture, architecture)
    if architecture not in ASSETS:
        raise RuntimeError(f"Deno {VERSION} is not available for architecture {architecture}")
    filename, expected_sha = ASSETS[architecture]
    cache = ROOT / "target/package-tools/deno" / VERSION / architecture
    cache.mkdir(parents=True, exist_ok=True)
    archive = cache / filename
    executable_name = "deno.exe" if architecture == "windows-x86_64" else "deno"
    executable = cache / executable_name
    license_file = cache / "LICENSE.md"

    archive_valid = archive.is_file() and _sha256(archive) == expected_sha
    executable_sha = BINARY_SHA256.get(architecture)
    executable_valid = (
        executable.is_file()
        and executable_sha is not None
        and _sha256(executable) == executable_sha
    )
    if not archive_valid:
        if offline:
            if not executable_valid:
                raise RuntimeError(
                    f"Deno {VERSION} ({architecture}) is not cached; run the package builder once without --offline"
                )
        else:
            _download(f"{RELEASE}/{filename}", archive)
            actual = _sha256(archive)
            if actual != expected_sha:
                archive.unlink(missing_ok=True)
                raise RuntimeError(f"Deno archive SHA-256 mismatch: {actual}")

    if not license_file.is_file() or _sha256(license_file) != LICENSE_SHA256:
        if offline:
            raise RuntimeError("Deno license is not cached; run the package builder once without --offline")
        _download(LICENSE_URL, license_file)
        actual = _sha256(license_file)
        if actual != LICENSE_SHA256:
            license_file.unlink(missing_ok=True)
            raise RuntimeError(f"Deno license SHA-256 mismatch: {actual}")

    if not executable_valid:
        if not archive.is_file() or _sha256(archive) != expected_sha:
            raise RuntimeError(f"verified Deno archive {VERSION} is required to extract the binary")
        with tempfile.TemporaryDirectory(prefix="deno-extract-", dir=cache) as temporary:
            with ZipFile(archive) as bundle:
                executable_name = "deno.exe" if architecture == "windows-x86_64" else "deno"
                matches = [info for info in bundle.infolist() if Path(info.filename).name == executable_name]
                if len(matches) != 1 or matches[0].is_dir():
                    raise RuntimeError("official Deno archive does not contain exactly one deno executable")
                temporary_executable = Path(temporary) / executable_name
                with bundle.open(matches[0]) as source, temporary_executable.open("wb") as target:
                    while chunk := source.read(1024 * 1024):
                        target.write(chunk)
            temporary_executable.chmod(0o755)
            temporary_executable.replace(executable)

    executable.chmod(0o755)
    if executable_sha is not None and _sha256(executable) != executable_sha:
        raise RuntimeError("Deno executable SHA-256 mismatch")
    version = subprocess.check_output([str(executable), "--version"], text=True).splitlines()[0]
    if not version.startswith(f"deno {VERSION} "):
        raise RuntimeError(f"unexpected Deno executable version: {version}")
    return executable, license_file


if __name__ == "__main__":
    executable, license_file = ensure_bundle("amd64")
    print(executable)
    print(license_file)

