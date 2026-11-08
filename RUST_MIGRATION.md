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
4. Writer PBP/SFO/PSISO e avvio asincrono dalla GUI: prototipo per una sola
   immagine per conversione; accetta BIN/IMG raw, ISO 2048 e CUE a traccia
   singola.
   Il database fornisce titolo, CDDA e download/cache delle immagini PIC0,
   PIC1 e ICON0 quando gli URL sono disponibili; le anteprime vengono mostrate
   nella pagina Converti.
5. Selezione e preview XMB: caricamento immagini del tema 98DEMAKE. I temi
   pittorici generativi sono elencati ma i loro filtri non sono ancora portati.
6. Supporto completo a CUE multi-traccia, CCD/CHD, multi-disco, audio,
   patch e LibCrypt.
7. Progress percentuale, packaging `.deb`/Flatpak Rust e
   rimozione del backend Python dal pacchetto.

Durante la migrazione gli script Python restano la versione completa e il
riferimento di comportamento. Il crate non è ancora un sostituto completo:
la conversione Rust non supporta ancora CUE multi-traccia, più dischi, tracce
CDDA, patch di compatibilità o i filtri dei temi generativi; non va usata come
sostituto per generare EBOOT destinati alla PSP finché il porting non sarà
validato.

Per compilare la GUI su Debian/Ubuntu sono richiesti i pacchetti di sviluppo
GTK4 e libadwaita, oltre a Rust/Cargo.
