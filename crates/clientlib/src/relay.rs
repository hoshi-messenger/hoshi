#[derive(Clone, Debug)]
pub struct RelayInfo {
    pub url: String,
}

impl RelayInfo {
    pub fn new(url: String) -> Self {
        Self { url }
    }
}
