use std::sync::atomic::Ordering::Relaxed;
use std::time::Instant;

use crate::telemetry::logging::summary::{self, SummaryOnDrop, WithRequestSummary};
use ntex::{
    http::body::{BodySize, MessageBody},
    router::{Path, Router},
    service::{Service, ServiceCtx},
    web::{self, DefaultError},
    Middleware, SharedCfg,
};

/// Matches whatever `graphql_endpoint_handler` serves
fn build_graphql_matcher(graphql_path: &str) -> Router<()> {
    let mut builder = Router::build();
    builder.path(graphql_path, ());
    if graphql_path != "/" {
        builder.prefix(graphql_path, ());
    }
    builder.finish()
}

/// Scopes the request summary as a task-local for the rest of the request, so every
/// downstream middleware (`PluginService`, the coprocessor, etc.) can enrich it
#[derive(Clone)]
pub struct RequestSummaryService {
    graphql_matcher: Router<()>,
}

impl RequestSummaryService {
    pub fn new(graphql_path: &str) -> Self {
        Self {
            graphql_matcher: build_graphql_matcher(graphql_path),
        }
    }
}

impl<S> Middleware<S, SharedCfg> for RequestSummaryService {
    type Service = RequestSummaryMiddleware<S>;

    fn create(&self, service: S, _cfg: SharedCfg) -> Self::Service {
        RequestSummaryMiddleware {
            service,
            graphql_matcher: self.graphql_matcher.clone(),
        }
    }
}

pub struct RequestSummaryMiddleware<S> {
    service: S,
    graphql_matcher: Router<()>,
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
        // Only requests headed for the GraphQL endpoint get HTTP request/summary logging
        if self
            .graphql_matcher
            .recognize(&mut Path::new(req.path()))
            .is_none()
        {
            return ctx.call(&self.service, req).await;
        }

        let started_at = Instant::now();

        // The guard is created while the task-local summary scope below is still active
        let (response, guard) = async {
            let response = ctx.call(&self.service, req).await?;

            // Re-records over whatever the handler already set (e.g. before a plugin's `on_end`
            // callback ran and read it) with the truly final response
            let status_code = response.status().as_u16();
            let payload_bytes = match response.response().body().size() {
                BodySize::Empty | BodySize::None => 0,
                BodySize::Sized(size) => i64::try_from(size).unwrap_or(i64::MAX),
                BodySize::Stream => -1,
            };
            summary::record(|s| {
                s.status_code.store(status_code, Relaxed);
                s.payload_bytes.store(payload_bytes, Relaxed);
            });

            Ok::<_, S::Error>((response, SummaryOnDrop::new(started_at)))
        }
        .with_request_summary()
        .await?;

        Ok(guard.attach_to_response(response))
    }
}
