#[derive(Clone, Debug)]
pub struct ChatMessage {
    pub id: String,
    pub created_at: i64,
    pub from: String,
    pub to: String,
    pub content: String,
}

impl ChatMessage {
    pub fn new(id: String, created_at: i64, from: String, to: String, content: String) -> Self {
        Self {
            id,
            created_at,
            from,
            to,
            content,
        }
    }
}
