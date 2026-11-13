# Pacchetti Debian e Flatpak

Entrambi i pacchetti contengono l'applicazione GTK4/libadwaita compilata in Rust, l'encoder ATRAC3, yt-dlp standalone e Deno. Su x86_64 includono anche il patcher LibCrypt standalone. Non richiedono yt-dlp, Deno, lcp o Python installati nel sistema. Le versioni incluse sono verificate con SHA-256. Deno permette a yt-dlp di risolvere le challenge JavaScript di YouTube.

La conversione accetta immagini CUE, CCD, CHD, ISO, BIN e IMG, anche contenute in archivi ZIP. Il patcher LibCrypt standalone è incluso nei bundle x86_64 e viene verificato tramite il digest SHA-256 pubblicato nella release GitHub upstream. Per un errore YouTube HTTP 403, è possibile selezionare nelle opzioni SND0 un file `cookies.txt` in formato Netscape esportato dal browser; l'app lo passa al yt-dlp incluso nel pacchetto. L'app preferisce sempre il downloader incluso, anche se è presente un override `POPFE_YT_DLP`.

## Debian

Servono Rust/Cargo, CMake, un compilatore C/C++, `libclang-dev`, GTK 4.10 o superiore, libadwaita 1.5 o superiore, `libopencv-contrib-dev`, `libsndfile1-dev`, `libssl-dev`, `poppler-utils`, `ffmpeg` e `7zip` (per i manuali PDF/JP2/RAR). Il `.deb` dichiara le dipendenze condivise rilevate dalla build; il Flatpak include OpenCV. Il pacchetto installato include un eseguibile autonomo `yt-dlp` e non richiede Python, Deno o il pacchetto `yt-dlp` del sistema.

```sh
python3 packaging/build_deb.py --offline
sudo apt install ./dist/pop-fe-psp_0.1.101_amd64.deb
```

Al primo build, esegui il comando senza `--offline` per scaricare e verificare yt-dlp, Deno e lcp x86_64 nelle rispettive cache sotto `target/package-tools/`. In seguito puoi usare `--offline` anche per i build Cargo. Lo script compila `atracdenc` se manca, compila l'app Rust in release e crea il `.deb` in `dist/` senza privilegi di root.

## Flatpak

La build compila Rust e `atracdenc` sull'host e assembla un bundle con il runtime GNOME 51. Servono gli strumenti e le librerie di sviluppo della build Debian, Flatpak, `poppler-utils`, `ffmpeg`, `7zip`, `libssl-dev` e il runtime `org.gnome.Platform//51`. Il bundle include yt-dlp standalone e Deno; non usa le versioni installate sull'host.

```sh
python3 packaging/build_flatpak.py --offline
flatpak install --user --bundle dist/org.popfe.PSPManager.flatpak
```

Se non hai ancora creato le cache yt-dlp e Deno, esegui una volta il build senza `--offline`. Installa prima il runtime se manca: `flatpak install flathub org.gnome.Platform//51`. Il bundle richiede il runtime sul sistema destinatario; il file `.flatpak` contiene l'app, l'encoder, yt-dlp standalone e Deno, non il runtime GNOME. Il manifest di `flatpak-builder` è in `packaging/org.popfe.PSPManager.yaml`.

## macOS e Windows (build da sorgente)

La CI compila l'app per macOS 26 Intel e Apple Silicon, macOS 27 Apple Silicon (runner preview) e Windows x86_64. La build macOS 26 verifica il target stabile Intel/Apple Silicon; quella macOS 27 controlla la compatibilità con la versione più recente disponibile sui runner. Per build locali su macOS servono Xcode Command Line Tools, Rust, CMake e le dipendenze native seguenti:

