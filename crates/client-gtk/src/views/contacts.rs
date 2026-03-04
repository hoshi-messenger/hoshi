use adw::{Avatar, Clamp, NavigationPage, NavigationSplitView, prelude::*};
use gtk::{Box, Button, CenterBox, Label, ListBox, ListBoxRow, ScrolledWindow, TextView};

use crate::{AppState, app_state::Contact};

fn contact_box(contact: &Contact) -> Box {
    let avatar = Avatar::builder()
        .name(&contact.alias)
        .size(40)
        .margin_start(8)
        .margin_end(8)
        .margin_top(8)
        .margin_bottom(8)
        .show_initials(true)
        .build();

    let alias_label = Label::builder()
        .halign(gtk::Align::Start)
        .label(&contact.alias)
        .build();
    alias_label.add_css_class("heading");

    let key_label = Label::builder()
        .halign(gtk::Align::Start)
        .label(&contact.public_key)
        .build();
    key_label.add_css_class("caption");
    key_label.add_css_class("dim-label");

    let vbox = Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .valign(gtk::Align::Center)
        .margin_end(8)
        .build();
    vbox.append(&alias_label);
    vbox.append(&key_label);

    let hbox = Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .build();
    hbox.append(&avatar);
    hbox.append(&vbox);

    hbox
}

fn add_contact(list: &ListBox, contact: &Contact) {
    let hbox = contact_box(contact);

    let row = ListBoxRow::new();
    row.set_widget_name(&contact.public_key);
    row.set_child(Some(&hbox));

    list.append(&row);
}

fn view_chat_page(_state: AppState, page: NavigationPage, contact: Option<Contact>) {
    if let Some(contact) = contact {
        let center_box = CenterBox::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();

        let top_bar = Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .build();

        let contact_info = contact_box(&contact);
        top_bar.append(&contact_info);
        center_box.set_start_widget(Some(&top_bar));

        let bottom_clamp = Clamp::builder().maximum_size(600).build();

        let bottom_box = Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .margin_bottom(4)
            .build();
        bottom_clamp.set_child(Some(&bottom_box));

        let message_input = TextView::builder()
            .wrap_mode(gtk::WrapMode::WordChar)
            .accepts_tab(false)
            .left_margin(8)
            .right_margin(8)
            .hexpand(true)
            .build();
        message_input.connect_realize(|msg_input| {
            msg_input.grab_focus();
        });

        let key_controller = gtk::EventControllerKey::new();
        {
            let message_input = message_input.clone();
            key_controller.connect_key_pressed(move |_, key, _, modifiers| {
                if key == gtk::gdk::Key::Return
                    && !modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK)
                {
                    let buffer = message_input.buffer();
                    let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
                    if !text.is_empty() {
                        println!("Send: {}", text);
                        buffer.set_text("");
                    }
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
        }
        message_input.add_controller(key_controller);

        bottom_box.append(&message_input);

        let send_btn = Button::builder()
            .icon_name("mail-send-symbolic")
            .valign(gtk::Align::End) // pin to bottom as input grows
            .build();
        bottom_box.append(&send_btn);

        center_box.set_end_widget(Some(&bottom_clamp));

        let scroll = ScrolledWindow::builder().vexpand(true).build();
        scroll.add_css_class("chat-background");

        let clamp = Clamp::builder().maximum_size(600).build();

        let vbox = Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .margin_top(16)
            .margin_bottom(16)
            .build();

        for i in 1..50 {
            let msg = format!("{i}: Test message");
            let from_me = (i & 1) == 0;
            let class = if from_me {
                "chat-message-from-me"
            } else {
                "chat-message-to-me"
            };
            let label = Label::builder()
                .css_classes([class, "chat-message"])
                .label(&msg)
                .halign(if from_me {
                    gtk::Align::End
                } else {
                    gtk::Align::Start
                })
                .build();

            vbox.append(&label);
        }

        clamp.set_child(Some(&vbox));
        scroll.set_child(Some(&clamp));

        center_box.set_center_widget(Some(&scroll));

        page.set_child(Some(&center_box));
    } else {
        // Show a hint message when there's no contact, should only occur on startup
        let wrap = ScrolledWindow::builder().build();
        wrap.add_css_class("chat-background");

        let label = Label::builder()
            .label("Select someone on the left to start chatting")
            .opacity(0.5)
            .hexpand(true)
            .vexpand(true)
            .build();
        wrap.set_child(Some(&label));
        page.set_child(Some(&wrap));
    }
}

fn view_contacts_page(state: AppState, page: NavigationPage, chat: NavigationPage) {
    let wrap = ScrolledWindow::builder().build();
    let list = ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .build();

    for contact in state.contacts.values() {
        add_contact(&list, contact);
    }

    {
        let state = state.clone();
        list.connect_row_activated(move |_, row| {
            let key = row.widget_name().to_string();
            if let Some(contact) = state.contacts.get(&key) {
                view_chat_page(state.clone(), chat.clone(), Some(contact.clone()));
            } else {
                view_chat_page(state.clone(), chat.clone(), None);
                println!("Selected: {key} - but couldn't find contact");
            }
        });
    }
    list.connect_realize(|list| {
        let list = list.clone();
        glib::source::idle_add_local_full(glib::Priority::HIGH, move || {
            list.unselect_all();
            glib::ControlFlow::Break
        });
    });

    wrap.set_child(Some(&list));
    page.set_child(Some(&wrap));
}

pub fn view_contact_list(state: AppState) {
    let page = NavigationPage::builder().build();

    let chat_nav = NavigationSplitView::builder().build();

    let chat_page = NavigationPage::builder().title("Chat").build();
    let contacts_page = NavigationPage::builder().title("Contacts").build();
    view_contacts_page(state.clone(), contacts_page.clone(), chat_page.clone());
    chat_nav.set_sidebar(Some(&contacts_page));

    view_chat_page(state.clone(), chat_page.clone(), None);
    chat_nav.set_content(Some(&chat_page));

    page.set_child(Some(&chat_nav));
    state.nav.add(&page);
}
