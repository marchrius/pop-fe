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
   di 32 dischi; accetta BIN/IMG raw, ISO 2048, CUE, CCD e CHD con tracce dati e audio.
   Le tracce audio vengono codificate in ATRAC3 con `atracdenc` e incluse nel
   PSISO assieme alla TOC; l'opzione CDDA conserva invece i settori audio raw e
   imposta i flag POPS corrispondenti.
   Il database fornisce titolo, CDDA e download/cache delle immagini PIC0,
   PIC1 e ICON0 quando gli URL sono disponibili; le anteprime vengono mostrate
   nella pagina Converti. I config POPS specifici per gioco e i dati LibCrypt
   vengono inclusi automaticamente nel PSISO.
5. Selezione e preview XMB: caricamento immagini del tema 98DEMAKE e
   generazione locale di ASCIIART, DOTPAINTING, OILPAINTING, WATERCOLOR e
   COLORSKETCH. Gli effetti sono implementazioni Rust leggere e compatibili
   con le dimensioni PSP; non replicano esattamente i filtri OpenCV originali.
6. PPF2 e PPF3 metodo 2 sono inclusi e vengono applicati automaticamente sui
   BIN/IMG raw e sui CUE raw a traccia singola quando ID/hash corrispondono.
   È portato anche il fix Xdelta registrato per Septentrion (richiede `xdelta3`,
   fornito dai pacchetti Debian e Flatpak). Restano gli altri romhack Xdelta e
   ulteriori opzioni audio. GUI e backend accettano CHD
   tramite `chdman` e CCD tramite parsing nativo; i pacchetti includono il
   decoder CHD.
7. Conversione espone CDDA, Force NTSC, Disc Swapping, watermark, PSX-undither,
   disattivazione PSTITLEIMG e PIC1 come logo. Conversione ed estrazione
   espongono percentuali determinate e cancellazione.
   Il `.deb` Rust è generato da `packaging/build_deb.py`; il bundle Flatpak usa
   GNOME 51 ed è creato da `packaging/build_flatpak.py`. L'avvio della GUI nel
   sandbox e su PSP resta da verificare.

Durante la migrazione gli script Python restano la versione completa e il
riferimento di comportamento. Il crate non è ancora un sostituto completo:
la conversione Rust non supporta ancora CDDA senza ricodifica e tutti
gli altri romhack Xdelta o il patcher LibCrypt `lcp`; non va usata come
sostituto per generare EBOOT destinati alla PSP finché il porting non sarà
validato. I filtri pittorici Rust sono approssimazioni dei filtri OpenCV.

Per compilare la GUI su Debian/Ubuntu sono richiesti i pacchetti di sviluppo
GTK4 e libadwaita, oltre a Rust/Cargo.
