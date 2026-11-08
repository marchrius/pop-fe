# Pacchetti Debian e Flatpak

Entrambi i pacchetti contengono l'applicazione GTK4/libadwaita compilata in Rust e l'encoder ATRAC3; non installano né avviano script Python.

## Debian

Servono Rust/Cargo, CMake, un compilatore C/C++, GTK 4.10 o superiore, libadwaita 1.5 o superiore, `libsndfile1-dev`, `libssl-dev`, `poppler-utils`, `ffmpeg` e `7zip` (per i manuali PDF/JP2/RAR).

```sh
python3 packaging/build_deb.py --offline
sudo apt install ./dist/pop-fe-psp_0.1.6_amd64.deb
```

Rimuovi `--offline` se Cargo deve scaricare dipendenze. Lo script compila `atracdenc` se manca, compila l'app Rust in release e crea il `.deb` in `dist/` senza privilegi di root.

## Flatpak

La build compila Rust e `atracdenc` sull'host e assembla un bundle con il runtime GNOME 51. Servono gli strumenti e le librerie di sviluppo della build Debian, Flatpak, `poppler-utils`, `ffmpeg`, `7zip`, `libssl-dev`, `yt-dlp` (incluso nel bundle con il suo modulo Python) e il runtime `org.gnome.Platform//51`.

```sh
python3 packaging/build_flatpak.py --offline
flatpak install --user --bundle dist/org.popfe.PSPManager.flatpak
```

Installa prima il runtime se manca: `flatpak install flathub org.gnome.Platform//51`. Il bundle richiede il runtime sul sistema destinatario; il file `.flatpak` contiene l'app e l'encoder, non il runtime GNOME. Il manifest di `flatpak-builder` è in `packaging/org.popfe.PSPManager.yaml`.

Avvio dal menu applicazioni oppure con `pop-fe-psp`. La lingua segue il sistema (italiano e inglese, inglese come fallback); le preferenze dei dialoghi sono salvate in `~/.config/org.popfe.PSPManager/dialogs.json`. Le immagini e i file temporanei usano `$XDG_CACHE_HOME/org.popfe.PSPManager`.
