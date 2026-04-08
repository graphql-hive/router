use bytes::Bytes;
use hive_router_config::headers::HOP_BY_HOP_HEADERS;
use hive_router_config::primitives::value_or_expression::ValueOrExpression;
use hive_router_internal::expressions::values::boolean::BooleanOrProgram;
use hive_router_internal::expressions::vrl::core::Value as VrlValue;
use hive_router_internal::expressions::{CompileExpression, ProgramHints, ValueOrProgram};
use ntex::http::header::{HeaderName, HeaderValue};
use ntex::http::HeaderMap;
use ntex::util::Bytes as NtexBytes;
use ntex::web;
use serde::Deserialize;
use serde::{ser::SerializeMap, Serialize, Serializer};
use sonic_rs::LazyValue;
use std::borrow::Cow;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::ops::ControlFlow;
use std::str::FromStr;

use crate::coprocessor::error::CoprocessorError;
use crate::coprocessor::protocol::{CoprocessorControl, COPROCESSOR_VERSION};

/// Outbound HTTP request sent to coprocessor service.
pub struct CoprocessorRequest<'a> {
    pub id: &'a str,
    pub body: Bytes,
}

pub trait Stage {
    /// Input data for the stage.
    type Input<'a>;

    const STAGE_NAME: &'static str;

    fn stage_name(&self) -> &'static str {
        Self::STAGE_NAME
    }

    /// We accept both stringified JSON (wrapped in quotes) and raw JSON
    fn parse_json_body<'a>(body: &'a LazyValue<'a>) -> Result<Cow<'a, str>, CoprocessorError> {
        let raw = body.as_raw_str();
        if raw.as_bytes().first() == Some(&b'"') {
            // JSON string token -> decode/unescape and use decoded text
            Ok(sonic_rs::from_str(raw)?)
        } else {
            // Non-string JSON token (object/array/number/bool/null) -> use raw JSON text
            Ok(Cow::Borrowed(raw))
        }
    }

    fn should_run(&self, input: &Self::Input<'_>) -> Result<bool, CoprocessorError>;

    fn build_request<'a>(
        &self,
        input: &Self::Input<'_>,
        id: &'a str,
    ) -> Result<CoprocessorRequest<'a>, CoprocessorError>;

    fn parse_response<'a>(
        &self,
        body: &'a Bytes,
    ) -> Result<StageResponsePayload<'a>, CoprocessorError> {
        let payload: StageResponsePayload<'a> = sonic_rs::from_slice(body)?;
        // Version check guarantees both sides speak the same protocol contract.
        if payload.version != COPROCESSOR_VERSION {
            return Err(CoprocessorError::UnsupportedVersion(payload.version));
        }

        Ok(payload)
    }

    fn break_output<'b>(
        &self,
        parsed: StageResponsePayload<'b>,
    ) -> Result<ControlFlow<web::HttpResponse, StageResponsePayload<'b>>, CoprocessorError> {
        // No break control means normal flow continues and mutations can be applied.
        let Some(status) = parsed.control.break_status() else {
            return Ok(ControlFlow::Continue(parsed));
        };

        let mut response = web::HttpResponse::Ok();
        response.status(status);

        if let Some(headers) = parsed.headers.as_ref() {
            headers.apply_to_response_builder(&mut response)?;
        }

        let response = if let Some(body) = parsed.body {
            let body = Self::parse_json_body(&body)?;
            response.body(CoprocessorResponseBody::from(body).into_ntex_bytes())
        } else {
            response.finish()
        };

        // Break short-circuits the pipeline with an immediate HTTP response.
        Ok(ControlFlow::Break(response))
    }

    fn apply_mutations<'b>(
        &self,
        parsed: StageResponsePayload<'b>,
        input: &mut Self::Input<'_>,
    ) -> Result<(), CoprocessorError>;
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum OneOrMore<'a> {
    One(#[serde(borrow)] Cow<'a, str>),
    More(#[serde(borrow)] Vec<Cow<'a, str>>),
}

#[derive(Deserialize)]
#[serde(transparent, bound(deserialize = "'de: 'a"))]
pub(crate) struct StageResponseHeaders<'a>(#[serde(borrow)] HashMap<Cow<'a, str>, OneOrMore<'a>>);

impl<'a> StageResponseHeaders<'a> {
    pub(crate) fn replace_into(&self, headers_mut: &mut HeaderMap) -> Result<(), CoprocessorError> {
        // Replace all the headers except for hop-by-hop headers
        let keys_to_remove: Vec<_> = headers_mut
            .keys()
            .filter(|name| !HOP_BY_HOP_HEADERS.contains(&name.as_str()))
            .cloned()
            .collect();

        for key in keys_to_remove {
            headers_mut.remove(&key);
        }

        self.try_for_each_parsed(|name, value| {
            headers_mut.append(name.clone(), value);
        })
    }

    pub(crate) fn apply_to_response_builder(
        &self,
        response: &mut web::HttpResponseBuilder,
    ) -> Result<(), CoprocessorError> {
        self.try_for_each_parsed(|name, value| {
            response.set_header(name.clone(), value);
        })
    }

    fn try_for_each_parsed<F>(&self, mut f: F) -> Result<(), CoprocessorError>
    where
        F: FnMut(&HeaderName, HeaderValue),
    {
        for (name, values) in &self.0 {
            let header_name = HeaderName::from_str(name.as_ref())
                .map_err(|error| CoprocessorError::InvalidHeaderName(error.to_string()))?;

            if HOP_BY_HOP_HEADERS.contains(&header_name.as_str()) {
                continue;
            }

            match values {
                OneOrMore::One(value) => {
                    let header_value = parse_header_value(value)?;
                    f(&header_name, header_value);
                }
                OneOrMore::More(values) => {
                    for value in values {
                        let header_value = parse_header_value(value)?;
                        f(&header_name, header_value);
                    }
                }
            }
        }

        Ok(())
    }
}

#[derive(Deserialize)]
pub struct StageResponsePayload<'a> {
    /// Coprocessor protocol version. Must match router runtime version.
    pub(crate) version: u8,
    /// Continue/break decision for stage execution.
    pub(crate) control: CoprocessorControl,
    #[serde(borrow)]
    pub(crate) headers: Option<StageResponseHeaders<'a>>,
    #[serde(borrow)]
    pub(crate) body: Option<LazyValue<'a>>,
}

