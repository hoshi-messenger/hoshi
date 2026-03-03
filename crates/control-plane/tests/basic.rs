mod common;

use common::with_control_plane;
use reqwest::Client;

#[tokio::test]
async fn basic_http() {
    with_control_plane(|state| async move {
        let client = Client::builder()
            .build()
            .expect("Failed to create reqwest client");

        let base = state.config.uri();
        let res = client.get(format!("{}/", base)).send().await.unwrap();
        let text = res.text().await.unwrap();

        assert!(
            text.contains("Hoshi"),
            "Hoshi doesn't appear on landing page"
        );
    })
    .await;
}

#[tokio::test]
async fn basic_config_db_tests() {
    with_control_plane(|state| async move {
        let val = state.db.get_config("test").await.unwrap();
        assert!(val.is_none(), "'test' has a value before we set it");

        state.db.set_config("test", b"123").await.unwrap();
        let val = state.db.get_config("test").await.unwrap().unwrap();
        assert_eq!(val, b"123");

        state.db.set_config("test", b"").await.unwrap();
        let val = state.db.get_config("test").await.unwrap().unwrap();
        assert_eq!(val, b"");

        let mut vec: Vec<u8> = Vec::new();
        for i in 1..4096 {
            let v = i as u8;
            vec.push(v);
        }
        state.db.set_config("test", &vec).await.unwrap();
        let val = state.db.get_config("test").await.unwrap().unwrap();
        assert_eq!(val, vec);
    })
    .await;
}
