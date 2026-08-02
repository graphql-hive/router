use std::sync::Arc;

use hive_router_internal::telemetry::logging::request_id::WithRequestIdentifiers;
use ntex::{
    service::{Service, ServiceCtx},
    web::{self, DefaultError},
    Middleware, SharedCfg,
};

use crate::{telemetry::HeaderExtractor, RouterSharedState};

/// Extracts the request/trace correlation identifiers and scopes them as a task-local
/// for the rest of the request, so every downstream middleware (`PluginService`,
/// the coprocessor, etc.) can log with the same correlation ids as the handler -
/// not just the handler itself, which is where this used to happen.
#[derive(Clone, Default)]
pub struct RequestIdentifiersService;

impl<S> Middleware<S, SharedCfg> for RequestIdentifiersService {
    type Service = RequestIdentifiersMiddleware<S>;

    fn create(&self, service: S, _cfg: SharedCfg) -> Self::Service {
        RequestIdentifiersMiddleware { service }
    }
}

pub struct RequestIdentifiersMiddleware<S> {
    service: S,
}

impl<S> Service<web::WebRequest<DefaultError>> for RequestIdentifiersMiddleware<S>
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
        let Some(shared_state) = req.app_state::<Arc<RouterSharedState>>().cloned() else {
            return ctx.call(&self.service, req).await;
        };

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
