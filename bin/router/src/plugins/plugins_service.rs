use std::sync::Arc;

use hive_router_plan_executor::{
    execution::plan::PlanExecutionOutput,
    hooks::on_http_request::{OnHttpRequestPayload, OnHttpResponsePayload},
    plugin_context::PluginContext,
    plugin_trait::ControlFlowResult,
};
use http::StatusCode;
use ntex::{
    http::ResponseBuilder,
    service::{Service, ServiceCtx},
    web::{self, DefaultError},
    Middleware,
};

use crate::RouterSharedState;

pub struct PluginService;

impl<S> Middleware<S> for PluginService {
    type Service = PluginMiddleware<S>;

    fn create(&self, service: S) -> Self::Service {
        PluginMiddleware { service }
    }
}

pub struct PluginMiddleware<S> {
    // This is special: We need this to avoid lifetime issues.
    service: S,
}

impl<S> Service<web::WebRequest<DefaultError>> for PluginMiddleware<S>
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
        let plugins = req
            .app_state::<Arc<RouterSharedState>>()
            .and_then(|shared_state| shared_state.plugins.clone());

        if let Some(plugins) = plugins.as_ref() {
            let plugin_context = Arc::new(PluginContext::default());
            req.extensions_mut().insert(plugin_context.clone());

            let mut start_payload = OnHttpRequestPayload {
                router_http_request: req,
                context: &plugin_context,
            };

            let mut on_end_callbacks = vec![];

            let mut early_response: Option<PlanExecutionOutput> = None;
            for plugin in plugins.iter() {
                let result = plugin.on_http_request(start_payload);
                start_payload = result.payload;
                match result.control_flow {
                    ControlFlowResult::Continue => {
                        // continue to next plugin
                    }
                    ControlFlowResult::OnEnd(callback) => {
                        on_end_callbacks.push(callback);
                    }
                    ControlFlowResult::EndResponse(response) => {
                        early_response = Some(response);
                        break;
                    }
                }
            }

            let req = start_payload.router_http_request;

            let response = if let Some(early_response) = early_response {
                let mut builder = ResponseBuilder::new(StatusCode::OK);
                for (key, value) in early_response.headers.iter() {
                    builder.header(key, value);
                }
                let res = builder.body(early_response.body);
                req.into_response(res)
            } else {
                ctx.call(&self.service, req).await?
            };

            let mut end_payload = OnHttpResponsePayload {
                response,
                context: &plugin_context,
            };

            for callback in on_end_callbacks.into_iter().rev() {
                let result = callback(end_payload);
                end_payload = result.payload;
                match result.control_flow {
                    ControlFlowResult::Continue => {
                        // continue to next callback
                    }
                    ControlFlowResult::EndResponse(_response) => {
                        // Short-circuit the request with the provided response
                        unimplemented!()
                    }
                    ControlFlowResult::OnEnd(_) => {
                        // This should not happen
                        unreachable!();
                    }
                }
            }

            return Ok(end_payload.response);
        }

        ctx.call(&self.service, req).await
    }
}
