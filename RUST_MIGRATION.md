# Migrazione Rust

La GUI e il backend PSP vengono riscritti in Rust/GTK4. I database del
progetto sono esportati in JSON incluso nel crate. I dati binari PSP usati dal
formato PBP sono risorse incluse; non vengono eseguiti gli script Python.

## Fasi

1. Struttura GTK4/libadwaita, localizzazione italiana/inglese e dialoghi con
   cronologia cartelle: avviati.
2. Parser di CUE semplice, lettura ISO9660, MD5/SYSTEM.CNF e database giochi:
   implementati per l'analisi iniziale. La materializzazione CHD con `chdman`
   e il calcolo MD5 rispondono alla cancellazione quando si sostituisce o
   rimuove l'immagine in analisi; anche la lettura dei download artwork viene
   interrotta ai confini dei blocchi ricevuti. L'analisi iniziale mostra un
   indicatore di attività ed è annullabile; l'app attende il worker annullato
   prima di terminare.
3. Estrazione delle sezioni PBP, immagini PSISO compresse, ISO, CUE e tracce AEA:
   implementata. L'ISO viene ricostruito dai settori dati Mode 1/Mode 2 Form 1.
   Le tracce ATRAC separate restano file AEA e non producono indici CUE
   inesistenti né padding sparso nel BIN; CDDA raw resta nel BIN. La verifica
   con PBP reali è ancora necessaria. I file vengono estratti in staging e
   pubblicati nella cartella scelta solo a operazione completata; annullamento
   o errori durante la decodifica non lasciano immagini parziali.
4. Writer PBP/SFO/PSISO e avvio asincrono dalla GUI: prototipo per un massimo
   di 32 dischi; accetta BIN/IMG raw, ISO 2048, CUE, CCD, CHD e archivi ZIP con
   tracce dati e audio.
   BIN/IMG e ISO usano una CUE sintetica per calcolare la TOC in settori CD,
   mantenendo corretto il lead-out PSISO anche per input senza CUE fornita;
   gli indirizzi traccia e il frame di lead-out seguono la convenzione POP-FE.
   PSTITLEIMG resta incluso per gli EBOOT multi-disco, come richiesto dal flusso
   POPS di cambio disco.
   Le tracce audio vengono codificate in ATRAC3 con `atracdenc` e incluse nel
   PSISO assieme alla TOC; l'opzione CDDA conserva invece i settori audio raw e
   imposta i flag POPS corrispondenti.
   Il database fornisce titolo, CDDA e download/cache delle immagini PIC0,
   PIC1 e ICON0 quando gli URL sono disponibili; PIC0 rispetta anche scala e
   offset definiti per gioco nel database. Per SND0 usa il cache upstream dei
   file ATRAC3 già pronti, limitati ai vincoli PSP; le anteprime artwork vengono
   mostrate nella pagina Converti. I config POPS specifici per gioco e i dati
   LibCrypt vengono inclusi automaticamente nel PSISO.
5. Selezione e preview XMB: caricamento immagini del tema 98DEMAKE e
   generazione locale di ASCIIART, DOTPAINTING, OILPAINTING, WATERCOLOR e
   COLORSKETCH. Gli effetti sono implementazioni Rust leggere e compatibili
   con le dimensioni PSP; non replicano esattamente i filtri OpenCV originali.
   Il caricamento del tema può essere annullato, si interrompe prima di caricare
   la nuova selezione e la chiusura dell'app attende la terminazione del worker.
6. PPF2 e PPF3 metodo 2 sono inclusi e vengono applicati automaticamente sui
   BIN/IMG raw e sui CUE con la traccia dati raw come prima traccia, anche con
   tracce audio successive, quando ID/hash corrispondono.
   Sono incluse anche le patch PPF LibCrypt presenti in `libcrypt/`, applicate
   automaticamente per i giochi nel cui record è indicato un file PPF; le
   varianti upstream con revisioni note vengono selezionate tramite MD5 del
   dump, compresa la revisione di Mánager de Liga che non richiede patch. La
   casella «Disable LibCrypt Patching» ne consente la disattivazione.
   È portato anche il fix Xdelta registrato per Septentrion (richiede `xdelta3`,
   fornito dai pacchetti Debian e Flatpak). GUI e backend accettano CHD
   tramite `chdman` e CCD tramite parsing nativo; i pacchetti includono il
   decoder CHD.
7. Conversione espone CDDA, Force NTSC, Disc Swapping, watermark, PSX-undither,
   disattivazione SND0 e PSTITLEIMG e PIC1 come logo. SND0 personalizzato si
   converte in ATRAC3, si limita ai vincoli PSP e può essere cercato/scaricato
   da YouTube tramite yt-dlp. ICON0 può usare la cornice PSN NTSC-U originale.
   Conversione ed estrazione
   espongono percentuali determinate e cancellazione.
   La GUI consente di selezionare patch romhack PPF/Xdelta, abbinate alle
   immagini disco nell'ordine selezionato. La sequenza è fix PPF automatico,
   Xdelta automatico, romhack selezionato e patch LibCrypt.
   I file `.pspconfig` accanto all'immagine vengono usati come fallback alle
   configurazioni specifiche incluse nel database.
   Il `.deb` Rust è generato da `packaging/build_deb.py`; il bundle Flatpak usa
   GNOME 51 ed è creato da `packaging/build_flatpak.py`. L'avvio della GUI nel
   sandbox e su PSP resta da verificare.

Durante la migrazione gli script Python restano il riferimento di comportamento.
Il crate non è ancora un sostituto completo:
La conversione standard ricodifica le tracce audio in ATRAC3; la modalità CDDA
esplicita conserva invece l'audio raw e imposta i flag POPS. Il
database descrive 230 ID LibCrypt: 229 hanno patch automatiche nel database, o
sono revisioni note senza protezione, con un caso di Final Fantasy VIII PAL che
usa ancora un PPF zip esterno. Le revisioni non riconosciute possono usare `lcp`
se disponibile nel PATH o in `POPFE_LCP`; i pacchetti non lo includono.
Non usare ancora il port come sostituto per generare EBOOT destinati alla PSP
finché non sarà validato su immagini e hardware reali. La decifratura DES dei
manuali PS3 PGD e la conversione in `DOCUMENT.DAT` non sono ancora state
verificate con file campione. I filtri pittorici Rust sono approssimazioni dei
filtri OpenCV.

Per compilare la GUI su Debian/Ubuntu sono richiesti i pacchetti di sviluppo
GTK4 e libadwaita, oltre a Rust/Cargo.
