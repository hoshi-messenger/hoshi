use std::time::{SystemTime, UNIX_EPOCH};

use axum::response::Html;

pub fn now() -> i64 {
    let seconds_since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    i64::try_from(seconds_since_epoch).unwrap_or(i64::MAX)
}

pub fn response_html(body: &str, title: &str) -> Html<String> {
    let body = format!(
        "<!DOCTYPE html>\n<html><head><title>{title}</title></head><body>{body}</body></html>"
    );
    body.into()
}
