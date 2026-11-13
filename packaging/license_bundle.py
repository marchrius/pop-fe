"""Fetch license texts included with optional bundled third-party tools."""

from pathlib import Path
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
APACHE_2_LICENSE_URL = "https://www.apache.org/licenses/LICENSE-2.0.txt"
GPL_2_LICENSE_URL = "https://www.gnu.org/licenses/old-licenses/gpl-2.0.txt"


def ensure_apache_2_license(offline: bool = False) -> Path:
    return _ensure_license("Apache-2.0.txt", APACHE_2_LICENSE_URL, "Apache License\nVersion 2.0", offline)


def ensure_gpl_2_license(offline: bool = False) -> Path:
    return _ensure_license(
        "GPL-2.0.txt", GPL_2_LICENSE_URL, "GNU GENERAL PUBLIC LICENSE\nVersion 2, June 1991", offline
    )


def _valid_license_text(text: str, marker: str) -> bool:
    normalized = "\n".join(line.strip() for line in text.splitlines())
    return marker in normalized


def _ensure_license(filename: str, url: str, marker: str, offline: bool) -> Path:
    cache = ROOT / "target/package-tools/licenses" / filename
    if cache.is_file() and _valid_license_text(cache.read_text(encoding="utf-8"), marker):
        return cache
    if offline:
        raise RuntimeError(f"{filename} is not cached; run the package builder once without --offline")
    request = urllib.request.Request(url, headers={"User-Agent": "POP-FE-PSP packaging"})
    with urllib.request.urlopen(request, timeout=30) as response:
        contents = response.read()
    text = contents.decode("utf-8")
    if not _valid_license_text(text, marker):
        raise RuntimeError(f"downloaded {filename} license text is invalid")
    cache.parent.mkdir(parents=True, exist_ok=True)
    temporary = cache.with_suffix(".tmp")
    temporary.write_bytes(contents)
    temporary.replace(cache)
    return cache
