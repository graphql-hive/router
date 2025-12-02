use std::sync::Arc;

use hive_router_plan_executor::{
    hooks::on_http_request::{OnHttpRequestHookPayload, OnHttpResponseHookPayload},
    plugin_context::PluginContext,
    plugin_trait::{EndControlFlow, StartControlFlow},
};
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

            let mut start_payload = OnHttpRequestHookPayload {
                router_http_request: req,
                context: &plugin_context,
            };

            let mut on_end_callbacks = vec![];

            for plugin in plugins.iter() {
                let result = plugin.on_http_request(start_payload);
                start_payload = result.payload;
                match result.control_flow {
                    StartControlFlow::Continue => {
                        // continue to next plugin
                    }
                    StartControlFlow::OnEnd(callback) => {
                        on_end_callbacks.push(callback);
                    }
                    StartControlFlow::EndResponse(response) => {
                        let mut resp_builder = ResponseBuilder::new(response.status);
                        for (key, value) in response.headers {
                            if let Some(key) = key {
                                resp_builder.header(key, value);
                            }
                        }
                        let response = start_payload
                            .router_http_request
                            .into_response(resp_builder.body(response.body.to_vec()));
                        return Ok(response);
                    }
                }
            }

            let req = start_payload.router_http_request;

            let response = ctx.call(&self.service, req).await?;

            let mut end_payload = OnHttpResponseHookPayload {
                response,
                context: &plugin_context,
            };

            for callback in on_end_callbacks.into_iter().rev() {
                let result = callback(end_payload);
                end_payload = result.payload;
                match result.control_flow {
                    EndControlFlow::Continue => {
                        // continue to next callback
                    }
                    EndControlFlow::EndResponse(response) => {
                        let mut resp_builder = ResponseBuilder::new(response.status);
                        for (key, value) in response.headers {
                            if let Some(key) = key {
                                resp_builder.header(key, value);
                            }
                        }
                        let response = resp_builder.body(response.body.to_vec());
                        end_payload.response = end_payload.response.into_response(response);
                        return Ok(end_payload.response);
                    }
                }
            }

            return Ok(end_payload.response);
        }

        ctx.call(&self.service, req).await
    }
}
