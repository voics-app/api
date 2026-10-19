//! HTTP interface: axum routers, extractors and error mapping. Handlers only
//! translate between the wire (contracts) and application use cases.

pub mod cors;
pub mod error;
pub mod extract;
pub mod images;
pub mod ratelimit;
pub mod routes;
pub mod state;

use axum::Router;

use state::AppState;

/// Every route the api service serves, under `/api/v1` for the public surface.
pub fn router(state: AppState) -> Router {
    let public = Router::new()
        .merge(routes::auth::routes())
        .merge(routes::users::routes())
        .merge(routes::devices::routes())
        .merge(routes::guilds::routes())
        .merge(routes::channels::routes())
        .merge(routes::members::routes())
        .merge(routes::invites::routes())
        .merge(routes::emojis::routes())
        .merge(routes::applications::routes())
        .merge(routes::webhooks::routes());
    let public = public.layer(axum::middleware::from_fn_with_state(
        state.clone(),
        ratelimit::enforce,
    ));
    Router::new()
        .merge(routes::health::routes())
        .merge(routes::internal::routes())
        .nest("/api/v1", public)
        .with_state(state)
}
