# Pacchetti Debian e Flatpak

Entrambi i pacchetti contengono l'applicazione GTK4/libadwaita compilata in Rust, l'encoder ATRAC3, yt-dlp standalone e Deno. Su x86_64 includono anche il patcher LibCrypt standalone. Non richiedono yt-dlp, Deno, lcp o Python installati nel sistema. Le versioni incluse sono verificate con SHA-256. Deno permette a yt-dlp di risolvere le challenge JavaScript di YouTube.

La conversione accetta immagini CUE, CCD, CHD, ISO, BIN e IMG, anche contenute in archivi ZIP. Per un errore YouTube HTTP 403, è possibile selezionare nelle opzioni SND0 un file `cookies.txt` in formato Netscape esportato dal browser; l'app lo passa al yt-dlp incluso nel pacchetto. L'app preferisce sempre il downloader incluso, anche se è presente un override `POPFE_YT_DLP`.

## Debian

Servono Rust/Cargo, CMake, un compilatore C/C++, GTK 4.10 o superiore, libadwaita 1.5 o superiore, `libsndfile1-dev`, `libssl-dev`, `poppler-utils`, `ffmpeg` e `7zip` (per i manuali PDF/JP2/RAR). Il pacchetto installato include un eseguibile autonomo `yt-dlp` e non richiede Python, Deno o il pacchetto `yt-dlp` del sistema.

```sh
python3 packaging/build_deb.py --offline
sudo apt install ./dist/pop-fe-psp_0.1.98_amd64.deb
```

Al primo build, esegui il comando senza `--offline` per scaricare e verificare yt-dlp, Deno e lcp x86_64 nelle rispettive cache sotto `target/package-tools/`. In seguito puoi usare `--offline` anche per i build Cargo. Lo script compila `atracdenc` se manca, compila l'app Rust in release e crea il `.deb` in `dist/` senza privilegi di root.

## Flatpak

La build compila Rust e `atracdenc` sull'host e assembla un bundle con il runtime GNOME 51. Servono gli strumenti e le librerie di sviluppo della build Debian, Flatpak, `poppler-utils`, `ffmpeg`, `7zip`, `libssl-dev` e il runtime `org.gnome.Platform//51`. Il bundle include yt-dlp standalone e Deno; non usa le versioni installate sull'host.

```sh
python3 packaging/build_flatpak.py --offline
flatpak install --user --bundle dist/org.popfe.PSPManager.flatpak
```

Se non hai ancora creato le cache yt-dlp e Deno, esegui una volta il build senza `--offline`. Installa prima il runtime se manca: `flatpak install flathub org.gnome.Platform//51`. Il bundle richiede il runtime sul sistema destinatario; il file `.flatpak` contiene l'app, l'encoder, yt-dlp standalone e Deno, non il runtime GNOME. Il manifest di `flatpak-builder` è in `packaging/org.popfe.PSPManager.yaml`.

Avvio dal menu applicazioni oppure con `pop-fe-psp`. La lingua segue il sistema (italiano e inglese, inglese come fallback); la cartella iniziale dei dialoghi è salvata in `~/.config/org.popfe.PSPManager/dialogs.json`, mentre le opzioni di conversione selezionate e la cartella PSP sono salvate in `~/.config/org.popfe.PSPManager/preferences.json`. Le immagini e i file temporanei usano `$XDG_CACHE_HOME/org.popfe.PSPManager`.
