mod chat;
mod modals;

use adw::{Avatar, NavigationPage, NavigationSplitView, prelude::*};
use gtk::{Box, Button, Image, Label, ListBox, ListBoxRow, ScrolledWindow};
use hoshi_clientlib::{CallPartyStatus, Contact, ContactType};

use crate::AppState;

use chat::view_chat_page;
use modals::{show_add_contact_dialog, show_delete_contact_dialog};

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

fn create_contact_box(state: AppState, contact: &Contact, wide_view: bool) -> Box {
    let display_name = state
        .client
        .user_alias(&contact.public_key)
        .unwrap_or_else(|| contact.alias.clone());

    let avatar_size = if wide_view { 64 } else { 40 };

    let avatar = Avatar::builder()
        .size(avatar_size)
        .margin_start(8)
        .margin_end(8)
        .margin_top(8)
        .margin_bottom(8)
        .show_initials(false)
        .text(&display_name)
        .build();

    let alias_label = Label::builder()
        .halign(gtk::Align::Start)
        .label(&display_name)
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
        .hexpand(true)
        .build();
    vbox.add_css_class("vbox");
    vbox.append(&alias_label);
    vbox.append(&key_label);

    let hbox = Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    hbox.append(&avatar);
    hbox.append(&vbox);

    match contact.contact_type {
        ContactType::Blocked => hbox.add_css_class("contact-type-blocked"),
        ContactType::Contact => hbox.add_css_class("contact-type-contact"),
        ContactType::Unknown => hbox.add_css_class("contact-type-unknown"),
    };

    if wide_view {
        hbox.add_css_class("wide-avatar");

        let call_button = create_icon_button("call-start-symbolic", "Call");
        call_button.add_css_class("flat");
        {
            let state = state.clone();
            let contact = contact.clone();
            call_button.connect_clicked(move |_| {
                let calls = state.client.calls();

                // First we stop if the contact is already in a call we're in
                for call in &calls {
                    if call.get_status(&contact.public_key).is_some() {
                        return;
                    }
                }

                // Then we try and invite them to the first call we're active in
                let public_key = state.client.public_key();
                for call in &calls {
                    if matches!(call.get_status(&public_key), Some(CallPartyStatus::Active)) {
                        if state
                            .client
                            .call_invite_party(call.id(), contact.clone())
                            .is_ok()
                        {
                            return;
                        }
                    }
                }

                // Finally just create a new call and call them
                let parties = vec![contact.clone()];
                state.client.call_start(parties);
            });
        }
        hbox.append(&call_button);

        if contact.contact_type == ContactType::Contact {
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
        } else {
            let add_button = create_icon_button("contact-new-symbolic", "Add Contact");
            add_button.add_css_class("flat");
            {
                let state = state.clone();
                let public_key = contact.public_key.clone();
                add_button.connect_clicked(move |_| {
                    show_add_contact_dialog(&state.win, state.clone(), Some(&public_key));
                });
            }
            hbox.append(&add_button);
        }
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
        add_contact
            .connect_clicked(move |_| show_add_contact_dialog(&state.win, state.clone(), None));
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
        client.contacts_watch(move |client, contacts| {
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
                    if client.contact_get(selected).is_none() {
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
