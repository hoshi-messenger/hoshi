use std::{cell::RefCell, collections::HashMap, rc::Rc, time::Duration};

use adw::prelude::*;
use gtk::{Button, CenterBox, Label, Orientation, Revealer, RevealerTransitionType};
use hoshi_clientlib::{Call, CallPartyStatus};

use crate::AppState;

struct CallBanner {
    pub revealer: Revealer,
    pub label: Label,
    pub accept_btn: Button,
    pub decline_btn: Button,
}

impl CallBanner {
    pub fn new(state: AppState, call: Call) -> Self {
        let revealer = Revealer::builder()
            .transition_type(RevealerTransitionType::SlideDown)
            .transition_duration(300)
            .build();

        let row = CenterBox::builder()
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(12)
            .margin_end(12)
            .build();

        let label = Label::new(None);

        let buttons = gtk::Box::builder()
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

        revealer.set_reveal_child(true);
        revealer.connect_child_revealed_notify(move |rev| {
            if !rev.is_child_revealed() {
                rev.unparent();
            }
        });

        let current_call_id: Rc<String> = Rc::new(call.id().to_string());
        {
            let state = state.clone();
            let current_call_id = current_call_id.clone();
            accept_btn.connect_clicked(move |_| {
                let id = current_call_id.to_string();
                if state.client.call_accept(&id).is_err() {
                    eprintln!("Couldn't accept call {id}");
                }
            });
        }
        {
            let state = state.clone();
            let current_call_id = current_call_id.clone();
            decline_btn.connect_clicked(move |_| {
                let id = current_call_id.to_string();
                if state.client.call_decline(&id).is_err() {
                    eprintln!("Couldn't accept call {id}");
                }
            });
        }
        {
            let state = state.clone();
            let current_call_id = current_call_id.clone();
            let label = label.clone();
            let revealer = revealer.clone();

            glib::timeout_add_local(Duration::from_millis(500), move || {
                if revealer.is_child_revealed()
                    && let Some(call) = state.client.call_get(&current_call_id)
                {
                    label.set_label(&call.get_call_label(state.client.own_contact()));
                    glib::ControlFlow::Continue
                } else {
                    glib::ControlFlow::Break
                }
            });
        }

        Self {
            revealer,
            label,
            accept_btn,
            decline_btn,
        }
    }

    pub fn update(&self, state: &AppState, call: &Call) {
        self.label
            .set_label(&call.get_call_label(state.client.own_contact()));

        if let Some(status) = call.get_status(&state.client.public_key()) {
            match status {
                CallPartyStatus::Ringing => {
                    self.accept_btn.set_visible(true);
                    self.decline_btn.add_css_class("destructive-action");
                    self.decline_btn.set_label("Decline");
                }
                CallPartyStatus::Active => {
                    self.accept_btn.set_visible(false);
                    self.decline_btn.add_css_class("destructive-action");
                    self.decline_btn.set_label("Hang up");
                }
                CallPartyStatus::HungUp => {
                    self.accept_btn.set_visible(false);
                    self.decline_btn.remove_css_class("destructive-action");
                    self.decline_btn.set_label("Hang up");
                }
            }
        }
    }

    pub fn close(&self) {
        self.revealer.set_reveal_child(false);
    }
}

pub fn init_call_banner(state: AppState) {
    let revealer_map: RefCell<HashMap<String, CallBanner>> = RefCell::new(HashMap::new());

    let watcher_state = state.clone();
    state.client.calls_watch(move |calls| {
        let mut revealer_map = revealer_map.borrow_mut();

        for call in calls.iter() {
            if let Some(banner) = revealer_map.get(call.id()) {
                banner.update(&watcher_state, call);
            } else {
                let banner = CallBanner::new(watcher_state.clone(), call.clone());
                banner.update(&watcher_state, call);
                revealer_map.insert(call.id().to_string(), banner);
            }
        }

        revealer_map.retain(|id, banner| {
            if calls.iter().position(|c| c.id() == id).is_none() {
                banner.close();
                false
            } else {
                true
            }
        });
    });
}
