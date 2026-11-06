# Pacchetto Ubuntu

Costruzione dal repository con `atracdenc/src/atracdenc` già compilato:

```sh
python3 packaging/build_deb.py
sudo apt install ./dist/pop-fe-psp_0.1.0_amd64.deb
```

Il pacchetto è nativo per la distribuzione e l'architettura della macchina di
build (attualmente Ubuntu 26.04 amd64). Le librerie native hanno dipendenze
versionate: non è un pacchetto universale per tutte le versioni Debian/Ubuntu.
La GUI usa Python e GTK/libadwaita di sistema. Il virtualenv di sviluppo non
viene incluso. APT installa le dipendenze dichiarate.

Avvio dal menu applicazioni oppure con `pop-fe-psp`.
L'app mostra fase corrente, indicatore animato e tempo trascorso durante
analisi, conversione ed estrazione. Il pulsante Interrompi arresta anche
i processi ausiliari. Il progress è indeterminato quando il backend non
espone una percentuale attendibile.
Cache e file di lavoro: `$XDG_CACHE_HOME/org.popfe.PSPManager`, normalmente
`~/.cache/org.popfe.PSPManager`. Gli script e le risorse installate restano
in `/usr/share/pop-fe-psp`.

CHD richiede `mame-tools`; patch Xdelta richiedono `xdelta3`.
PSX-undither e libcrypt-patcher esterno non sono inclusi in questa build.
Non selezionare PSX-undither senza il relativo eseguibile.
Il pacchetto non contiene immagini disco dei giochi.
