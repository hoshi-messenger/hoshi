use adw::ApplicationWindow;
use adw::prelude::AdwApplicationWindowExt;
use gtk::{Button, prelude::*};
use gtk::{Application, glib};

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
    // Create a button with label and margins
    let button = Button::builder()
        .label("Press me!")
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    // Connect to "clicked" signal of `button`
    button.connect_clicked(|button: &Button| {
        // Set the label to "Hello World!" after the button has been clicked on
        button.set_label("Hello World!");
    });

    // Create a window
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Hoshi Messenger")
        .build();

    window.set_content(Some(&button));

    // Present window
    window.present();
}
