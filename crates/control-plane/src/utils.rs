use std::time::{SystemTime, UNIX_EPOCH};

use axum::response::Html;

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

pub fn response_html(body: &str, title: &str) -> Html<String> {
    let body = format!("<!DOCTYPE html>\n<html><head><title>{title}</title></head><body>{body}</body></html>");
    body.into()
}