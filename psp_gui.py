#!/usr/bin/env python3
"""Tk frontend for converting PSX images to/from PSP EBOOT.PBP files."""

from __future__ import annotations

import importlib.util
import queue
import shutil
import subprocess
import sys
import tempfile
import threading
import tkinter as tk
from pathlib import Path
from tkinter import filedialog, messagebox, ttk

from PIL import Image, ImageTk
from gamedb import games, themes


ROOT = Path(__file__).resolve().parent
POPFE = ROOT / "pop-fe.py"
POPSTATION = ROOT / "popstation.py"
CONFIG_PATH = ROOT / "psp_gui.config"


class ScrollableFrame(ttk.Frame):
    """A ttk frame with a vertical scrollbar and mouse-wheel support."""

    def __init__(self, parent: tk.Misc) -> None:
        super().__init__(parent)
        self.grid_rowconfigure(0, weight=1)
        self.grid_columnconfigure(0, weight=1)
        self.canvas = tk.Canvas(self, highlightthickness=0, borderwidth=0, background="#f4f6fb")
        self.canvas.grid(row=0, column=0, sticky="nsew")
        scrollbar = ttk.Scrollbar(self, orient="vertical", command=self.canvas.yview)
        scrollbar.grid(row=0, column=1, sticky="ns")
        self.canvas.configure(yscrollcommand=scrollbar.set)
        self.inner = ttk.Frame(self.canvas, padding=(4, 4, 14, 12))
        self.window = self.canvas.create_window((0, 0), window=self.inner, anchor="nw")
        self.inner.bind("<Configure>", self._update_scrollregion)
        self.canvas.bind("<Configure>", self._fit_width)
        self.canvas.bind("<Enter>", self._bind_mousewheel)
        self.canvas.bind("<Leave>", self._unbind_mousewheel)

    def _update_scrollregion(self, _event=None) -> None:
        self.canvas.configure(scrollregion=self.canvas.bbox("all"))

    def _fit_width(self, event) -> None:
        self.canvas.itemconfigure(self.window, width=event.width)

    def _bind_mousewheel(self, _event=None) -> None:
        self.canvas.bind_all("<MouseWheel>", self._mousewheel)
        self.canvas.bind_all("<Button-4>", self._mousewheel)
        self.canvas.bind_all("<Button-5>", self._mousewheel)

    def _unbind_mousewheel(self, _event=None) -> None:
        self.canvas.unbind_all("<MouseWheel>")
        self.canvas.unbind_all("<Button-4>")
        self.canvas.unbind_all("<Button-5>")

    def _mousewheel(self, event) -> None:
        if getattr(event, "num", None) == 4:
            amount = -1
        elif getattr(event, "num", None) == 5:
            amount = 1
        else:
            amount = -int(event.delta / 120)
        self.canvas.yview_scroll(amount, "units")


