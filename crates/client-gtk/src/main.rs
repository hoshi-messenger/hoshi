use adw::{Application, ApplicationWindow, HeaderBar, NavigationPage, NavigationView, ToolbarView};
use adw::prelude::{AdwApplicationWindowExt, NavigationPageExt};
use gtk::{Box, Button, Orientation, prelude::*};
use gtk::glib;

const APP_ID: &str = "org.hoshi.hoshi-client-gtk";

fn main() -> glib::ExitCode {
    // Create a new application
    let app = Application::builder().application_id(APP_ID).build();

    // Connect to "activate" signal of `app`
    app.connect_activate(build_ui);

    // Run the application
    app.run()
}

fn build_ui(app: &Application) {

    let navigation = NavigationView::builder()
        .build();

    let page = NavigationPage::builder()
        .title("Hoshi Messenger")
        .build();

    navigation.push(&page);

    let toolbar = ToolbarView::builder()
        .top_bar_style(adw::ToolbarStyle::Flat)
        .build();

    let header_bar = HeaderBar::builder()
        .build();

    toolbar.add_top_bar(&header_bar);

    let vbox = Box::builder()
        .orientation(Orientation::Vertical)
        .build();

    // Create a button with label and margins
    let button = Button::builder()
        .label("Press me!")
        .build();

    // Connect to "clicked" signal of `button`
    button.connect_clicked(|button: &Button| {
        // Set the label to "Hello World!" after the button has been clicked on
        button.set_label("Hello World!");
    });
    
    vbox.append(&button);
    toolbar.set_content(Some(&vbox));

    page.set_child(Some(&toolbar));

    // Create a window
    let window = ApplicationWindow::builder()
        .application(app)
        //.title("Hoshi Messenger")
        .build();

    window.set_content(Some(&navigation));

    // Present window
    window.present();
}
