use std::{collections::BTreeMap, sync::Arc};

use bytes::Bytes;
use hive_router_internal::expressions::{lib::ToVrlValue, vrl::core::Value};
use http::Method;
use ntex::http::HeaderMap as NtexHeaderMap;

use crate::request_context::{RequestContextError, SharedRequestContext};

pub struct OperationDetails<'exec> {
    pub name: Option<&'exec str>,
    pub query: &'exec str,
    pub kind: &'static str,
}

pub struct ClientRequestDetails<'exec> {
    pub method: &'exec Method,
    pub url: &'exec http::Uri,
    pub headers: Arc<NtexHeaderMap>,
    pub operation: OperationDetails<'exec>,
    pub jwt: Arc<JwtRequestDetails>,
}

pub enum JwtRequestDetails {
    Authenticated {
        token: String,
        prefix: Option<String>,
        claims: sonic_rs::Value,
        scopes: Option<Vec<String>>,
    },
    Unauthenticated,
}

impl JwtRequestDetails {
    pub fn update_request_context(
        &self,
        request_context: &SharedRequestContext,
    ) -> Result<(), RequestContextError> {
        request_context.update(|ctx| match self {
            JwtRequestDetails::Authenticated { scopes, .. } => {
                ctx.authentication.jwt_status = Some(true);
                ctx.authentication.jwt_scopes = scopes
                    .as_ref()
                    .map(|scopes| scopes.iter().cloned().collect());
            }
            JwtRequestDetails::Unauthenticated => {
                ctx.authentication.jwt_scopes = None;
                ctx.authentication.jwt_status = Some(false);
            }
        })
    }
}

impl From<&ClientRequestDetails<'_>> for Value {
    fn from(details: &ClientRequestDetails) -> Self {
        // .request.headers
        let headers_value = details.headers.to_vrl_value();

        // .request.url
        let url_value = details.url.to_vrl_value();

        // .request.operation
        let operation_value = Self::Object(BTreeMap::from([
            ("name".into(), details.operation.name.into()),
            ("type".into(), details.operation.kind.into()),
            ("query".into(), details.operation.query.into()),
        ]));

        // .request.jwt
        let jwt_value = match details.jwt.as_ref() {
            JwtRequestDetails::Authenticated {
                token,
                prefix,
                claims,
                scopes,
            } => Self::Object(BTreeMap::from([
                ("authenticated".into(), Value::Boolean(true)),
                ("token".into(), token.to_string().into()),
                (
                    "prefix".into(),
                    prefix.as_deref().unwrap_or_default().into(),
                ),
                ("claims".into(), claims.to_vrl_value()),
                (
                    "scopes".into(),
                    match scopes {
                        Some(scopes) => Value::Array(
                            scopes
                                .iter()
                                .map(|v| Value::Bytes(Bytes::from(v.clone())))
                                .collect(),
                        ),
                        None => Value::Array(vec![]),
                    },
                ),
            ])),
            JwtRequestDetails::Unauthenticated => Self::Object(BTreeMap::from([
                ("authenticated".into(), Value::Boolean(false)),
                ("token".into(), Value::Null),
                ("prefix".into(), Value::Null),
                ("claims".into(), Value::Object(BTreeMap::new())),
                ("scopes".into(), Value::Array(vec![])),
            ])),
        };

        Self::Object(BTreeMap::from([
            ("method".into(), details.method.as_str().into()),
            ("headers".into(), headers_value),
            ("url".into(), url_value),
            ("operation".into(), operation_value),
            ("jwt".into(), jwt_value),
        ]))
    }
}
