#!/usr/bin/env /usr/bin/python3
"""Native GTK 4 frontend for POP-FE PSP conversion and extraction."""

from __future__ import annotations

import os
import json
import subprocess
import signal
import threading
import time
import uuid
from pathlib import Path

from psp_i18n import _
from psp_paths import FolderHistory

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Gdk", "4.0")
gi.require_version("Adw", "1")
from gi.repository import Adw, Gdk, Gio, GLib, Gtk, Pango


ROOT = Path(__file__).resolve().parent
APP_VERSION = (ROOT / "VERSION").read_text(encoding="utf-8").strip()
APP_ID = "org.popfe.PSPManager"
WORK_ROOT = Path(GLib.get_user_cache_dir()) / APP_ID
PYTHON = ROOT / ".venv" / "bin" / "python"
if not PYTHON.exists():
    PYTHON = Path("/usr/bin/python3")


CSS = """
/* Leave widget colors and interaction states to the installed GTK theme.
   Scope layout rules to this window, including its native titlebar. */
.popfe-window .page { padding: 16px; }
.popfe-window .page-heading { font-size: 20px; font-weight: 700; }
.popfe-window .subtitle { margin-bottom: 8px; }
.popfe-window .section { border: 1px solid alpha(currentColor, 0.16); border-radius: 12px; padding: 12px; }
.popfe-window .preview { background-color: alpha(currentColor, 0.04); border-radius: 8px; padding: 8px; }
.popfe-window .page-tabs { margin: 10px; }
.popfe-window .disc-list { background: transparent; }
"""


