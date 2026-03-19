use adw::{EntryRow, NavigationPage, PreferencesGroup, PreferencesPage, prelude::*};
use gtk::Entry;

use crate::AppState;

pub fn view_settings(state: AppState) {
    let page = NavigationPage::builder().title("Settings").build();

    let prefs = PreferencesPage::new();

    let profile_group = PreferencesGroup::builder()
        .title("Profile")
        .description("Type in the name you want your friends to see.")
        .build();

    let current_alias = state
        .client
        .user_alias(&state.client.public_key())
        .unwrap_or_default();
    let alias_row = EntryRow::builder()
        .title("Display Name")
        .text(&current_alias)
        .show_apply_button(true)
        .build();

    {
        let state = state.clone();
        alias_row.connect_apply(move |row| {
            let alias = row.text().to_string();
            state.client.set_user_alias(&alias);
        });
    }

    profile_group.add(&alias_row);
    prefs.add(&profile_group);

    let identity_group = PreferencesGroup::builder()
        .title("Identity")
        .description("Your public key identifies you on the network.")
        .build();

    let key_row = EntryRow::builder()
        .title("Public Key")
        .text(&state.client.public_key())
        .show_apply_button(true)
        .build();

    {
        let state = state.clone();
        key_row.connect_apply(move |row| {
            let new_key = row.text().to_string();
            if !new_key.is_empty() {
                state
                    .client
                    .set_public_key(new_key)
                    .expect("Couldn't save public key");
            }
        });
    }

    identity_group.add(&key_row);
    prefs.add(&identity_group);

    page.set_child(Some(&prefs));
    state.nav.push(&page);
}

pub fn prompt_user_alias_if_needed(state: &AppState) {
    let pk = state.client.public_key();
    if state.client.user_alias(&pk).is_some() {
        return;
    }

    let dialog = adw::AlertDialog::builder()
        .heading("Welcome to Hoshi!")
        .body("Choose a name so your friends know who you are.")
        .build();

    let alias_entry = Entry::builder().placeholder_text("Display Name").build();
    alias_entry.connect_map(|e| {
        e.grab_focus();
    });

    dialog.set_extra_child(Some(&alias_entry));

    dialog.add_response("skip", "Skip");
    dialog.add_response("save", "Save");
    dialog.set_default_response(Some("save"));
    dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);

    alias_entry.set_activates_default(true);

    let state = state.clone();
    dialog.connect_response(None, move |dialog, response| {
        if response == "save" {
            let alias = alias_entry.text().to_string();
            if !alias.is_empty() {
                state.client.set_user_alias(&alias);
            }
        }
        dialog.close();
    });

    dialog.present(Some(&state.win));
}