class PSPGui(tk.Tk):
    def __init__(self) -> None:
        super().__init__()
        self.title("POP-FE · PSP PBP Manager")
        self.geometry("860x680")
        self.minsize(720, 560)
        self.process: subprocess.Popen[str] | None = None
        self.events: queue.Queue[tuple[str, str]] = queue.Queue()
        self.active_button: ttk.Button | None = None
        self.input_temp_dirs: list[Path] = []
        self.last_dir = self._load_last_dir()
        self.metadata_busy = False
        self.theme_preview_busy = False
        self.preview_images: dict[str, ImageTk.PhotoImage] = {}
        self.preview_labels: dict[str, ttk.Label] = {}
        self.status = tk.StringVar(value="Pronto")
        self._configure_style()
        self._build()
        self.after(100, self._drain_events)
        self.protocol("WM_DELETE_WINDOW", self._close)

    def _configure_style(self) -> None:
        style = ttk.Style(self)
        try:
            style.theme_use("clam")
        except tk.TclError:
            pass
        style.configure(".", font=("TkDefaultFont", 10))
        style.configure("TFrame", background="#f4f6fb")
        style.configure("TLabelframe", background="#f4f6fb", bordercolor="#d7dce8")
        style.configure("TLabelframe.Label", background="#f4f6fb", foreground="#27324a", font=("TkDefaultFont", 10, "bold"))
        style.configure("TLabel", background="#f4f6fb", foreground="#27324a")
        style.configure("Heading.TLabel", background="#f4f6fb", foreground="#17213a", font=("TkDefaultFont", 13, "bold"))
        style.configure("Muted.TLabel", background="#f4f6fb", foreground="#667085", font=("TkDefaultFont", 9))
        style.configure("TNotebook", background="#f4f6fb", borderwidth=0)
        style.configure("TNotebook.Tab", padding=(18, 9), font=("TkDefaultFont", 10, "bold"))
        style.map("TNotebook.Tab", background=[("selected", "#ffffff")], foreground=[("selected", "#4055c7")])
        style.configure("TButton", padding=(12, 7), background="#ffffff", foreground="#27324a")
        style.configure("Accent.TButton", padding=(14, 8), background="#4055c7", foreground="#ffffff", font=("TkDefaultFont", 10, "bold"))
        style.map("Accent.TButton", background=[("active", "#3346ad"), ("disabled", "#aeb7d9")])
        self.configure(background="#f4f6fb")

    def _build(self) -> None:
        main = ttk.Frame(self, padding=16)
        main.pack(fill="both", expand=True)
        main.columnconfigure(0, weight=1)
        main.rowconfigure(0, weight=3)
        main.rowconfigure(1, weight=1)
        notebook = ttk.Notebook(main)
        notebook.grid(row=0, column=0, sticky="nsew", pady=(0, 12))
        self._build_conversion_tab(notebook)
        self._build_extraction_tab(notebook)

        log_frame = ttk.LabelFrame(main, text="Log", padding=8)
        log_frame.grid(row=1, column=0, sticky="nsew")
        log_frame.columnconfigure(0, weight=1)
        log_frame.rowconfigure(0, weight=1)
        self.log = tk.Text(log_frame, height=10, state="disabled", wrap="word")
        self.log.grid(row=0, column=0, sticky="nsew")
        scroll = ttk.Scrollbar(log_frame, orient="vertical", command=self.log.yview)
        scroll.grid(row=0, column=1, sticky="ns")
        self.log.configure(yscrollcommand=scroll.set)

        bottom = ttk.Frame(main)
        bottom.grid(row=2, column=0, sticky="ew", pady=(8, 0))
        bottom.columnconfigure(0, weight=1)
        ttk.Label(bottom, textvariable=self.status).grid(row=0, column=0, sticky="w")
        self.global_cancel = ttk.Button(bottom, text="Annulla", command=self._cancel, state="disabled")
        self.global_cancel.grid(row=0, column=1)

    def _build_conversion_tab(self, notebook: ttk.Notebook) -> None:
        scrollable = ScrollableFrame(notebook)
        tab = scrollable.inner
        tab.columnconfigure(1, weight=1)
        notebook.add(scrollable, text="Converti")
        ttk.Label(tab, text="Da CUE/BIN/IMG/ISO/CDDA a EBOOT.PBP", style="Heading.TLabel").grid(row=0, column=0, columnspan=3, sticky="w", pady=(0, 12))

        ttk.Label(tab, text="Immagini disco").grid(row=1, column=0, sticky="nw", pady=4)
        files_frame = ttk.Frame(tab)
        files_frame.grid(row=1, column=1, columnspan=2, sticky="nsew", pady=4)
        files_frame.columnconfigure(0, weight=1)
        files_frame.rowconfigure(0, weight=1)
        self.convert_files: list[str] = []
        self.convert_list = tk.Listbox(files_frame, height=7, selectmode=tk.EXTENDED)
        self.convert_list.grid(row=0, column=0, rowspan=2, sticky="nsew")
        scroll = ttk.Scrollbar(files_frame, orient="vertical", command=self.convert_list.yview)
        scroll.grid(row=0, column=1, rowspan=2, sticky="ns")
        self.convert_list.configure(yscrollcommand=scroll.set)
        ttk.Button(files_frame, text="Aggiungi…", command=self._add_conversion_files).grid(row=0, column=2, padx=(8, 0), sticky="ew")
        ttk.Button(files_frame, text="Rimuovi", command=self._remove_conversion_files).grid(row=1, column=2, padx=(8, 0), pady=(4, 0), sticky="ew")

        self.convert_output = tk.StringVar()
        self.convert_title = tk.StringVar()
        self.convert_id = tk.StringVar()
        self.convert_no_assets = tk.BooleanVar(value=False)
        self.convert_no_libcrypt = tk.BooleanVar(value=False)
        self.convert_cdda = tk.BooleanVar(value=False)
        self.convert_undither = tk.BooleanVar(value=False)
        self.convert_watermark = tk.BooleanVar(value=False)
        self.convert_theme = tk.StringVar()
        self.convert_cover = tk.StringVar()
        self.convert_pic0 = tk.StringVar()
        self.convert_pic1 = tk.StringVar()
        self.convert_logo = tk.StringVar()
        self.convert_snd0 = tk.StringVar()
        self.convert_manual = tk.StringVar()
        self._path_row(tab, 2, "Root PSP", self.convert_output, self._choose_conversion_output)
        self._entry_row(tab, 3, "Titolo", self.convert_title, "opzionale")
        self._entry_row(tab, 4, "Game ID", self.convert_id, "es. SLUS-00664")

        options = ttk.LabelFrame(tab, text="Opzioni", padding=8)
        options.grid(row=5, column=0, columnspan=3, sticky="ew", pady=(12, 8))
        ttk.Checkbutton(options, text="Non scaricare copertine/audio", variable=self.convert_no_assets).pack(anchor="w")
        ttk.Checkbutton(options, text="Non applicare patch libcrypt", variable=self.convert_no_libcrypt).pack(anchor="w")
        ttk.Checkbutton(options, text="Usa audio CDDA (compatibilità ridotta)", variable=self.convert_cdda).pack(anchor="w")
        ttk.Checkbutton(options, text="Applica PSX-undither", variable=self.convert_undither).pack(anchor="w")
        ttk.Checkbutton(options, text="Aggiungi watermark con ID e titolo", variable=self.convert_watermark).pack(anchor="w")

        assets = ttk.LabelFrame(tab, text="Assets e metadati", padding=8)
        assets.grid(row=6, column=0, columnspan=3, sticky="ew", pady=(4, 8))
        assets.columnconfigure(1, weight=1)
        ttk.Label(assets, text="Lascia vuoto per il caricamento automatico da POP-FE.", style="Muted.TLabel").grid(row=0, column=0, columnspan=3, sticky="w", pady=(0, 5))
        ttk.Label(assets, text="Tema").grid(row=1, column=0, sticky="w", pady=2)
        self.theme_combo = ttk.Combobox(assets, textvariable=self.convert_theme, values=[""] + list(themes), state="readonly")
        self.theme_combo.grid(row=1, column=1, columnspan=2, sticky="ew", pady=2)
        self.theme_combo.bind("<<ComboboxSelected>>", self._theme_selected)
        self._asset_row(assets, 2, "Cover / ICON0", self.convert_cover, lambda: self._choose_asset(self.convert_cover, "Seleziona cover"))
        self._asset_row(assets, 3, "PIC0", self.convert_pic0, lambda: self._choose_asset(self.convert_pic0, "Seleziona PIC0"))
        self._asset_row(assets, 4, "PIC1", self.convert_pic1, lambda: self._choose_asset(self.convert_pic1, "Seleziona PIC1"))
        self._asset_row(assets, 5, "Logo", self.convert_logo, lambda: self._choose_asset(self.convert_logo, "Seleziona logo"))
        self._asset_row(assets, 6, "SND0 audio", self.convert_snd0, lambda: self._choose_asset(self.convert_snd0, "Seleziona SND0"))
        self._asset_row(assets, 7, "Manuale", self.convert_manual, self._choose_manual)
        preview = ttk.Frame(assets)
        preview.grid(row=8, column=0, columnspan=3, sticky="ew", pady=(10, 2))
        for index, (key, caption) in enumerate((("cover", "ICON0"), ("pic0", "PIC0"), ("pic1", "PIC1"))):
            card = ttk.Frame(preview, padding=5)
            card.grid(row=0, column=index, padx=(0, 10), sticky="w")
            image_label = ttk.Label(card, text="Nessuna anteprima", anchor="center", width=22)
            image_label.pack()
            ttk.Label(card, text=caption, style="Muted.TLabel", anchor="center").pack(fill="x", pady=(4, 0))
            self.preview_labels[key] = image_label
        self.convert_button = ttk.Button(tab, text="Crea EBOOT.PBP", style="Accent.TButton", command=self._start_conversion)
        self.convert_button.grid(row=7, column=2, sticky="e", pady=(8, 0))

    def _build_extraction_tab(self, notebook: ttk.Notebook) -> None:
        scrollable = ScrollableFrame(notebook)
        tab = scrollable.inner
        tab.columnconfigure(1, weight=1)
        notebook.add(scrollable, text="Estrai")
        ttk.Label(tab, text="Da EBOOT.PBP a BIN/CUE/ISO/CDDA", style="Heading.TLabel").grid(row=0, column=0, columnspan=3, sticky="w", pady=(0, 12))
        self.extract_pbp = tk.StringVar()
        self.extract_output = tk.StringVar()
        self._path_row(tab, 1, "File PBP", self.extract_pbp, self._choose_pbp)
        self._path_row(tab, 2, "Cartella output", self.extract_output, self._choose_extraction_output)
        ttk.Label(tab, text="L'estrazione crea PSISO*.img, PSISO*.cue, PSISO*.toc e le eventuali tracce TRACK_* nella cartella scelta.", wraplength=650, justify="left").grid(row=3, column=0, columnspan=3, sticky="w", pady=(18, 8))
        self.extract_button = ttk.Button(tab, text="Estrai contenuti", style="Accent.TButton", command=self._start_extraction)
        self.extract_button.grid(row=4, column=2, sticky="e", pady=(8, 0))

    @staticmethod
    def _path_row(parent: ttk.Frame, row: int, label: str, variable: tk.StringVar, command) -> None:
        ttk.Label(parent, text=label).grid(row=row, column=0, sticky="w", pady=5)
        ttk.Entry(parent, textvariable=variable).grid(row=row, column=1, sticky="ew", pady=5)
        ttk.Button(parent, text="Scegli…", command=command).grid(row=row, column=2, padx=(8, 0), pady=5)

    @staticmethod
    def _entry_row(parent: ttk.Frame, row: int, label: str, variable: tk.StringVar, hint: str) -> None:
        ttk.Label(parent, text=label).grid(row=row, column=0, sticky="w", pady=5)
        ttk.Entry(parent, textvariable=variable).grid(row=row, column=1, sticky="ew", pady=5)
        ttk.Label(parent, text=hint).grid(row=row, column=2, sticky="w", padx=(8, 0))

    @staticmethod
    def _asset_row(parent: ttk.Frame, row: int, label: str, variable: tk.StringVar, command, choose_button: bool = True) -> None:
        ttk.Label(parent, text=label).grid(row=row, column=0, sticky="w", pady=2)
        ttk.Entry(parent, textvariable=variable).grid(row=row, column=1, sticky="ew", pady=2)
        if choose_button:
            ttk.Button(parent, text="Scegli…", command=command).grid(row=row, column=2, padx=(8, 0), pady=2)

    def _choose_asset(self, variable: tk.StringVar, title: str) -> None:
        path = self._select_file(title, "File supportati", ["*.png", "*.jpg", "*.jpeg", "*.wav", "*.mp3", "*.m4a", "*.ogg", "*.flac", "*.at3", "*.snd0"])
        if path:
            variable.set(path)
            self._remember_path(path)
            for key, target in (("cover", self.convert_cover), ("pic0", self.convert_pic0), ("pic1", self.convert_pic1)):
                if variable is target:
                    self._update_preview(key, path)
                    break

    def _choose_manual(self) -> None:
        path = self._select_file("Seleziona manuale", "Manuali", ["*.pdf", "*.zip", "*.cbz", "*.cbr", "*.rar", "*.dat", "*.manual"])
        if path:
            self.convert_manual.set(path)
            self._remember_path(path)

    def _start_metadata_scan(self, filename: str) -> None:
        self.metadata_busy = True
        self.status.set("Analisi automatica del disco…")
        allow_remote = not self.convert_no_assets.get()
        threading.Thread(target=self._scan_metadata, args=(filename, allow_remote), daemon=True).start()

    def _scan_metadata(self, filename: str, allow_remote: bool) -> None:
        temp_dir = Path(tempfile.mkdtemp(prefix="pop-fe-metadata-"))
        try:
            spec = importlib.util.spec_from_file_location("popfe_metadata_runtime", POPFE)
            if spec is None or spec.loader is None:
                raise RuntimeError("impossibile caricare pop-fe.py")
            popfe = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(popfe)
            source = filename
            if Path(filename).suffix.lower() == ".iso":
                cue = temp_dir / "track01.cue"
                cue.write_text(
                    f'FILE "{Path(filename).resolve()}" BINARY\n'
                    "  TRACK 01 MODE1/2048\n"
                    "    INDEX 01 00:00:00\n",
                    encoding="utf-8",
                )
                source = str(cue)
            temp_files: list[str] = []
            subdir = str(temp_dir) + "/"
            cue_file, real_cue, _img = popfe.process_disk_file(source, 0, temp_files, subdir=subdir)
            if not cue_file:
                raise RuntimeError("formato disco non riconosciuto")
            disc_id, _md5 = popfe.get_disc_id(cue_file, real_cue, str(temp_dir / "metadata.iso"), is_psp=True)
            result: dict[str, object] = {
                "id": disc_id,
                "title": popfe.get_title_from_game(disc_id),
                "manual": popfe.find_local_manual(real_cue),
                "cdda": disc_id in games and "psp-use-cdda" in games[disc_id],
            }
            source_path = Path(filename)
            stem = source_path.with_suffix("")
            for key, suffix in (("cover", "_cover.png"), ("pic0", "_pic0.png"), ("pic1", "_pic1.png")):
                candidate = Path(str(stem) + suffix)
                if candidate.is_file():
                    result[key] = str(candidate)
            if allow_remote:
                cache_dir = ROOT / "pop-fe-psp-work" / "gui-assets"
                cache_dir.mkdir(parents=True, exist_ok=True)
                try:
                    game = popfe.get_game_from_gamelist(disc_id)
                    remote_assets = (
                        ("cover", popfe.get_icon0_from_game, cache_dir / f"{disc_id}-ICON0.png"),
                        ("pic0", popfe.get_pic0_from_game, cache_dir / f"{disc_id}-PIC0.png"),
                        ("pic1", popfe.get_pic1_from_game, cache_dir / f"{disc_id}-PIC1.png"),
                    )
                    for key, loader, cache_path in remote_assets:
                        if key in result:
                            continue
                        if cache_path.is_file():
                            result[key] = str(cache_path)
                            continue
                        if key == "cover":
                            image = loader(disc_id, game, real_cue, str(cache_path))
                        else:
                            image = loader(disc_id, game, real_cue)
                        if image:
                            image.save(cache_path)
                            result[key] = str(cache_path)
                except Exception:
                    # Asset downloads are best-effort; conversion will retry them.
                    pass
            self.events.put(("metadata", result))
        except Exception as exc:
            self.events.put(("metadata_error", str(exc)))
        finally:
            shutil.rmtree(temp_dir, ignore_errors=True)

    def _apply_metadata(self, data: dict[str, object]) -> None:
        self.metadata_busy = False
        self.convert_id.set(str(data.get("id", "")))
        self.convert_title.set(str(data.get("title", "")))
        if data.get("manual") and not self.convert_manual.get():
            self.convert_manual.set(str(data["manual"]))
        if data.get("cover") and not self.convert_cover.get():
            self.convert_cover.set(str(data["cover"]))
        if data.get("cover"):
            self._update_preview("cover", str(data["cover"]))
        if data.get("pic0") and not self.convert_pic0.get():
            self.convert_pic0.set(str(data["pic0"]))
        if data.get("pic0"):
            self._update_preview("pic0", str(data["pic0"]))
        if data.get("pic1") and not self.convert_pic1.get():
            self.convert_pic1.set(str(data["pic1"]))
        if data.get("pic1"):
            self._update_preview("pic1", str(data["pic1"]))
        if data.get("cdda"):
            self.convert_cdda.set(True)
        self.status.set("Dati automatici caricati")
        if self.convert_theme.get():
            self._start_theme_preview()

    def _theme_selected(self, _event=None) -> None:
        self._start_theme_preview()

    def _start_theme_preview(self) -> None:
        theme = self.convert_theme.get().strip()
        disc_id = self.convert_id.get().strip()
        if not theme or not disc_id or self.theme_preview_busy:
            return
        self.theme_preview_busy = True
        self.status.set(f"Caricamento anteprima tema {theme}…")
        threading.Thread(target=self._load_theme_preview, args=(theme, disc_id), daemon=True).start()

    def _load_theme_preview(self, theme: str, disc_id: str) -> None:
        result: dict[str, str] = {}
        cache_dir = ROOT / "pop-fe-psp-work" / "gui-assets"
        cache_dir.mkdir(parents=True, exist_ok=True)
        try:
            spec = importlib.util.spec_from_file_location("popfe_theme_runtime", POPFE)
            if spec is None or spec.loader is None:
                raise RuntimeError("impossibile caricare pop-fe.py")
            popfe = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(popfe)
            for key, filename in (("cover", "ICON0.PNG"), ("pic0", "PIC0.PNG"), ("pic1", "PIC1.PNG")):
                cache_path = cache_dir / f"{disc_id}-{theme}-{key}.png"
                if not cache_path.is_file():
                    image = popfe.get_image_from_theme(theme, disc_id, str(cache_dir), filename)
                    if image:
                        image.save(cache_path)
                if cache_path.is_file():
                    result[key] = str(cache_path)
            self.events.put(("theme_preview", result))
        except Exception as exc:
            self.events.put(("theme_preview_error", str(exc)))
        finally:
            self.theme_preview_busy = False

    def _update_preview(self, key: str, filename: str) -> None:
        label = self.preview_labels.get(key)
        if label is None:
            return
        try:
            image = Image.open(filename).convert("RGBA")
            image.thumbnail((180, 105), Image.Resampling.LANCZOS)
            photo = ImageTk.PhotoImage(image)
            self.preview_images[key] = photo
            label.configure(image=photo, text="")
        except (OSError, ValueError):
            self.preview_images.pop(key, None)
            label.configure(image="", text="Anteprima non disponibile")

    @staticmethod
    def _load_last_dir() -> str:
        try:
            saved = Path(CONFIG_PATH.read_text(encoding="utf-8").strip()).expanduser()
            if saved.is_dir():
                return str(saved)
        except (OSError, ValueError):
            pass
        return str(Path.home())

    def _remember_path(self, selected: str) -> None:
        path = Path(selected).expanduser()
        directory = path if path.is_dir() else path.parent
        if not directory.is_dir():
            directory = Path.home()
        self.last_dir = str(directory)
        try:
            CONFIG_PATH.write_text(self.last_dir + "\n", encoding="utf-8")
        except OSError:
            pass

    def _choose_theme(self) -> None:
        theme_names = [""] + list(themes)
        dialog = tk.Toplevel(self)
        dialog.title("Seleziona tema")
        dialog.transient(self)
        value = tk.StringVar(value=self.convert_theme.get())
        ttk.Label(dialog, text="Tema POP-FE").pack(padx=16, pady=(14, 4))
        combo = ttk.Combobox(dialog, textvariable=value, values=theme_names, state="readonly", width=28)
        combo.pack(padx=16, pady=4)
        ttk.Button(dialog, text="OK", command=lambda: (self.convert_theme.set(value.get()), dialog.destroy())).pack(pady=(4, 14))

    def _select_file(self, title: str, label: str, patterns: list[str]) -> str:
        paths = self._select_files(title, label, patterns, multiple=False)
        return paths[0] if paths else ""

    def _select_files(self, title: str, label: str, patterns: list[str], multiple: bool = True) -> tuple[str, ...]:
        zenity = shutil.which("zenity")
        if zenity:
            command = [zenity, "--file-selection", "--title", title, "--filename", str(Path(self.last_dir)) + "/"]
            if multiple:
                command += ["--multiple", "--separator", "|"]
            command += ["--file-filter", f"{label} | {' '.join(patterns)}", "--file-filter", "Tutti i file | *"]
            result = subprocess.run(command, capture_output=True, text=True)
            if result.returncode == 0 and result.stdout.strip():
                return tuple(item for item in result.stdout.strip().split("|") if item)
            return ()
        paths = filedialog.askopenfilenames(initialdir=self.last_dir, title=title, filetypes=[(label, " ".join(patterns)), ("Tutti i file", "*")]) if multiple else filedialog.askopenfilename(initialdir=self.last_dir, title=title, filetypes=[(label, " ".join(patterns)), ("Tutti i file", "*")])
        return tuple(paths) if isinstance(paths, (tuple, list)) else ((paths,) if paths else ())

    def _select_directory(self, title: str) -> str:
        zenity = shutil.which("zenity")
        if zenity:
            command = [zenity, "--file-selection", "--directory", "--title", title, "--filename", str(Path(self.last_dir)) + "/"]
            result = subprocess.run(command, capture_output=True, text=True)
            return result.stdout.strip() if result.returncode == 0 else ""
        return filedialog.askdirectory(initialdir=self.last_dir, title=title)

    def _add_conversion_files(self) -> None:
        paths = self._select_files("Seleziona immagini PSX", "Immagini PSX", ["*.cue", "*.ccd", "*.bin", "*.img", "*.iso", "*.zip", "*.chd"])
        if paths:
            self._remember_path(paths[-1])
        for path in paths:
            if path not in self.convert_files:
                self.convert_files.append(path)
                self.convert_list.insert(tk.END, path)
        if paths and not self.metadata_busy:
            self._start_metadata_scan(self.convert_files[0])

    def _remove_conversion_files(self) -> None:
        for index in reversed(self.convert_list.curselection()):
            self.convert_files.pop(index)
            self.convert_list.delete(index)

    def _choose_conversion_output(self) -> None:
        path = self._select_directory("Seleziona la root della PSP")
        if path:
            self.convert_output.set(path)
            self._remember_path(path)

    def _choose_pbp(self) -> None:
        path = self._select_file("Seleziona EBOOT.PBP", "PSP EBOOT", ["*.PBP", "*.pbp"])
        if path:
            self.extract_pbp.set(path)
            self._remember_path(path)

    def _choose_extraction_output(self) -> None:
        path = self._select_directory("Seleziona la cartella di estrazione")
        if path:
            self.extract_output.set(path)
            self._remember_path(path)

    def _start_conversion(self) -> None:
        if not self.convert_files:
            messagebox.showwarning("File mancanti", "Seleziona almeno un'immagine CUE/BIN/ISO.")
            return
        if not self.convert_output.get():
            messagebox.showwarning("Destinazione mancante", "Seleziona la root della PSP.")
            return
        try:
            input_files = self._prepare_conversion_files()
        except OSError as exc:
            messagebox.showerror("Preparazione input fallita", str(exc))
            return
        command = [sys.executable, str(POPFE), "--psp-dir", self.convert_output.get()]
        if self.convert_title.get().strip():
            command += ["--title", self.convert_title.get().strip()]
        if self.convert_id.get().strip():
            command += ["--game_id", self.convert_id.get().strip()]
        if self.convert_no_assets.get():
            command.append("--force-no-assets")
        if self.convert_no_libcrypt.get():
            command.append("--no-libcrypt")
        if self.convert_cdda.get():
            command.append("--psp-use-cdda")
        if self.convert_undither.get():
            command.append("--psx-undither")
        if self.convert_watermark.get():
            command.append("--watermark")
        for option, variable in (("--theme", self.convert_theme), ("--cover", self.convert_cover), ("--pic0", self.convert_pic0), ("--pic1", self.convert_pic1), ("--logo", self.convert_logo), ("--snd0", self.convert_snd0), ("--manual", self.convert_manual)):
            if variable.get().strip():
                command += [option, variable.get().strip()]
        command += input_files
        self._run_command(command, self.convert_button, "EBOOT.PBP creato sulla PSP.", "Conversione")

    def _prepare_conversion_files(self) -> list[str]:
        """Create temporary CUE wrappers for raw 2048-byte ISO files."""
        prepared: list[str] = []
        for filename in self.convert_files:
            if Path(filename).suffix.lower() != ".iso":
                prepared.append(filename)
                continue
            temp_dir = Path(tempfile.mkdtemp(prefix="pop-fe-iso-"))
            self.input_temp_dirs.append(temp_dir)
            cue_path = temp_dir / "track01.cue"
            cue_path.write_text(
                f'FILE "{Path(filename).resolve()}" BINARY\n'
                "  TRACK 01 MODE1/2048\n"
                "    INDEX 01 00:00:00\n",
                encoding="utf-8",
            )
            prepared.append(str(cue_path))
        return prepared

    def _start_extraction(self) -> None:
        if not self.extract_pbp.get() or not Path(self.extract_pbp.get()).is_file():
            messagebox.showwarning("PBP mancante", "Seleziona un file EBOOT.PBP valido.")
            return
        if not self.extract_output.get():
            messagebox.showwarning("Destinazione mancante", "Seleziona la cartella di estrazione.")
            return
        output = Path(self.extract_output.get())
        output.mkdir(parents=True, exist_ok=True)
        command = [sys.executable, str(POPSTATION), "-v", "dump_pbp", self.extract_pbp.get()]
        self._run_command(command, self.extract_button, "Estrazione completata.", "Estrazione", cwd=output)

    def _run_command(self, command: list[str], button: ttk.Button, success: str, label: str, cwd: Path | None = None) -> None:
        if self.process is not None:
            return
        if not POPFE.is_file() or not POPSTATION.is_file():
            messagebox.showerror("Installazione incompleta", "Non trovo gli script pop-fe.py/popstation.py.")
            return
        self._write_log("$ " + " ".join(self._quote(arg) for arg in command))
        self.status.set(f"{label} in corso…")
        self.active_button = button
        button.configure(state="disabled")
        self.global_cancel.configure(state="normal")
        thread = threading.Thread(target=self._run, args=(command, success, cwd or ROOT), daemon=True)
        thread.start()

    def _run(self, command: list[str], success: str, cwd: Path) -> None:
        try:
            self.process = subprocess.Popen(command, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1)
            assert self.process.stdout is not None
            for line in self.process.stdout:
                self.events.put(("log", line.rstrip()))
            code = self.process.wait()
            self.events.put(("done", f"{code}\t{success}"))
        except Exception as exc:
            self.events.put(("error", str(exc)))
        finally:
            for temp_dir in self.input_temp_dirs:
                shutil.rmtree(temp_dir, ignore_errors=True)
            self.input_temp_dirs.clear()

    def _drain_events(self) -> None:
        try:
            while True:
                event, value = self.events.get_nowait()
                if event == "log":
                    self._write_log(value)
                    continue
                if event == "metadata":
                    self._apply_metadata(value)
                    continue
                if event == "metadata_error":
                    self.metadata_busy = False
                    self.status.set("Analisi automatica non disponibile")
                    self._write_log("INFO: dati automatici non disponibili: " + value)
                    continue
                if event == "theme_preview":
                    self.theme_preview_busy = False
                    for key, filename in value.items():
                        self._update_preview(key, filename)
                    self.status.set("Anteprima tema caricata")
                    continue
                if event == "theme_preview_error":
                    self.theme_preview_busy = False
                    self.status.set("Anteprima tema non disponibile")
                    self._write_log("INFO: anteprima tema non disponibile: " + value)
                    continue
                if self.active_button is not None:
                    self.active_button.configure(state="normal")
                self.global_cancel.configure(state="disabled")
                self.process = None
                if event == "done":
                    code, success = value.split("\t", 1)
                    if code == "0":
                        self.status.set("Completato")
                        messagebox.showinfo("Completato", success)
                    else:
                        self.status.set(f"Errore (codice {code})")
                else:
                    self.status.set("Errore")
                    self._write_log("ERRORE: " + value)
        except queue.Empty:
            pass
        self.after(100, self._drain_events)

    def _cancel(self) -> None:
        if self.process is not None:
            self.process.terminate()
            self._write_log("Operazione annullata.")

    def _write_log(self, line: str) -> None:
        self.log.configure(state="normal")
        self.log.insert(tk.END, line + "\n")
        self.log.see(tk.END)
        self.log.configure(state="disabled")

    @staticmethod
    def _quote(value: str) -> str:
        return repr(value) if any(char.isspace() for char in value) else value

    def _close(self) -> None:
        if self.process is not None:
            if not messagebox.askyesno("Operazione attiva", "Vuoi interrompere l'operazione?"):
                return
            self.process.terminate()
        self.destroy()


if __name__ == "__main__":
    PSPGui().mainloop()
