use adw::{Application, ApplicationWindow, NavigationView, prelude::*};

use crate::view_contact_list;

#[derive(Debug, Clone)]
pub struct AppState {
    pub app: Application,
    pub nav: NavigationView,
    pub win: ApplicationWindow,
}

impl AppState {
    pub fn start(app: Application) {
        adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);

        let nav = NavigationView::builder().build();

        //build_page(&nav, "Hoshi Manager", true);

        // Create a window
        let win = ApplicationWindow::builder()
            .application(&app)
            .build();
        win.set_content(Some(&nav));

        let state = Self {
            app,
            nav,
            win: win.clone(),
        };
        view_contact_list(state);
        win.present();
    }
}