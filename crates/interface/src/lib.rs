//! HTTP interface: axum routers, extractors and error mapping. Handlers only
//! translate between the wire (contracts) and application use cases.

pub mod cors;
pub mod error;
pub mod extract;
pub mod http_metrics;
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
        .merge(routes::mfa::routes())
        .merge(routes::passkeys::routes())
        .merge(routes::sessions::routes())
        .merge(routes::users::routes())
        .merge(routes::connections::routes())
        .merge(routes::devices::routes())
        .merge(routes::guilds::routes())
        .merge(routes::discovery::routes())
        .merge(routes::templates::routes())
        .merge(routes::channels::routes())
        .merge(routes::members::routes())
        .merge(routes::invites::routes())
        .merge(routes::emojis::routes())
        .merge(routes::applications::routes())
        .merge(routes::integrations::routes())
        .merge(routes::reports::routes())
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
        .layer(axum::middleware::from_fn(http_metrics::record))
}