```sh
brew install gtk4 libadwaita opencv libsndfile openssl@3 pkg-config cmake ffmpeg poppler sevenzip rom-tools xdelta dylibbundler librsvg
OPENSSL_DIR="$(brew --prefix openssl@3)" \
OPENSSL_LIB_DIR="$(brew --prefix openssl@3)/lib" \
OPENSSL_INCLUDE_DIR="$(brew --prefix openssl@3)/include" \
PKG_CONFIG_PATH="$(brew --prefix openssl@3)/lib/pkgconfig:${PKG_CONFIG_PATH}" \
RUSTFLAGS="-L native=$(brew --prefix openssl@3)/lib" cargo build --locked --release
cmake -S atracdenc/src -B target/atracdenc-build -DCMAKE_BUILD_TYPE=Release
cmake --build target/atracdenc-build --parallel
python3 packaging/build_macos_app.py
```

Su Windows si usa MSYS2 UCRT64 con GTK 4, libadwaita, OpenCV, libsndfile, OpenSSL, CMake, Ninja e la toolchain Rust GNU x86_64. L'app va avviata dall'ambiente UCRT64 affinché GTK e OpenCV trovino le DLL e i dati di runtime. Il workflow `.github/workflows/rust-platforms.yml` contiene la configurazione di build e packaging.

In UCRT64, la build locale usa gli stessi pacchetti della CI:

```sh
pacman -S --needed base-devel mingw-w64-ucrt-x86_64-toolchain mingw-w64-ucrt-x86_64-cmake mingw-w64-ucrt-x86_64-ninja mingw-w64-ucrt-x86_64-gtk4 mingw-w64-ucrt-x86_64-libadwaita mingw-w64-ucrt-x86_64-opencv mingw-w64-ucrt-x86_64-libsndfile mingw-w64-ucrt-x86_64-openssl mingw-w64-ucrt-x86_64-pkgconf mingw-w64-ucrt-x86_64-clang mingw-w64-ucrt-x86_64-python mingw-w64-ucrt-x86_64-ffmpeg mingw-w64-ucrt-x86_64-poppler mingw-w64-ucrt-x86_64-7zip xdelta3
rustup toolchain install stable-x86_64-pc-windows-gnu --profile minimal
cargo +stable-x86_64-pc-windows-gnu build --locked --release
cmake -S atracdenc/src -B target/atracdenc-build -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build target/atracdenc-build --parallel
python packaging/build_windows_portable.py
```

Gli artefatti CI contengono una `.app` macOS per ciascuna architettura e uno ZIP Windows portabile. Su Windows il bundle include GTK/OpenCV, OpenSSL con il provider legacy, encoder ATRAC3, yt-dlp, Deno, i patcher LibCrypt e Xdelta, FFmpeg, Poppler e 7-Zip. Per le immagini CHD, scarica la distribuzione Windows ufficiale di [MAME](https://www.mamedev.org/release.html) ed estrai `chdman.exe` e le DLL adiacenti nella cartella dell'app; in alternativa aggiungi quella cartella al `PATH`. Il patcher LibCrypt macOS incluso è Intel x86_64 e sui Mac Apple Silicon richiede Rosetta. Gli artefatti CI non sono firmati/notarizzati per la distribuzione pubblica.

Su macOS estrai lo ZIP, sposta `POP-FE PSP Manager.app` in `Applicazioni` e aprila. Essendo un artefatto non firmato/notarizzato, al primo avvio macOS può richiedere l'apertura dal menu contestuale. Su Windows estrai lo ZIP in una cartella e avvia `Start POP-FE PSP Manager.cmd`.

Avvio Linux dal menu applicazioni oppure con `pop-fe-psp`. La lingua segue il sistema (italiano e inglese, inglese come fallback). La configurazione è salvata nella cartella standard della piattaforma: `~/.config/org.popfe.PSPManager` su Linux, `~/Library/Application Support/org.popfe.PSPManager` su macOS e `%APPDATA%/org.popfe.PSPManager` su Windows. Cache e file temporanei usano la cartella cache standard del sistema.
