use adw::{NavigationPage, prelude::*};
use gtk::{Box, Button};

use crate::AppState;

pub fn view_settings(state: AppState) {
    let page = NavigationPage::builder().title("Hoshi Messenger").build();
    let vbox = Box::builder().orientation(gtk::Orientation::Vertical).build();

    let button = Button::builder().label("Settings!").build();
    button.connect_clicked(|button: &Button| {
        button.set_label("Hello World!");
    });

    vbox.append(&button);
    page.set_child(Some(&vbox));
    state.nav.push(&page);
}