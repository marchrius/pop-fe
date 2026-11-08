# Migrazione Rust

La GUI e il backend PSP verranno riscritti in Rust/GTK4. I database e gli
asset del progetto restano dati di input e verranno convertiti in formati
serializzabili quando il nuovo backend li userà; gli eseguibili specializzati
come `atracdenc`, `ffmpeg` e gli strumenti di patch potranno restare processi
esterni.

## Fasi

1. Crate GTK4/libadwaita, localizzazione e struttura delle due pagine.
2. Dialoghi, cronologia cartelle, analisi disco e flussi cancellabili.
3. Parser CUE/CCD/ISO, riconoscimento giochi, patch e generazione PBP.
4. Estrazione PBP, caricamento automatico degli asset e anteprima XMB.
5. Packaging `.deb`/Flatpak Rust e rimozione del backend Python dal pacchetto.

Durante la migrazione gli script Python restano la versione funzionante e il
riferimento di comportamento. Il crate non è ancora il sostituto completo.

Per compilare la GUI su Debian/Ubuntu sono richiesti i pacchetti di sviluppo
GTK4 e libadwaita, oltre a Rust/Cargo.
