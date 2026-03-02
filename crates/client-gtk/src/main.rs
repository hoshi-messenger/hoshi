use adw::Application;
use glib::ExitCode;
use hoshi_client_gtk::AppState;
use adw::prelude::*;

const APP_ID: &str = "org.hoshi.hoshi-client-gtk";

fn main() -> ExitCode {
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(|app| {
        AppState::start(app.clone());
    });
    app.run()
}

