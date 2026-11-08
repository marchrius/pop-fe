use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use adw::prelude::*;
use gtk::gio;

use crate::i18n::tr;
use crate::paths::FolderHistory;

enum TaskMessage<T> {
    Progress(f64),
    Warning(String),
    Finished(T),
}

#[derive(Clone)]
struct DiscFields {
    id: gtk::Entry,
    title: gtk::Entry,
    psp_root: gtk::Entry,
    cdda: gtk::CheckButton,
    disable_libcrypt_patching: gtk::CheckButton,
    force_ntsc: gtk::CheckButton,
    allow_disc_swapping: gtk::CheckButton,
    disable_title_image: gtk::CheckButton,
    use_pic1_logo: gtk::CheckButton,
    watermark: gtk::CheckButton,
    psx_undither: gtk::CheckButton,
    disable_asset_downloads: gtk::CheckButton,
    status: gtk::Label,
    assets: Rc<RefCell<crate::create::PbpAssets>>,
    base_assets: Rc<RefCell<crate::create::PbpAssets>>,
    asset_overrides: Rc<RefCell<crate::create::PbpAssets>>,
    asset_override_entries: Rc<RefCell<Vec<gtk::Entry>>>,
    manual_entry: gtk::Entry,
    theme: gtk::DropDown,
    analysis_generation: Rc<Cell<u64>>,
    theme_generation: Rc<Cell<u64>>,
    theme_loading: Rc<Cell<bool>>,
    icon_preview: gtk::Picture,
    pic0_preview: gtk::Picture,
    pic1_preview: gtk::Picture,
}

const THEME_NAMES: [&str; 7] = [
    "Automatic",
    "98DEMAKE",
    "ASCIIART",
    "DOTPAINTING",
    "OILPAINTING",
    "WATERCOLOR",
    "COLORSKETCH",
];

pub fn activate(application: &adw::Application) {
    let operation_busy = Rc::new(Cell::new(false));
    let window = adw::ApplicationWindow::builder()
        .application(application)
        .title("POP-FE PSP Manager")
        .default_width(820)
        .default_height(740)
        .build();

    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::SlideLeftRight)
        .hexpand(true)
        .vexpand(true)
        .build();
    stack.add_titled(
        &convert_page(&window, operation_busy.clone()),
        Some("convert"),
        &tr("Convert"),
    );
    stack.add_titled(
        &extract_page(&window, operation_busy.clone()),
        Some("extract"),
        &tr("Extract"),
    );

    let switcher = gtk::StackSwitcher::builder()
        .stack(&stack)
        .halign(gtk::Align::Center)
        .build();
    let header = adw::HeaderBar::builder().title_widget(&switcher).build();
    let menu = gio::Menu::new();
    menu.append(Some(&tr("About POP-FE")), Some("win.about"));
    menu.append(Some(&tr("Quit")), Some("win.quit"));
    let menu_button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text(tr("Main Menu"))
        .popover(&gtk::PopoverMenu::from_model(Some(&menu)))
        .build();
    header.pack_end(&menu_button);

    let about = gio::SimpleAction::new("about", None);
    let about_parent = window.clone();
    about.connect_activate(move |_, _| {
        adw::AboutWindow::builder()
            .transient_for(&about_parent)
            .application_name("POP-FE PSP Manager")
            .application_icon("org.popfe.PSPManager")
            .version(env!("CARGO_PKG_VERSION"))
            .developer_name("POP-FE contributors")
            .comments(tr(
                "Convert and extract PlayStation disc images for PSP with POP-FE.",
            ))
            .license_type(gtk::License::Lgpl21)
            .website("https://github.com/selphie87/pop-fe")
            .build()
            .present();
    });
    window.add_action(&about);

    let quit = gio::SimpleAction::new("quit", None);
    let quit_window = window.clone();
    quit.connect_activate(move |_, _| quit_window.close());
    window.add_action(&quit);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&stack));
    window.set_content(Some(&toolbar));
    window.present();
}

fn page(title: &str, subtitle: &str) -> (gtk::ScrolledWindow, gtk::Box) {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(14)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();
    let heading = gtk::Label::builder()
        .label(title)
        .xalign(0.0)
        .wrap(true)
        .css_classes(["title-2"])
        .build();
    let description = gtk::Label::builder()
        .label(subtitle)
        .xalign(0.0)
        .wrap(true)
        .css_classes(["dim-label"])
        .build();
    content.append(&heading);
    content.append(&description);

    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .hexpand(true)
        .vexpand(true)
        .child(&content)
        .build();
    (scroll, content)
}

fn section(title: &str) -> (gtk::Frame, gtk::Box) {
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    let frame = gtk::Frame::builder().label(title).child(&body).build();
    (frame, body)
}

