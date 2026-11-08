use adw::prelude::*;
use pop_fe_psp::app;

fn main() -> gtk::glib::ExitCode {
    let application = adw::Application::builder()
        .application_id("org.popfe.PSPManager")
        .build();

    application.connect_activate(app::activate);
    application.run()
}
