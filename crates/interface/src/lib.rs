//! HTTP interface: axum routers, extractors and error mapping. Handlers only
//! translate between the wire (contracts) and application use cases.

pub mod error;
pub mod extract;
pub mod images;
pub mod routes;
pub mod state;

use axum::Router;

use state::AppState;

/// Every route the api service serves, under `/api/v1` for the public surface.
pub fn router(state: AppState) -> Router {
    let public = Router::new()
        .merge(routes::auth::routes())
        .merge(routes::users::routes())
        .merge(routes::devices::routes());
    Router::new()
        .merge(routes::health::routes())
        .merge(routes::internal::routes())
        .nest("/api/v1", public)
        .with_state(state)
}
