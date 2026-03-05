use std::collections::HashMap;

fn generate_emoji_alias(public_key: &str) -> String {
    const EMOJIS: &[&str] = &[
        "🐶", "🐱", "🐭", "🐹", "🐰", "🦊", "🐻", "🐼", "🐨", "🐯", "🦁", "🐮", "🐷", "🐸", "🐵",
        "🐔", "🐧", "🐦", "🦆", "🦉", "🦇", "🐺", "🐗", "🐴", "🦄", "🐝", "🐛", "🦋", "🐌", "🐞",
        "🦎", "🐍", "🐢", "🦖", "🦕", "🐙", "🦑", "🦐", "🦀", "🐡", "🐠", "🐟", "🐬", "🐳", "🦈",
        "🐊", "🦧", "🦥", "🦦", "🦔",
    ];

    let hash: u64 = public_key.bytes().fold(0xcbf29ce484222325u64, |acc, b| {
        acc.wrapping_mul(0x100000001b3).wrapping_add(b as u64) // FNV-1a
    });

    let first = EMOJIS[(hash % EMOJIS.len() as u64) as usize];
    let second = EMOJIS[((hash >> 8) % EMOJIS.len() as u64) as usize];

    format!("{}{}", first, second)
}

#[derive(Debug, Clone)]
pub struct Contact {
    pub public_key: String,
    pub alias: String,
}
impl Contact {
    pub fn new(public_key: String, alias: Option<String>) -> Contact {
        let alias = alias.unwrap_or_else(|| generate_emoji_alias(&public_key));
        Self { public_key, alias }
    }

    pub(crate) fn placeholder_contacts() -> HashMap<String, Contact> {
        HashMap::from([
            (
                "123456".to_string(),
                Contact::new("123456".to_string(), None),
            ),
            (
                "test".to_string(),
                Contact::new("test".to_string(), Some("Testuser".to_string())),
            ),
        ])
    }
}
