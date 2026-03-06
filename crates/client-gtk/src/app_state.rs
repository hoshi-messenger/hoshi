use base64::prelude::*;
use hoshi_clientlib::HoshiClient;
use std::{rc::Rc, time::Duration};

use adw::{Application, ApplicationWindow, HeaderBar, NavigationView, ToolbarView, prelude::*};
use gtk::CssProvider;

use crate::{view_contact_list, view_settings};

#[derive(Debug, Clone)]
pub struct AppState {
    pub app: Application,
    pub nav: NavigationView,
    pub header: HeaderBar,
    pub toolbar: ToolbarView,
    pub win: ApplicationWindow,

    pub client: Rc<HoshiClient>,
}

fn add_css() {
    let bytes = include_bytes!("../assets/chat_bg.png");
    let b64 = BASE64_STANDARD.encode(bytes); // using the `base64` crate

    let provider = CssProvider::new();
    provider.load_from_string(&format!(
        "
        .chat-background {{
            background-image: url('data:image/png;base64,{b64}');
            background-repeat: repeat;
        }}

        .chat-message {{
            background-color: rgba(255,0,255,0.3);
            padding: 8px;
            border-radius:16px;
            margin-bottom:16px;
            font-size: 16px;
        }}

        .chat-message-from-me {{
            background-color: rgba(192,156,255,0.2);
            margin-left:48px;
        }}

        .chat-message-to-me {{
            background-color: rgba(228,228,228,0.2);
            margin-right:48px;
        }}

        .bg-transparent {{
            background: transparent;
        }}

        .bg-darken {{
            background: rgba(0,0,0,0.4);
        }}

        .bg-lighten {{
            background: rgba(255,255,255,0.15);
        }}

        .wide-avatar .heading {{
            font-size: 1.8em;
        }}

        .message-input-frame {{
            border-radius: 18px;
            border: none;
            background: rgba(0,0,0,0.33);
            padding: 0 8px;
            box-shadow: none;
        }}

        .message-input {{
            background: transparent;
            border-radius: 18px;
            box-shadow: none;
            font-size: 16px;
        }}
    "
    ));

    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().unwrap(),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

fn force_dark_mode() {
    adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
}

impl AppState {
    fn spawn_client_handler_future(&self) {
        let client = self.client.clone();
        glib::spawn_future_local(async move {
            let msgs = client.step();
            // Adaptable timeout, make sure we don't poll as often if there
            // are no messages in the mpsc
            let millis = if msgs == 0 { 4 } else { 64 };
            glib::timeout_future(Duration::from_millis(millis)).await;
        });
    }

    pub fn start(app: Application) {
        force_dark_mode();
        add_css();

        let toolbar = ToolbarView::builder()
            .top_bar_style(adw::ToolbarStyle::Raised)
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

        let client = HoshiClient::new().expect("Couldn't create HoshiClient");

        let state = Self {
            app,
            nav,
            header,
            toolbar,
            win: win.clone(),
            client: Rc::new(client),
        };
        state.spawn_client_handler_future();
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