fn convert_page(
    parent: &adw::ApplicationWindow,
    operation_busy: Rc<Cell<bool>>,
) -> gtk::ScrolledWindow {
    let (scroll, content) = page(
        &tr("Convert PSX Images to EBOOT.PBP"),
        &tr("Automatic assets, themes and previews are used during conversion."),
    );
    let files: Rc<RefCell<Vec<PathBuf>>> = Rc::default();
    let folder_history = Rc::new(RefCell::new(FolderHistory::load()));
    let (settings, settings_body) = section(&tr("Game Details"));
    let psp_root = append_entry(&settings_body, &tr("PSP Root Folder"), "");
    let title = append_entry(&settings_body, &tr("Title"), "");
    let id = append_entry(&settings_body, &tr("Game ID"), "");
    let icon_preview = asset_preview(64, 64);
    let pic0_preview = asset_preview(132, 76);
    let pic1_preview = asset_preview(160, 90);
    let theme = gtk::DropDown::from_strings(&THEME_NAMES);
    let manual_entry = gtk::Entry::builder().hexpand(true).build();
    let theme_generation = Rc::new(Cell::new(0));
    let theme_loading = Rc::new(Cell::new(false));
    let fields = DiscFields {
        id,
        title,
        psp_root,
        cdda: gtk::CheckButton::with_label(&tr("Use CDDA")),
        disable_libcrypt_patching: gtk::CheckButton::with_label(&tr("Disable LibCrypt Patching")),
        force_ntsc: gtk::CheckButton::with_label(&tr("Force NTSC")),
        allow_disc_swapping: gtk::CheckButton::with_label(&tr("Allow Disc Swapping")),
        disable_title_image: gtk::CheckButton::with_label(&tr("Disable PSTITLEIMG")),
        use_pic1_logo: gtk::CheckButton::with_label(&tr("Use PIC1 as Logo")),
        watermark: gtk::CheckButton::with_label(&tr("Watermark")),
        psx_undither: gtk::CheckButton::with_label(&tr("PSX-undither")),
        disable_asset_downloads: gtk::CheckButton::with_label(&tr("Disable Asset Downloads")),
        status: gtk::Label::new(Some(&tr("Ready"))),
        assets: Rc::new(RefCell::new(crate::create::PbpAssets::default())),
        base_assets: Rc::new(RefCell::new(crate::create::PbpAssets::default())),
        asset_overrides: Rc::new(RefCell::new(crate::create::PbpAssets::default())),
        asset_override_entries: Rc::new(RefCell::new(Vec::new())),
        manual_entry,
        theme,
        analysis_generation: Rc::new(Cell::new(0)),
        theme_generation,
        theme_loading,
        icon_preview,
        pic0_preview,
        pic1_preview,
    };
    let fields_for_download_toggle = fields.clone();
    let files_for_download_toggle = files.clone();
    fields.disable_asset_downloads.connect_toggled(move |_| {
        if !files_for_download_toggle.borrow().is_empty() {
            analyze_first(&files_for_download_toggle, &fields_for_download_toggle);
        }
    });
    let (frame, body) = section(&tr("Disc Images"));
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .build();
    list.add_css_class("boxed-list");
    body.append(&list);
    let actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    let add = gtk::Button::with_label(&tr("Add Images…"));
    add.add_css_class("suggested-action");
    add.set_hexpand(true);
    let clear = gtk::Button::with_label(&tr("Clear Images"));
    clear.set_sensitive(false);
    let parent_for_dialog = parent.clone();
    let files_for_dialog = files.clone();
    let list_for_dialog = list.clone();
    let clear_for_dialog = clear.clone();
    let fields_for_dialog = fields.clone();
    let history_for_dialog = folder_history.clone();
    add.connect_clicked(move |_| {
        let chooser = gtk::FileDialog::builder()
            .title(&tr("Select Disc Images"))
            .modal(true)
            .build();
        let initial = gio::File::for_path(history_for_dialog.borrow().current());
        chooser.set_initial_folder(Some(&initial));
        let files = files_for_dialog.clone();
        let list = list_for_dialog.clone();
        let clear = clear_for_dialog.clone();
        let fields = fields_for_dialog.clone();
        let history = history_for_dialog.clone();
        chooser.open_multiple(
            Some(&parent_for_dialog),
            None::<&gio::Cancellable>,
            move |result| {
                if let Ok(selected) = result {
                    for index in 0..selected.n_items() {
                        if let Some(file) = selected
                            .item(index)
                            .and_then(|item| item.downcast::<gio::File>().ok())
                            && let Some(path) = file.path()
                        {
                            history.borrow_mut().remember_file(&path);
                            let mut current = files.borrow_mut();
                            if !current.contains(&path) {
                                current.push(path);
                            }
                        }
                    }
                    refresh_disc_list(&list, &files, &clear, &fields);
                    analyze_first(&files, &fields);
                }
            },
        );
    });
    let list_for_clear = list.clone();
    let files_for_clear = files.clone();
    let clear_for_clear = clear.clone();
    let fields_for_clear = fields.clone();
    clear.connect_clicked(move |_| {
        files_for_clear.borrow_mut().clear();
        refresh_disc_list(
            &list_for_clear,
            &files_for_clear,
            &clear_for_clear,
            &fields_for_clear,
        );
        reset_disc_fields(&fields_for_clear);
    });
    actions.append(&add);
    actions.append(&clear);
    body.append(&actions);
    content.append(&frame);
    content.append(&settings);

    let (options, option_body) = section(&tr("Options"));
    for label in [
        "Disable Asset Downloads",
        "Disable LibCrypt Patching",
        "Use CDDA",
        "PSX-undither",
        "Watermark",
        "Disable PSTITLEIMG",
        "Use PIC1 as Logo",
        "Force NTSC",
        "Allow Disc Swapping",
    ] {
        match label {
            "Use CDDA" => {
                fields.cdda.set_sensitive(true);
                fields.cdda.set_tooltip_text(Some(&tr(
                    "Store CD audio as raw CDDA tracks instead of ATRAC3",
                )));
                option_body.append(&fields.cdda);
            }
            "Disable LibCrypt Patching" => {
                fields.disable_libcrypt_patching.set_tooltip_text(Some(&tr(
                    "Skip the automatic LibCrypt sector patch when one is available",
                )));
                option_body.append(&fields.disable_libcrypt_patching);
            }
            "Disable Asset Downloads" => option_body.append(&fields.disable_asset_downloads),
            "Force NTSC" => option_body.append(&fields.force_ntsc),
            "Allow Disc Swapping" => option_body.append(&fields.allow_disc_swapping),
            "Disable PSTITLEIMG" => option_body.append(&fields.disable_title_image),
            "Use PIC1 as Logo" => option_body.append(&fields.use_pic1_logo),
            "Watermark" => option_body.append(&fields.watermark),
            "PSX-undither" => option_body.append(&fields.psx_undither),
            _ => {
                let option = gtk::CheckButton::with_label(&tr(label));
                option.set_sensitive(false);
                option.set_tooltip_text(Some(&tr("This option is not ported yet")));
                option_body.append(&option);
            }
        }
    }
    content.append(&options);
    let (artwork, artwork_body) = section(&tr("Game Artwork"));
    for (key, label) in [
        ("icon0", "ICON0"),
        ("pic0", "PIC0"),
        ("pic1", "PIC1"),
        ("logo", "Logo"),
        ("snd0", "SND0"),
    ] {
        let entry = append_entry(
            &artwork_body,
            &tr("Override {name}").replace("{name}", label),
            "",
        );
        let browse = gtk::Button::with_label(&tr("Browse…"));
        fields
            .asset_override_entries
            .borrow_mut()
            .push(entry.clone());
        if let Some(row) = entry
            .parent()
            .and_then(|widget| widget.downcast::<gtk::Box>().ok())
        {
            row.append(&browse);
        }
        let fields_for_change = fields.clone();
        let key_for_change = key.to_string();
        entry.connect_changed(move |entry| {
            let text = entry.text();
            set_asset_override(
                &fields_for_change,
                &key_for_change,
                (!text.is_empty()).then(|| PathBuf::from(text.as_str())),
            );
            refresh_asset_previews(&fields_for_change);
        });
        let parent_for_dialog = parent.clone();
        let entry_for_dialog = entry.clone();
        let history_for_dialog = folder_history.clone();
        browse.connect_clicked(move |_| {
            let dialog = gtk::FileDialog::builder()
                .title(&tr("Select Asset File"))
                .build();
            dialog.set_initial_folder(Some(&gio::File::for_path(
                history_for_dialog.borrow().current(),
            )));
            let entry = entry_for_dialog.clone();
            let history = history_for_dialog.clone();
            dialog.open(
                Some(&parent_for_dialog),
                None::<&gio::Cancellable>,
                move |result| {
                    if let Ok(file) = result
                        && let Some(path) = file.path()
                    {
                        history.borrow_mut().remember_file(&path);
                        entry.set_text(&path.to_string_lossy());
                    }
                },
            );
        });
    }
    let manual_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .build();
    let manual_label = gtk::Label::new(Some(&tr("Manual / PDF / ZIP")));
    manual_label.set_xalign(0.0);
    manual_label.set_width_chars(22);
    manual_row.append(&manual_label);
    manual_row.append(&fields.manual_entry);
    let manual_browse = gtk::Button::with_label(&tr("Browse…"));
    manual_row.append(&manual_browse);
    artwork_body.append(&manual_row);
    let parent_for_manual = parent.clone();
    let manual_entry = fields.manual_entry.clone();
    let history_for_manual = folder_history.clone();
    manual_browse.connect_clicked(move |_| {
        let dialog = gtk::FileDialog::builder()
            .title(&tr("Select Manual File"))
            .build();
        dialog.set_initial_folder(Some(&gio::File::for_path(
            history_for_manual.borrow().current(),
        )));
        let entry = manual_entry.clone();
        let history = history_for_manual.clone();
        dialog.open(
            Some(&parent_for_manual),
            None::<&gio::Cancellable>,
            move |result| {
                if let Ok(file) = result
                    && let Some(path) = file.path()
                {
                    history.borrow_mut().remember_file(&path);
                    entry.set_text(&path.to_string_lossy());
                }
            },
        );
    });
    let artwork_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .halign(gtk::Align::Center)
        .build();
    for (label, picture) in [
        ("ICON0", &fields.icon_preview),
        ("PIC0", &fields.pic0_preview),
        ("PIC1", &fields.pic1_preview),
    ] {
        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .halign(gtk::Align::Center)
            .build();
        let caption = gtk::Label::new(Some(label));
        caption.add_css_class("dim-label");
        column.append(&caption);
        column.append(picture);
        artwork_row.append(&column);
    }
    artwork_body.append(&artwork_row);
    let theme_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .build();
    let theme_label = gtk::Label::new(Some(&tr("Theme")));
    theme_label.set_xalign(0.0);
    theme_label.set_hexpand(true);
    theme_row.append(&theme_label);
    theme_row.append(&fields.theme);
    let preview_button = gtk::Button::with_label(&tr("XMB Preview"));
    let preview_parent = parent.clone();
    let preview_fields = fields.clone();
    let preview_busy = operation_busy.clone();
    preview_button.connect_clicked(move |_| {
        show_xmb_preview(&preview_parent, &preview_fields, &preview_busy)
    });
    theme_row.append(&preview_button);
    artwork_body.append(&theme_row);
    content.append(&artwork);
    let theme_fields = fields.clone();
    fields
        .theme
        .connect_selected_notify(move |_| refresh_theme_art(&theme_fields));
    fields.status.set_xalign(0.0);
    fields.status.add_css_class("dim-label");
    content.append(&fields.status);

    let convert = gtk::Button::with_label(&tr("Create EBOOT.PBP"));
    convert.add_css_class("suggested-action");
    convert.set_halign(gtk::Align::End);
    convert.set_sensitive(true);
    let root_browse = gtk::Button::with_label(&tr("Browse…"));
    let root = fields.psp_root.clone();
    let root_history = folder_history.clone();
    let root_parent = parent.clone();
    root_browse.connect_clicked(move |_| {
        let dialog = gtk::FileDialog::builder()
            .title(&tr("Select PSP Root Folder"))
            .build();
        dialog.set_initial_folder(Some(&gio::File::for_path(root_history.borrow().current())));
        let root = root.clone();
        let history = root_history.clone();
        dialog.select_folder(
            Some(&root_parent),
            None::<&gio::Cancellable>,
            move |result| {
                if let Ok(file) = result
                    && let Some(path) = file.path()
                {
                    history.borrow_mut().remember_directory(&path);
                    root.set_text(&path.to_string_lossy());
                }
            },
        );
    });
    if let Some(row) = fields
        .psp_root
        .parent()
        .and_then(|widget| widget.downcast::<gtk::Box>().ok())
    {
        row.append(&root_browse);
    }
    let files_for_create = files.clone();
    let fields_for_create = fields.clone();
    let button_for_create = convert.clone();
    let warning_state: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let warning_state_for_create = warning_state.clone();
    let operation_busy_for_create = operation_busy.clone();
    let progress = gtk::ProgressBar::new();
    progress.set_show_text(true);
    progress.set_text(Some(&tr("Ready")));
    content.append(&progress);
    let progress_for_create = progress.clone();
    let cancel = gtk::Button::with_label(&tr("Cancel"));
    cancel.set_sensitive(false);
    let cancel_state: Rc<RefCell<Option<Arc<AtomicBool>>>> = Rc::default();
    let state_for_cancel = cancel_state.clone();
    let fields_for_cancel = fields.clone();
    cancel.connect_clicked(move |_| {
        if let Some(token) = state_for_cancel.borrow().as_ref() {
            token.store(true, std::sync::atomic::Ordering::Relaxed);
            fields_for_cancel.status.set_text(&tr("Stopping"));
        }
    });
    let cancel_for_create = cancel.clone();
    let state_for_create = cancel_state.clone();
    convert.connect_clicked(move |_| {
        if operation_busy_for_create.get() {
            fields_for_create
                .status
                .set_text(&tr("Another operation is in progress"));
            return;
        }
        *warning_state_for_create.borrow_mut() = None;
        let selected = files_for_create.borrow().clone();
        if selected.is_empty() || selected.len() > 32 {
            fields_for_create
                .status
                .set_text(&tr("Select between one and 32 disc images"));
            return;
        }
        for image in &selected {
            let extension = image
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !["bin", "img", "iso", "cue", "chd", "ccd"].contains(&extension.as_str()) {
                fields_for_create
                    .status
                    .set_text(&tr("Select CUE, CCD, CHD, ISO or raw BIN/IMG images"));
                return;
            }
        }
        let id = fields_for_create.id.text().to_string();
        if fields_for_create.theme_loading.get() {
            fields_for_create
                .status
                .set_text(&tr("Wait for theme artwork to finish loading"));
            return;
        }
        if id.len() != 9 || fields_for_create.psp_root.text().is_empty() {
            fields_for_create.status.set_text(&tr(
                "Select a PSP folder and identify the disc before conversion",
            ));
            return;
        }
        let title = fields_for_create.title.text().to_string();
        let manual_source = if fields_for_create.manual_entry.text().is_empty() {
            None
        } else {
            Some(fields_for_create.manual_entry.text().to_string())
        };
        let mut assets = fields_for_create.assets.borrow().clone();
        if fields_for_create.use_pic1_logo.is_active() {
            assets.logo = assets.pic1.clone();
        }
        let psp = PathBuf::from(fields_for_create.psp_root.text().as_str());
        let output = psp.join("PSP/GAME").join(&id).join("EBOOT.PBP");
        let config = crate::create::PbpCreateConfig {
            images: selected,
            output: output.clone(),
            game_id: id,
            title,
            manual_source,
            assets,
            include_title_image: !fields_for_create.disable_title_image.is_active(),
            use_cdda: fields_for_create.cdda.is_active(),
            disable_libcrypt_patching: fields_for_create.disable_libcrypt_patching.is_active(),
            force_ntsc: fields_for_create.force_ntsc.is_active(),
            allow_disc_swapping: fields_for_create.allow_disc_swapping.is_active(),
            watermark: fields_for_create.watermark.is_active(),
            psx_undither: fields_for_create.psx_undither.is_active(),
        };
        fields_for_create.status.set_text(&tr("Writing EBOOT.PBP"));
        progress_for_create.set_text(Some(&tr("Conversion in progress")));
        progress_for_create.set_fraction(0.0);
        button_for_create.set_sensitive(false);
        cancel_for_create.set_sensitive(true);
        let cancellation = Arc::new(AtomicBool::new(false));
        *state_for_create.borrow_mut() = Some(cancellation.clone());
        operation_busy_for_create.set(true);
        let (sender, receiver) = mpsc::sync_channel(1);
        let progress_sender = sender.clone();
        let warning_sender = sender.clone();
        thread::spawn(move || {
            let result = crate::create::create_from_image_with_progress_and_warnings(
                &config,
                &cancellation,
                |fraction| {
                    let _ = progress_sender.try_send(TaskMessage::Progress(fraction));
                },
                |warning| {
                    let _ = warning_sender.send(TaskMessage::Warning(warning));
                },
            )
            .map_err(|error| error.to_string())
            .map(|()| output);
            let _ = sender.send(TaskMessage::Finished(result));
        });
        let fields = fields_for_create.clone();
        let button = button_for_create.clone();
        let progress = progress_for_create.clone();
        let cancel_button = cancel_for_create.clone();
        let cancel_state = state_for_create.clone();
        let warning_state = warning_state_for_create.clone();
        let operation_busy = operation_busy_for_create.clone();
        gtk::glib::timeout_add_local(Duration::from_millis(100), move || {
            match receiver.try_recv() {
                Ok(TaskMessage::Progress(fraction)) => {
                    progress.set_fraction(fraction);
                    progress.set_text(Some(&format!("{:.0}%", fraction * 100.0)));
                    gtk::glib::ControlFlow::Continue
                }
                Ok(TaskMessage::Warning(warning)) => {
                    fields
                        .status
                        .set_text(&tr("Conversion warning: {error}").replace("{error}", &warning));
                    *warning_state.borrow_mut() = Some(warning);
                    gtk::glib::ControlFlow::Continue
                }
                Ok(TaskMessage::Finished(Ok(path))) => {
                    operation_busy.set(false);
                    let mut message = format!("{}: {}", tr("Conversion complete"), path.display());
                    if let Some(warning) = warning_state.borrow().as_deref() {
                        message.push('\n');
                        message.push_str(
                            &tr("Manual could not be added: {error}").replace("{error}", warning),
                        );
                    }
                    fields.status.set_text(&message);
                    progress.set_fraction(1.0);
                    progress.set_text(Some(&tr("Completed")));
                    button.set_sensitive(true);
                    cancel_button.set_sensitive(false);
                    *cancel_state.borrow_mut() = None;
                    gtk::glib::ControlFlow::Break
                }
                Ok(TaskMessage::Finished(Err(error))) => {
                    operation_busy.set(false);
                    if error == "operation cancelled" {
                        fields.status.set_text(&tr("Operation stopped"));
                        progress.set_text(Some(&tr("Operation stopped")));
                    } else {
                        fields
                            .status
                            .set_text(&tr("Conversion failed: {error}").replace("{error}", &error));
                        progress.set_text(Some(&tr("Error")));
                    }
                    button.set_sensitive(true);
                    cancel_button.set_sensitive(false);
                    *cancel_state.borrow_mut() = None;
                    gtk::glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => gtk::glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => {
                    operation_busy.set(false);
                    fields.status.set_text(&tr("Conversion failed"));
                    button.set_sensitive(true);
                    cancel_button.set_sensitive(false);
                    *cancel_state.borrow_mut() = None;
                    gtk::glib::ControlFlow::Break
                }
            }
        });
    });
    let conversion_actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .build();
    conversion_actions.append(&cancel);
    conversion_actions.append(&convert);
    content.append(&conversion_actions);
    scroll
}

fn refresh_disc_list(
    list: &gtk::ListBox,
    files: &Rc<RefCell<Vec<PathBuf>>>,
    clear: &gtk::Button,
    fields: &DiscFields,
) {
    while let Some(row) = list.first_child() {
        list.remove(&row);
    }
    for path in files.borrow().iter() {
        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(8)
            .margin_end(8)
            .build();
        let label = gtk::Label::builder()
            .label(path.to_string_lossy())
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .hexpand(true)
            .build();
        label.set_tooltip_text(Some(&path.to_string_lossy()));
        row.append(&label);
        let remove = gtk::Button::from_icon_name("list-remove-symbolic");
        remove.add_css_class("flat");
        remove.set_tooltip_text(Some(&tr("Remove Image")));
        let path_to_remove = path.clone();
        let files = files.clone();
        let list_for_remove = list.clone();
        let clear = clear.clone();
        let fields = fields.clone();
        remove.connect_clicked(move |_| {
            let removed_first = files.borrow().first() == Some(&path_to_remove);
            files.borrow_mut().retain(|file| file != &path_to_remove);
            refresh_disc_list(&list_for_remove, &files, &clear, &fields);
            if removed_first {
                reset_disc_fields(&fields);
                analyze_first(&files, &fields);
            }
        });
        row.append(&remove);
        list.append(&row);
    }
    clear.set_sensitive(!files.borrow().is_empty());
}

fn analyze_first(files: &Rc<RefCell<Vec<PathBuf>>>, fields: &DiscFields) {
    let Some(path) = files.borrow().first().cloned() else {
        return;
    };
    fields
        .status
        .set_text(&tr("Analyzing disc and loading assets"));
    let generation = fields.analysis_generation.get().wrapping_add(1);
    fields.analysis_generation.set(generation);
    fields.id.set_text("");
    fields.title.set_text("");
    fields
        .theme_generation
        .set(fields.theme_generation.get().wrapping_add(1));
    fields.theme_loading.set(false);
    *fields.assets.borrow_mut() = crate::create::PbpAssets::default();
    *fields.base_assets.borrow_mut() = crate::create::PbpAssets::default();
    set_picture(&fields.icon_preview, None);
    set_picture(&fields.pic0_preview, None);
    set_picture(&fields.pic1_preview, None);
    let download_assets = !fields.disable_asset_downloads.is_active();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let result = crate::database::Database::embedded()
            .analyze_image(&path)
            .map(|metadata| {
                let assets = if download_assets {
                    crate::assets::load_game_assets(&metadata.id)
                } else {
                    crate::create::PbpAssets::default()
                };
                (metadata, assets)
            });
        let _ = sender.send(result.map_err(|error| error.to_string()));
    });
    let fields = fields.clone();
    gtk::glib::timeout_add_local(Duration::from_millis(80), move || {
        if fields.analysis_generation.get() != generation {
            return gtk::glib::ControlFlow::Break;
        }
        match receiver.try_recv() {
            Ok(Ok((metadata, assets))) => {
                fields.id.set_text(&metadata.id);
                fields.title.set_text(&metadata.title);
                fields.cdda.set_active(metadata.use_cdda);
                if fields.manual_entry.text().is_empty() {
                    fields
                        .manual_entry
                        .set_text(metadata.manual.as_deref().unwrap_or(""));
                }
                *fields.base_assets.borrow_mut() = assets;
                refresh_theme_art(&fields);
                fields.status.set_text(&tr("Automatic data loaded"));
                gtk::glib::ControlFlow::Break
            }
            Ok(Err(_error)) => {
                fields.id.set_text("UNKN00000");
                *fields.assets.borrow_mut() = crate::create::PbpAssets::default();
                fields
                    .status
                    .set_text(&tr("Analysis failed: check the log"));
                gtk::glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => gtk::glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => {
                fields
                    .status
                    .set_text(&tr("Analysis failed: check the log"));
                gtk::glib::ControlFlow::Break
            }
        }
    });
}

