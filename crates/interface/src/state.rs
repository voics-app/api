//! Shared handler state: the application ports and the internal shared secret.

use std::sync::Arc;

use application::Ports;

#[derive(Clone)]
pub struct AppState {
    pub ports: Ports,
    /// `INTERNAL_SERVICE_TOKEN` (P2), compared against `X-Internal-Token`.
    pub internal_token: Arc<str>,
}
