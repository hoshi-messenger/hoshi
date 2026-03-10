use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use adw::prelude::*;
use gtk::{
    Box, Button, CenterBox, DrawingArea, Label, Orientation, Overlay, Revealer,
    RevealerTransitionType,
};
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

    let center = Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .valign(gtk::Align::Center)
        .build();

    let call_label = Label::new(Some("Call"));
    let sep1 = Label::new(Some("—"));
    let timer_label = Label::new(Some("00:00"));
    let sep2 = Label::new(Some("—"));
    let parties_box = Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();

    center.append(&call_label);
    center.append(&sep1);
    center.append(&timer_label);
    center.append(&sep2);
    center.append(&parties_box);

    row.set_center_widget(Some(&center));

    let end_btn = Button::builder()
        .label("End Call")
        .css_classes(["destructive-action"])
        .build();
    row.set_end_widget(Some(&end_btn));
    revealer.set_child(Some(&row));
    state.toolbar.add_top_bar(&revealer);

    {
        let state = state.clone();
        end_btn.connect_clicked(move |_| {
            state.client.call_stop();
        });
    }

    let call_started: Rc<RefCell<Option<std::time::Instant>>> = Rc::new(RefCell::new(None));
    let call_started_for_timer = call_started.clone();

    // Entries: (public_key, shared activity cell, ring DrawingArea).
    // An empty key "" is used as sentinel for the local "Me" entry.
    // The watcher rebuilds this on every call state change; the timer polls it.
    type PartyRings = Rc<RefCell<Vec<(String, Rc<Cell<f32>>, DrawingArea)>>>;
    let party_rings: PartyRings = Rc::new(RefCell::new(vec![]));
    let party_rings_for_timer = party_rings.clone();

    state.client.active_call_watch(move |call| match call {
        Some(call) => {
            *call_started.borrow_mut() = call.call_started;

            while let Some(child) = parties_box.first_child() {
                parties_box.remove(&child);
            }
            party_rings.borrow_mut().clear();

            // Helper: build an avatar+ring widget and register the ring for timer polling.
            let make_party_widget = |label_text: &str, ring_key: String, initial_activity: f32| {
                let activity_cell = Rc::new(Cell::new(initial_activity));

                let ring = DrawingArea::new();
                ring.set_size_request(28, 28);
                let ac = activity_cell.clone();
                ring.set_draw_func(move |_, cr, w, h| {
                    let activity = ac.get();
                    let line_width = (activity.sqrt() * 9.0).min(8.0) as f64;
                    if line_width < 0.5 {
                        return;
                    }
                    cr.set_source_rgba(0.2, 0.85, 0.2, 1.0);
                    cr.set_line_width(line_width);
                    let r = (w.min(h) as f64 / 2.0) - line_width / 2.0;
                    cr.arc(
                        w as f64 / 2.0,
                        h as f64 / 2.0,
                        r,
                        0.0,
                        2.0 * std::f64::consts::PI,
                    );
                    let _ = cr.stroke();
                });

                let overlay = Overlay::new();
                let avatar = adw::Avatar::builder()
                    .size(28)
                    .text(label_text)
                    .show_initials(true)
                    .build();
                overlay.set_child(Some(&avatar));
                overlay.add_overlay(&ring);

                party_rings
                    .borrow_mut()
                    .push((ring_key, activity_cell, ring));

                let row = Box::builder()
                    .orientation(Orientation::Horizontal)
                    .spacing(4)
                    .valign(gtk::Align::Center)
                    .build();
                row.append(&overlay);
                row.append(&Label::new(Some(label_text)));
                row
            };

            // "Me" always comes first.
            parties_box.append(&make_party_widget("Me", String::new(), 0.0));

            for party in call.get_parties() {
                if !matches!(party.status, CallPartyStatus::Active) {
                    continue;
                }
                parties_box.append(&make_party_widget(
                    &party.contact.alias,
                    party.contact.public_key.clone(),
                    party.voice_activity,
                ));
            }

            let hung_up = call
                .get_party_status_pairs()
                .iter()
                .any(|(_, s)| matches!(s, CallPartyStatus::HungUp));
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
            *call_started.borrow_mut() = None;
            revealer.set_reveal_child(false);
        }
    });

    // Update timer label and ring widths every 200 ms.
    glib::timeout_add_local(Duration::from_millis(200), move || {
        if let Some(started) = *call_started_for_timer.borrow() {
            let secs = started.elapsed().as_secs();
            timer_label.set_text(&format!("{:02}:{:02}", secs / 60, secs % 60));
        }

        for (key, activity_cell, ring) in party_rings_for_timer.borrow().iter() {
            let activity = if key.is_empty() {
                state.client.active_call_local_voice_activity()
            } else {
                state.client.active_call_voice_activity(key)
            };
            activity_cell.set(activity);
            ring.queue_draw();
        }

        glib::ControlFlow::Continue
    });
}
