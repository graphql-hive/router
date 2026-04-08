use hive_router_config::coprocessor::CoprocessorConfig;
use hive_router_internal::http::read_body_stream;
use hive_router_internal::telemetry::traces::spans::coprocessor::CoprocessorSpan;
use hive_router_internal::telemetry::TelemetryContext;
use http::{Method as HttpMethod, Uri};
use ntex::http::HeaderMap;
use ntex::web::{self, DefaultError};
use std::ops::ControlFlow;
use std::sync::Arc;
use tracing::{debug, error, info, Instrument};

use crate::coprocessor::client::CoprocessorClient;
use crate::coprocessor::error::CoprocessorError;
use crate::coprocessor::stage::Stage;
use crate::coprocessor::stages::graphql::{
    GraphqlAnalysisInput, GraphqlAnalysisStage, GraphqlRequestInput, GraphqlRequestStage,
    GraphqlResponseInput, GraphqlResponseStage,
};
use crate::coprocessor::stages::router::{
    RouterRequestInput, RouterRequestStage, RouterResponseInput, RouterResponseStage,
};
use crate::plugins::hooks::on_graphql_params::GraphQLParams;

pub struct CoprocessorRuntime {
    router_request: Option<StageRuntime<RouterRequestStage>>,
    router_response: Option<StageRuntime<RouterResponseStage>>,
    graphql_request: Option<StageRuntime<GraphqlRequestStage>>,
    graphql_analysis: Option<StageRuntime<GraphqlAnalysisStage>>,
    graphql_response: Option<StageRuntime<GraphqlResponseStage>>,
    body_size_limit: usize,
}

#[derive(Default)]
pub struct PerformedMutations {
    pub body: bool,
    pub headers: bool,
}

struct StageRuntime<S: Stage> {
    client: Arc<CoprocessorClient>,
    stage: S,
    telemetry_context: Arc<TelemetryContext>,
}

pub struct MutableRequestState<'a> {
    pub method: &'a HttpMethod,
    pub uri: &'a Uri,
    pub headers: &'a mut HeaderMap,
}

impl<A: Stage> StageRuntime<A> {
    fn new(
        client: Arc<CoprocessorClient>,
        stage: A,
        telemetry_context: Arc<TelemetryContext>,
    ) -> Self {
        Self {
            client,
            stage,
            telemetry_context,
        }
    }

    async fn execute<'a>(
        &self,
        input: &mut A::Input<'a>,
    ) -> Result<ControlFlow<web::HttpResponse, PerformedMutations>, CoprocessorError> {
        let result = self.execute_internal(input).await;

        if result.is_err() {
            let stage_name = self.stage.stage_name();
            let metrics = &self.telemetry_context.metrics.coprocessor;
            metrics.record_error(stage_name);
        }

        result
    }

    async fn execute_internal<'a>(
        &self,
        input: &mut A::Input<'a>,
    ) -> Result<ControlFlow<web::HttpResponse, PerformedMutations>, CoprocessorError> {
        let mut performed_mutations = PerformedMutations::default();

        // Skip remote call when condition says stage should not run
        if !self.stage.should_run(input)? {
            return Ok(ControlFlow::Continue(performed_mutations));
        }

        let stage_name = self.stage.stage_name();
        let metrics = &self.telemetry_context.metrics.coprocessor;
        metrics.record_request(stage_name);

        let start = std::time::Instant::now();
        let id = uuid::Uuid::new_v4().to_string();
        let span = CoprocessorSpan::new(stage_name, &id).span;

        async {
          // Build stage payload and call the coprocessor
          // TODO: Include `id` / `request_id` in coprocessor payloads for log correlation
          //       once Dotan's logging PR is merged.
          let request = self.stage.build_request(input, &id).map_err(|err| {
              error!(%err, coprocessor.stage = stage_name, "Coprocessor failed to build request");
              err
          })?;
            debug!(
                coprocessor.id = %id,
                coprocessor.stage = stage_name,
                "Sending coprocessor request"
            );

            let response = self.client.send(request.body).await.map_err(|err| {
                error!(%err, coprocessor.id = %id, coprocessor.stage = stage_name, "Coprocessor request failed");
                err
            })?;

            metrics.record_duration(stage_name, start.elapsed().as_secs_f64());

            if !response.status().is_success() {
                return Err(CoprocessorError::UnexpectedStatus(response.status()));
            }

            // Parse the response
            let mut parsed = self.stage.parse_response(response.body()).map_err(|err| {
                error!(%err, coprocessor.id = %id, coprocessor.stage = stage_name, "Coprocessor failed to parse response");
                err
            })?;

            if parsed.body.is_some() {
                performed_mutations.body = true;
            }
            if parsed.headers.is_some() {
                performed_mutations.headers = true;
            }

            // Handle possible break decision first
            match self.stage.break_output(parsed) {
                Ok(ControlFlow::Continue(p)) => {
                    parsed = p;
                }
                Ok(ControlFlow::Break(response)) => {
                    info!(
                        coprocessor.id = %id,
                        coprocessor.stage = stage_name,
                        status_code = %response.status(),
                        "Coprocessor short-circuited the request"
                    );
                    return Ok(ControlFlow::Break(response));
                }
                Err(err) => {
                    error!(%err, coprocessor.id = %id, coprocessor.stage = stage_name, "Coprocessor failed to break output");
                    return Err(err);
                }
            }

            if performed_mutations.body || performed_mutations.headers {
                info!(
                    coprocessor.id = %id,
                    coprocessor.stage = stage_name,
                    body = performed_mutations.body,
                    headers = performed_mutations.headers,
                    "Coprocessor mutated the request"
                );
            }

            // Apply mutations only when flow continues
            self.stage.apply_mutations(parsed, input).map_err(|err| {
                error!(%err, coprocessor.id = %id, coprocessor.stage = stage_name, "Coprocessor failed to apply mutations");
                err
            })?;

            // Continue the pipeline.
            Ok(ControlFlow::Continue(performed_mutations))
        }
        .instrument(span)
        .await
    }
}

