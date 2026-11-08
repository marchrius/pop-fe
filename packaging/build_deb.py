#!/usr/bin/env python3
"""Build the native Rust GTK4 .deb package without root."""

import argparse
from email.utils import formatdate
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def install(source, target, mode=0o644):
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, target)
    target.chmod(mode)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--version', default=(ROOT / 'VERSION').read_text(encoding='utf-8').strip())
    parser.add_argument('--offline', action='store_true', help='build Cargo dependencies offline')
    args = parser.parse_args()
    if not args.version or any(c not in '0123456789.+~-' for c in args.version):
        parser.error('version must be a numeric Debian version')

    encoder = ROOT / 'atracdenc/src/atracdenc'
    if not encoder.is_file():
        encoder_build = ROOT / 'target/atracdenc-build'
        subprocess.run([
            'cmake', '-S', str(ROOT / 'atracdenc/src'), '-B', str(encoder_build),
            '-DCMAKE_BUILD_TYPE=Release',
        ], check=True)
        subprocess.run(['cmake', '--build', str(encoder_build), '--parallel'], check=True)
        encoder = encoder_build / 'atracdenc'
    if not encoder.is_file():
        parser.error('CMake completed but did not produce the atracdenc executable')
    command = ['cargo', 'build', '--release', '--locked', '--manifest-path', str(ROOT / 'Cargo.toml')]
    if args.offline:
        command.append('--offline')
    subprocess.run(command, cwd=ROOT, check=True)
    binary = ROOT / 'target/release/pop-fe-psp'
    if not binary.is_file():
        parser.error('Cargo did not produce target/release/pop-fe-psp')

    arch = subprocess.check_output(['dpkg', '--print-architecture'], text=True).strip()
    output = ROOT / 'dist'
    output.mkdir(exist_ok=True)
    package = output / f'pop-fe-psp_{args.version}_{arch}.deb'
    with tempfile.TemporaryDirectory(prefix='pop-fe-rust-deb-') as temporary:
        stage = Path(temporary)
        app = stage / 'usr/lib/pop-fe-psp'
        install(binary, app / 'pop-fe-psp', 0o755)
        install(encoder, app / 'atracdenc', 0o755)
        install(ROOT / 'packaging/pop-fe-psp', stage / 'usr/bin/pop-fe-psp', 0o755)
        install(ROOT / 'packaging/org.popfe.PSPManager.desktop', stage / 'usr/share/applications/org.popfe.PSPManager.desktop')
        shutil.copytree(ROOT / 'packaging/icons', stage / 'usr/share/icons')

        doc = stage / 'usr/share/doc/pop-fe-psp'
        install(ROOT / 'LICENCE-LGPL-2.1.txt', doc / 'copyright')
        install(ROOT / 'packaging/README.md', doc / 'README.md')
        (doc / 'changelog.Debian').write_text(
            f'pop-fe-psp ({args.version}) unstable; urgency=medium\n\n'
            '  * Native Rust GTK4 application package.\n\n'
            f' -- POP-FE local build <noreply@localhost>  {formatdate(localtime=True)}\n',
            encoding='utf-8',
        )

        control = stage / 'DEBIAN/control'
        control.parent.mkdir()
        depends = [
            'libc6', 'libstdc++6', 'libsndfile1', 'xdelta3', 'mame-tools',
            'libgtk-4-1 (>= 4.10.0)', 'libadwaita-1-0 (>= 1.5.0)',
            'libgdk-pixbuf-2.0-0',
        ]
        control.write_text(
            f'Package: pop-fe-psp\nVersion: {args.version}\nArchitecture: {arch}\n'
            'Maintainer: POP-FE local build <noreply@localhost>\n'
            'Section: utils\nPriority: optional\n'
            f'Depends: {", ".join(depends)}\n'
            'Description: GTK 4 PlayStation disc converter for PSP\n'
            ' Native Rust application for converting and extracting EBOOT.PBP files.\n',
            encoding='utf-8',
        )
        for path in [stage, *stage.rglob('*')]:
            if path.is_dir():
                path.chmod(0o755)
        subprocess.run(['dpkg-deb', '--root-owner-group', '--build', str(stage), str(package)], check=True)
    print(package)


if __name__ == '__main__':
    main()
