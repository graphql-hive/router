use std::borrow::Cow;
use std::collections::HashMap;

use hive_router_config::coprocessor::{
    CoprocessorGraphqlAnalysisIncludeConfig, CoprocessorGraphqlRequestIncludeConfig,
    CoprocessorGraphqlResponseIncludeConfig, CoprocessorHookConfig,
};
use hive_router_internal::expressions::lib::ToVrlValue;
use hive_router_internal::expressions::values::boolean::BooleanOrProgram;
use ntex::http::body::{Body, ResponseBody};
use ntex::http::HeaderMap;
use ntex::web;

use crate::coprocessor::error::CoprocessorError;
use crate::coprocessor::protocol::COPROCESSOR_VERSION;
use crate::coprocessor::runtime::MutableRequestState;
use crate::coprocessor::stage::{
    compile_condition, evaluate_condition, CoprocessorRequest, CoprocessorRequestBody,
    CoprocessorResponseBody, HeaderMapJsonRef, Stage, StageResponsePayload,
};
use crate::plugins::hooks::on_graphql_params::GraphQLParams;

pub struct GraphqlRequestStage {
    condition: Option<BooleanOrProgram>,
    include: CoprocessorGraphqlRequestIncludeConfig,
}

pub struct GraphqlResponseStage {
    condition: Option<BooleanOrProgram>,
    include: CoprocessorGraphqlResponseIncludeConfig,
}

pub struct GraphqlAnalysisStage {
    condition: Option<BooleanOrProgram>,
    include: CoprocessorGraphqlAnalysisIncludeConfig,
}

pub struct GraphqlRequestInput<'a> {
    request: &'a web::HttpRequest,
    request_headers: &'a mut HeaderMap,
    graphql_params: &'a mut GraphQLParams,
    sdl: Option<&'a str>,
}

pub struct GraphqlResponseInput<'a> {
    pub(crate) response: web::HttpResponse,
    request: &'a web::HttpRequest,
    sdl: Option<&'a str>,
}

pub struct GraphqlAnalysisInput<'a> {
    request: MutableRequestState<'a>,
    graphql_params: &'a GraphQLParams,
    sdl: Option<&'a str>,
}

impl GraphqlRequestStage {
    pub fn from_config(
        config: &CoprocessorHookConfig<CoprocessorGraphqlRequestIncludeConfig>,
    ) -> Result<Self, CoprocessorError> {
        Ok(Self {
            condition: compile_condition(config.condition.as_ref())?,
            include: config.include.clone(),
        })
    }

    pub fn include_sdl(&self) -> bool {
        self.include.sdl
    }
}

impl GraphqlResponseStage {
    pub fn from_config(
        config: &CoprocessorHookConfig<CoprocessorGraphqlResponseIncludeConfig>,
    ) -> Result<Self, CoprocessorError> {
        Ok(Self {
            condition: compile_condition(config.condition.as_ref())?,
            include: config.include.clone(),
        })
    }

    pub fn include_sdl(&self) -> bool {
        self.include.sdl
    }
}

impl GraphqlAnalysisStage {
    pub fn from_config(
        config: &CoprocessorHookConfig<CoprocessorGraphqlAnalysisIncludeConfig>,
    ) -> Result<Self, CoprocessorError> {
        Ok(Self {
            condition: compile_condition(config.condition.as_ref())?,
            include: config.include.clone(),
        })
    }

    pub fn include_sdl(&self) -> bool {
        self.include.sdl
    }
}

impl<'a> GraphqlRequestInput<'a> {
    pub fn new(
        request: &'a web::HttpRequest,
        request_headers: &'a mut HeaderMap,
        graphql_params: &'a mut GraphQLParams,
        sdl: Option<&'a str>,
    ) -> Self {
        Self {
            request,
            request_headers,
            graphql_params,
            sdl,
        }
    }
}

impl<'a> GraphqlAnalysisInput<'a> {
    pub fn new(
        request: MutableRequestState<'a>,
        graphql_params: &'a GraphQLParams,
        sdl: Option<&'a str>,
    ) -> Self {
        Self {
            request,
            graphql_params,
            sdl,
        }
    }
}

impl<'a> GraphqlResponseInput<'a> {
    pub fn new(
        response: web::HttpResponse,
        request: &'a web::HttpRequest,
        sdl: Option<&'a str>,
    ) -> Self {
        Self {
            response,
            request,
            sdl,
        }
    }
}

impl Stage for GraphqlRequestStage {
    type Input<'a> = GraphqlRequestInput<'a>;

    const STAGE_NAME: &'static str = "graphql.request";

    fn should_run(&self, input: &Self::Input<'_>) -> Result<bool, CoprocessorError> {
        evaluate_condition(self.condition.as_ref(), |hints| {
            hints.context_builder(|root| {
                root.insert_object("request", |req| {
                    req.insert_lazy("method", || input.request.method().as_str().into())
                        .insert_lazy("headers", || input.request.headers().to_vrl_value())
                        .insert_lazy("url", || input.request.uri().to_vrl_value())
                        .insert_object("operation", |op| {
                            op.insert_lazy("name", || {
                                input.graphql_params.operation_name.clone().into()
                            })
                            .insert_lazy("query", || input.graphql_params.query.clone().into());
                        });
                });
            })
        })
    }

