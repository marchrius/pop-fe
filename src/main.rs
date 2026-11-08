mod app;
mod disc;
mod i18n;

fn main() -> gtk::glib::ExitCode {
    let application = adw::Application::builder()
        .application_id("org.popfe.PSPManager")
        .build();

    application.connect_activate(app::activate);
    application.run()
}
