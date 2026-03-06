use adw::{ApplicationWindow, Avatar, Clamp, NavigationPage, NavigationSplitView, prelude::*};
use gtk::{
    Box, Button, Entry, Image, Label, ListBox, ListBoxRow, MenuButton, ScrolledWindow, TextView,
};
use hoshi_clientlib::{ChatMessage, Contact};

use crate::AppState;

fn show_add_contact_dialog(parent: &ApplicationWindow, state: AppState) {
    let dialog = adw::AlertDialog::new(Some("New Contact"), None);

    let vbox = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_top(8)
        .build();

    let public_key_entry = Entry::builder().placeholder_text("Public Key").build();

    let alias_entry = Entry::builder()
        .placeholder_text("Alias (optional)")
        .build();

    vbox.append(&public_key_entry);
    vbox.append(&alias_entry);

    dialog.set_extra_child(Some(&vbox));

    dialog.add_response("cancel", "Cancel");
    dialog.add_response("add", "Add");
    dialog.set_default_response(Some("add"));
    dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);

    public_key_entry.set_activates_default(true);
    alias_entry.set_activates_default(true);

    dialog.connect_response(None, move |dialog, response| {
        if response == "add" {
            let public_key = public_key_entry.text().to_string();
            let alias = alias_entry.text().to_string();
            if !public_key.is_empty() {
                let alias = if alias.is_empty() { None } else { Some(alias) };
                let contact = Contact::new(public_key, alias);
                state
                    .client
                    .contact_upsert(contact)
                    .expect("Couldn't add contact");
            }
        }
        dialog.close();
    });

    dialog.present(Some(parent));
}

fn show_edit_contact_dialog(parent: &ApplicationWindow, state: AppState, contact: &Contact) {
    let dialog = adw::AlertDialog::new(Some("Edit Contact"), None);

    let vbox = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_top(8)
        .build();

    let public_key_entry = Entry::builder()
        .text(&contact.public_key)
        .editable(false)
        .sensitive(false)
        .build();

    let alias_entry = Entry::builder()
        .text(&contact.alias)
        .placeholder_text("Alias (optional)")
        .build();

    alias_entry.connect_map(|alias| {
        alias.grab_focus();
    });

    vbox.append(&public_key_entry);
    vbox.append(&alias_entry);

    dialog.set_extra_child(Some(&vbox));

    dialog.add_response("cancel", "Cancel");
    dialog.add_response("save", "Save");
    dialog.set_default_response(Some("save"));
    dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);

    alias_entry.set_activates_default(true);

    let public_key = contact.public_key.clone();
    dialog.connect_response(None, move |dialog, response| {
        if response == "save" {
            let alias = alias_entry.text().to_string();
            let alias = if alias.is_empty() { None } else { Some(alias) };
            let contact = Contact::new(public_key.clone(), alias);
            state
                .client
                .contact_upsert(contact)
                .expect("Couldn't update contact");
        }
        dialog.close();
    });

    dialog.present(Some(parent));
}

fn show_delete_contact_dialog(parent: &ApplicationWindow, state: AppState, public_key: &str) {
    if let Some(contact) = state.client.contact_get(public_key) {
        let dialog = adw::AlertDialog::new(
            Some("Delete Contact"),
            Some(&format!(
                "Are you sure you want to delete {}?",
                contact.alias
            )),
        );

        dialog.add_response("cancel", "Cancel");
        dialog.add_response("delete", "Delete");
        dialog.set_default_response(Some("cancel"));
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);

        let public_key = contact.public_key.clone();
        dialog.connect_response(None, move |dialog, response| {
            if response == "delete" {
                state
                    .client
                    .contact_delete(&public_key)
                    .expect("Couldn't delete contact");
            }
            dialog.close();
        });

        dialog.present(Some(parent));
    };
}

