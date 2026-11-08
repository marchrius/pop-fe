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
4. Writer PBP/SFO/PSISO e avvio asincrono dalla GUI: prototipo funzionante
   solo per una singola immagine BIN/IMG raw da 2352 byte.
5. Supporto completo a CUE multi-traccia, ISO/CCD/CHD, multi-disco, audio,
   patch, LibCrypt, acquisizione automatica di asset/temi e anteprima XMB.
6. Progress dettagliato/cancellazione, packaging `.deb`/Flatpak Rust e
   rimozione del backend Python dal pacchetto.

Durante la migrazione gli script Python restano la versione completa e il
riferimento di comportamento. Il crate non è ancora un sostituto completo:
la conversione Rust non supporta ancora i formati più comuni `.cue`/`.iso`,
più dischi o le patch di compatibilità, e non va usata come sostituto per
generare EBOOT destinati alla PSP finché il porting non sarà validato.

Per compilare la GUI su Debian/Ubuntu sono richiesti i pacchetti di sviluppo
GTK4 e libadwaita, oltre a Rust/Cargo.
