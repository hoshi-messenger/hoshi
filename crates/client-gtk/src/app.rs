use std::ops::Deref;

use adw::prelude::*;
use adw::{ApplicationWindow, HeaderBar, NavigationPage, NavigationView, ToolbarView};
use gtk::{Box, Button, Orientation};

use crate::AppState;

pub fn build_main_view(toolbar: &ToolbarView) {
    let vbox = Box::builder().orientation(Orientation::Vertical).build();

    // Create a button with label and margins
    let button = Button::builder().label("Press me!").build();

    // Connect to "clicked" signal of `button`
    button.connect_clicked(|button: &Button| {
        // Set the label to "Hello World!" after the button has been clicked on
        button.set_label("Hello World!");
    });

    vbox.append(&button);
    toolbar.set_content(Some(&vbox));
}

pub fn build_page(navigation: &NavigationView, title: &str, static_page: bool) {
    let page = NavigationPage::builder().title(title).build();
    let toolbar = ToolbarView::builder()
        .top_bar_style(adw::ToolbarStyle::Flat)
        .build();

    let header_bar = HeaderBar::builder().build();

    toolbar.add_top_bar(&header_bar);
    build_main_view(&toolbar);

    page.set_child(Some(&toolbar));
    if static_page {
        navigation.add(&page);
    } else {
        navigation.push(&page);
    }
}

pub fn build_ui(state: AppState) {
    adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);

    let app = state.app.deref();

    let navigation = NavigationView::builder().build();

    build_page(&navigation, "Hoshi Manager", true);

    // Create a window
    let window = ApplicationWindow::builder()
        .application(app)
        .build();

    window.set_content(Some(&navigation));

    // Present window
    window.present();
}
