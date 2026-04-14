use std::{
    rc::Rc,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Poll},
};

use http::{header, StatusCode};
use ntex::{
    http::body::{Body, BodySize, MessageBody},
    service::{Service, ServiceCtx},
    util::Bytes,
    web::{self, DefaultError},
    Middleware, SharedCfg,
};

use crate::RouterSharedState;

#[derive(Clone)]
pub struct LongLivedClientLimitService {
    // false means the middleware is entirely bypassed on every request
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

        // try to reserve a slot, bail if at the limit
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

        // wrap the body so the guard lives until the stream is fully consumed
        let response = response.map_body(|_head, body| {
            let wrapped = GuardedBody {
                inner: body.into_body().into(),
                _guard: guard,
            };
            Body::from_message(wrapped).into()
        });

        Ok(response)
    }
}

// decrements the counter when dropped
struct LongLivedClientGuard(Arc<AtomicUsize>);

impl Drop for LongLivedClientGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

// wraps the body and keeps the guard alive until it's fully consumed and dropped.
// one extra vtable call per chunk on top of the Box<dyn MessageBody> dispatch streaming bodies
// already go through - negligible next to actual I/O cost per chunk.
struct GuardedBody {
    inner: Body,
    _guard: LongLivedClientGuard,
}

impl MessageBody for GuardedBody {
    fn size(&self) -> BodySize {
        self.inner.size()
    }

    fn poll_next_chunk(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Rc<dyn std::error::Error>>>> {
        self.inner.poll_next_chunk(cx)
    }
}

// cheapest check first:
// 1. upgrade: websocket - two header lookups, no parsing
// 2. accept: streaming - one header lookup + fast substring pre-filter, full parse only if needed
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

    // fast substring scan before full Accept parse to avoid cost on regular requests
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

#[inline]
fn looks_like_streaming_accept(accept: &str) -> bool {
    // covers: multipart/mixed, text/event-stream
    accept.contains("multipart") || accept.contains("event-stream")
}
