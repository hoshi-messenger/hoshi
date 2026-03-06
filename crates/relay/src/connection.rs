use hoshi_clientlib::HoshiEnvelope;
use tokio::sync::mpsc;

pub struct HoshiConnection {
    pub id: uuid::Uuid,
    pub tx: mpsc::Sender<HoshiEnvelope>,
}
