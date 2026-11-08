use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gio;
use gtk::prelude::*;

use crate::i18n::tr;

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
    stack.add_titled(&extract_page(), Some("extract"), &tr("Extract"));

    let switcher = gtk::StackSwitcher::builder()
        .stack(&stack)
        .halign(gtk::Align::Center)
        .build();
    let header = adw::HeaderBar::builder().title_widget(&switcher).build();

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(&stack);
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
    add.connect_clicked(move |_| {
        let chooser = gtk::FileChooserNative::new(
            Some(&tr("Select Disc Images")),
            Some(&parent_for_dialog),
            gtk::FileChooserAction::Open,
            Some(&tr("Open")),
            Some(&tr("Cancel")),
        );
        chooser.set_select_multiple(true);
        let files = files_for_dialog.clone();
        let list = list_for_dialog.clone();
        let clear = clear_for_dialog.clone();
        chooser.connect_response(move |dialog, response| {
            if response == gtk::ResponseType::Accept {
                let selected = dialog.files();
                for index in 0..selected.n_items() {
                    if let Some(file) = selected
                        .item(index)
                        .and_then(|item| item.downcast::<gio::File>().ok())
                        && let Some(path) = file.path()
                    {
                        let mut current = files.borrow_mut();
                        if !current.contains(&path) {
                            current.push(path);
                        }
                    }
                }
                refresh_disc_list(&list, &files, &clear);
            }
            dialog.destroy();
        });
        chooser.show();
    });
    let list_for_clear = list.clone();
    let files_for_clear = files.clone();
    let clear_for_clear = clear.clone();
    clear.connect_clicked(move |_| {
        files_for_clear.borrow_mut().clear();
        refresh_disc_list(&list_for_clear, &files_for_clear, &clear_for_clear);
    });
    actions.append(&add);
    actions.append(&clear);
    body.append(&actions);
    content.append(&frame);

    let (settings, settings_body) = section(&tr("Game Details"));
    append_entry(&settings_body, &tr("PSP Root Folder"), "");
    append_entry(&settings_body, &tr("Title"), "");
    append_entry(&settings_body, &tr("Game ID"), "");
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
        option_body.append(&gtk::CheckButton::with_label(&tr(label)));
    }
    content.append(&options);

    let convert = gtk::Button::with_label(&tr("Create EBOOT.PBP"));
    convert.add_css_class("suggested-action");
    convert.set_halign(gtk::Align::End);
    convert.set_sensitive(false);
    content.append(&convert);
    scroll
}

fn refresh_disc_list(list: &gtk::ListBox, files: &Rc<RefCell<Vec<PathBuf>>>, clear: &gtk::Button) {
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
        let list = list.clone();
        let clear = clear.clone();
        remove.connect_clicked(move |_| {
            files.borrow_mut().retain(|file| file != &path_to_remove);
            refresh_disc_list(&list, &files, &clear);
        });
        row.append(&remove);
        list.append(&row);
    }
    clear.set_sensitive(!files.borrow().is_empty());
}

fn extract_page() -> gtk::ScrolledWindow {
    let (scroll, content) = page(
        &tr("Extract EBOOT.PBP Contents"),
        &tr("Export disc images, CUE files and audio tracks stored in the PBP."),
    );
    let (inputs, input_body) = section(&tr("Extraction"));
    append_entry(&input_body, &tr("PBP File"), "");
    append_entry(&input_body, &tr("Output Folder"), "");
    content.append(&inputs);
    let extract = gtk::Button::with_label(&tr("Extract Contents"));
    extract.add_css_class("suggested-action");
    extract.set_halign(gtk::Align::End);
    extract.set_sensitive(false);
    content.append(&extract);
    scroll
}

fn append_entry(parent: &gtk::Box, label: &str, value: &str) {
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
}
