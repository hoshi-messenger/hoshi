use adw::{Clamp, NavigationPage, prelude::*};
use gtk::{Box, Button, DrawingArea, Label, MenuButton, ScrolledWindow, TextView};
use hoshi_clientlib::{ChatMessage, Contact};

use crate::AppState;

use super::{clear_box, create_contact_box};

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
        .margin_top(2)
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
        .margin_top(2)
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

    // Slightly higher maximum_size than the input box due to the bubble arrows
    let clamp = Clamp::builder().maximum_size(600 + 10 * 2).build();

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
            .messages_watch(chat_id, move |_, _chat_id, messages| {
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
                        .selectable(true)
                        .wrap(true)
                        .wrap_mode(gtk::pango::WrapMode::WordChar)
                        .build();

                    // r, g, b, a matching the bubble CSS color
                    let (r, g, b, a) = if from_me {
                        (128.0 / 255.0, 156.0 / 255.0, 255.0 / 255.0, 0.4_f64)
                    } else {
                        (56.0 / 255.0, 48.0 / 255.0, 72.0 / 255.0, 0.4_f64)
                    };
                    let arrow = DrawingArea::builder()
                        .content_width(10)
                        .content_height(16)
                        .valign(gtk::Align::End)
                        .margin_bottom(20)
                        .build();
                    arrow.set_draw_func(move |_, cr, w, h| {
                        let w = w as f64;
                        let h = h as f64;
                        cr.set_source_rgba(r, g, b, a);
                        if from_me {
                            // Start top-left, curve inward (downward first) to the tip,
                            // then straight back up the bottom edge.
                            cr.move_to(0.0, 0.0);
                            cr.curve_to(0.0, h * 0.4, w * 0.5, h * 0.6, w, h * 0.9);
                            cr.line_to(0.0, h);
                        } else {
                            // Mirror for received: start top-right, curve inward to tip.
                            cr.move_to(w, 0.0);
                            cr.curve_to(w, h * 0.4, w * 0.5, h * 0.6, 0.0, h * 0.9);
                            cr.line_to(w, h);
                        }
                        cr.close_path();
                        let _ = cr.fill();
                    });

                    let row = Box::builder()
                        .orientation(gtk::Orientation::Horizontal)
                        .halign(if from_me {
                            gtk::Align::End
                        } else {
                            gtk::Align::Start
                        })
                        .build();

                    if from_me {
                        row.append(&label);
                        row.append(&arrow);
                    } else {
                        row.append(&arrow);
                        row.append(&label);
                    }
                    vbox.append(&row);
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

pub(super) fn view_chat_page(state: AppState, page: NavigationPage, contact: Option<Contact>) {
    if let Some(contact) = contact {
        view_contact_chat_page(state, page, contact);
    } else {
        view_empty_chat_page(state, page);
    }
}
