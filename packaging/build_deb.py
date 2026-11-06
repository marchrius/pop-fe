#!/usr/bin/env python3
"""Build a native Ubuntu .deb, without root or including the dev virtualenv."""

import argparse
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
    parser.add_argument('--version', default='0.1.0')
    args = parser.parse_args()
    if not args.version or any(c not in '0123456789.+~-' for c in args.version):
        parser.error('version must be a numeric Debian version')
    arch = subprocess.check_output(['dpkg', '--print-architecture'], text=True).strip()
    encoder = ROOT / 'atracdenc/src/atracdenc'
    if not encoder.is_file():
        parser.error('compile atracdenc before packaging (see packaging/README.md)')
    output = ROOT / 'dist'
    output.mkdir(exist_ok=True)
    package = output / f'pop-fe-psp_{args.version}_{arch}.deb'
    with tempfile.TemporaryDirectory(prefix='pop-fe-deb-') as temp:
        stage = Path(temp)
        app = stage / 'usr/share/pop-fe-psp'
        for path in ROOT.glob('*.py'):
            install(path, app / path.name)
        for directory in ('ppf', 'pspconfigs', 'romhacks'):
            shutil.copytree(ROOT / directory, app / directory)
        install(ROOT / 'binmerge/binmerge', app / 'binmerge/binmerge', 0o755)
        install(ROOT / 'Cue2cu2/cue2cu2.py', app / 'Cue2cu2/cue2cu2.py', 0o755)
        install(encoder, app / 'atracdenc/src/atracdenc', 0o755)
        for name in ('LICENCE-LGPL-2.1.txt',):
            install(ROOT / name, app / name)
        install(ROOT / 'atracdenc/LICENSE', app / 'atracdenc/LICENSE')
        install(ROOT / 'packaging/pop-fe-psp', stage / 'usr/bin/pop-fe-psp', 0o755)
        install(ROOT / 'packaging/org.popfe.PSPManager.desktop', stage / 'usr/share/applications/org.popfe.PSPManager.desktop')
        shutil.copytree(ROOT / 'packaging/icons', stage / 'usr/share/icons')
        doc = stage / 'usr/share/doc/pop-fe-psp'
        install(ROOT / 'LICENCE-LGPL-2.1.txt', doc / 'copyright')
        install(ROOT / 'packaging/README.md', doc / 'README.md')
        control = stage / 'DEBIAN/control'
        control.parent.mkdir()
        # The included encoder is compiled on this host. Pin the native library
        # minimums to their build-host versions rather than claiming portability.
        native = []
        for lib in ('libc6', 'libstdc++6', 'libsndfile1'):
            version = subprocess.check_output(['dpkg-query', '-W', '-f=${Version}', lib], text=True).strip()
            native.append(f'{lib} (>= {version})')
        depends = native + [
            'python3 (>= 3.10)', 'python3-gi', 'gir1.2-gtk-4.0 (>= 4.10)',
            'gir1.2-adw-1 (>= 1.5)', 'python3-pil', 'python3-requests',
            'python3-pycdlib', 'python3-pycryptodome', 'python3-pypdf',
            'python3-rarfile', 'python3-ecdsa', 'python3-opencv',
            'python3-scipy', 'python3-sklearn', 'yt-dlp', 'ffmpeg', 'wget',
        ]
        control.write_text(
            f'Package: pop-fe-psp\nVersion: {args.version}\nArchitecture: {arch}\n'
            'Maintainer: POP-FE local build <noreply@localhost>\n'
            'Section: utils\nPriority: optional\n'
            f'Depends: {", ".join(depends)}\n'
            'Recommends: mame-tools, xdelta3\n'
            'Description: GTK 4 PlayStation disc converter for PSP\n'
            ' Convert disc images to EBOOT.PBP and extract PBP contents.\n'
            ' Includes a native libadwaita interface, desktop launcher and icon.\n',
            encoding='utf-8',
        )
        subprocess.run(['dpkg-deb', '--root-owner-group', '--build', str(stage), str(package)], check=True)
    print(package)


if __name__ == '__main__':
    main()