fn reset_disc_fields(fields: &DiscFields) {
    fields
        .analysis_generation
        .set(fields.analysis_generation.get().wrapping_add(1));
    fields.id.set_text("");
    fields.title.set_text("");
    fields.cdda.set_active(false);
    fields.status.set_text(&tr("Ready"));
    fields
        .theme_generation
        .set(fields.theme_generation.get().wrapping_add(1));
    fields.theme_loading.set(false);
    *fields.assets.borrow_mut() = crate::create::PbpAssets::default();
    *fields.base_assets.borrow_mut() = crate::create::PbpAssets::default();
    *fields.asset_overrides.borrow_mut() = crate::create::PbpAssets::default();
    for entry in fields.asset_override_entries.borrow().iter() {
        entry.set_text("");
    }
    fields.manual_entry.set_text("");
    set_picture(&fields.icon_preview, None);
    set_picture(&fields.pic0_preview, None);
    set_picture(&fields.pic1_preview, None);
}

fn refresh_theme_art(fields: &DiscFields) {
    let game_id = fields.id.text().to_string();
    if game_id.is_empty() {
        return;
    }
    let selected = fields.theme.selected() as usize;
    let base = fields.base_assets.borrow().clone();
    let generation = fields.theme_generation.get().wrapping_add(1);
    fields.theme_generation.set(generation);
    fields.theme_loading.set(false);
    if selected == 0 || selected >= THEME_NAMES.len() {
        let mut assets = base;
        apply_asset_overrides(&mut assets, &fields.asset_overrides.borrow());
        *fields.assets.borrow_mut() = assets;
        refresh_asset_previews(fields);
        return;
    }
    fields.theme_loading.set(true);
    fields.status.set_text(&tr("Loading theme artwork"));
    let theme_name = THEME_NAMES[selected].to_string();
    let allow_downloads = !fields.disable_asset_downloads.is_active();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let assets = if allow_downloads {
            crate::assets::load_theme_assets(&game_id, &theme_name, &base)
        } else {
            crate::assets::load_theme_assets_offline(&game_id, &theme_name, &base)
        };
        let _ = sender.send(assets);
    });
    let fields = fields.clone();
    gtk::glib::timeout_add_local(Duration::from_millis(100), move || {
        match receiver.try_recv() {
            Ok(mut assets) if fields.theme_generation.get() == generation => {
                apply_asset_overrides(&mut assets, &fields.asset_overrides.borrow());
                fields.theme_loading.set(false);
                *fields.assets.borrow_mut() = assets;
                refresh_asset_previews(&fields);
                fields.status.set_text(&tr("Theme artwork loaded"));
                gtk::glib::ControlFlow::Break
            }
            Ok(_) => gtk::glib::ControlFlow::Break,
            Err(mpsc::TryRecvError::Empty) => gtk::glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => {
                if fields.theme_generation.get() == generation {
                    fields.theme_loading.set(false);
                }
                gtk::glib::ControlFlow::Break
            }
        }
    });
}

