use adw::{HeaderBar, NavigationPage, ToolbarView, prelude::*};
use gtk::{Box, Button};

use crate::AppState;

pub fn view_contact_list(state: AppState) {
    let page = NavigationPage::builder().title("Hoshi Messenger").build();
    let toolbar = ToolbarView::builder()
        .top_bar_style(adw::ToolbarStyle::Flat)
        .build();

    let header_bar = HeaderBar::builder().build();

    toolbar.add_top_bar(&header_bar);
    


    let vbox = Box::builder().orientation(gtk::Orientation::Vertical).build();

    let button = Button::builder().label("Press me!").build();
    button.connect_clicked(|button: &Button| {
        button.set_label("Hello World!");
    });

    vbox.append(&button);
    toolbar.set_content(Some(&vbox));




    page.set_child(Some(&toolbar));
    state.nav.add(&page);
}