use std::{ops::ControlFlow, sync::Arc};

use crate::executor::{
    hooks::on_http_request::{OnHttpRequestHookPayload, OnHttpResponseHookPayload},
    plugin_context::PluginContext,
    plugin_trait::{EndControlFlow, StartControlFlow},
    plugins::hooks,
    request_context::{RequestContextExt, SharedRequestContext},
};
use ntex::{
    service::{Ctx, Service},
    web, Middleware,
};

use crate::{RouterPaths, RouterSharedState};

pub struct PluginService {
    shared_state: Arc<RouterSharedState>,
    paths: RouterPaths,
    prometheus_endpoint: Option<String>,
}

impl PluginService {
    pub fn new(
        shared_state: Arc<RouterSharedState>,
        paths: RouterPaths,
        prometheus_endpoint: Option<String>,
    ) -> Self {
        Self {
            shared_state,
            paths,
            prometheus_endpoint,
        }
    }
}

impl<S, St> Middleware<S, St> for PluginService {
    type Service = PluginMiddleware<S>;

    fn create(&self, _st: &St, service: S) -> Self::Service {
        PluginMiddleware {
            service,
            shared_state: self.shared_state.clone(),
            paths: self.paths.clone(),
            prometheus_endpoint: self.prometheus_endpoint.clone(),
        }
    }
}

pub struct PluginMiddleware<S> {
    // This is special: We need this to avoid lifetime issues.
    service: S,
    // held here so requests don't look it up in the app state map every time
    shared_state: Arc<RouterSharedState>,
    paths: RouterPaths,
    prometheus_endpoint: Option<String>,
}

impl<S, St> Service<St, web::WebRequest<()>> for PluginMiddleware<S>
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
        mut req: web::WebRequest<()>,
        ctx: Ctx<'_, Self, St>,
    ) -> Result<Self::Res, Self::Error> {
        // The prometheus endpoint (if it's on the same port) bypasses both plugins and the
        // coprocessor entirely.
        if self.prometheus_endpoint.as_deref() == Some(req.path()) {
            return ctx.call(&self.service, req).await;
        }

        // Health and readiness run the plugin `on_http_request`/`on_end` chain (so a
        // plugin-selected supergraph can be resolved for readiness), but never the coprocessor
        let is_health = req.path() == self.paths.health;
        let is_probe = is_health || req.path() == self.paths.readiness;

        let request_context = SharedRequestContext::default();
        req.write_request_context(request_context.clone());

        let coprocessor_runtime = if is_probe {
            None
        } else {
            self.shared_state.coprocessor.as_ref()
        };

        let plugins = self.shared_state.plugins.clone();

        if let Some(coprocessor_runtime) = coprocessor_runtime {
            match coprocessor_runtime.on_router_request(req).await {
                ControlFlow::Break(response) => return Ok(response),
                ControlFlow::Continue(new_req) => req = new_req,
            }
        }

        if let Some(plugins) = plugins.as_ref() {
            let plugin_context = Arc::new(PluginContext::default());
            req.extensions_mut().insert(plugin_context.clone());

            let mut start_payload = OnHttpRequestHookPayload {
                router_http_request: req,
                context: &plugin_context,
                request_context: request_context.for_plugin::<hooks::OnHttpRequest>(),
            };

            let mut on_end_callbacks = Vec::with_capacity(plugins.len());

            for plugin in plugins.as_ref() {
                let result = plugin.on_http_request(start_payload).await;
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
                    request_context: request_context.for_plugin::<hooks::OnHttpRequest>(),
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
