use std::rc::Rc;

use adw::Application;
use glib::ExitCode;
use gtk::prelude::*;

use crate::build_ui;

#[derive(Debug, Clone)]
pub struct AppState {
    pub app: Rc<Application>,
}

impl AppState {
    pub fn new(app: Application) -> AppState {
        Self {
            app: Rc::new(app)
        }
    }

    pub fn run(&self) -> ExitCode {
        // Connect to "activate" signal of `app`
        let state = self.clone();
        self.app.connect_activate(move |_| build_ui(state.clone()));

        self.app.run()
    }
}