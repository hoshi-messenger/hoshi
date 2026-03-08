use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use gtk::{Box, Button, CenterBox, Label, Orientation, Revealer, RevealerTransitionType};
use hoshi_clientlib::CallPartyStatus;

use crate::AppState;

pub fn init_incoming_call_banner(state: AppState) {
    let revealer = Revealer::builder()
        .transition_type(RevealerTransitionType::SlideDown)
        .build();

    let row = CenterBox::builder()
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(12)
        .margin_end(12)
        .build();

    let label = Label::new(None);

    let buttons = Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    let accept_btn = Button::builder()
        .label("Accept")
        .css_classes(["suggested-action"])
        .build();
    let decline_btn = Button::builder()
        .label("Decline")
        .css_classes(["destructive-action"])
        .build();
    buttons.append(&accept_btn);
    buttons.append(&decline_btn);

    row.set_center_widget(Some(&label));
    row.set_end_widget(Some(&buttons));
    revealer.set_child(Some(&row));
    state.toolbar.add_top_bar(&revealer);

    let current_call_id: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

    {
        let state = state.clone();
        let current_call_id = current_call_id.clone();
        accept_btn.connect_clicked(move |_| {
            let id = current_call_id.borrow().clone();
            if let Some(id) = id {
                state.client.incoming_call_accept(&id);
            }
        });
    }

    {
        let state = state.clone();
        let current_call_id = current_call_id.clone();
        decline_btn.connect_clicked(move |_| {
            let id = current_call_id.borrow().clone();
            if let Some(id) = id {
                state.client.incoming_call_decline(&id);
            }
        });
    }

    state.client.incoming_calls_watch(move |calls| {
        if let Some(call) = calls.first() {
            *current_call_id.borrow_mut() = Some(call.id().to_string());
            let names = call.get_party_names().join(", ");
            label.set_text(&format!("Incoming call from {names}"));
            revealer.set_reveal_child(true);
        } else {
            *current_call_id.borrow_mut() = None;
            revealer.set_reveal_child(false);
        }
    });
}

pub fn init_call_state_banner(state: AppState) {
    let revealer = Revealer::builder()
        .transition_type(RevealerTransitionType::SlideDown)
        .build();

    let row = CenterBox::builder()
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(12)
        .margin_end(12)
        .build();

    let label = Label::new(None);
    let end_btn = Button::builder()
        .label("End Call")
        .css_classes(["destructive-action"])
        .build();

    row.set_center_widget(Some(&label));
    row.set_end_widget(Some(&end_btn));
    revealer.set_child(Some(&row));
    state.toolbar.add_top_bar(&revealer);

    {
        let state = state.clone();
        end_btn.connect_clicked(move |_| {
            state.client.call_stop();
        });
    }

    state.client.active_call_watch(move |call| match call {
        Some(call) => {
            let pairs = call.get_party_status_pairs();
            let hung_up = pairs
                .iter()
                .any(|(_, s)| matches!(s, CallPartyStatus::HungUp));
            let status_str = pairs
                .iter()
                .map(|(name, status)| match status {
                    CallPartyStatus::Ringing => format!("{name} (ringing)"),
                    CallPartyStatus::Active => format!("{name} (active)"),
                    CallPartyStatus::HungUp => format!("{name} (hung up)"),
                })
                .collect::<Vec<_>>()
                .join(", ");
            label.set_text(&format!("Call: {status_str}"));
            if hung_up {
                end_btn.set_label("Close");
                end_btn.remove_css_class("destructive-action");
            } else {
                end_btn.set_label("End Call");
                end_btn.add_css_class("destructive-action");
            }
            revealer.set_reveal_child(true);
        }
        None => {
            revealer.set_reveal_child(false);
        }
    });
}
