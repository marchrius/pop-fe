#!/usr/bin/env python3
"""Build an installable Flatpak bundle from the native Rust release binary."""

import argparse
from pathlib import Path
import shutil
import subprocess
import tempfile

from yt_dlp_bundle import ensure_bundle
from deno_bundle import ensure_bundle as ensure_deno_bundle

ROOT = Path(__file__).resolve().parents[1]
APP_ID = 'org.popfe.PSPManager'
RUNTIME = 'org.gnome.Platform'
RUNTIME_VERSION = '51'


def run(*command):
    subprocess.run(command, check=True, cwd=ROOT)


def host_architecture():
    architecture = subprocess.check_output(['flatpak', '--default-arch'], text=True).strip()
    if not architecture:
        raise RuntimeError('Flatpak did not report a host architecture')
    return architecture


def ensure_encoder():
    encoder = ROOT / 'atracdenc/src/atracdenc'
    if encoder.is_file():
        return encoder
    build_dir = ROOT / 'target/atracdenc-build'
    run('cmake', '-S', str(ROOT / 'atracdenc/src'), '-B', str(build_dir), '-DCMAKE_BUILD_TYPE=Release')
    run('cmake', '--build', str(build_dir), '--parallel')
    encoder = build_dir / 'atracdenc'
    if not encoder.is_file():
        raise RuntimeError('CMake did not produce atracdenc')
    return encoder


def find_runtime(runtime_ref):
    for scope in ('--user', '--system'):
        result = subprocess.run(
            ['flatpak', 'info', scope, '--show-location', runtime_ref],
            text=True,
            capture_output=True,
        )
        if result.returncode == 0:
            return Path(result.stdout.strip())
    raise RuntimeError(
        f'{runtime_ref} must be installed. Install it with: '
        f'flatpak install flathub {runtime_ref}'
    )


def verify_runtime_libraries(binaries, runtime_location, arch, bundled_library_dirs=()):
    loader_names = {
        'x86_64': 'ld-linux-x86-64.so.2',
        'aarch64': 'ld-linux-aarch64.so.1',
        'i386': 'ld-linux.so.2',
        'arm': 'ld-linux-armhf.so.3',
    }
    multiarch = {
        'x86_64': 'x86_64-linux-gnu',
        'aarch64': 'aarch64-linux-gnu',
        'i386': 'i386-linux-gnu',
        'arm': 'arm-linux-gnueabihf',
    }
    loader_name = loader_names.get(arch)
    if loader_name is None:
        raise RuntimeError(f'unsupported Flatpak architecture for ABI validation: {arch}')
    files = runtime_location / 'files'
    triple = multiarch[arch]
    loader_candidates = [
        files / 'lib' / triple / loader_name,
        files / 'usr/lib' / triple / loader_name,
        files / 'lib' / loader_name,
        files / 'usr/lib' / loader_name,
    ]
    loader = next((path for path in loader_candidates if path.is_file()), None)
    if loader is None:
        raise RuntimeError(f'cannot find {loader_name} in the GNOME runtime')
    library_dirs = [
        files / 'lib' / triple,
        files / 'usr/lib' / triple,
        files / 'lib',
        files / 'usr/lib',
    ] + list(bundled_library_dirs)
    library_path = ':'.join(str(path) for path in library_dirs)
    runtime_root = files.resolve()
    for binary in binaries:
        result = subprocess.run(
            [str(loader), '--library-path', library_path, '--list', str(binary)],
            text=True,
            capture_output=True,
        )
        if result.returncode != 0 or 'not found' in result.stdout:
            raise RuntimeError(
                f'{binary.name} cannot load with {RUNTIME}//{RUNTIME_VERSION}: '
                f'{result.stderr.strip() or result.stdout.strip()}'
            )
        for line in result.stdout.splitlines():
            if '=>' not in line:
                continue
            resolved = line.split('=>', 1)[1].strip().split(' (', 1)[0]
            resolved_path = Path(resolved).resolve() if resolved.startswith('/') else None
            requested_name = Path(resolved).name if resolved.startswith('/') else ''
            bundled_match = resolved_path and any(
                (directory / requested_name).is_file() for directory in bundled_library_dirs
            )
            if resolved_path and not resolved_path.is_relative_to(runtime_root) and not bundled_match:
                raise RuntimeError(
                    f'{binary.name} resolves {resolved} outside the Flatpak runtime'
                )


def missing_runtime_dependencies(binary, runtime_location):
    files = (runtime_location / 'files').resolve()
    result = subprocess.run(['ldd', str(binary)], text=True, capture_output=True, check=True)
    missing = {}
    for line in result.stdout.splitlines():
        if '=>' not in line:
            continue
        name, target = line.split('=>', 1)
        resolved = target.strip().split(' (', 1)[0]
        source = Path(resolved)
        if resolved.startswith('/') and source.is_file() and not source.resolve().is_relative_to(files):
            missing[name.strip()] = source
    return missing


