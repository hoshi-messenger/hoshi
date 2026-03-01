use adw::Application;

mod app;
mod app_state;

pub(crate) use app::*;
pub(crate) use app_state::AppState;
use glib::ExitCode;

const APP_ID: &str = "org.hoshi.hoshi-client-gtk";

pub fn run() -> ExitCode {
    // Create a new application
    let app = Application::builder().application_id(APP_ID).build();

    let state = AppState::new(app);
    state.run()
}

