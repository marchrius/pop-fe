# Pacchetto Debian

Il pacchetto contiene l'applicazione GTK4/libadwaita compilata in Rust e
l'encoder ATRAC3 usato per le tracce audio. Non installa né avvia script Python.

Per compilare dal repository servono Rust/Cargo, CMake, un compilatore C/C++,
GTK 4.10 o superiore, libadwaita 1.5 o superiore e `libsndfile1-dev`.
Il pacchetto risultante include il binario `atracdenc`; sul sistema di
destinazione sono necessarie solo le librerie runtime elencate nel control.

```sh
python3 packaging/build_deb.py --offline
sudo apt install ./dist/pop-fe-psp_0.1.6_amd64.deb
```

Rimuovi `--offline` se Cargo deve scaricare dipendenze non presenti nella cache.
Lo script compila prima l'app Rust in release, compila `atracdenc` se manca e
crea il `.deb` in `dist/` senza privilegi di root. L'architettura del pacchetto
segue quella del sistema di build.

Il pacchetto dichiara GTK4 e libadwaita come dipendenze di sistema. La GUI usa
la lingua di sistema (italiano e inglese, inglese come fallback); le preferenze
dei dialoghi sono conservate in
`~/.config/org.popfe.PSPManager/dialogs.json`. Le immagini scaricate e i file
temporanei usano `$XDG_CACHE_HOME/org.popfe.PSPManager`.

Avvio dal menu applicazioni oppure con `pop-fe-psp`. Conversione ed estrazione
vengono eseguite in background con indicatore di avanzamento e cancellazione.
L'applicazione non include immagini disco dei giochi.
