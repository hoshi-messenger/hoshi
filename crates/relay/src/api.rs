use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HealthzResponse {
    pub status: String,
    pub public_key: String,
    pub control_plane_uri: String,
}
