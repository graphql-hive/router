use std::sync::Arc;

use crate::telemetry::logging::request_id::WithRequestIdentifiers;
use ntex::{
    service::{Ctx, Service},
    web,
    Middleware,
};

use crate::{telemetry::HeaderExtractor, RouterSharedState};

/// Extracts the request/trace correlation identifiers and scopes them as a task-local
/// for the rest of the request, so every downstream middleware (`PluginService`,
/// the coprocessor, etc.) can log with the same correlation ids as the handler -
/// not just the handler itself, which is where this used to happen.
#[derive(Clone, Default)]
pub struct RequestIdentifiersService;

impl<S, St> Middleware<S, St> for RequestIdentifiersService {
    type Service = RequestIdentifiersMiddleware<S>;

    fn create(&self, _st: &St, service: S) -> Self::Service {
        RequestIdentifiersMiddleware { service }
    }
}

pub struct RequestIdentifiersMiddleware<S> {
    service: S,
}

impl<S, St> Service<St, web::WebRequest<()>> for RequestIdentifiersMiddleware<S>
where
    S: Service<St, web::WebRequest<()>, Res = web::WebResponse>,
{
    type Res = web::WebResponse;
    type Error = S::Error;

    ntex::forward_bind!(St, service);
    ntex::forward_ready!(St, service);
    ntex::forward_shutdown!(St, service);

    async fn call(
        &self,
        req: web::WebRequest<()>,
        ctx: Ctx<'_, Self, St>,
    ) -> Result<Self::Res, Self::Error> {
        let Some(shared_state) = req.app_state::<Arc<RouterSharedState>>() else {
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
