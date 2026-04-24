use crate::AppState;
use adw::{ApplicationWindow, prelude::*};
use gtk::{Box, Entry};

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

    vbox.append(&public_key_entry);

    dialog.set_extra_child(Some(&vbox));

    dialog.add_response("cancel", "Cancel");
    dialog.add_response("add", "Add");
    dialog.set_default_response(Some("add"));
    dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);

    public_key_entry.set_activates_default(true);

    let error_parent = parent.clone();
    dialog.connect_response(None, move |dialog, response| {
        if response == "add" {
            let public_key = hoshi_clientlib::normalize_public_key(&public_key_entry.text());
            if !public_key.is_empty() {
                let contact = hoshi_clientlib::Contact::new(public_key);
                if state.client.contact_upsert(contact).is_err() {
                    let error_dialog = adw::AlertDialog::new(
                        Some("Couldn't add contact"),
                        Some("Incorrect public key, please double check."),
                    );
                    error_dialog.add_response("ok", "OK");
                    error_dialog.present(Some(&error_parent));
                    return;
                }
            }
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
        let display_name = state.client.display_name(&contact.public_key);
        let dialog = adw::AlertDialog::new(
            Some("Delete Contact"),
            Some(&format!(
                "Are you sure you want to delete {}?",
                display_name
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
