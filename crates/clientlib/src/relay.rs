#[derive(Clone, Debug)]
pub struct RelayInfo {
    pub url: String,
}

impl RelayInfo {
    pub fn new(url: String) -> Self {
        Self { url }
    }
}

impl From<&str> for RelayInfo {
    fn from(value: &str) -> Self {
        Self {
            url: value.to_string(),
        }
    }
}

impl From<String> for RelayInfo {
    fn from(value: String) -> Self {
        Self { url: value }
    }
}
