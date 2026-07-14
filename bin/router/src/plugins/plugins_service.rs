use std::{ops::ControlFlow, sync::Arc};

use graphql_tools::static_graphql::schema::Document;
use hive_router_plan_executor::{
    hooks::on_http_request::{OnHttpRequestHookPayload, OnHttpResponseHookPayload},
    plugin_context::PluginContext,
    plugin_trait::{EndControlFlow, StartControlFlow},
    plugins::hooks,
    request_context::{RequestContextExt, SharedRequestContext},
};
use ntex::{
    http::StatusCode,
    service::{Service, ServiceCtx},
    web::{self, DefaultError},
    Middleware, SharedCfg,
};
use tracing::error;

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

        let request_context = SharedRequestContext::default();
        req.write_request_context(request_context.clone());

        let coprocessor_runtime = shared_state
            .as_ref()
            .and_then(|shared_state| shared_state.coprocessor.as_ref());

        let plugins = shared_state
            .as_ref()
            .and_then(|state| state.plugins.clone());

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

            // A plugin may have selected a schema document via `set_schema_document`. Resolve it
            // through the router-owned schema-state cache here, once, so both HTTP and WebSocket
            // entry points can just read the resulting `Arc<SchemaState>` from request extensions.
            let selected_document = req.extensions().get::<Arc<Document>>().cloned();
            if let Some(document) = selected_document {
                let shared_state = shared_state
                    .as_ref()
                    .expect("router shared state must be present when plugins are configured");
                match shared_state.schema_state_cache.resolve(
                    document,
                    shared_state.router_config.clone(),
                    shared_state.telemetry_context.clone(),
                ) {
                    Ok(schema_state) => {
                        req.extensions_mut().insert(schema_state);
                    }
                    Err(err) => {
                        // if schema-state build fails, we intentionally return an internal error.
                        // falling back to the default schema could expose fields the plugin intended
                        // to remove or change - posing a security threat
                        error!(error = %err, "failed to build schema state for plugin-selected document");
                        let error_response = web::HttpResponse::build(
                            StatusCode::INTERNAL_SERVER_ERROR,
                        )
                        .body("Failed to build schema state for the selected schema document");
                        return Ok(req.into_response(error_response));
                    }
                }
            }

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
