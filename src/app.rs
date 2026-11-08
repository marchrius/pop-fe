use std::cell::RefCell;
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

#[derive(Clone)]
struct DiscFields {
    id: gtk::Entry,
    title: gtk::Entry,
    psp_root: gtk::Entry,
    cdda: gtk::CheckButton,
    status: gtk::Label,
}

pub fn activate(application: &adw::Application) {
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
    stack.add_titled(&convert_page(&window), Some("convert"), &tr("Convert"));
    stack.add_titled(&extract_page(&window), Some("extract"), &tr("Extract"));

    let switcher = gtk::StackSwitcher::builder()
        .stack(&stack)
        .halign(gtk::Align::Center)
        .build();
    let header = adw::HeaderBar::builder().title_widget(&switcher).build();

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

fn convert_page(parent: &adw::ApplicationWindow) -> gtk::ScrolledWindow {
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
    let fields = DiscFields {
        id,
        title,
        psp_root,
        cdda: gtk::CheckButton::with_label(&tr("Use CDDA")),
        status: gtk::Label::new(Some(&tr("Ready"))),
    };
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
        if label == "Use CDDA" {
            option_body.append(&fields.cdda);
        } else {
            option_body.append(&gtk::CheckButton::with_label(&tr(label)));
        }
    }
    content.append(&options);
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
        let selected = files_for_create.borrow().clone();
        if selected.len() != 1 {
            fields_for_create.status.set_text(&tr(
                "Select one disc image; multiple discs are not supported yet",
            ));
            return;
        }
        let extension = selected[0]
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !["bin", "img", "iso", "cue"].contains(&extension.as_str()) {
            fields_for_create
                .status
                .set_text(&tr("Select a CUE, ISO or raw BIN/IMG image"));
            return;
        }
        let id = fields_for_create.id.text().to_string();
        if id.len() != 9 || fields_for_create.psp_root.text().is_empty() {
            fields_for_create.status.set_text(&tr(
                "Select a PSP folder and identify the disc before conversion",
            ));
            return;
        }
        let title = fields_for_create.title.text().to_string();
        let psp = PathBuf::from(fields_for_create.psp_root.text().as_str());
        let output = psp.join("PSP/GAME").join(&id).join("EBOOT.PBP");
        let config = crate::create::PbpCreateConfig {
            image: selected[0].clone(),
            output: output.clone(),
            game_id: id,
            title,
            assets: crate::create::PbpAssets::default(),
            include_title_image: true,
        };
        fields_for_create.status.set_text(&tr("Writing EBOOT.PBP"));
        progress_for_create.set_text(Some(&tr("Conversion in progress")));
        progress_for_create.pulse();
        button_for_create.set_sensitive(false);
        cancel_for_create.set_sensitive(true);
        let cancellation = Arc::new(AtomicBool::new(false));
        *state_for_create.borrow_mut() = Some(cancellation.clone());
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(
                crate::create::create_from_image_cancellable(&config, &cancellation)
                    .map_err(|error| error.to_string())
                    .map(|()| output),
            );
        });
        let fields = fields_for_create.clone();
        let button = button_for_create.clone();
        let progress = progress_for_create.clone();
        let cancel_button = cancel_for_create.clone();
        let cancel_state = state_for_create.clone();
        gtk::glib::timeout_add_local(Duration::from_millis(100), move || {
            match receiver.try_recv() {
                Ok(Ok(path)) => {
                    fields.status.set_text(&format!(
                        "{}: {}",
                        tr("Conversion complete"),
                        path.display()
                    ));
                    progress.set_fraction(1.0);
                    progress.set_text(Some(&tr("Completed")));
                    button.set_sensitive(true);
                    cancel_button.set_sensitive(false);
                    *cancel_state.borrow_mut() = None;
                    gtk::glib::ControlFlow::Break
                }
                Ok(Err(error)) => {
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
                Err(mpsc::TryRecvError::Empty) => {
                    progress.pulse();
                    gtk::glib::ControlFlow::Continue
                }
                Err(mpsc::TryRecvError::Disconnected) => {
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
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let result = crate::database::Database::embedded().analyze_image(&path);
        let _ = sender.send(result.map_err(|error| error.to_string()));
    });
    let fields = fields.clone();
    gtk::glib::timeout_add_local(Duration::from_millis(80), move || {
        match receiver.try_recv() {
            Ok(Ok(metadata)) => {
                fields.id.set_text(&metadata.id);
                fields.title.set_text(&metadata.title);
                fields.cdda.set_active(metadata.use_cdda);
                fields.status.set_text(&tr("Automatic data loaded"));
                gtk::glib::ControlFlow::Break
            }
            Ok(Err(_error)) => {
                fields.id.set_text("UNKN00000");
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
    fields.id.set_text("");
    fields.title.set_text("");
    fields.cdda.set_active(false);
    fields.status.set_text(&tr("Ready"));
}

fn extract_page(parent: &adw::ApplicationWindow) -> gtk::ScrolledWindow {
    let (scroll, content) = page(
        &tr("Extract EBOOT.PBP Contents"),
        &tr("Export disc images, CUE files and audio tracks stored in the PBP."),
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
    extract.connect_clicked(move |_| {
        let pbp = PathBuf::from(pbp_for_extract.text().as_str());
        let output = PathBuf::from(output_for_extract.text().as_str());
        if !pbp.is_file() || output.as_os_str().is_empty() {
            status_for_extract.set_text(&tr("Select the PBP file and output folder"));
            return;
        }
        status_for_extract.set_text(&tr("Extraction in progress"));
        progress_for_extract.set_text(Some(&tr("Extraction in progress")));
        progress_for_extract.pulse();
        extract_for_extract.set_sensitive(false);
        cancel_button_for_extract.set_sensitive(true);
        let cancellation = Arc::new(AtomicBool::new(false));
        *state_for_extract.borrow_mut() = Some(cancellation.clone());
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result = crate::pbp::Pbp::open(&pbp)
                .and_then(|container| container.extract_cancellable(&output, &cancellation))
                .map_err(|error| error.to_string());
            let _ = sender.send(result);
        });
        let status = status_for_extract.clone();
        let button = extract_for_extract.clone();
        let progress = progress_for_extract.clone();
        let cancel_button = cancel_button_for_extract.clone();
        let cancel_state = state_for_extract.clone();
        gtk::glib::timeout_add_local(Duration::from_millis(100), move || {
            match receiver.try_recv() {
                Ok(Ok(files)) => {
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
                Ok(Err(error)) => {
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
                Err(mpsc::TryRecvError::Empty) => {
                    progress.pulse();
                    gtk::glib::ControlFlow::Continue
                }
                Err(mpsc::TryRecvError::Disconnected) => {
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
