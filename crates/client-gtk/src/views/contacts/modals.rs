use adw::{ApplicationWindow, prelude::*};
use gtk::{Box, Entry};
use hoshi_clientlib::Contact;

use crate::AppState;

pub(super) fn show_add_contact_dialog(
    parent: &ApplicationWindow,
    state: AppState,
    prefill_key: Option<&str>,
) {
    let dialog = adw::AlertDialog::builder().heading("New Contact").build();

    let vbox = Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_top(8)
        .build();

    let mut pk_builder = Entry::builder().placeholder_text("Public Key");
    if let Some(key) = prefill_key {
        pk_builder = pk_builder.text(key).editable(false).sensitive(false);
    }
    let public_key_entry = pk_builder.build();
    if prefill_key.is_none() {
        public_key_entry.connect_map(|public_key_entry: &Entry| {
            public_key_entry.grab_focus();
        });
    }

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

pub(super) fn show_edit_contact_dialog(
    parent: &ApplicationWindow,
    state: AppState,
    contact: &Contact,
) {
    let dialog = adw::AlertDialog::new(Some("Edit Contact"), None);

    let vbox = Box::builder()
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

pub(super) fn show_delete_contact_dialog(
    parent: &ApplicationWindow,
    state: AppState,
    public_key: &str,
) {
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
