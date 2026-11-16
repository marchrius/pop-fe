# POP-FE PSP Manager

**The Rust and GTK 4 rewrite of [pop-fe](https://github.com/marchrius/pop-fe)** converts PlayStation disc images into PSP `EBOOT.PBP` files and extracts supported content from existing PBP files.

The application can identify games, load matching metadata and artwork, preview the selected theme in an XMB-style screen, process multi-disc games, and report progress while operations run. It is available in English and Italian; English is the fallback language.

## Features

- Convert CUE, CCD, CHD, ISO, BIN, IMG, or ZIP disc images to `EBOOT.PBP`.
- Combine multiple disc images into one PBP and configure disc swapping.
- Extract disc images and available audio tracks from PBP files. ISO output is produced when the source data track supports it.
- Load game IDs, metadata, assets, manuals, and compatibility fixes automatically when a disc is analyzed.
- Preview ICON0, PIC0, PIC1, and the selected theme using the actual artwork.
- Add manual PPF/Xdelta patches and override artwork or manuals.
- Show operation progress and allow cancellable analysis, conversion, theme loading, and extraction.

## Install

See the [installation and build guide](packaging/README.md) for Linux packages and source builds. macOS and Windows release packages are published on the [GitHub Releases](https://github.com/marchrius/pop-fe/releases) page when available.

## Usage

See the [user guide](docs/USER_GUIDE.md) for conversion, extraction, options, artwork, English screenshots, and troubleshooting.

## Project history

This repository contains a Rust rewrite of the original Python `pop-fe` project. The older scripts and their documentation remain for historical and compatibility reference; see [README.legacy.md](README.legacy.md) for the old command-line guide. The current desktop application is the GTK 4 Rust program built from `src/`.

## License

The Rust application is distributed under LGPL-2.1-or-later. See [LICENSE](LICENCE-LGPL-2.1.txt) and the bundled third-party notices in `licenses/`.
