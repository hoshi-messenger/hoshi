use adw::{EntryRow, NavigationPage, PreferencesGroup, PreferencesPage, prelude::*};

use crate::AppState;

pub fn view_settings(state: AppState) {
    let page = NavigationPage::builder().title("Settings").build();

    let prefs = PreferencesPage::new();

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
