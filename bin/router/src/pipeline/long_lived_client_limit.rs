use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use http::{header, StatusCode};
use ntex::{
    service::{Ctx, Service},
    web, Middleware,
};

use crate::{RouterAppState, RouterSharedState};

/// Counting happens in two places because the two kinds of long-lived clients
/// become known at different points of the request lifecycle:
///
/// - WebSocket connections are identified from the upgrade headers alone, so the
///   middleware below reserves a slot before the connection is established.
/// - HTTP streaming responses (subscriptions over SSE/multipart) are only known
///   once the GraphQL operation has been parsed, so the pipeline reserves a slot
///   via [`try_reserve_long_lived_client`] right before executing a subscription.
///
/// The `Accept` header is deliberately NOT used for classification: clients like
/// urql or Apollo Client advertise `text/event-stream`/`multipart/mixed` on every
/// operation, so regular queries would otherwise count toward (and get rejected
/// by) the limit.
#[derive(Clone)]
pub struct LongLivedClientLimitService {
    // false means the middleware is entirely bypassed on every request
    enabled: bool,
}

impl LongLivedClientLimitService {
    pub fn new(router_config: &crate::config::HiveRouterConfig) -> Self {
        let limit = router_config.traffic_shaping.router.max_long_lived_clients;
        Self {
            // the middleware only counts WebSocket connections; HTTP streaming
            // responses are counted in the pipeline where the operation is known
            enabled: limit > 0 && router_config.websocket.enabled,
        }
    }
}

impl<S> Middleware<S, RouterAppState> for LongLivedClientLimitService {
    type Service = LongLivedClientLimitMiddleware<S>;

    fn create(&self, _st: &RouterAppState, service: S) -> Self::Service {
        LongLivedClientLimitMiddleware {
            service,
            enabled: self.enabled,
        }
    }
}

pub struct LongLivedClientLimitMiddleware<S> {
    service: S,
    enabled: bool,
}

impl<S> Service<RouterAppState, web::WebRequest<()>> for LongLivedClientLimitMiddleware<S>
where
    S: Service<RouterAppState, web::WebRequest<()>, Res = web::WebResponse>,
{
    type Res = web::WebResponse;
    type Error = S::Error;

    ntex::forward_bind!(RouterAppState, service);
    ntex::forward_ready!(RouterAppState, service);
    ntex::forward_shutdown!(RouterAppState, service);

    async fn call(
        &self,
        req: web::WebRequest<()>,
        ctx: Ctx<'_, Self, RouterAppState>,
    ) -> Result<Self::Res, Self::Error> {
        if !self.enabled {
            return ctx.call(&self.service, req).await;
        }

        if !is_websocket_upgrade(req.headers()) {
            // we only limit websocket connections as long-lived clients here,
            // HTTP streaming responses are limited in the pipeline. see LongLivedClientLimitService
            // comment above for details
            return ctx.call(&self.service, req).await;
        }

        // the shared state comes from the typed app state in the pipeline ctx
        let _guard = match try_reserve_long_lived_client(&ctx.st().shared) {
            Ok(Some(guard)) => guard,
            // limit disabled, nothing to track
            Ok(None) => return ctx.call(&self.service, req).await,
            Err(LongLivedClientLimitReached) => {
                return Ok(req.into_response(LongLivedClientLimitReached));
            }
        };

        // `ws::start` drives the whole connection inside the handler, so this call resolves
        // only once the socket is closed and holding the guard across it covers the
        // connection's lifetime. a rejected upgrade resolves right away and frees the slot
        ctx.call(&self.service, req).await
    }
}

/// The long-lived client limit has been reached, no slot could be reserved.
pub struct LongLivedClientLimitReached;

impl From<LongLivedClientLimitReached> for web::HttpResponse {
    fn from(_: LongLivedClientLimitReached) -> Self {
        web::HttpResponse::builder(StatusCode::SERVICE_UNAVAILABLE)
            .header(header::RETRY_AFTER, "5")
            .body("Too many long-lived clients")
    }
}

/// Tries to reserve a long-lived client slot against
/// `traffic_shaping.router.max_long_lived_clients`.
///
/// Returns `Ok(None)` when the limit is disabled (set to `0`), `Ok(Some(guard))`
/// when a slot was reserved (the slot is released when the guard is dropped),
/// and `Err` when the limit has been reached.
pub fn try_reserve_long_lived_client(
    shared_state: &RouterSharedState,
) -> Result<Option<LongLivedClientGuard>, LongLivedClientLimitReached> {
    let limit = shared_state
        .router_config
        .traffic_shaping
        .router
        .max_long_lived_clients;
    if limit == 0 {
        return Ok(None);
    }

    let counter = shared_state.long_lived_client_count.clone();

    // try to reserve a slot, bail if at the limit
    counter
        .try_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            if current < limit {
                Some(current + 1)
            } else {
                None
            }
        })
        .map(|_| Some(LongLivedClientGuard(counter)))
        .map_err(|_| LongLivedClientLimitReached)
}

// decrements the counter when dropped
pub struct LongLivedClientGuard(Arc<AtomicUsize>);

impl Drop for LongLivedClientGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

// websocket: Connection: Upgrade + Upgrade: websocket
// both headers must be present and contain the expected values (case-insensitive)
#[inline]
fn is_websocket_upgrade(headers: &ntex::http::HeaderMap) -> bool {
    headers
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
        && headers
            .get(header::CONNECTION)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.to_ascii_lowercase().contains("upgrade"))
}
