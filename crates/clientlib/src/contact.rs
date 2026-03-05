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

    pub fn placeholder_contact() -> Contact {
        use std::time::{SystemTime, UNIX_EPOCH};

        const FIRST_NAMES: &[&str] = &[
            "Alice", "Bob", "Carol", "Dave", "Eve", "Frank", "Grace", "Heidi",
        ];
        const LAST_NAMES: &[&str] = &[
            "Smith", "Jones", "Patel", "Müller", "Tanaka", "Rossi", "Dubois", "García",
        ];

        // Poor man's RNG — good enough for placeholder data
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .subsec_nanos() as u64;

        let hash = seed.wrapping_mul(0x517cc1b727220a95);

        let first = FIRST_NAMES[(hash % FIRST_NAMES.len() as u64) as usize];
        let last = LAST_NAMES[((hash >> 3) % LAST_NAMES.len() as u64) as usize];

        let public_key = format!("{:016x}{:016x}", hash, hash.wrapping_mul(0x100000001b3));

        Contact::new(public_key, Some(format!("{} {}", first, last)))
    }
}