    fn build_request<'a>(
        &self,
        input: &Self::Input<'_>,
        id: &'a str,
    ) -> Result<CoprocessorRequest<'a>, CoprocessorError> {
        let payload = GraphqlRequestPayload {
            version: COPROCESSOR_VERSION,
            stage: Self::STAGE_NAME,
            id,
            method: self
                .include
                .method
                .then(|| input.request.method().as_str().into()),
            path: self.include.path.then(|| input.request.path().into()),
            headers: self
                .include
                .headers
                .then_some(HeaderMapJsonRef(input.request_headers)),
            body: build_graphql_body_payload_ref(self.include.body, input.graphql_params),
            context: self.include.context.then_some(sonic_rs::Value::new()),
            sdl: self.include.sdl.then_some(input.sdl).flatten(),
        };

        Ok(CoprocessorRequest {
            id,
            body: sonic_rs::to_vec(&payload)?.into(),
        })
    }

    fn apply_mutations<'b>(
        &self,
        parsed: StageResponsePayload<'b>,
        input: &mut Self::Input<'_>,
    ) -> Result<(), CoprocessorError> {
        // GraphQL request stage mutates headers and body. Method/path stay read-only.
        if let Some(headers) = parsed.headers {
            headers.replace_into(input.request_headers)?;
        }

        if let Some(body) = parsed.body {
            let fields: GraphqlBodyPayload = sonic_rs::from_str(&Self::parse_json_body(&body)?)?;
            fields.apply_to(input.graphql_params, Self::STAGE_NAME)?;
            return Ok(());
        }

        Ok(())
    }
}

impl Stage for GraphqlAnalysisStage {
    type Input<'a> = GraphqlAnalysisInput<'a>;

    const STAGE_NAME: &'static str = "graphql.analysis";

    fn should_run(&self, input: &Self::Input<'_>) -> Result<bool, CoprocessorError> {
        evaluate_condition(self.condition.as_ref(), |hints| {
            hints.context_builder(|root| {
                root.insert_object("request", |req| {
                    req.insert_lazy("method", || input.request.method.as_str().into())
                        .insert_lazy("headers", || input.request.headers.to_vrl_value())
                        .insert_lazy("url", || input.request.uri.to_vrl_value())
                        .insert_object("operation", |op| {
                            op.insert_lazy("name", || {
                                input.graphql_params.operation_name.clone().into()
                            })
                            .insert_lazy("query", || input.graphql_params.query.clone().into());
                        });
                });
            })
        })
    }

    fn build_request<'a>(
        &self,
        input: &Self::Input<'_>,
        id: &'a str,
    ) -> Result<CoprocessorRequest<'a>, CoprocessorError> {
        let payload = GraphqlAnalysisPayload {
            version: COPROCESSOR_VERSION,
            stage: Self::STAGE_NAME,
            id,
            method: self
                .include
                .method
                .then(|| input.request.method.as_str().into()),
            path: self.include.path.then(|| input.request.uri.path().into()),
            headers: self
                .include
                .headers
                .then_some(HeaderMapJsonRef(input.request.headers)),
            body: build_graphql_body_payload_ref(self.include.body, input.graphql_params),
            context: self.include.context.then_some(sonic_rs::Value::new()),
            sdl: self.include.sdl.then_some(input.sdl).flatten(),
        };

        Ok(CoprocessorRequest {
            id,
            body: sonic_rs::to_vec(&payload)?.into(),
        })
    }

    fn apply_mutations<'b>(
        &self,
        parsed: StageResponsePayload<'b>,
        input: &mut Self::Input<'_>,
    ) -> Result<(), CoprocessorError> {
        if let Some(headers) = parsed.headers {
            headers.replace_into(input.request.headers)?;
        }

        if parsed.body.is_some() {
            return Err(CoprocessorError::ForbiddenStageMutation {
                stage: Self::STAGE_NAME,
                field: "body",
            });
        }

        Ok(())
    }
}

impl Stage for GraphqlResponseStage {
    type Input<'a> = GraphqlResponseInput<'a>;

    const STAGE_NAME: &'static str = "graphql.response";

    fn should_run(&self, input: &Self::Input<'_>) -> Result<bool, CoprocessorError> {
        evaluate_condition(self.condition.as_ref(), |hints| {
            hints.context_builder(|root| {
                root.insert_object("request", |req| {
                    req.insert_lazy("method", || input.request.method().as_str().into())
                        .insert_lazy("headers", || input.request.headers().to_vrl_value())
                        .insert_lazy("url", || input.request.uri().to_vrl_value());
                });
                root.insert_object("response", |res| {
                    res.insert_lazy("headers", || input.response.headers().to_vrl_value())
                        .insert_lazy("status_code", || input.response.status().as_u16().into());
                });
            })
        })
    }