fn create_contact_box(state: AppState, contact: &Contact, wide_view: bool) -> Box {
    let avatar_size = if wide_view { 64 } else { 40 };

    let avatar = Avatar::builder()
        .size(avatar_size)
        .margin_start(8)
        .margin_end(8)
        .margin_top(8)
        .margin_bottom(8)
        .show_initials(false)
        .text(&contact.alias)
        .build();

    let alias_label = Label::builder()
        .halign(gtk::Align::Start)
        .label(&contact.alias)
        .build();
    alias_label.add_css_class("heading");

    let key_label = Label::builder()
        .halign(gtk::Align::Start)
        .label(&contact.public_key)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    key_label.add_css_class("caption");
    key_label.add_css_class("dim-label");

    let vbox = Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .valign(gtk::Align::Center)
        .margin_start(4)
        .margin_end(4)
        .hexpand(true)
        .build();
    vbox.append(&alias_label);
    vbox.append(&key_label);

    let hbox = Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .build();
    hbox.append(&avatar);
    hbox.append(&vbox);

    if wide_view {
        hbox.add_css_class("wide-avatar");

        let edit_button = create_icon_button("document-edit-symbolic", "Edit");
        edit_button.add_css_class("flat");
        {
            let state = state.clone();
            let contact = contact.clone();
            edit_button.connect_clicked(move |_| {
                show_edit_contact_dialog(&state.win, state.clone(), &contact);
            });
        }
        hbox.append(&edit_button);

        let delete_button = create_icon_button("user-trash-symbolic", "Delete");
        delete_button.add_css_class("flat");
        {
            let state = state.clone();
            let public_key = contact.public_key.clone();
            delete_button.connect_clicked(move |_| {
                show_delete_contact_dialog(&state.win, state.clone(), &public_key);
            });
        }
        hbox.append(&delete_button);
    }

    hbox
}

fn add_contact_row(
    state: AppState,
    list: &ListBox,
    contact: &Contact,
    wide_view: bool,
) -> ListBoxRow {
    let hbox = create_contact_box(state, contact, wide_view);

    let row = ListBoxRow::new();
    row.set_widget_name(&contact.public_key);
    row.set_child(Some(&hbox));

    list.append(&row);
    row
}

fn clear_box(b: &Box) {
    while let Some(child) = b.first_child() {
        b.remove(&child);
    }
}