def missing_runtime_dependency_closure(binaries, runtime_location):
    """Collect host libraries required by bundled command-line tools."""
    files = (runtime_location / 'files').resolve()
    pending = list(binaries)
    seen = set()
    missing = {}
    while pending:
        binary = pending.pop()
        if binary in seen:
            continue
        seen.add(binary)
        for name, source in missing_runtime_dependencies(binary, runtime_location).items():
            if name not in missing:
                missing[name] = source
                pending.append(source)
    return missing


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--offline', action='store_true', help='build Cargo dependencies offline')
    parser.add_argument('--output', type=Path, default=ROOT / 'dist/org.popfe.PSPManager.flatpak')
    args = parser.parse_args()

    cargo = ['cargo', 'build', '--release', '--locked', '--manifest-path', str(ROOT / 'Cargo.toml')]
    if args.offline:
        cargo.append('--offline')
    run(*cargo)
    binary = ROOT / 'target/release/pop-fe-psp'
    encoder = ensure_encoder()
    xdelta = shutil.which('xdelta3')
    if not xdelta:
        raise RuntimeError('xdelta3 is required to package automatic Xdelta compatibility fixes')
    xdelta = Path(xdelta)
    arch = host_architecture()
    yt_dlp, yt_dlp_license, yt_dlp_third_party = ensure_bundle(offline=args.offline)
    deno, deno_license = ensure_deno_bundle(arch, offline=args.offline)

    runtime_ref = f'{RUNTIME}//{RUNTIME_VERSION}'
    runtime_location = find_runtime(runtime_ref)
    chdman = shutil.which('chdman')
    if not chdman:
        raise RuntimeError('chdman (MAME tools) is required to package CHD support')
    chdman = Path(chdman)
    pdftoppm = shutil.which('pdftoppm')
    ffmpeg = shutil.which('ffmpeg')
    if not pdftoppm or not ffmpeg:
        raise RuntimeError('poppler-utils and ffmpeg are required to package automatic manual support')
    pdftoppm = Path(pdftoppm)
    ffmpeg = Path(ffmpeg)
    sevenzip = Path('/usr/lib/7zip/7z')
    sevenzip_module = Path('/usr/lib/7zip/7z.so')
    if not sevenzip.is_file() or not sevenzip_module.is_file():
        raise RuntimeError('7zip is required to package automatic RAR/CBR manual support')
    openssl_legacy = next(Path('/usr/lib').glob('*/ossl-modules/legacy.so'), None)
    if openssl_legacy is None:
        raise RuntimeError('OpenSSL legacy provider is required for encrypted PS3 manuals')
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='pop-fe-flatpak-') as temporary:
        temporary = Path(temporary)
        stage = temporary / 'stage'
        repo = temporary / 'repo'
        run('flatpak', 'build-init', '--arch', arch, str(stage), APP_ID, RUNTIME, RUNTIME, RUNTIME_VERSION)
        app = stage / 'files/app'
        install_pairs = [
            (binary, app / 'bin/pop-fe-psp'),
            (encoder, app / 'bin/atracdenc'),
            (xdelta, app / 'bin/xdelta3'),
            (chdman, app / 'bin/chdman'),
            (pdftoppm, app / 'bin/pdftoppm'),
            (ffmpeg, app / 'bin/ffmpeg'),
            (ROOT / 'packaging/org.popfe.PSPManager.desktop', app / 'share/applications/org.popfe.PSPManager.desktop'),
            (ROOT / 'packaging/icons/hicolor/scalable/apps/org.popfe.PSPManager.svg',
             app / 'share/icons/hicolor/scalable/apps/org.popfe.PSPManager.svg'),
            (ROOT / 'licenses/libcrypt-patcher-BSD-3-Clause.txt',
             app / 'share/licenses/libcrypt-patcher-BSD-3-Clause.txt'),
            (yt_dlp, app / 'bin/yt-dlp'),
            (yt_dlp_license, app / 'share/licenses/yt-dlp-LICENSE.txt'),
            (yt_dlp_third_party, app / 'share/licenses/yt-dlp-THIRD_PARTY_LICENSES.txt'),
            (deno, app / 'bin/deno'),
            (deno_license, app / 'share/licenses/deno-LICENSE.md'),
        ]
        for source, target in install_pairs:
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)
            target.chmod(0o755 if target.name in ('pop-fe-psp', 'atracdenc', 'xdelta3', 'chdman', 'pdftoppm', 'ffmpeg', 'yt-dlp', 'deno') else 0o644)

        app_lib = app / 'lib'
        sevenzip_target = app_lib / '7zip/7z'
        sevenzip_module_target = app_lib / '7zip/7z.so'
        sevenzip_target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(sevenzip, sevenzip_target)
        shutil.copy2(sevenzip_module, sevenzip_module_target)
        sevenzip_target.chmod(0o755)
        sevenzip_module_target.chmod(0o644)
        wrapper = app / 'bin/7z'
        wrapper.write_text('#!/bin/sh\nexec /app/lib/7zip/7z "$@"\n', encoding='utf-8')
        wrapper.chmod(0o755)

        legacy_target = app_lib / 'ossl-modules/legacy.so'
        legacy_target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(openssl_legacy, legacy_target)

        bundled_tools = [binary, chdman, pdftoppm, ffmpeg, deno, sevenzip_target, sevenzip_module_target]
        for name, source in missing_runtime_dependency_closure(bundled_tools, runtime_location).items():
            target = app_lib / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)
            target.chmod(0o755)
        verify_runtime_libraries(
            [binary, encoder, xdelta, app / 'bin/chdman', app / 'bin/pdftoppm', app / 'bin/ffmpeg', app / 'bin/deno', sevenzip_target, sevenzip_module_target], runtime_location, arch,
            bundled_library_dirs=[app_lib],
        )

        shutil.copytree('/usr/share/poppler', app / 'share/poppler', dirs_exist_ok=True)

        run(
            'flatpak', 'build-finish', '--command=pop-fe-psp', '--share=ipc',
            '--socket=wayland', '--socket=fallback-x11', '--device=dri',
            '--share=network', '--talk-name=org.freedesktop.portal.Desktop', str(stage),
            '--env=LD_LIBRARY_PATH=/app/lib', '--env=OPENSSL_MODULES=/app/lib/ossl-modules',
        )
        run('flatpak', 'build-export', '--arch', arch, str(repo), str(stage), 'stable')
        run('flatpak', 'build-bundle', '--arch', arch, str(repo), str(args.output), APP_ID, 'stable')
    print(args.output)


if __name__ == '__main__':
    main()
