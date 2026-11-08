# Migrazione Rust

La GUI e il backend PSP vengono riscritti in Rust/GTK4. I database del
progetto sono esportati in JSON incluso nel crate. I dati binari PSP usati dal
formato PBP sono risorse incluse; non vengono eseguiti gli script Python.

## Fasi

1. Struttura GTK4/libadwaita, localizzazione italiana/inglese e dialoghi con
   cronologia cartelle: avviati.
2. Parser di CUE semplice, lettura ISO9660, MD5/SYSTEM.CNF e database giochi:
   implementati per l'analisi iniziale.
3. Estrazione delle sezioni PBP, immagini PSISO compresse, CUE e tracce AEA:
   implementata, da verificare con PBP reali.
4. Writer PBP/SFO/PSISO e avvio asincrono dalla GUI: prototipo per un massimo
   di 32 dischi; accetta BIN/IMG raw, ISO 2048 e CUE con tracce dati e audio.
   Le tracce audio vengono codificate in ATRAC3 con `atracdenc` e incluse nel
   PSISO assieme alla TOC.
   Il database fornisce titolo, CDDA e download/cache delle immagini PIC0,
   PIC1 e ICON0 quando gli URL sono disponibili; le anteprime vengono mostrate
   nella pagina Converti. I config POPS specifici per gioco e i dati LibCrypt
   vengono inclusi automaticamente nel PSISO.
5. Selezione e preview XMB: caricamento immagini del tema 98DEMAKE. I temi
   pittorici generativi sono elencati ma i loro filtri non sono ancora portati.
6. Restano CCD/CHD, CDDA senza ricodifica, Xdelta e ulteriori opzioni audio.
   PPF2 e PPF3 metodo 2 sono inclusi e vengono applicati automaticamente sui
   BIN/IMG raw e sui CUE raw a traccia singola quando ID/hash corrispondono.
7. Conversione ed estrazione espongono percentuali determinate e cancellazione.
   Il `.deb` Rust è generato da `packaging/build_deb.py`; il bundle Flatpak usa
   GNOME 51 ed è creato da `packaging/build_flatpak.py`. L'avvio della GUI nel
   sandbox e su PSP resta da verificare.

Durante la migrazione gli script Python restano la versione completa e il
riferimento di comportamento. Il crate non è ancora un sostituto completo:
la conversione Rust non supporta ancora CCD/CHD, CDDA senza ricodifica,
Xdelta o i filtri dei temi generativi; non va usata come
sostituto per generare EBOOT destinati alla PSP finché il porting non sarà
validato.

Per compilare la GUI su Debian/Ubuntu sono richiesti i pacchetti di sviluppo
GTK4 e libadwaita, oltre a Rust/Cargo.
