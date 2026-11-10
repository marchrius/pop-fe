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
   rimuove l'immagine in analisi. Il parser CUE ricrea anche l'INDEX 00
   virtuale di 150 frame per le tracce audio che non lo dichiarano, come il
   parser originale. I nomi FILE CUE non quotati possono contenere spazi e
   vengono separati dal tipo usando l'ultimo campo; gli indici non supportati
   vengono rifiutati esplicitamente; la numerazione deve essere consecutiva
   da 01 (massimo 99 tracce), ogni traccia deve dichiarare `INDEX 01` e gli
   indici `0`/`1` senza zero iniziale restano compatibili.
   Anche la lettura dei download artwork viene
   interrotta ai confini dei blocchi ricevuti. I processi esterni vengono
   isolati in gruppi e terminati insieme ai discendenti quando si annulla.
   L'analisi iniziale mostra un
   indicatore di attività ed è annullabile; l'app attende il worker annullato
   prima di terminare.
3. Estrazione delle sezioni PBP, immagini PSISO compresse, ISO, CUE, TOC e
   tracce AEA:
   implementata. L'ISO viene ricostruito dai settori dati Mode 1/Mode 2 Form 1;
   la modalità della traccia nel CUE estratto viene dedotta dal primo settore.
   L'estrazione conserva anche HEADER.DAT e STARTDAT.DAT come il dumper
   originale.
   Le tracce ATRAC separate restano file AEA e non producono indici CUE
   inesistenti né padding sparso nel BIN; CDDA raw resta nel BIN. La verifica
   con PBP reali è ancora necessaria. I file vengono estratti in staging e
   pubblicati nella cartella scelta solo a operazione completata; annullamento
   o errori durante la decodifica non lasciano immagini parziali.
4. Writer PBP/SFO/PSISO e avvio asincrono dalla GUI: prototipo per un massimo
   di 32 dischi; accetta BIN/IMG raw, ISO 2048, CUE, CCD, CHD e archivi ZIP con
   tracce dati e audio. Se si seleziona direttamente un BIN/IMG con un CUE
   omonimo accanto, anche con differenze di maiuscole nel nome base, il CUE
   viene caricato automaticamente per conservare TOC e
   tracce audio.
   BIN/IMG e ISO usano una CUE sintetica per calcolare la TOC in settori CD,
   mantenendo corretto il lead-out PSISO anche per input senza CUE fornita;
   gli indirizzi traccia e il frame di lead-out seguono la convenzione POP-FE.
   PSTITLEIMG resta incluso per gli EBOOT multi-disco, come richiesto dal flusso
   POPS di cambio disco.
   Le tracce audio vengono codificate in ATRAC3 con `atracdenc` e incluse nel
   PSISO assieme alla TOC; l'opzione CDDA conserva invece i settori audio raw e
   imposta i flag POPS corrispondenti.
   Il database fornisce titolo, CDDA e download/cache delle immagini PIC0,
   PIC1 e ICON0 quando gli URL sono disponibili. Sono ripristinati anche il
   cache upstream per hash, i sidecar accanto all'immagine e i fallback alle
   copertine/screenshot PSX Data Center quando il database non definisce
   l'immagine; i campi null espliciti continuano a disabilitare il fallback.
   PIC0 rispetta anche scala e offset definiti per gioco nel database,
   modificabili dalla GUI con anteprima aggiornata in tempo reale; il PIC0 a
   1000×560 è quello effettivamente incluso nel PBP. Per SND0 usa l'audio del tema quando
   disponibile, altrimenti il cache upstream dei file ATRAC3 già pronti; l'audio
   viene limitato ai vincoli PSP; se YouTube risponde HTTP 403, il download
   riprova automaticamente tramite IPv4 e abilita i componenti EJS remoti di
   yt-dlp tramite Deno per risolvere le challenge JavaScript di YouTube. Le
   anteprime artwork vengono mostrate nella
   pagina Converti. I config POPS specifici per gioco e i dati
   LibCrypt vengono inclusi automaticamente nel PSISO.
5. Selezione e preview XMB: caricamento immagini e SND0 dei temi 98DEMAKE e
   generazione locale di ASCIIART, DOTPAINTING, OILPAINTING, WATERCOLOR e
   COLORSKETCH. Il PIC0 segue il flusso di disegno e ridimensionamento della
   GUI originale; i filtri pittorici PIC1 restano approssimazioni Rust dei
   filtri OpenCV originali.
   Il caricamento del tema può essere annullato, si interrompe prima di caricare
   la nuova selezione e la chiusura dell'app attende la terminazione del worker.
6. PPF2 e PPF3 metodo 2 sono inclusi e vengono applicati automaticamente sui
   BIN/IMG raw e sui CUE con la traccia dati raw come prima traccia, anche con
   tracce audio successive, quando ID/hash corrispondono.
   Sono incluse le patch PPF LibCrypt locali e upstream; vengono applicate
   automaticamente per i giochi mappati nel database. Gli alias con lo stesso
   URL Redump e la stessa magic word condividono la patch upstream solo quando
   la corrispondenza è univoca. Le varianti con revisioni note vengono
   selezionate tramite MD5 del dump e firme byte/offset per le revisioni
   coperte da upstream, compresa la revisione di Mánager de Liga che non
   richiede patch. La
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
   GNOME 51 ed è creato da `packaging/build_flatpak.py`. I pacchetti 0.1.65
   includono yt-dlp standalone e Deno verificati con SHA-256, senza dipendenze
   runtime da Python, yt-dlp o Deno di sistema. L'avvio della GUI nel sandbox e
   la compatibilità degli EBOOT su PSP reale restano da verificare.

Durante la migrazione gli script Python restano il riferimento di comportamento.
Il crate non è ancora un sostituto completo:
La conversione standard ricodifica le tracce audio in ATRAC3; la modalità CDDA
esplicita conserva invece l'audio raw e imposta i flag POPS. Il
database descrive 230 ID LibCrypt: ora tutti hanno una patch automatica inclusa
oppure una revisione identificata che non richiede patch. Per le sette
famiglie con firme upstream il backend Rust riconosce le revisioni senza
dipendere da `lcp`; per le revisioni ancora sconosciute può usare `lcp` se
disponibile nel PATH o in `POPFE_LCP`, altrimenti non applica una patch
predefinita potenzialmente errata. I pacchetti non includono `lcp`.
Non usare ancora il port come sostituto per generare EBOOT destinati alla PSP
finché non sarà validato su immagini e hardware reali. La decifratura DES dei
manuali PS3 PGD e la conversione in `DOCUMENT.DAT` non sono ancora state
verificate con file campione. I filtri pittorici Rust sono approssimazioni dei
filtri OpenCV.

Per compilare la GUI su Debian/Ubuntu sono richiesti i pacchetti di sviluppo
GTK4 e libadwaita, oltre a Rust/Cargo.
