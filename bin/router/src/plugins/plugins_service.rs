use std::sync::Arc;

use hive_router_plan_executor::{
    hooks::on_http_request::{OnHttpRequestHookPayload, OnHttpResponseHookPayload},
    plugin_context::PluginContext,
    plugin_trait::{EndControlFlow, StartControlFlow},
};
use ntex::{
    service::{Service, ServiceCtx},
    web::{self, DefaultError},
    Middleware, SharedCfg,
};

use crate::RouterSharedState;

pub struct PluginService;

impl<S> Middleware<S, SharedCfg> for PluginService {
    type Service = PluginMiddleware<S>;

    fn create(&self, service: S, _cfg: SharedCfg) -> Self::Service {
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
        mut req: web::WebRequest<DefaultError>,
        ctx: ServiceCtx<'_, Self>,
    ) -> Result<Self::Response, Self::Error> {
        let plugins = req
            .app_state::<Arc<RouterSharedState>>()
            .and_then(|shared_state| shared_state.plugins.clone());

        if let Some(plugins) = plugins.as_ref() {
            let plugin_context = Arc::new(PluginContext::default());
            req.extensions_mut().insert(plugin_context.clone());

            let mut start_payload = OnHttpRequestHookPayload {
                router_http_request: req,
                context: &plugin_context,
            };

            let mut on_end_callbacks = Vec::with_capacity(plugins.len());

            for plugin in plugins.as_ref() {
                let result = plugin.on_http_request(start_payload);
                start_payload = result.payload;
                match result.control_flow {
                    StartControlFlow::Proceed => {
                        // continue to next plugin
                    }
                    StartControlFlow::OnEnd(callback) => {
                        on_end_callbacks.push(callback);
                    }
                    StartControlFlow::EndWithResponse(response) => {
                        return Ok(start_payload.router_http_request.into_response(response));
                    }
                }
            }

            // Give the ownership back to variables
            req = start_payload.router_http_request;

            let mut response = ctx.call(&self.service, req).await?;

            if !on_end_callbacks.is_empty() {
                let mut end_payload = OnHttpResponseHookPayload {
                    response,
                    context: &plugin_context,
                };

                for callback in on_end_callbacks.into_iter().rev() {
                    let result = callback(end_payload);
                    end_payload = result.payload;
                    match result.control_flow {
                        EndControlFlow::Proceed => {
                            // continue to next callback
                        }
                        EndControlFlow::EndWithResponse(response) => {
                            end_payload.response = end_payload.response.into_response(response);
                            return Ok(end_payload.response);
                        }
                    }
                }

                // Give the ownership back to variables
                response = end_payload.response;
            }
            return Ok(response);
        }

        ctx.call(&self.service, req).await
    }
}