impl CoprocessorRuntime {
    pub fn from_config(
        config: &CoprocessorConfig,
        telemetry_context: Arc<TelemetryContext>,
        body_size_limit: usize,
    ) -> Result<Self, CoprocessorError> {
        let client = Arc::new(CoprocessorClient::new(
            config.clone(),
            telemetry_context.clone(),
        )?);

        let router_request = config
            .stages
            .router
            .request
            .as_ref()
            .map(RouterRequestStage::from_config)
            .transpose()?
            .map(|adapter| StageRuntime::new(client.clone(), adapter, telemetry_context.clone()));

        let router_response = config
            .stages
            .router
            .response
            .as_ref()
            .map(RouterResponseStage::from_config)
            .transpose()?
            .map(|adapter| StageRuntime::new(client.clone(), adapter, telemetry_context.clone()));

        let graphql_request = config
            .stages
            .graphql
            .request
            .as_ref()
            .map(GraphqlRequestStage::from_config)
            .transpose()?
            .map(|adapter| StageRuntime::new(client.clone(), adapter, telemetry_context.clone()));

        let graphql_response = config
            .stages
            .graphql
            .response
            .as_ref()
            .map(GraphqlResponseStage::from_config)
            .transpose()?
            .map(|adapter| StageRuntime::new(client.clone(), adapter, telemetry_context.clone()));

        let graphql_analysis = config
            .stages
            .graphql
            .analysis
            .as_ref()
            .map(GraphqlAnalysisStage::from_config)
            .transpose()?
            .map(|adapter| StageRuntime::new(client, adapter, telemetry_context));

        Ok(Self {
            router_request,
            router_response,
            graphql_request,
            graphql_analysis,
            graphql_response,
            body_size_limit,
        })
    }

    // TODO: provide real public SDL
    pub fn graphql_request_needs_sdl(&self) -> bool {
        self.graphql_request
            .as_ref()
            .map(|stage| stage.stage.include_sdl())
            .unwrap_or(false)
    }

    // TODO: provide real public SDL
    pub fn graphql_response_needs_sdl(&self) -> bool {
        self.graphql_response
            .as_ref()
            .map(|stage| stage.stage.include_sdl())
            .unwrap_or(false)
    }

    // TODO: provide real public SDL
    pub fn graphql_analysis_needs_sdl(&self) -> bool {
        self.graphql_analysis
            .as_ref()
            .map(|stage| stage.stage.include_sdl())
            .unwrap_or(false)
    }

