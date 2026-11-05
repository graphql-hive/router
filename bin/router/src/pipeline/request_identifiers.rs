use std::sync::Arc;

use crate::telemetry::logging::request_id::WithRequestIdentifiers;
use ntex::{
    service::{Ctx, Service},
    web, Middleware,
};

use crate::{telemetry::HeaderExtractor, RouterAppState};

/// Extracts the request/trace correlation identifiers and scopes them as a task-local
/// for the rest of the request, so every downstream middleware (`PluginService`,
/// the coprocessor, etc.) can log with the same correlation ids as the handler -
/// not just the handler itself, which is where this used to happen.
#[derive(Clone, Default)]
pub struct RequestIdentifiersService;

impl<S> Middleware<S, RouterAppState> for RequestIdentifiersService {
    type Service = RequestIdentifiersMiddleware<S>;

    fn create(&self, _st: &RouterAppState, service: S) -> Self::Service {
        RequestIdentifiersMiddleware { service }
    }
}

pub struct RequestIdentifiersMiddleware<S> {
    service: S,
}

impl<S> Service<RouterAppState, web::WebRequest<()>> for RequestIdentifiersMiddleware<S>
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
        // the typed app state rides along in the pipeline ctx, no lookup or clone needed
        let shared_state = &ctx.st().shared;
        let parent_ctx = shared_state
            .telemetry_context
            .extract_context(&HeaderExtractor(req.headers()));

        let identifiers = Arc::new(
            shared_state
                .telemetry_context
                .logging_correlation_extractor
                .extract(req.headers(), &parent_ctx),
        );

        ctx.call(&self.service, req)
            .with_request_id(identifiers)
            .await
    }
}
