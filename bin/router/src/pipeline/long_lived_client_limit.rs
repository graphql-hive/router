use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use http::{header, StatusCode};
use ntex::{
    service::{Service, ServiceCtx},
    web::{self, DefaultError},
    Middleware, SharedCfg,
};

use crate::RouterSharedState;

/// pre-resolved at app construction time so the per-request path is branch-free
#[derive(Clone)]
pub struct LongLivedClientLimitService {
    /// false means the middleware is entirely bypassed on every request
    enabled: bool,
}

impl LongLivedClientLimitService {
    pub fn new(router_config: &hive_router_config::HiveRouterConfig) -> Self {
        let limit = router_config.traffic_shaping.router.max_long_lived_clients;
        let has_long_lived = router_config.subscriptions.enabled || router_config.websocket.enabled;
        Self {
            enabled: limit > 0 && has_long_lived,
        }
    }
}

impl<S> Middleware<S, SharedCfg> for LongLivedClientLimitService {
    type Service = LongLivedClientLimitMiddleware<S>;

    fn create(&self, service: S, _cfg: SharedCfg) -> Self::Service {
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

impl<S> Service<web::WebRequest<DefaultError>> for LongLivedClientLimitMiddleware<S>
where
    S: Service<web::WebRequest<DefaultError>, Response = web::WebResponse, Error = web::Error>,
{
    type Response = web::WebResponse;
    type Error = S::Error;

    ntex::forward_ready!(service);

    async fn call(
        &self,
        req: web::WebRequest<DefaultError>,
        ctx: ServiceCtx<'_, Self>,
    ) -> Result<Self::Response, Self::Error> {
        if !self.enabled {
            return ctx.call(&self.service, req).await;
        }

        if !is_long_lived_request(req.headers()) {
            return ctx.call(&self.service, req).await;
        }

        let shared_state = match req.app_state::<Arc<RouterSharedState>>() {
            Some(s) => s,
            None => return ctx.call(&self.service, req).await,
        };

        let limit = shared_state
            .router_config
            .traffic_shaping
            .router
            .max_long_lived_clients;
        let counter = shared_state.long_lived_client_count.clone();

        // try to reserve a slot; back off if we're at the limit
        let prev = counter.fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            if current < limit {
                Some(current + 1)
            } else {
                None
            }
        });

        if prev.is_err() {
            let error_response = web::HttpResponse::build(StatusCode::SERVICE_UNAVAILABLE)
                .header(header::RETRY_AFTER, "5")
                .body("Too many long-lived clients");
            return Ok(req.into_response(error_response));
        }

        let guard = LongLivedClientGuard(counter);
        let response = ctx.call(&self.service, req).await?;
        drop(guard);

        Ok(response)
    }
}

/// decrements the counter when dropped
struct LongLivedClientGuard(Arc<AtomicUsize>);

impl Drop for LongLivedClientGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// returns true if the request is a websocket upgrade or an http streaming request.
///
/// deliberately ordered cheapest check first:
/// 1. upgrade: websocket - two header lookups, no parsing
/// 2. accept streaming - one header lookup + fast substring pre-filter, full parse only if needed
#[inline]
fn is_long_lived_request(headers: &ntex::http::HeaderMap) -> bool {
    // websocket: Connection: Upgrade + Upgrade: websocket
    // both headers must be present and contain the expected values (case-insensitive)
    if headers
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
        && headers
            .get(header::CONNECTION)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.to_ascii_lowercase().contains("upgrade"))
    {
        return true;
    }

    // http streaming: Accept header contains a known streaming content type.
    // we do a fast substring scan before handing off to the full Accept parser
    // to avoid the parse cost on the hot path for regular requests.
    let accept = match headers.get(header::ACCEPT).and_then(|v| v.to_str().ok()) {
        Some(v) if !v.is_empty() => v,
        _ => return false,
    };

    if !looks_like_streaming_accept(accept) {
        return false;
    }

    use crate::pipeline::header::StreamContentType;
    use headers_accept::Accept;
    use std::str::FromStr;

    Accept::from_str(accept)
        .ok()
        .and_then(|a| a.negotiate(StreamContentType::media_types().iter()))
        .is_some()
}

/// fast pre-filter: returns true if the raw Accept string contains any substring
/// that could match a known streaming content type, avoiding the full parse on
/// the vast majority of regular (application/json) requests.
#[inline]
fn looks_like_streaming_accept(accept: &str) -> bool {
    // covers: multipart/mixed, text/event-stream
    accept.contains("multipart") || accept.contains("event-stream")
}