fn set_asset_override(fields: &DiscFields, key: &str, path: Option<PathBuf>) {
    let mut overrides = fields.asset_overrides.borrow_mut();
    let target = match key {
        "icon0" => &mut overrides.icon0,
        "pic0" => &mut overrides.pic0,
        "pic1" => &mut overrides.pic1,
        "logo" => &mut overrides.logo,
        "snd0" => &mut overrides.snd0,
        _ => return,
    };
    *target = path.clone();
    drop(overrides);

    if let Some(path) = path {
        let mut assets = fields.assets.borrow_mut();
        match key {
            "icon0" => assets.icon0 = Some(path),
            "pic0" => assets.pic0 = Some(path),
            "pic1" => assets.pic1 = Some(path),
            "logo" => assets.logo = Some(path),
            "snd0" => assets.snd0 = Some(path),
            _ => {}
        }
        drop(assets);
        refresh_asset_previews(fields);
    } else if !fields.id.text().is_empty() {
        refresh_theme_art(fields);
    }
}

fn apply_asset_overrides(
    assets: &mut crate::create::PbpAssets,
    overrides: &crate::create::PbpAssets,
) {
    if overrides.icon0.is_some() {
        assets.icon0.clone_from(&overrides.icon0);
    }
    if overrides.icon1.is_some() {
        assets.icon1.clone_from(&overrides.icon1);
    }
    if overrides.pic0.is_some() {
        assets.pic0.clone_from(&overrides.pic0);
    }
    if overrides.pic1.is_some() {
        assets.pic1.clone_from(&overrides.pic1);
    }
    if overrides.snd0.is_some() {
        assets.snd0.clone_from(&overrides.snd0);
    }
    if overrides.logo.is_some() {
        assets.logo.clone_from(&overrides.logo);
    }
}