class PSPGtk(Adw.Application):
    def __init__(self) -> None:
        super().__init__(application_id="org.popfe.PSPManager")
        self.window: Adw.ApplicationWindow | None = None
        self.files: list[str] = []
        self.process: subprocess.Popen[str] | None = None
        self.busy = False
        self.cancel_requested = False
        self.progress_timer = None
        self.folder_history = FolderHistory(Path(GLib.get_user_config_dir()) / APP_ID / "dialogs.json")
        self.log_views: list[Gtk.TextView] = []
        self.status: Gtk.Label | None = None
        self.convert_button: Gtk.Button | None = None
        self.cancel_button: Gtk.Button | None = None
        self.title_entry: Gtk.Entry | None = None
        self.id_entry: Gtk.Entry | None = None
        self.psp_entry: Gtk.Entry | None = None
        self.no_assets: Gtk.CheckButton | None = None
        self.no_libcrypt: Gtk.CheckButton | None = None
        self.use_cdda: Gtk.CheckButton | None = None
        self.undither: Gtk.CheckButton | None = None
        self.watermark: Gtk.CheckButton | None = None
        self.nopstitleimg: Gtk.CheckButton | None = None
        self.pic1aslogo: Gtk.CheckButton | None = None
        self.force_ntsc: Gtk.CheckButton | None = None
        self.swap_disc: Gtk.CheckButton | None = None
        self.theme: Gtk.DropDown | None = None
        self.asset_entries: dict[str, Gtk.Entry] = {}
        self.preview: dict[str, Gtk.Picture] = {}
        self.extract_pbp: Gtk.Entry | None = None
        self.extract_dir: Gtk.Entry | None = None
        self.css_provider: Gtk.CssProvider | None = None

    def do_activate(self) -> None:
        if self.window:
            self.window.present()
            return
        self._load_css()
        self.window = Adw.ApplicationWindow(application=self)
        self.window.add_css_class("popfe-window")
        self.window.set_title("POP-FE PSP Manager")
        self.window.set_icon_name(APP_ID)
        icon_theme = Gtk.IconTheme.get_for_display(self.window.get_display())
        icon_theme.add_search_path(str(ROOT / "packaging" / "icons"))
        self.window.set_default_size(820, 740)
        self.window.set_resizable(True)
        self.window.set_size_request(560, 480)
        self._setup_actions()
        header = Adw.HeaderBar()
        content = self._build_root()
        header.set_title_widget(self.switcher)
        self.menu_button = Gtk.MenuButton(icon_name="open-menu-symbolic")
        self.menu_button.set_tooltip_text(_("Main Menu"))
        self.menu_button.set_menu_model(self._main_menu())
        header.pack_end(self.menu_button)
        toolbar = Adw.ToolbarView()
        toolbar.add_top_bar(header)
        toolbar.set_content(content)
        self.window.set_content(toolbar)
        self.window.connect("close-request", self._close_requested)
        self.window.present()

    def _setup_actions(self) -> None:
        callbacks = {
            "open": lambda *_: self._open_current(),
            "convert": lambda *_: self.stack.set_visible_child_name("convert"),
            "extract": lambda *_: self.stack.set_visible_child_name("extract"),
            "clear-log": lambda *_: [view.get_buffer().set_text("") for view in self.log_views],
            "help": lambda *_: self._show_help(),
            "about": lambda *_: self._show_about(),
            "quit": lambda *_: self.window.close(),
        }
        for name, callback in callbacks.items():
            action = Gio.SimpleAction.new(name, None)
            action.connect("activate", callback)
            self.add_action(action)
        for action, keys in {
            "open": ["<Primary>o"], "convert": ["<Alt>1"],
            "extract": ["<Alt>2"], "help": ["F1"], "quit": ["<Primary>q"],
        }.items():
            self.set_accels_for_action("app." + action, keys)

    @staticmethod
    def _main_menu() -> Gio.Menu:
        menu = Gio.Menu()
        for items in (
            ((_("Open Images or PBP…"), "open"), (_("Convert"), "convert"), (_("Extract"), "extract")),
            ((_("Clear Log"), "clear-log"),),
            ((_("Help and Shortcuts"), "help"), (_("About POP-FE"), "about")),
            ((_("Quit"), "quit"),),
        ):
            section = Gio.Menu()
            for label, action in items:
                section.append(label, "app." + action)
            menu.append_section(None, section)
        return menu

    def _open_current(self) -> None:
        if self.stack.get_visible_child_name() == "extract":
            self._choose_pbp(None)
        else:
            self._choose_inputs(None)

    def _show_help(self) -> None:
        dialog = Adw.AlertDialog(
            heading=_("POP-FE Help"),
            body=_("Convert: add game images, check the data and previews, choose the output folder and create EBOOT.PBP.\n\n"
                   "Extract: select the PBP and an output folder.\n\n"
                   "Ctrl+O — Open Images or PBP\nAlt+1 — Convert\nAlt+2 — Extract\nF1 — Help\nCtrl+Q — Quit"),
        )
        dialog.add_response("close", _("Close"))
        dialog.present(self.window)

    def _show_about(self) -> None:
        dialog = Adw.AboutDialog(
            application_name="POP-FE PSP Manager",
            application_icon=APP_ID,
            version=APP_VERSION,
            comments=_("Convert and extract PlayStation disc images for PSP with POP-FE."),
            license_type=Gtk.License.LGPL_2_1,
        )
        dialog.present(self.window)

    def _close_requested(self, _window) -> bool:
        if not self.busy:
            return False
        dialog = Adw.AlertDialog(heading=_("Stop the operation?"),
                                body=_("An operation is running. Quitting will stop it."))
        dialog.add_response("cancel", _("Continue"))
        dialog.add_response("quit", _("Stop and Quit"))
        dialog.set_response_appearance("quit", Adw.ResponseAppearance.DESTRUCTIVE)
        dialog.set_default_response("cancel")
        dialog.set_close_response("cancel")
        dialog.connect("response", self._close_response)
        dialog.present(self.window)
        return True

    def _close_response(self, _dialog, response: str) -> None:
        if response == "quit":
            self._cancel(None)
            self.quit()

    def _load_css(self) -> None:
        provider = Gtk.CssProvider()
        provider.load_from_data(CSS, -1)
        Gtk.StyleContext.add_provider_for_display(Gdk.Display.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        self.css_provider = provider
        # libadwaita follows the system color preference via the desktop portal.
        self.get_style_manager().set_color_scheme(Adw.ColorScheme.DEFAULT)

    def _build_root(self) -> Gtk.Widget:
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
        switcher = Gtk.StackSwitcher()
        stack = Gtk.Stack(transition_type=Gtk.StackTransitionType.SLIDE_LEFT_RIGHT)
        self.stack = stack
        self.switcher = switcher
        switcher.set_stack(stack)
        switcher.set_halign(Gtk.Align.CENTER)
        root.append(stack)
        stack.set_vexpand(True)
        stack.add_titled(self._scroll_page(self._build_convert()), "convert", _("Convert"))
        stack.add_titled(self._scroll_page(self._build_extract()), "extract", _("Extract"))
        progress_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        progress_box.set_margin_start(16)
        progress_box.set_margin_end(16)
        progress_box.set_margin_top(8)
        progress_box.set_margin_bottom(12)
        progress_row = Gtk.Box(spacing=8)
        self.spinner = Gtk.Spinner()
        self.progress_label = Gtk.Label(label=_("Ready"), xalign=0, hexpand=True, wrap=True)
        self.stop_button = Gtk.Button(label=_("Stop"), sensitive=False)
        self.stop_button.connect("clicked", self._cancel)
        progress_row.append(self.spinner)
        progress_row.append(self.progress_label)
        progress_row.append(self.stop_button)
        self.progress_bar = Gtk.ProgressBar(show_text=False)
        progress_box.append(progress_row)
        progress_box.append(self.progress_bar)
        root.append(progress_box)
        return root

    def _begin_progress(self, phase: str) -> None:
        self.busy = True
        self.cancel_requested = False
        self.phase = phase
        self.started_at = time.monotonic()
        self.spinner.start()
        self.stop_button.set_sensitive(True)
        self.cancel_button.set_sensitive(True)
        self.convert_button.set_sensitive(False)
        self.xmb_button.set_sensitive(False)
        self.extract_button.set_sensitive(False)
        self.lookup_action("open").set_enabled(False)
        self.progress_timer = GLib.timeout_add(100, self._pulse_progress)
        self._pulse_progress()

    def _pulse_progress(self) -> bool:
        self.progress_bar.pulse()
        elapsed = int(time.monotonic() - self.started_at)
        self.progress_label.set_text(f"{self.phase} · {elapsed // 60:02d}:{elapsed % 60:02d}")
        return GLib.SOURCE_CONTINUE

    def _end_progress(self, message: str, success: bool = False) -> None:
        if self.progress_timer is not None:
            GLib.source_remove(self.progress_timer)
            self.progress_timer = None
        self.busy = False
        self.spinner.stop()
        self.progress_bar.set_fraction(1.0 if success else 0.0)
        self.progress_label.set_text(message)
        self.status.set_text(message)
        self.stop_button.set_sensitive(False)
        self.cancel_button.set_sensitive(False)
        self.convert_button.set_sensitive(True)
        self.xmb_button.set_sensitive(True)
        self.extract_button.set_sensitive(True)
        self.lookup_action("open").set_enabled(True)

    def _set_phase(self, phase: str) -> bool:
        self.phase = phase
        return GLib.SOURCE_REMOVE

    @staticmethod
    def _scroll_page(page: Gtk.Widget) -> Gtk.ScrolledWindow:
        scroll = Gtk.ScrolledWindow()
        scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        scroll.set_vexpand(True)
        scroll.set_hexpand(True)
        scroll.set_child(page)
        return scroll

    def _page(self, title: str, subtitle: str) -> Gtk.Box:
        page = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=14)
        page.add_css_class("page")
        heading = Gtk.Label(label=title, xalign=0, wrap=True)
        heading.add_css_class("page-heading")
        page.append(heading)
        description = Gtk.Label(label=subtitle, xalign=0, wrap=True)
        description.add_css_class("subtitle")
        description.add_css_class("dim-label")
        page.append(description)
        return page

    def _build_convert(self) -> Gtk.Widget:
        page = self._page(_("Convert PSX Images to EBOOT.PBP"), _("Automatic assets, themes and previews are used during conversion."))
        input_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        input_box.add_css_class("section")
        input_box.append(Gtk.Label(label=_("Disc Images"), xalign=0))
        self.file_list = Gtk.ListBox()
        self.file_list.add_css_class("disc-list")
        self.file_list.set_selection_mode(Gtk.SelectionMode.NONE)
        self.file_list.set_vexpand(True)
        input_box.append(self.file_list)
        add = Gtk.Button(label=_("Add Images…"))
        add.connect("clicked", self._choose_inputs)
        input_box.append(add)
        page.append(input_box)

        form = Gtk.Grid(column_spacing=12, row_spacing=8)
        form.add_css_class("section")
        self.psp_entry = self._entry_row(form, 0, _("PSP Root Folder"))
        form.attach(self._browse_button(self._choose_psp), 2, 0, 1, 1)
        self.title_entry = self._entry_row(form, 1, _("Title"))
        self.id_entry = self._entry_row(form, 2, _("Game ID"))
        page.append(form)

        options = Gtk.FlowBox(selection_mode=Gtk.SelectionMode.NONE,
                              column_spacing=12, row_spacing=6,
                              min_children_per_line=1, max_children_per_line=3)
        options.add_css_class("section")
        self.no_assets = self._check(_("Disable Asset Downloads"))
        self.no_libcrypt = self._check(_("Disable LibCrypt Patching"))
        self.use_cdda = self._check(_("Use CDDA"))
        self.undither = self._check(_("PSX-undither"))
        self.watermark = self._check(_("Watermark"))
        self.nopstitleimg = self._check(_("Disable PSTITLEIMG"))
        self.pic1aslogo = self._check(_("Use PIC1 as Logo"))
        self.force_ntsc = self._check(_("Force NTSC"))
        self.swap_disc = self._check(_("Allow Disc Swapping"))
        for check in (self.no_assets, self.no_libcrypt, self.use_cdda, self.undither, self.watermark, self.nopstitleimg, self.pic1aslogo, self.force_ntsc, self.swap_disc):
            options.insert(check, -1)
        page.append(options)

        assets = Gtk.Grid(column_spacing=12, row_spacing=8)
        assets.add_css_class("section")
        assets.attach(Gtk.Label(label=_("Assets and Theme"), xalign=0), 0, 0, 1, 1)
        themes = Gtk.StringList.new([_("Automatic"), "98DEMAKE", "ASCIIART", "DOTPAINTING", "OILPAINTING", "WATERCOLOR", "COLORSKETCH"])
        self.theme = Gtk.DropDown(model=themes)
        assets.attach(self.theme, 1, 0, 2, 1)
        self.theme.connect("notify::selected", self._theme_changed)
        for row, key, label in ((1, "cover", _("Cover / ICON0")), (2, "pic0", "PIC0"), (3, "pic1", "PIC1"), (4, "logo", _("Logo")), (5, "snd0", "SND0"), (6, "manual", _("Manual"))):
            entry = self._asset_row(assets, row, key, label)
            self.asset_entries[key] = entry
        previews = Gtk.FlowBox(selection_mode=Gtk.SelectionMode.NONE,
                               column_spacing=12, row_spacing=12,
                               min_children_per_line=1, max_children_per_line=3)
        for key, label in (("cover", "ICON0"), ("pic0", "PIC0"), ("pic1", "PIC1")):
            card = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
            picture = Gtk.Picture()
            picture.set_size_request(144, 90)
            picture.set_can_shrink(True)
            picture.set_content_fit(Gtk.ContentFit.CONTAIN)
            picture.add_css_class("preview")
            self.preview[key] = picture
            card.append(picture)
            card.append(Gtk.Label(label=label))
            previews.insert(card, -1)
        assets.attach(previews, 0, 7, 3, 1)
        self.xmb_button = Gtk.Button(label=_("XMB Preview"))
        self.xmb_button.set_halign(Gtk.Align.END)
        self.xmb_button.connect("clicked", self._preview_xmb)
        assets.attach(self.xmb_button, 0, 8, 3, 1)
        page.append(assets)

        actions = Gtk.Box(spacing=10, halign=Gtk.Align.END)
        self.status = Gtk.Label(label=_("Ready"), xalign=0, wrap=True)
        actions.append(self.status)
        self.cancel_button = Gtk.Button(label=_("Cancel"))
        self.cancel_button.set_sensitive(False)
        self.cancel_button.connect("clicked", self._cancel)
        actions.append(self.cancel_button)
        self.convert_button = Gtk.Button(label=_("Create EBOOT.PBP"))
        self.convert_button.add_css_class("suggested-action")
        self.convert_button.connect("clicked", self._convert)
        actions.append(self.convert_button)
        page.append(actions)
        page.append(self._build_log())
        return page

    def _build_extract(self) -> Gtk.Widget:
        page = self._page(_("Extract EBOOT.PBP Contents"), _("Export disc images, CUE files and audio tracks stored in the PBP."))
        form = Gtk.Grid(column_spacing=12, row_spacing=12)
        form.add_css_class("section")
        self.extract_pbp = self._entry_row(form, 0, _("PBP File"))
        form.attach(self._browse_button(self._choose_pbp), 2, 0, 1, 1)
        self.extract_dir = self._entry_row(form, 1, _("Output Folder"))
        form.attach(self._browse_button(self._choose_extract_dir), 2, 1, 1, 1)
        page.append(form)
        action = Gtk.Button(label=_("Extract Contents"), halign=Gtk.Align.END)
        self.extract_button = action
        action.add_css_class("suggested-action")
        action.connect("clicked", self._extract)
        page.append(action)
        page.append(self._build_log())
        return page

    def _build_log(self) -> Gtk.Widget:
        frame = Gtk.Frame(label="Log")
        frame.set_vexpand(True)
        log_view = Gtk.TextView(editable=False, monospace=True, wrap_mode=Gtk.WrapMode.WORD_CHAR)
        self.log_views.append(log_view)
        scroll = Gtk.ScrolledWindow()
        scroll.set_min_content_height(130)
        scroll.set_child(log_view)
        frame.set_child(scroll)
        return frame

    @staticmethod
    def _entry_row(grid: Gtk.Grid, row: int, label: str) -> Gtk.Entry:
        grid.attach(Gtk.Label(label=label, xalign=0), 0, row, 1, 1)
        entry = Gtk.Entry(hexpand=True)
        grid.attach(entry, 1, row, 1, 1)
        return entry

    def _asset_row(self, grid: Gtk.Grid, row: int, key: str, label: str) -> Gtk.Entry:
        grid.attach(Gtk.Label(label=label, xalign=0), 0, row, 1, 1)
        entry = Gtk.Entry(hexpand=True)
        grid.attach(entry, 1, row, 1, 1)
        button = self._browse_button(lambda *_: self._choose_asset(key))
        grid.attach(button, 2, row, 1, 1)
        return entry

    @staticmethod
    def _check(label: str) -> Gtk.CheckButton:
        return Gtk.CheckButton(label=label)

    @staticmethod
    def _browse_button(callback) -> Gtk.Button:
        button = Gtk.Button(label=_("Browse…"))
        button.connect("clicked", callback)
        return button

    def _dialog(self) -> Gtk.FileDialog:
        dialog = Gtk.FileDialog()
        dialog.set_initial_folder(Gio.File.new_for_path(str(self.folder_history.current())))
        return dialog

    def _choose_inputs(self, _button) -> None:
        if self.busy:
            return
        self._dialog().open_multiple(self.window, None, self._inputs_done)

    def _inputs_done(self, dialog, result) -> None:
        if self.busy:
            return
        try:
            files = dialog.open_multiple_finish(result)
        except GLib.Error:
            return
        for index in range(files.get_n_items()):
            path = files.get_item(index).get_path()
            self.folder_history.remember(path)
            if path and path not in self.files:
                self.files.append(path)
                item = Gtk.Label(label=path, xalign=0, ellipsize=Pango.EllipsizeMode.MIDDLE)
                item.set_tooltip_text(path)
                self.file_list.append(item)
        if self.files:
            self._begin_progress(_("Analyzing disc and loading assets"))
            threading.Thread(target=self._analyze_first, args=(self.files[0],), daemon=True).start()

    def _analyze_first(self, path: str) -> None:
        backend = ROOT / "psp_backend.py"
        try:
            process = subprocess.Popen([str(PYTHON), "-u", str(backend), "analyze", path], cwd=ROOT,
                                       stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                                       start_new_session=True)
            self.process = process
            if self.cancel_requested:
                self._terminate_process()
            output, errors = process.communicate()
            if process.returncode != 0:
                raise RuntimeError(errors or _("Analysis exited with code {code}").format(code=process.returncode))
            data = json.loads(output)
            GLib.idle_add(self._apply_backend_metadata, data)
        except Exception as exc:
            GLib.idle_add(self._append_log, _("Analysis did not complete: {error}\n").format(error=exc))
            GLib.idle_add(self._analysis_failed)

    def _analysis_failed(self) -> bool:
        self.process = None
        self._end_progress(_("Operation stopped") if self.cancel_requested else _("Analysis failed: check the log"))
        return GLib.SOURCE_REMOVE

    def _apply_backend_metadata(self, data: dict) -> bool:
        self.process = None
        if self.cancel_requested:
            self._end_progress(_("Operation stopped"))
            return GLib.SOURCE_REMOVE
        self.id_entry.set_text(data.get("disc_id", ""))
        self.title_entry.set_text(data.get("title", ""))
        if data.get("manual"):
            self.asset_entries["manual"].set_text(data["manual"])
        for key, filename in data.get("assets", {}).items():
            if key in self.asset_entries:
                self.asset_entries[key].set_text(filename)
            if key in self.preview:
                self.preview[key].set_filename(filename)
        if data.get("cdda"):
            self.use_cdda.set_active(True)
        self._end_progress(_("Automatic data loaded"), success=True)
        return GLib.SOURCE_REMOVE

    def _choose_psp(self, _button) -> None:
        self._dialog().select_folder(self.window, None, self._folder_done, self.psp_entry)

    def _choose_pbp(self, _button) -> None:
        self._dialog().open(self.window, None, self._file_done, self.extract_pbp)

    def _choose_extract_dir(self, _button) -> None:
        self._dialog().select_folder(self.window, None, self._folder_done, self.extract_dir)

    def _choose_asset(self, key: str) -> None:
        self._dialog().open(self.window, None, self._file_done, self.asset_entries[key])

    def _file_done(self, dialog, result, target: Gtk.Entry) -> None:
        try:
            path = dialog.open_finish(result).get_path()
        except GLib.Error:
            return
        if not path:
            return
        self.folder_history.remember(path)
        target.set_text(path)
        self._set_preview_for_entry(target, path)

    def _folder_done(self, dialog, result, target: Gtk.Entry) -> None:
        try:
            path = dialog.select_folder_finish(result).get_path()
        except GLib.Error:
            return
        if path:
            self.folder_history.remember(path, directory=True)
            target.set_text(path)

    def _set_preview_for_entry(self, target: Gtk.Entry, path: str) -> None:
        for key, entry in self.asset_entries.items():
            if target is entry and key in self.preview:
                self.preview[key].set_filename(path)

    def _theme_changed(self, *_args) -> None:
        self.xmb_button.set_tooltip_text(_("Show the selected theme with the game's actual images"))

    def _preview_xmb(self, _button) -> None:
        if self.busy:
            return
        if not self.id_entry.get_text():
            self._append_log(_("Load a disc first and wait for its data to be acquired.\n"))
            return
        directory = WORK_ROOT / "previews" / uuid.uuid4().hex
        directory.mkdir(parents=True, exist_ok=True)
        config_path = directory / "config.json"
        config_path.write_text(json.dumps(self._conversion_config()), encoding="utf-8")
        window = Adw.Window(transient_for=self.window, modal=False)
        window.set_title(_("XMB Preview · {theme}").format(theme=self.theme.get_selected_item().get_string()))
        window.set_default_size(720, 480)
        toolbar = Adw.ToolbarView()
        toolbar.add_top_bar(Adw.HeaderBar())
        content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12)
        for edge in ("top", "bottom", "start", "end"):
            getattr(content, "set_margin_" + edge)(16)
        message = Gtk.Label(label=_("Preparing theme images…"), wrap=True)
        picture = Gtk.Picture(can_shrink=True, content_fit=Gtk.ContentFit.CONTAIN, vexpand=True)
        content.append(picture)
        content.append(message)
        toolbar.set_content(content)
        window.set_content(toolbar)
        window.present()
        self._begin_progress(_("Preparing XMB preview"))
        threading.Thread(target=self._preview_worker, args=(config_path, directory, picture, message), daemon=True).start()

    def _preview_worker(self, config_path, directory, picture, message) -> None:
        try:
            process = subprocess.Popen(
                [str(PYTHON), "-u", str(ROOT / "psp_backend.py"), "preview", "--config", str(config_path), "--workdir", str(directory)],
                cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True,
            )
            self.process = process
            if self.cancel_requested:
                self._terminate_process()
            output, errors = process.communicate()
            if process.returncode:
                raise RuntimeError(errors or _("Preparation interrupted"))
            result = json.loads(output)
            GLib.idle_add(self._preview_ready, picture, message, result["image"])
        except Exception as exc:
            GLib.idle_add(self._preview_failed, message, str(exc))

    def _preview_ready(self, picture, message, filename) -> bool:
        self.process = None
        if self.cancel_requested:
            return self._preview_failed(message, _("Operation stopped"))
        picture.set_filename(filename)
        message.set_text(_("PSP XMB simulation · actual images from the selected theme. System graphics may differ on the console; audio is not played."))
        self._end_progress(_("XMB preview ready"), success=True)
        return GLib.SOURCE_REMOVE

    def _preview_failed(self, message, error) -> bool:
        self.process = None
        message.set_text(_("Preview unavailable. Check the log."))
        self._append_log(_("XMB preview: {error}\n").format(error=error))
        self._end_progress(_("Operation stopped") if self.cancel_requested else _("Preview unavailable"))
        return GLib.SOURCE_REMOVE

    def _convert(self, _button) -> None:
        if self.busy:
            return
        if not self.files or not self.psp_entry.get_text():
            self._append_log(_("Select at least one disc image and the PSP root folder.\n"))
            return
        config_dir = WORK_ROOT
        config_dir.mkdir(parents=True, exist_ok=True)
        config_path = config_dir / "gtk-config.json"
        config_path.write_text(json.dumps(self._conversion_config()), encoding="utf-8")
        command = [str(PYTHON), str(ROOT / "psp_backend.py"), "create", "--config", str(config_path)]
        self._run(command, _("Conversion"))

    def _conversion_config(self) -> dict:
        assets = {key: entry.get_text() for key, entry in self.asset_entries.items() if entry.get_text() and key != "manual"}
        return {
            "theme": ("", "98DEMAKE", "ASCIIART", "DOTPAINTING", "OILPAINTING", "WATERCOLOR", "COLORSKETCH")[self.theme.get_selected()],
            "files": self.files,
            "output": self.psp_entry.get_text(),
            "title": self.title_entry.get_text(),
            "disc_ids": [self.id_entry.get_text()] if self.id_entry.get_text() else None,
            "assets": assets,
            "manual": self.asset_entries["manual"].get_text() or None,
            "snd0": self.asset_entries["snd0"].get_text() or None,
            "options": {
                "no_libcrypt": self.no_libcrypt.get_active(),
                "use_cdda": self.use_cdda.get_active(),
                "undither": self.undither.get_active(),
                "watermark": self.watermark.get_active(),
                "nopstitleimg": self.nopstitleimg.get_active(),
                "pic1aslogo": self.pic1aslogo.get_active(),
                "force_ntsc": self.force_ntsc.get_active(),
                "swap_disc": self.swap_disc.get_active(),
            },
        }

    def _extract(self, _button) -> None:
        if self.busy:
            return
        if not self.extract_pbp.get_text() or not self.extract_dir.get_text():
            self._append_log(_("Select the PBP and an output folder.\n"))
            return
        Path(self.extract_dir.get_text()).mkdir(parents=True, exist_ok=True)
        command = [str(PYTHON), str(ROOT / "popstation.py"), "-v", "dump_pbp", self.extract_pbp.get_text()]
        self._run(command, _("Extraction"), cwd=Path(self.extract_dir.get_text()))

    def _run(self, command: list[str], label: str, cwd: Path = ROOT) -> None:
        if self.busy:
            return
        self._append_log("$ " + " ".join(command) + "\n")
        command.insert(1, "-u")
        try:
            self.process = subprocess.Popen(command, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                            text=True, bufsize=1, start_new_session=True)
        except OSError as exc:
            self._append_log(_("Could not start: {error}\n").format(error=exc))
            return
        self._begin_progress(_("{operation} in progress").format(operation=label))
        threading.Thread(target=self._read_process, args=(label,), daemon=True).start()

    def _read_process(self, label: str) -> None:
        assert self.process and self.process.stdout
        for line in self.process.stdout:
            if line.startswith("POPFE_PHASE: "):
                GLib.idle_add(self._set_phase, line.removeprefix("POPFE_PHASE: ").strip())
            elif line.startswith("Dumping "):
                GLib.idle_add(self._set_phase, _("Extracting: {section}").format(section=line.removeprefix("Dumping ").strip()))
            GLib.idle_add(self._append_log, line)
        code = self.process.wait()
        GLib.idle_add(self._finished, label, code)

    def _finished(self, label: str, code: int) -> bool:
        self.process = None
        message = _("Operation stopped") if self.cancel_requested else (_("Completed") if code == 0 else _("Error (code {code}): check the log").format(code=code))
        self._end_progress(message, success=code == 0 and not self.cancel_requested)
        return GLib.SOURCE_REMOVE

    def _cancel(self, _button) -> None:
        if self.busy:
            self.cancel_requested = True
            self.phase = _("Stopping")
            self.stop_button.set_sensitive(False)
            self._terminate_process()

    def _terminate_process(self) -> None:
        process = self.process
        if process is not None:
            try:
                os.killpg(process.pid, signal.SIGTERM)
                GLib.timeout_add_seconds(3, self._force_stop, process)
            except ProcessLookupError:
                pass

    @staticmethod
    def _force_stop(process) -> bool:
        if process.poll() is None:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        return GLib.SOURCE_REMOVE

    def _append_log(self, text: str) -> bool:
        for log_view in self.log_views:
            buffer = log_view.get_buffer()
            buffer.insert(buffer.get_end_iter(), text, -1)
        return GLib.SOURCE_REMOVE


if __name__ == "__main__":
    PSPGtk().run(None)