pub fn compile_condition(
    condition: Option<&ValueOrExpression<bool>>,
) -> Result<Option<BooleanOrProgram>, CoprocessorError> {
    match condition {
        Some(ValueOrExpression::Value(value)) => Ok(Some(ValueOrProgram::Value(*value))),
        Some(ValueOrExpression::Expression { expression }) => {
            let program = expression.compile_expression(None)?;
            let hints = ProgramHints::from_program(&program);
            Ok(Some(ValueOrProgram::Program(Box::new(program), hints)))
        }
        None => Ok(None),
    }
}

pub fn evaluate_condition<F>(
    condition: Option<&BooleanOrProgram>,
    vrl_context_fn: F,
) -> Result<bool, CoprocessorError>
where
    F: FnOnce(&ProgramHints) -> VrlValue,
{
    let Some(condition) = condition else {
        return Ok(true);
    };

    condition
        .resolve_with_hints(vrl_context_fn)
        .map_err(|error| CoprocessorError::ConditionEvaluation(error.to_string()))
}

fn parse_header_value(value: &str) -> Result<HeaderValue, CoprocessorError> {
    HeaderValue::from_str(value)
        .map_err(|error| CoprocessorError::InvalidHeaderValue(error.to_string()))
}

/// Borrowed raw bytes used to build coprocessor request payload bodies.
/// Bytes must be valid UTF-8 before being written into JSON stage payloads.
pub struct CoprocessorRequestBody<'a>(&'a [u8]);

impl<'a> CoprocessorRequestBody<'a> {
    /// Converts raw bytes into UTF-8 text for JSON payload fields.
    pub fn try_to_utf8(self, context: &'static str) -> Result<&'a str, CoprocessorError> {
        std::str::from_utf8(self.0)
            .map_err(|source| CoprocessorError::InvalidUtf8Body { context, source })
    }
}

impl<'a> From<&'a [u8]> for CoprocessorRequestBody<'a> {
    fn from(value: &'a [u8]) -> Self {
        Self(value)
    }
}

impl<'a> From<&'a NtexBytes> for CoprocessorRequestBody<'a> {
    fn from(value: &'a NtexBytes) -> Self {
        Self(value.as_ref())
    }
}

/// Text body received from coprocessor stage responses.
pub struct CoprocessorResponseBody<'a>(Cow<'a, str>);

impl<'a> CoprocessorResponseBody<'a> {
    /// Converts stage text into `ntex::Bytes` for http server APIs.
    pub fn into_ntex_bytes(self) -> NtexBytes {
        match self.0 {
            Cow::Borrowed(value) => NtexBytes::copy_from_slice(value.as_bytes()),
            Cow::Owned(value) => NtexBytes::from(value),
        }
    }
}

impl<'a> From<Cow<'a, str>> for CoprocessorResponseBody<'a> {
    fn from(value: Cow<'a, str>) -> Self {
        Self(value)
    }
}

/// Borrowed view used to serialize ntex headers as protocol JSON.
pub struct HeaderMapJsonRef<'a>(pub &'a HeaderMap);

impl Serialize for HeaderMapJsonRef<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Protocol requires header values as arrays: {"name": ["value1", ...]}
        enum HeaderValues<'a> {
            One(&'a str),
            Many(Vec<&'a str>),
        }

        // Group duplicate header names so multi-value headers are serialized once.
        let mut grouped_headers: HashMap<&str, HeaderValues<'_>> =
            HashMap::with_capacity(self.0.len());
        for (name, value) in self.0.iter() {
            let Ok(value) = value.to_str() else {
                continue;
            };

            match grouped_headers.entry(name.as_str()) {
                Entry::Vacant(entry) => {
                    entry.insert(HeaderValues::One(value));
                }
                Entry::Occupied(mut entry) => match entry.get_mut() {
                    HeaderValues::One(previous) => {
                        let previous = *previous;
                        entry.insert(HeaderValues::Many(vec![previous, value]));
                    }
                    HeaderValues::Many(values) => {
                        values.push(value);
                    }
                },
            }
        }

        let mut map = serializer.serialize_map(Some(grouped_headers.len()))?;
        for (name, values) in grouped_headers {
            match values {
                HeaderValues::One(value) => {
                    map.serialize_entry(name, &[value])?;
                }
                HeaderValues::Many(values) => {
                    map.serialize_entry(name, &values)?;
                }
            }
        }
        map.end()
    }
}
