use adw::Banner;

use crate::AppState;

pub fn init_call_state_banner(state: AppState) {
    let banner = Banner::new("");
    banner.set_button_label(Some("End Call"));

    state.toolbar.add_top_bar(&banner);

    {
        let state = state.clone();
        banner.connect_button_clicked(move |_| {
            state.client.call_stop();
        });
    }

    state.client.active_call_watch(move |call| match call {
        Some(call) => {
            let names = call.get_party_names().join(", ");
            banner.set_title(&format!("Calling {names}"));
            banner.set_revealed(true);
        }
        None => {
            banner.set_revealed(false);
        }
    });
}
