use std::borrow::Cow;

use futures::stream;
use hive_router_config::coprocessor::{
    CoprocessorHookConfig, CoprocessorRouterRequestIncludeConfig,
    CoprocessorRouterResponseIncludeConfig,
};
use hive_router_internal::expressions::{lib::ToVrlValue, values::boolean::BooleanOrProgram};
use ntex::http::{
    body::{Body, ResponseBody},
    error::PayloadError,
    Payload, Response, StatusCode,
};
use ntex::util::Bytes as NtexBytes;
use ntex::web::{self, DefaultError};

use crate::coprocessor::error::CoprocessorError;
use crate::coprocessor::protocol::COPROCESSOR_VERSION;
use crate::coprocessor::stage::{
    compile_condition, evaluate_condition, CoprocessorRequest, CoprocessorRequestBody,
    CoprocessorResponseBody, HeaderMapJsonRef, Stage, StageResponsePayload,
};

pub struct RouterRequestStage {
    condition: Option<BooleanOrProgram>,
    include: CoprocessorRouterRequestIncludeConfig,
}

pub struct RouterResponseStage {
    condition: Option<BooleanOrProgram>,
    include: CoprocessorRouterResponseIncludeConfig,
}

pub struct RouterRequestInput {
    pub(crate) request: web::WebRequest<DefaultError>,
    request_body: Option<NtexBytes>,
    /// This tells runtime whether it should restore original body bytes after stage execution
    body_replaced: bool,
}

pub struct RouterResponseInput {
    pub(crate) response: web::WebResponse,
}

impl RouterRequestStage {
    pub fn from_config(
        config: &CoprocessorHookConfig<CoprocessorRouterRequestIncludeConfig>,
    ) -> Result<Self, CoprocessorError> {
        Ok(Self {
            condition: compile_condition(config.condition.as_ref())?,
            include: config.include.clone(),
        })
    }

    pub fn include_body(&self) -> bool {
        self.include.body
    }
}

impl RouterResponseStage {
    pub fn from_config(
        config: &CoprocessorHookConfig<CoprocessorRouterResponseIncludeConfig>,
    ) -> Result<Self, CoprocessorError> {
        Ok(Self {
            condition: compile_condition(config.condition.as_ref())?,
            include: config.include.clone(),
        })
    }
}

impl RouterRequestInput {
    pub fn new(request: web::WebRequest<DefaultError>, request_body: Option<NtexBytes>) -> Self {
        Self {
            request,
            request_body,
            body_replaced: false,
        }
    }

    pub(crate) fn restore_request_body_if_unchanged(&mut self) {
        // If coprocessor replaced body, restoring old bytes would be wasted and wrong
        if self.body_replaced {
            return;
        }

        if let Some(body) = self.request_body.as_ref() {
            // We restore payload from saved bytes so downstream handlers can still read the body.
            // TODO: avoid rebuilding payload stream when we can preserve original payload state.
            self.request.set_payload(payload_from_bytes(body.clone()));
        }
    }
}

impl RouterResponseInput {
    pub fn new(response: web::WebResponse) -> Self {
        Self { response }
    }
}

impl Stage for RouterRequestStage {
    type Input<'a> = RouterRequestInput;

    const STAGE_NAME: &'static str = "router.request";

    fn should_run(&self, input: &Self::Input<'_>) -> Result<bool, CoprocessorError> {
        evaluate_condition(self.condition.as_ref(), |hints| {
            hints.context_builder(|root| {
                root.insert_object("request", |req| {
                    req.insert_lazy("method", || input.request.method().as_str().into())
                        .insert_lazy("headers", || input.request.headers().to_vrl_value())
                        .insert_lazy("url", || input.request.uri().to_vrl_value());
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
            Some(
                CoprocessorRequestBody::from(input.request_body.as_deref().unwrap_or_default())
                    .try_to_utf8(Self::STAGE_NAME)?
                    .into(),
            )
        } else {
            None
        };

        let payload = RouterRequestPayload {
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
                .then(|| HeaderMapJsonRef(input.request.headers())),
            body,
            context: self.include.context.then_some(sonic_rs::Value::new()),
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
        // Router request stage can mutate request headers and body properties before downstream processing
        if let Some(headers) = parsed.headers {
            headers.replace_into(&mut input.request.head_mut().headers)?;
        }

        if let Some(body) = parsed.body {
            input.body_replaced = true;
            let body_str = Self::parse_json_body(&body)?;
            if body_str.is_empty() {
                input.request.set_payload(Payload::None);
            } else {
                input.request.set_payload(payload_from_bytes(
                    CoprocessorResponseBody::from(body_str).into_ntex_bytes(),
                ));
            }
        }

        Ok(())
    }
}

impl Stage for RouterResponseStage {
    type Input<'a> = RouterResponseInput;

    const STAGE_NAME: &'static str = "router.response";

    fn should_run(&self, input: &Self::Input<'_>) -> Result<bool, CoprocessorError> {
        evaluate_condition(self.condition.as_ref(), |hints| {
            hints.context_builder(|root| {
                root.insert_object("request", |req| {
                    req.insert_lazy("method", || {
                        input.response.request().method().as_str().into()
                    })
                    .insert_lazy("headers", || {
                        input.response.request().headers().to_vrl_value()
                    })
                    .insert_lazy("url", || input.response.request().uri().to_vrl_value());
                })
                .insert_object("response", |res| {
                    res.insert_lazy("headers", || input.response.headers().to_vrl_value());
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
            match input.response.response().body() {
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

        let payload = RouterResponsePayload {
            version: COPROCESSOR_VERSION,
            stage: Self::STAGE_NAME,
            id,
            headers: self
                .include
                .headers
                .then(|| HeaderMapJsonRef(input.response.response().headers())),
            body,
            context: self.include.context.then_some(sonic_rs::Value::new()),
            status_code: self
                .include
                .status_code
                .then_some(input.response.response().status().as_u16()),
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
        // Router response stage can mutate outgoing headers and body before plugin::on_end run
        if let Some(headers) = parsed.headers {
            headers.replace_into(input.response.response_mut().headers_mut())?;
        }

        if let Some(body) = parsed.body {
            let previous_response =
                std::mem::replace(input.response.response_mut(), Response::new(StatusCode::OK));

            let body_str = Self::parse_json_body(&body)?;
            if body_str.is_empty() {
                *input.response.response_mut() = previous_response.set_body(Body::None);
            } else {
                let body_bytes = CoprocessorResponseBody::from(body_str).into_ntex_bytes();
                *input.response.response_mut() =
                    previous_response.set_body(Body::Bytes(body_bytes));
            }
        }

        Ok(())
    }
}

fn payload_from_bytes(body: NtexBytes) -> Payload {
    if body.is_empty() {
        return Payload::None;
    }

    Payload::from_stream(stream::iter([Ok::<NtexBytes, PayloadError>(body)]))
}

#[derive(serde::Serialize)]
struct RouterRequestPayload<'a> {
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
    body: Option<Cow<'a, str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<sonic_rs::Value>,
}

#[derive(serde::Serialize)]
struct RouterResponsePayload<'a> {
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
}