fn view_contact_chat_page(state: AppState, page: NavigationPage, contact: Contact) {
    let center_box = Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();

    let top_bar = Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .build();
    top_bar.add_css_class("bg-lighten");

    let contact_info = create_contact_box(state.clone(), &contact, true);
    top_bar.append(&contact_info);
    center_box.append(&top_bar);

    let bottom_clamp = Clamp::builder().maximum_size(600).build();
    bottom_clamp.add_css_class("bg-lighten");

    let bottom_box = Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .margin_top(4)
        .margin_bottom(4)
        .build();
    bottom_clamp.set_child(Some(&bottom_box));

    let input_frame = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .margin_top(4)
        .margin_bottom(4)
        .build();
    input_frame.add_css_class("message-input-frame");

    let message_input = TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .accepts_tab(false)
        .left_margin(8)
        .right_margin(4)
        .top_margin(8)
        .bottom_margin(8)
        .hexpand(true)
        .build();
    message_input.add_css_class("message-input");
    message_input.set_size_request(-1, 36);
    message_input.connect_map(|msg_input| {
        msg_input.grab_focus();
    });

    let emoji_chooser = gtk::EmojiChooser::new();
    let emoji_btn = MenuButton::builder()
        .icon_name("face-smile-symbolic")
        .valign(gtk::Align::Start)
        .margin_top(4)
        .popover(&emoji_chooser) // set via builder directly
        .build();
    emoji_btn.add_css_class("flat");
    {
        let message_input = message_input.clone();
        emoji_chooser.connect_emoji_picked(move |_, emoji| {
            message_input.buffer().insert_at_cursor(emoji);
            let message_input = message_input.clone();
            glib::idle_add_local_once(move || {
                message_input.grab_focus();
            });
        });
    }

    let send_btn = Button::builder()
        .icon_name("mail-send-symbolic")
        .valign(gtk::Align::Start)
        .build();
    send_btn.add_css_class("flat");
    send_btn.add_css_class("message-send-btn");

    {
        let state = state.clone();
        let contact = contact.clone();
        let message_input = message_input.clone();
        let send_btn = send_btn.clone();
        send_btn.connect_clicked(move |_| {
            let buffer = message_input.buffer();
            let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
            if !text.is_empty() {
                let from = state.client.public_key();
                let to = contact.public_key.clone();
                let content = text.to_string();
                let msg = ChatMessage::create(from, to, content);
                state.client.message_upsert(msg).expect("Couldn't send msg");
                buffer.set_text("");
            }
        });
    }

    let key_controller = gtk::EventControllerKey::new();
    {
        let send_btn = send_btn.clone();
        key_controller.connect_key_pressed(move |_, key, _, modifiers| {
            if key == gtk::gdk::Key::Return
                && !modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK)
            {
                send_btn.emit_clicked();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }
    message_input.add_controller(key_controller);

    input_frame.append(&emoji_btn);
    input_frame.append(&message_input);
    input_frame.append(&send_btn);
    bottom_box.append(&input_frame);

    let scroll = ScrolledWindow::builder().vexpand(true).build();
    center_box.add_css_class("chat-background");

    let clamp = Clamp::builder().maximum_size(600).build();

    let vbox = Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .margin_top(16)
        .margin_bottom(16)
        .build();

    {
        let state = state.clone();
        let moi = state.client.public_key();
        let chat_id = ChatMessage::calc_chat_id(&state.client.public_key(), &contact.public_key);
        let vbox = vbox.clone();
        state
            .client
            .messages_watch(chat_id, move |_chat_id, messages| {
                let mut sorted = messages.values().collect::<Vec<&ChatMessage>>();
                sorted.sort();
                clear_box(&vbox);
                for msg in sorted {
                    let from_me = msg.from == moi;

                    let class = if from_me {
                        "chat-message-from-me"
                    } else {
                        "chat-message-to-me"
                    };
                    let label = Label::builder()
                        .css_classes([class, "chat-message"])
                        .label(&msg.content)
                        .halign(if from_me {
                            gtk::Align::End
                        } else {
                            gtk::Align::Start
                        })
                        .build();
                    vbox.append(&label);
                }
            });
    }

    clamp.set_child(Some(&vbox));
    scroll.set_child(Some(&clamp));

    center_box.append(&scroll);
    center_box.append(&bottom_clamp);

    page.set_child(Some(&center_box));
}

fn view_empty_chat_page(_state: AppState, page: NavigationPage) {
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

fn view_chat_page(state: AppState, page: NavigationPage, contact: Option<Contact>) {
    if let Some(contact) = contact {
        view_contact_chat_page(state, page, contact);
    } else {
        view_empty_chat_page(state, page);
    }
}

fn create_icon_button(icon: &str, label: &str) -> Button {
    let button = Button::builder().valign(gtk::Align::Center).build();
    let button_box = Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(4)
        .build();
    button_box.append(&Image::from_icon_name(icon));
    button_box.append(&Label::new(Some(label)));
    button.set_child(Some(&button_box));
    button
}

fn contact_list_buttons(state: AppState) -> Box {
    let hbox = Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(8)
        .margin_end(8)
        .build();
    hbox.add_css_class("contact-buttons");

    let add_contact = create_icon_button("contact-new-symbolic", "New Contact");
    hbox.append(&add_contact);
    {
        let state = state.clone();
        add_contact.connect_clicked(move |_| show_add_contact_dialog(&state.win, state.clone()));
    }

    hbox
}

fn view_contacts_page(state: AppState, page: NavigationPage, chat: NavigationPage) {
    let vbox = Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();

    vbox.add_css_class("bg-darken");

    let wrap = ScrolledWindow::builder().vexpand(true).build();
    let list = ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .build();
    list.add_css_class("bg-transparent");

    {
        let list = list.clone().downgrade();
        let chat = chat.clone();
        let client = &state.client;
        let state = state.clone();
        client.contacts_watch(move |contacts| {
            // More efficient diffing would be nice in the future, good enough for an MVP though
            if let Some(list) = list.upgrade() {
                let selected = list.selected_row().map(|r| r.widget_name().to_string());
                list.remove_all();

                let mut sorted_contacts = contacts.values().collect::<Vec<&Contact>>();
                sorted_contacts.sort_by(|a, b| a.public_key.cmp(&b.public_key));

                for contact in &sorted_contacts {
                    let row = add_contact_row(state.clone(), &list, contact, false);
                    if let Some(selected) = &selected {
                        if &contact.public_key == selected {
                            row.activate();
                        }
                    }
                }

                if let Some(selected) = &selected {
                    if state.client.contact_get(selected).is_none() {
                        view_chat_page(state.clone(), chat.clone(), None);
                        list.unselect_all();
                    }
                } else {
                    view_chat_page(state.clone(), chat.clone(), None);
                    list.unselect_all();
                }
            }
        });
    }

    {
        let state = state.clone();
        list.connect_row_activated(move |_, row| {
            let key = row.widget_name().to_string();
            if let Some(contact) = state.client.contact_get(&key) {
                view_chat_page(state.clone(), chat.clone(), Some(contact));
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
    let button_box = contact_list_buttons(state);
    vbox.append(&wrap);
    vbox.append(&button_box);
    page.set_child(Some(&vbox));
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
