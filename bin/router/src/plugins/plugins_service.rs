use std::{ops::ControlFlow, sync::Arc};

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

use crate::{RouterPaths, RouterSharedState};

pub struct PluginService {
    paths: RouterPaths,
    prometheus_endpoint: Option<String>,
}

impl PluginService {
    pub fn new(paths: RouterPaths, prometheus_endpoint: Option<String>) -> Self {
        Self {
            paths,
            prometheus_endpoint,
        }
    }
}

impl<S> Middleware<S, SharedCfg> for PluginService {
    type Service = PluginMiddleware<S>;

    fn create(&self, service: S, _cfg: SharedCfg) -> Self::Service {
        PluginMiddleware {
            service,
            paths: self.paths.clone(),
            prometheus_endpoint: self.prometheus_endpoint.clone(),
        }
    }
}

pub struct PluginMiddleware<S> {
    // This is special: We need this to avoid lifetime issues.
    service: S,
    paths: RouterPaths,
    prometheus_endpoint: Option<String>,
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
        let shared_state = req.app_state::<Arc<RouterSharedState>>().cloned();

        // Determine if the request should be handled by plugins.
        // The exceptions are:
        // - health endpoint
        // - readiness endpoint
        // - prometheus endpoint (if it's on the same port)
        let should_run = {
            let path = req.path();
            path != self.paths.health
                && path != self.paths.readiness
                && self.prometheus_endpoint.as_deref() != Some(path)
        };

        if !should_run {
            return ctx.call(&self.service, req).await;
        }

        let coprocessor_runtime = shared_state
            .as_ref()
            .and_then(|shared_state| shared_state.coprocessor.as_ref());

        if let Some(coprocessor_runtime) = coprocessor_runtime {
            match coprocessor_runtime.on_router_request(req).await {
                ControlFlow::Break(response) => return Ok(response),
                ControlFlow::Continue(new_req) => req = new_req,
            }
        }

        let plugins = shared_state
            .as_ref()
            .and_then(|state| state.plugins.clone());

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

            if let Some(coprocessor_runtime) = coprocessor_runtime {
                response = coprocessor_runtime.on_router_response(response).await;
            }

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

        let mut response = ctx.call(&self.service, req).await?;

        if let Some(coprocessor_runtime) = coprocessor_runtime {
            response = coprocessor_runtime.on_router_response(response).await;
        }

        Ok(response)
    }
}