fn refresh_asset_previews(fields: &DiscFields) {
    let assets = fields.assets.borrow();
    set_picture(&fields.icon_preview, assets.icon0.as_deref());
    set_picture(&fields.pic0_preview, assets.pic0.as_deref());
    set_picture(&fields.pic1_preview, assets.pic1.as_deref());
}

fn show_xmb_preview(
    parent: &adw::ApplicationWindow,
    fields: &DiscFields,
    operation_busy: &Rc<Cell<bool>>,
) {
    if operation_busy.get() {
        fields
            .status
            .set_text(&tr("Another operation is in progress"));
        return;
    }
    let game_id = fields.id.text().to_string();
    if game_id.is_empty() {
        fields
            .status
            .set_text(&tr("Load a disc before previewing the theme"));
        return;
    }
    let theme_name = THEME_NAMES
        .get(fields.theme.selected() as usize)
        .copied()
        .unwrap_or("Automatic");
    let window = gtk::Window::builder()
        .title(tr("XMB Preview · {theme}").replace("{theme}", theme_name))
        .transient_for(parent)
        .default_width(600)
        .default_height(390)
        .build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    let stage = gtk::Overlay::new();
    stage.set_size_request(480, 272);
    let background = asset_preview(480, 272);
    let assets = fields.assets.borrow().clone();
    set_picture(&background, assets.pic1.as_deref());
    stage.set_child(Some(&background));

    let tile = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(14)
        .margin_start(28)
        .margin_end(28)
        .margin_top(18)
        .margin_bottom(18)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    tile.add_css_class("card");
    let icon = asset_preview(68, 68);
    set_picture(&icon, assets.icon0.as_deref());
    tile.append(&icon);
    let title = gtk::Label::new(Some(fields.title.text().as_str()));
    title.add_css_class("title-3");
    title.set_wrap(true);
    title.set_max_width_chars(24);
    tile.append(&title);
    stage.add_overlay(&tile);

    let detail = asset_preview(264, 158);
    set_picture(&detail, assets.pic0.as_deref());
    detail.set_halign(gtk::Align::End);
    detail.set_valign(gtk::Align::End);
    detail.set_margin_end(16);
    detail.set_margin_bottom(18);
    detail.set_visible(assets.pic0.is_some());
    stage.add_overlay(&detail);
    content.append(&stage);
    let caption_text = tr("PSP XMB simulation · actual artwork from the selected game and theme.");
    let caption = gtk::Label::new(Some(&caption_text));
    caption.set_wrap(true);
    caption.add_css_class("dim-label");
    content.append(&caption);
    window.set_child(Some(&content));
    window.present();
    if fields.theme.selected() > 0 {
        operation_busy.set(true);
        let base = fields.base_assets.borrow().clone();
        let id = game_id.clone();
        let selected_theme = theme_name.to_string();
        let allow_downloads = !fields.disable_asset_downloads.is_active();
        let overrides = fields.asset_overrides.borrow().clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let mut assets = if allow_downloads {
                crate::assets::load_theme_assets(&id, &selected_theme, &base)
            } else {
                crate::assets::load_theme_assets_offline(&id, &selected_theme, &base)
            };
            apply_asset_overrides(&mut assets, &overrides);
            let _ = sender.send(assets);
        });
        let background = background.clone();
        let icon = icon.clone();
        let detail = detail.clone();
        let operation_busy = operation_busy.clone();
        gtk::glib::timeout_add_local(Duration::from_millis(100), move || {
            match receiver.try_recv() {
                Ok(assets) => {
                    set_picture(&background, assets.pic1.as_deref());
                    set_picture(&icon, assets.icon0.as_deref());
                    set_picture(&detail, assets.pic0.as_deref());
                    detail.set_visible(assets.pic0.is_some());
                    operation_busy.set(false);
                    gtk::glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => gtk::glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => {
                    operation_busy.set(false);
                    gtk::glib::ControlFlow::Break
                }
            }
        });
    }
}

