# Installation and builds

POP-FE PSP Manager is the Rust/GTK 4 rewrite of the original Python `pop-fe` project. These instructions apply to the current Rust application. The legacy scripts in the repository have separate dependencies and workflows.

## Linux: Debian and Fedora packages

GitHub releases include `.deb` packages for Debian/Ubuntu and `.rpm` packages built on Fedora, for x86_64 and AArch64. Building requires Rust/Cargo, CMake, a C/C++ compiler, `libclang`, GTK 4.10+, libadwaita 1.5+, OpenCV contrib development files, libsndfile development files, OpenSSL development files, Poppler utilities, FFmpeg, and 7-Zip. `chdman` from MAME tools is needed to read CHD images. Packages include standalone `yt-dlp` and Deno; they do not require Python, Deno, or the system `yt-dlp` package at runtime.

On Debian or Ubuntu, install the native build dependencies and run the GUI from the checkout with:

```sh
sudo apt install build-essential cmake pkg-config libclang-dev libgtk-4-dev libadwaita-1-dev libopencv-contrib-dev libsndfile1-dev libssl-dev poppler-utils ffmpeg p7zip-full
cargo run --locked
```

Build and install:

```sh
python3 packaging/build_deb.py
sudo apt install ./dist/pop-fe-psp_*_amd64.deb
```

On Fedora 45, install the native build dependencies, then build an RPM:

```sh
sudo dnf install gcc gcc-c++ cmake pkgconf-pkg-config clang-devel gtk4-devel \
  libadwaita-devel opencv-devel libsndfile-devel openssl-devel python3 rpm-build
python3 packaging/build_rpm.py
sudo dnf install ./dist/pop-fe-psp-*.rpm
```

After the package tools have been downloaded and verified once, `--offline` can be used for a subsequent build:

```sh
python3 packaging/build_deb.py --offline
```

The script creates the package under `dist/` without running the build as root.

## Linux: Flatpak bundle

The Flatpak build also runs locally. It requires the Linux build dependencies above, Flatpak/Flatpak Builder, and the GNOME Platform runtime version listed in `packaging/org.popfe.PSPManager.yaml`. The resulting bundle includes the application and standalone `yt-dlp`/Deno, but the target system must have the referenced GNOME runtime installed.

```sh
flatpak install flathub org.gnome.Platform//51
python3 packaging/build_flatpak.py
flatpak install --user --bundle dist/org.popfe.PSPManager.flatpak
```

Use `python3 packaging/build_flatpak.py --offline` after the required package caches have been populated. The manifest is `packaging/org.popfe.PSPManager.yaml`.

## macOS

GitHub Actions builds macOS 26 packages for Intel and Apple Silicon and a macOS 27 preview package for Apple Silicon. These are ZIP files containing an `.app`. They are not signed or notarized. Local builds require Xcode Command Line Tools, Rust, CMake, Python 3, and the Homebrew dependencies below:

```sh
brew install gtk4 libadwaita opencv libsndfile openssl@3 llvm pkg-config cmake ffmpeg poppler sevenzip rom-tools xdelta dylibbundler librsvg
```

Then build the application and helpers:

```sh
OPENSSL_DIR="$(brew --prefix openssl@3)" \
OPENSSL_LIB_DIR="$(brew --prefix openssl@3)/lib" \
OPENSSL_INCLUDE_DIR="$(brew --prefix openssl@3)/include" \
PKG_CONFIG_PATH="$(brew --prefix openssl@3)/lib/pkgconfig:${PKG_CONFIG_PATH}" \
LIBCLANG_PATH="$(brew --prefix llvm)/lib" \
RUSTFLAGS="-L native=$(brew --prefix openssl@3)/lib -C link-arg=-Wl,-rpath,$(brew --prefix llvm)/lib -C link-arg=-Wl,-headerpad_max_install_names" \
cargo build --locked --release
cmake -S atracdenc/src -B target/atracdenc-build -DCMAKE_BUILD_TYPE=Release -DCMAKE_POLICY_VERSION_MINIMUM=3.5
cmake --build target/atracdenc-build --parallel
python3 packaging/build_macos_app.py
```

Extract the ZIP and move `POP-FE PSP Manager.app` to Applications. Because the app is not signed or notarized, macOS may require opening it from Finder's context menu the first time. CHD support requires `chdman` from MAME tools on `PATH`.

## Windows

GitHub Actions builds portable x86_64 and ARM64 ZIPs. x86_64 uses MSYS2 UCRT64 and the GNU Rust toolchain; ARM64 uses the native MSYS2 CLANGARM64 environment and Rust's Windows GNU-LLVM target. Both ZIPs include the GTK/OpenCV runtime, OpenSSL, ATRAC3 encoder, `yt-dlp`, Deno, FFmpeg, Poppler, 7-Zip, and Xdelta. The x86_64 ZIP also includes the LibCrypt patcher; no upstream Windows ARM64 build is available. These ZIPs are not installers. Extract one and run `Start POP-FE PSP Manager.cmd`.

CHD support requires `chdman.exe` from the official MAME distribution and its adjacent DLLs. Put them beside the application or add their directory to `PATH`.

To build locally, use an MSYS2 UCRT64 shell and install the same packages listed in `.github/workflows/rust-platforms.yml`. Then run:

```sh
rustup toolchain install stable-x86_64-pc-windows-gnu --profile minimal
cargo +stable-x86_64-pc-windows-gnu build --locked --release
git apply --directory=atracdenc -p1 packaging/patches/atracdenc-windows-gcc.patch
cmake -S atracdenc/src -B target/atracdenc-build -G Ninja -DCMAKE_BUILD_TYPE=Release -DCMAKE_POLICY_VERSION_MINIMUM=3.5
cmake --build target/atracdenc-build --parallel
python packaging/build_windows_portable.py
```

The patch adapts the ATRAC3 encoder source for the Windows MinGW build. Do not apply it repeatedly to the same checkout.

## GitHub releases

The release workflow builds Linux `.deb` and Fedora `.rpm` packages for x86_64 and AArch64, macOS Intel and Apple Silicon packages, and portable Windows x86_64 and ARM64 ZIPs when a `v*` tag is pushed. The Windows ARM64 ZIP uses native GTK/OpenCV builds and does not include the LibCrypt patcher, which has no upstream ARM64 release. Flatpak bundles are built locally and are not attached by that workflow.

```sh
git tag v<version>
git push origin v<version>
```

The release workflow expects every platform build to succeed before publishing the release. Check the version in `Cargo.toml` and the workflow status before tagging a production release.

## Runtime paths and language

The application follows the system language when it is Italian or English, and otherwise uses English. Configuration is stored under `org.popfe.PSPManager` in the platform's standard configuration directory; cache and temporary files use the platform cache directory.
