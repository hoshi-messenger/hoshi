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

pub fn init_call_banner(state: AppState) {}
