use adw::Application;
use adw::prelude::*;
use clap::Parser;
use glib::ExitCode;
use hoshi_client_gtk::{AppState, Args};

const APP_ID: &str = "org.hoshi.hoshi-client-gtk";

fn main() -> ExitCode {
    let args = Args::parse();
    let mut app_builder = Application::builder().application_id(APP_ID);
    if cfg!(debug_assertions) {
        app_builder = app_builder.flags(gtk::gio::ApplicationFlags::NON_UNIQUE);
    }
    let app = app_builder.build();
    app.connect_activate(move |app| {
        AppState::start(app.clone(), args.clone());
    });
    // Pass only program name so GTK doesn't see our custom args
    let argv0: Vec<String> = std::env::args().take(1).collect();
    app.run_with_args(&argv0)
}