    pub async fn on_router_request(
        &self,
        mut req: web::WebRequest<DefaultError>,
    ) -> ControlFlow<web::WebResponse, web::WebRequest<DefaultError>> {
        let Some(stage) = &self.router_request else {
            return ControlFlow::Continue(req);
        };

        // We read the request body only when this stage needs to include body
        let request_body = if stage.stage.include_body() {
            let body_stream = web::types::Payload(req.take_payload());
            let new_body = match read_body_stream(&req, body_stream, self.body_size_limit).await {
                Ok(body) => body,
                // We deliberately do not map to CoprocessorError here,
                // to follow the same logic for status codes
                Err(err) => {
                    error!(%err, "coprocessor {} stage failed", stage.stage.stage_name());
                    return ControlFlow::Break(
                        // TODO: We should also return back the body, in proper format (json, text)
                        req.into_response(web::HttpResponse::new(err.status_code())),
                    );
                }
            };

            Some(new_body)
        } else {
            None
        };

        let mut input = RouterRequestInput::new(req, request_body);

        match stage
            .execute(&mut input)
            .await
            .unwrap_or_else(|err| error_to_break(stage, err))
        {
            ControlFlow::Continue(_) => {
                // On continue, restore the original body only when coprocessor did not replace it
                input.restore_request_body_if_unchanged();
                ControlFlow::Continue(input.request)
            }
            ControlFlow::Break(response) => {
                // On break, we return immediately and skip body restoration to avoid unnecessary work
                ControlFlow::Break(input.request.into_response(response))
            }
        }
    }

    pub async fn on_router_response(&self, response: web::WebResponse) -> web::WebResponse {
        let Some(stage) = &self.router_response else {
            return response;
        };

        let mut input = RouterResponseInput::new(response);

        match stage
            .execute(&mut input)
            .await
            .unwrap_or_else(|err| error_to_break(stage, err))
        {
            ControlFlow::Continue(_) => input.response,
            ControlFlow::Break(response) => input.response.into_response(response),
        }
    }

    pub async fn on_graphql_request(
        &self,
        request: &web::HttpRequest,
        request_headers: &mut HeaderMap,
        graphql_request: &mut GraphQLParams,
        sdl: Option<&str>,
    ) -> Result<ControlFlow<web::HttpResponse, PerformedMutations>, CoprocessorError> {
        let Some(stage) = &self.graphql_request else {
            return Ok(ControlFlow::Continue(Default::default()));
        };

        let mut input = GraphqlRequestInput::new(request, request_headers, graphql_request, sdl);
        stage.execute(&mut input).await
    }

    pub async fn on_graphql_response(
        &self,
        response: web::HttpResponse,
        request: &web::HttpRequest,
        sdl: Option<&str>,
    ) -> Result<ControlFlow<web::HttpResponse, web::HttpResponse>, CoprocessorError> {
        let Some(stage) = &self.graphql_response else {
            return Ok(ControlFlow::Continue(response));
        };

        let mut input = GraphqlResponseInput::new(response, request, sdl);
        Ok(stage
            .execute(&mut input)
            .await?
            .map_continue(|_| input.response))
    }

    pub async fn on_graphql_analysis(
        &self,
        request: MutableRequestState<'_>,
        graphql_request: &GraphQLParams,
        sdl: Option<&str>,
    ) -> Result<ControlFlow<web::HttpResponse, PerformedMutations>, CoprocessorError> {
        let Some(stage) = &self.graphql_analysis else {
            return Ok(ControlFlow::Continue(Default::default()));
        };

        let mut input = GraphqlAnalysisInput::new(request, graphql_request, sdl);
        stage.execute(&mut input).await
    }
}

fn error_to_break<A: Stage>(
    stage: &StageRuntime<A>,
    err: CoprocessorError,
) -> ControlFlow<web::HttpResponse, PerformedMutations> {
    // Stage error metrics and specific logs are already recorded in execute() where possible,
    // but keeping a fallback log here correctly reports the failure that causes short-circuit
    // in case it's not caught earlier, and always breaks the request.
    error!(%err, coprocessor.stage = stage.stage.stage_name(), "coprocessor stage failed");
    ControlFlow::Break(web::HttpResponse::new(err.status_code()))
}
