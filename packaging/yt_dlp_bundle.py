"""Download and verify the upstream yt-dlp zipimport executable for packaging."""

import hashlib
from pathlib import Path
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
VERSION = "2026.10.26"
RELEASE = f"https://github.com/yt-dlp/yt-dlp/releases/download/{VERSION}"
RAW = f"https://raw.githubusercontent.com/yt-dlp/yt-dlp/{VERSION}"
EXECUTABLE_SHA256 = "e51e69e89489f199530f13d32a4342da3ddb2cf34842f364603a066db05afbf3"
LICENSE_SHA256 = "7e12e5df4bae12cb21581ba157ced20e1986a0508dd10d0e8a4ab9a4cf94e85c"
THIRD_PARTY_SHA256 = "472aefe951c7db35e1657c1d13fd337140511ed6f2b329205105ad441c5a02b7"


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


def ensure_bundle(offline: bool = False) -> tuple[Path, Path, Path]:
    """Return the executable and upstream licenses, fetching a pinned release if needed."""
    cache = ROOT / "target/package-tools/yt-dlp" / VERSION
    cache.mkdir(parents=True, exist_ok=True)
    executable = cache / "yt-dlp"
    license_file = cache / "LICENSE"
    third_party = cache / "THIRD_PARTY_LICENSES.txt"

    if not executable.is_file() or _sha256(executable) != EXECUTABLE_SHA256:
        if offline:
            raise RuntimeError(
                f"yt-dlp {VERSION} is not cached; run the package builder once without --offline"
            )
        _download(f"{RELEASE}/yt-dlp", executable)
        actual = _sha256(executable)
        if actual != EXECUTABLE_SHA256:
            executable.unlink(missing_ok=True)
            raise RuntimeError(f"yt-dlp SHA-256 mismatch: {actual}")
    executable.chmod(0o755)

    for path, url, expected in (
        (license_file, f"{RAW}/LICENSE", LICENSE_SHA256),
        (third_party, f"{RAW}/THIRD_PARTY_LICENSES.txt", THIRD_PARTY_SHA256),
    ):
        if not path.is_file() or _sha256(path) != expected:
            if offline:
                raise RuntimeError(
                    f"yt-dlp {VERSION} license files are not cached; run the package builder once without --offline"
                )
            _download(url, path)
            actual = _sha256(path)
            if actual != expected:
                path.unlink(missing_ok=True)
                raise RuntimeError(f"yt-dlp license SHA-256 mismatch for {path.name}: {actual}")
    if "unencumbered software" not in license_file.read_text(encoding="utf-8"):
        raise RuntimeError("downloaded yt-dlp license file is invalid")
    if "THIRD-PARTY LICENSES" not in third_party.read_text(encoding="utf-8"):
        raise RuntimeError("downloaded yt-dlp third-party license file is invalid")
    return executable, license_file, third_party


if __name__ == "__main__":
    executable, license_file, third_party = ensure_bundle()
    print(executable)
    print(license_file)
    print(third_party)
