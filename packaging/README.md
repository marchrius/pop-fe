# Pacchetto Ubuntu

Costruzione dal repository con `atracdenc/src/atracdenc` già compilato:

```sh
python3 packaging/build_deb.py
sudo apt install ./dist/pop-fe-psp_0.1.4_amd64.deb
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

Dopo il caricamento dei dati di un disco, seleziona il tema e premi
**Anteprima XMB**. Una finestra separata mostra la composizione PSP a 480×272
con ICON0, PIC0 e PIC1 reali. Il tema viene preparato in background e le
immagini vengono risolte con la stessa funzione usata per creare il PBP.
Si tratta di una simulazione grafica, senza riproduzione audio.

L'interfaccia segue la lingua di sistema: italiano e inglese sono supportati,
con inglese come fallback. Per una singola esecuzione dal terminale:
`LANGUAGE=it pop-fe-psp` oppure `LANGUAGE=en pop-fe-psp`.
Chiudi prima eventuali istanze già aperte. I cataloghi UTF-8 sono in `locales/`.
I messaggi diagnostici emessi dagli strumenti esterni conservano la loro lingua.

Tutti i dialoghi condividono l'ultima cartella selezionata, salvata in
`~/.config/org.popfe.PSPManager/dialogs.json` (oppure sotto `$XDG_CONFIG_HOME`).
Se la cartella è stata rimossa, smontata o non è accessibile si apre la home.
Annullare il dialogo non modifica la preferenza.
Cache e file di lavoro: `$XDG_CACHE_HOME/org.popfe.PSPManager`, normalmente
`~/.cache/org.popfe.PSPManager`. Gli script e le risorse installate restano
in `/usr/share/pop-fe-psp`.

CHD richiede `mame-tools`; patch Xdelta richiedono `xdelta3`.
PSX-undither e libcrypt-patcher esterno non sono inclusi in questa build.
Non selezionare PSX-undither senza il relativo eseguibile.
Il pacchetto non contiene immagini disco dei giochi.
