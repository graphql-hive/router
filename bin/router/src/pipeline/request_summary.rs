use crate::telemetry::logging::summary::WithRequestSummary;
use ntex::{
    service::{Service, ServiceCtx},
    web::{self, DefaultError},
    Middleware, SharedCfg,
};

/// Scopes the request summary as a task-local for the rest of the request, so every
/// downstream middleware (`PluginService`, the coprocessor, etc.) can enrich it - not
/// just the handler itself, which is where this used to happen.
#[derive(Clone, Default)]
pub struct RequestSummaryService;

impl<S> Middleware<S, SharedCfg> for RequestSummaryService {
    type Service = RequestSummaryMiddleware<S>;

    fn create(&self, service: S, _cfg: SharedCfg) -> Self::Service {
        RequestSummaryMiddleware { service }
    }
}

pub struct RequestSummaryMiddleware<S> {
    service: S,
}

impl<S> Service<web::WebRequest<DefaultError>> for RequestSummaryMiddleware<S>
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
        ctx.call(&self.service, req).with_request_summary().await
    }
}