fn asset_preview(width: i32, height: i32) -> gtk::Picture {
    gtk::Picture::builder()
        .width_request(width)
        .height_request(height)
        .can_shrink(true)
        .build()
}

fn set_picture(picture: &gtk::Picture, path: Option<&std::path::Path>) {
    if let Some(path) = path {
        let filename = path.to_string_lossy();
        picture.set_filename(Some(filename.as_ref()));
    } else {
        picture.set_filename(None::<&std::path::Path>);
    }
}

fn extract_page(
    parent: &adw::ApplicationWindow,
    operation_busy: Rc<Cell<bool>>,
) -> gtk::ScrolledWindow {
    let (scroll, content) = page(
        &tr("Extract EBOOT.PBP Contents"),
        &tr("Export ISO and raw BIN/CUE images plus audio tracks stored in the PBP."),
    );
    let (inputs, input_body) = section(&tr("Extraction"));
    let pbp_path = append_entry(&input_body, &tr("PBP File"), "");
    let output_path = append_entry(&input_body, &tr("Output Folder"), "");
    let history = Rc::new(RefCell::new(FolderHistory::load()));
    let choose_pbp = gtk::Button::with_label(&tr("Browse…"));
    let parent_for_pbp = parent.clone();
    let entry_for_pbp = pbp_path.clone();
    let history_for_pbp = history.clone();
    choose_pbp.connect_clicked(move |_| {
        let dialog = gtk::FileDialog::builder()
            .title(&tr("Select PBP File"))
            .build();
        dialog.set_initial_folder(Some(&gio::File::for_path(
            history_for_pbp.borrow().current(),
        )));
        let entry = entry_for_pbp.clone();
        let history = history_for_pbp.clone();
        dialog.open(
            Some(&parent_for_pbp),
            None::<&gio::Cancellable>,
            move |result| {
                if let Ok(file) = result
                    && let Some(path) = file.path()
                {
                    history.borrow_mut().remember_file(&path);
                    entry.set_text(&path.to_string_lossy());
                }
            },
        );
    });
    let choose_output = gtk::Button::with_label(&tr("Browse…"));
    let parent_for_output = parent.clone();
    let entry_for_output = output_path.clone();
    let history_for_output = history.clone();
    choose_output.connect_clicked(move |_| {
        let dialog = gtk::FileDialog::builder()
            .title(&tr("Select Output Folder"))
            .build();
        dialog.set_initial_folder(Some(&gio::File::for_path(
            history_for_output.borrow().current(),
        )));
        let entry = entry_for_output.clone();
        let history = history_for_output.clone();
        dialog.select_folder(
            Some(&parent_for_output),
            None::<&gio::Cancellable>,
            move |result| {
                if let Ok(file) = result
                    && let Some(path) = file.path()
                {
                    history.borrow_mut().remember_directory(&path);
                    entry.set_text(&path.to_string_lossy());
                }
            },
        );
    });
    if let Some(row) = pbp_path
        .parent()
        .and_then(|widget| widget.downcast::<gtk::Box>().ok())
    {
        row.append(&choose_pbp);
    }
    if let Some(row) = output_path
        .parent()
        .and_then(|widget| widget.downcast::<gtk::Box>().ok())
    {
        row.append(&choose_output);
    }
    content.append(&inputs);
    let extract = gtk::Button::with_label(&tr("Extract Contents"));
    extract.add_css_class("suggested-action");
    extract.set_halign(gtk::Align::End);
    let status = gtk::Label::new(Some(&tr("Ready")));
    status.set_xalign(0.0);
    status.add_css_class("dim-label");
    content.append(&status);
    let extract_progress = gtk::ProgressBar::new();
    extract_progress.set_show_text(true);
    extract_progress.set_text(Some(&tr("Ready")));
    content.append(&extract_progress);
    let pbp_for_extract = pbp_path.clone();
    let output_for_extract = output_path.clone();
    let status_for_extract = status.clone();
    let extract_for_extract = extract.clone();
    let progress_for_extract = extract_progress.clone();
    let extract_cancel = gtk::Button::with_label(&tr("Cancel"));
    extract_cancel.set_sensitive(false);
    let extract_cancel_state: Rc<RefCell<Option<Arc<AtomicBool>>>> = Rc::default();
    let state_on_cancel = extract_cancel_state.clone();
    let status_on_cancel = status.clone();
    extract_cancel.connect_clicked(move |_| {
        if let Some(token) = state_on_cancel.borrow().as_ref() {
            token.store(true, std::sync::atomic::Ordering::Relaxed);
            status_on_cancel.set_text(&tr("Stopping"));
        }
    });
    let cancel_button_for_extract = extract_cancel.clone();
    let state_for_extract = extract_cancel_state.clone();
    let operation_busy_for_extract = operation_busy.clone();
    extract.connect_clicked(move |_| {
        if operation_busy_for_extract.get() {
            status_for_extract.set_text(&tr("Another operation is in progress"));
            return;
        }
        let pbp = PathBuf::from(pbp_for_extract.text().as_str());
        let output = PathBuf::from(output_for_extract.text().as_str());
        if !pbp.is_file() || output.as_os_str().is_empty() {
            status_for_extract.set_text(&tr("Select the PBP file and output folder"));
            return;
        }
        status_for_extract.set_text(&tr("Extraction in progress"));
        progress_for_extract.set_text(Some(&tr("Extraction in progress")));
        progress_for_extract.set_fraction(0.0);
        extract_for_extract.set_sensitive(false);
        cancel_button_for_extract.set_sensitive(true);
        let cancellation = Arc::new(AtomicBool::new(false));
        *state_for_extract.borrow_mut() = Some(cancellation.clone());
        operation_busy_for_extract.set(true);
        let (sender, receiver) = mpsc::sync_channel(1);
        let progress_sender = sender.clone();
        thread::spawn(move || {
            let result = crate::pbp::Pbp::open(&pbp)
                .and_then(|container| {
                    container.extract_with_progress(&output, &cancellation, |fraction| {
                        let _ = progress_sender.try_send(TaskMessage::Progress(fraction));
                    })
                })
                .map_err(|error| error.to_string());
            let _ = sender.send(TaskMessage::Finished(result));
        });
        let status = status_for_extract.clone();
        let button = extract_for_extract.clone();
        let progress = progress_for_extract.clone();
        let cancel_button = cancel_button_for_extract.clone();
        let cancel_state = state_for_extract.clone();
        let operation_busy = operation_busy_for_extract.clone();
        gtk::glib::timeout_add_local(Duration::from_millis(100), move || {
            match receiver.try_recv() {
                Ok(TaskMessage::Progress(fraction)) => {
                    progress.set_fraction(fraction);
                    progress.set_text(Some(&format!("{:.0}%", fraction * 100.0)));
                    gtk::glib::ControlFlow::Continue
                }
                Ok(TaskMessage::Warning(warning)) => {
                    status.set_text(&warning);
                    gtk::glib::ControlFlow::Continue
                }
                Ok(TaskMessage::Finished(Ok(files))) => {
                    operation_busy.set(false);
                    status.set_text(
                        &tr("Extraction complete: {count} files")
                            .replace("{count}", &files.len().to_string()),
                    );
                    progress.set_fraction(1.0);
                    progress.set_text(Some(&tr("Completed")));
                    button.set_sensitive(true);
                    cancel_button.set_sensitive(false);
                    *cancel_state.borrow_mut() = None;
                    gtk::glib::ControlFlow::Break
                }
                Ok(TaskMessage::Finished(Err(error))) => {
                    operation_busy.set(false);
                    if error == "operation cancelled" {
                        status.set_text(&tr("Operation stopped"));
                        progress.set_text(Some(&tr("Operation stopped")));
                    } else {
                        status
                            .set_text(&tr("Extraction failed: {error}").replace("{error}", &error));
                        progress.set_text(Some(&tr("Error")));
                    }
                    button.set_sensitive(true);
                    cancel_button.set_sensitive(false);
                    *cancel_state.borrow_mut() = None;
                    gtk::glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => gtk::glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => {
                    operation_busy.set(false);
                    status.set_text(&tr("Extraction failed"));
                    progress.set_text(Some(&tr("Error")));
                    button.set_sensitive(true);
                    cancel_button.set_sensitive(false);
                    *cancel_state.borrow_mut() = None;
                    gtk::glib::ControlFlow::Break
                }
            }
        });
    });
    let extraction_actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .build();
    extraction_actions.append(&extract_cancel);
    extraction_actions.append(&extract);
    content.append(&extraction_actions);
    scroll
}

fn append_entry(parent: &gtk::Box, label: &str, value: &str) -> gtk::Entry {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .build();
    let caption = gtk::Label::builder()
        .label(label)
        .xalign(0.0)
        .width_chars(18)
        .build();
    let entry = gtk::Entry::builder().text(value).hexpand(true).build();
    row.append(&caption);
    row.append(&entry);
    parent.append(&row);
    entry
}
