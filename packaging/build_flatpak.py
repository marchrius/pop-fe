#!/usr/bin/env python3
"""Build an installable Flatpak bundle from the native Rust release binary."""

import argparse
from pathlib import Path
import shutil
import subprocess
import tempfile

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


def verify_runtime_libraries(binaries, runtime_location, arch):
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
    ]
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
            if resolved.startswith('/') and not Path(resolved).resolve().is_relative_to(runtime_root):
                raise RuntimeError(
                    f'{binary.name} resolves {resolved} outside the Flatpak runtime'
                )


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

    runtime_ref = f'{RUNTIME}//{RUNTIME_VERSION}'
    runtime_location = find_runtime(runtime_ref)
    verify_runtime_libraries([binary, encoder, xdelta], runtime_location, arch)

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
            (ROOT / 'packaging/org.popfe.PSPManager.desktop', app / 'share/applications/org.popfe.PSPManager.desktop'),
            (ROOT / 'packaging/icons/hicolor/scalable/apps/org.popfe.PSPManager.svg',
             app / 'share/icons/hicolor/scalable/apps/org.popfe.PSPManager.svg'),
        ]
        for source, target in install_pairs:
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)
            target.chmod(0o755 if target.name in ('pop-fe-psp', 'atracdenc') else 0o644)

        run(
            'flatpak', 'build-finish', '--command=pop-fe-psp', '--share=ipc',
            '--socket=wayland', '--socket=fallback-x11', '--device=dri',
            '--share=network', '--talk-name=org.freedesktop.portal.Desktop', str(stage),
        )
        run('flatpak', 'build-export', '--arch', arch, str(repo), str(stage), 'stable')
        run('flatpak', 'build-bundle', '--arch', arch, str(repo), str(args.output), APP_ID, 'stable')
    print(args.output)


if __name__ == '__main__':
    main()