    fn build_request<'a>(
        &self,
        input: &Self::Input<'_>,
        id: &'a str,
    ) -> Result<CoprocessorRequest<'a>, CoprocessorError> {
        let body = if self.include.body {
            match input.response.body() {
                ResponseBody::Body(Body::Bytes(bytes)) => Some(
                    CoprocessorRequestBody::from(bytes)
                        .try_to_utf8(Self::STAGE_NAME)?
                        .into(),
                ),
                _ => None,
            }
        } else {
            None
        };

        let payload = GraphqlResponsePayload {
            version: COPROCESSOR_VERSION,
            stage: Self::STAGE_NAME,
            id,
            headers: self
                .include
                .headers
                .then(|| HeaderMapJsonRef(input.response.headers())),
            body,
            context: self.include.context.then_some(sonic_rs::Value::new()),
            status_code: self
                .include
                .status_code
                .then_some(input.response.status().as_u16()),
            sdl: self.include.sdl.then_some(input.sdl).flatten(),
        };

        Ok(CoprocessorRequest {
            id,
            body: sonic_rs::to_vec(&payload)?.into(),
        })
    }

    fn apply_mutations<'b>(
        &self,
        parsed: StageResponsePayload<'b>,
        input: &mut Self::Input<'_>,
    ) -> Result<(), CoprocessorError> {
        // GraphQL response stage mutates headers and body only. The status property is controlled by break.
        if let Some(headers) = parsed.headers {
            headers.replace_into(input.response.headers_mut())?;
        }

        if let Some(body) = parsed.body {
            let body = Self::parse_json_body(&body)?;
            let new_body = ntex::http::body::Body::Bytes(
                CoprocessorResponseBody::from(body).into_ntex_bytes(),
            );
            input.response = std::mem::replace(
                &mut input.response,
                web::HttpResponse::new(ntex::http::StatusCode::OK),
            )
            .set_body(new_body);
        }

        Ok(())
    }
}

#[derive(serde::Serialize)]
struct GraphqlRequestPayload<'a> {
    version: u8,
    stage: &'static str,
    id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    method: Option<Cow<'a, str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<Cow<'a, str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    headers: Option<HeaderMapJsonRef<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<GraphqlBodyPayloadRef<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<sonic_rs::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sdl: Option<&'a str>,
}

#[derive(serde::Serialize)]
struct GraphqlResponsePayload<'a> {
    version: u8,
    stage: &'static str,
    id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    headers: Option<HeaderMapJsonRef<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<Cow<'a, str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<sonic_rs::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status_code: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sdl: Option<&'a str>,
}

#[derive(serde::Serialize)]
struct GraphqlAnalysisPayload<'a> {
    version: u8,
    stage: &'static str,
    id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    method: Option<Cow<'a, str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<Cow<'a, str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    headers: Option<HeaderMapJsonRef<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<GraphqlBodyPayloadRef<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<sonic_rs::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sdl: Option<&'a str>,
}

impl GraphqlBodyPayload {
    fn apply_to(
        self,
        target: &mut GraphQLParams,
        stage_name: &'static str,
    ) -> Result<(), CoprocessorError> {
        if self
            .query
            .as_ref()
            .is_some_and(|query| query.trim().is_empty())
        {
            return Err(CoprocessorError::InvalidStageBody {
                stage: stage_name,
                expected: "'query' must be a non-empty string",
                reason: "query is empty".to_string(),
            });
        }

        if let Some(query) = self.query {
            target.query = Some(query);
        }

        if let Some(operation_name) = self.operation_name {
            target.operation_name = Some(operation_name);
        }

        if let Some(variables) = self.variables {
            target.variables = variables;
        }

        if let Some(extensions) = self.extensions {
            target.extensions = Some(extensions);
        }

        Ok(())
    }
}

#[derive(serde::Serialize)]
struct GraphqlBodyPayloadRef<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    query: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    operation_name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    variables: Option<&'a HashMap<String, sonic_rs::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extensions: Option<&'a HashMap<String, sonic_rs::Value>>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphqlBodyPayload {
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    operation_name: Option<String>,
    #[serde(default)]
    variables: Option<HashMap<String, sonic_rs::Value>>,
    #[serde(default)]
    extensions: Option<HashMap<String, sonic_rs::Value>>,
}

fn build_graphql_body_payload_ref<'a>(
    selection: hive_router_config::coprocessor::GraphqlBodySelection,
    graphql_params: &'a GraphQLParams,
) -> Option<GraphqlBodyPayloadRef<'a>> {
    if selection.is_empty() {
        return None;
    }

    Some(GraphqlBodyPayloadRef {
        query: selection
            .query
            .then_some(graphql_params.query.as_deref())
            .flatten(),
        operation_name: selection
            .operation_name
            .then_some(graphql_params.operation_name.as_deref())
            .flatten(),
        variables: selection.variables.then_some(&graphql_params.variables),
        extensions: selection
            .extensions
            .then_some(graphql_params.extensions.as_ref())
            .flatten(),
    })
}
