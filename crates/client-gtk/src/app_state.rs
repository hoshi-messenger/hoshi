use adw::{Application, ApplicationWindow, HeaderBar, NavigationView, ToolbarView, prelude::*};

use crate::{view_contact_list, view_settings};

#[derive(Debug, Clone)]
pub struct AppState {
    pub app: Application,
    pub nav: NavigationView,
    pub header: HeaderBar,
    pub toolbar: ToolbarView,
    pub win: ApplicationWindow,
}

fn force_dark_mode() {
    adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
}

impl AppState {
    pub fn start(app: Application) {
        force_dark_mode();



        let toolbar = ToolbarView::builder()
            .top_bar_style(adw::ToolbarStyle::Flat)
            .build();

        let header = HeaderBar::builder().build();

        let settings_btn = gtk::ToggleButton::new();
        
        settings_btn.add_css_class("flat");
        settings_btn.set_tooltip_text(Some("Settings"));
        let settings_icon = gtk::Image::from_icon_name("emblem-system-symbolic");
        settings_btn.set_child(Some(&settings_icon));
        settings_btn.set_active(false);
        
        header.pack_end(&settings_btn);
        toolbar.add_top_bar(&header);


        let nav = NavigationView::builder().build();
        toolbar.set_content(Some(&nav));

        // Create a window
        let win = ApplicationWindow::builder()
            .application(&app)
            .title("Hoshi Messenger")
            .build();
        win.set_content(Some(&toolbar));


        // Get rid of 5px padding
        win.remove_css_class("solid-csd");

        let state = Self {
            app,
            nav,
            header,
            toolbar,
            win: win.clone(),
        };
        {
            let state = state.clone();
            settings_btn.connect_clicked(move |btn| {
                if !btn.is_active() {
                    state.nav.pop();
                } else {
                    view_settings(state.clone());
                }
            });
        }
        view_contact_list(state);
        win.present();
    }
}