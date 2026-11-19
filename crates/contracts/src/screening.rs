use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct ScreeningDto {
    pub enabled: bool,
    pub rules: Vec<String>,
    pub revision: i64,
    pub pending: bool,
}

#[derive(Debug, Deserialize)]
pub struct SetScreeningRequest {
    pub enabled: bool,
    pub rules: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct AcceptScreeningRequest {
    pub revision: i64,
}
