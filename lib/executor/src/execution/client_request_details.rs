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

pub struct MutableClientRequestDetails<'exec> {
    pub method: &'exec Method,
    pub url: &'exec http::Uri,
    pub headers: NtexHeaderMap,
    pub operation: OperationDetails<'exec>,
    pub jwt: Arc<JwtRequestDetails>,
}

pub struct ClientRequestDetails<'exec> {
    pub method: &'exec Method,
    pub url: &'exec http::Uri,
    pub headers: Arc<NtexHeaderMap>,
    pub operation: OperationDetails<'exec>,
    pub jwt: Arc<JwtRequestDetails>,
}

// Trait for accessing read-only client request details.
// It's created to do not leak the mutable implementation details.
pub trait ClientRequestDetailsView {
    fn method(&self) -> &Method;
    fn url(&self) -> &http::Uri;
    fn headers(&self) -> &NtexHeaderMap;
    fn operation<'a>(&'a self) -> &'a OperationDetails<'a>;
    fn jwt(&self) -> &JwtRequestDetails;

    fn to_vrl_value(&self) -> Value {
        request_details_to_vrl_value(self)
    }
}

impl ClientRequestDetailsView for MutableClientRequestDetails<'_> {
    fn method(&self) -> &Method {
        self.method
    }

    fn url(&self) -> &http::Uri {
        self.url
    }

    fn headers(&self) -> &NtexHeaderMap {
        &self.headers
    }

    fn operation<'a>(&'a self) -> &'a OperationDetails<'a> {
        &self.operation
    }

    fn jwt(&self) -> &JwtRequestDetails {
        &self.jwt
    }
}

impl ClientRequestDetailsView for ClientRequestDetails<'_> {
    fn method(&self) -> &Method {
        self.method
    }

    fn url(&self) -> &http::Uri {
        self.url
    }

    fn headers(&self) -> &NtexHeaderMap {
        &self.headers
    }

    fn operation<'a>(&'a self) -> &'a OperationDetails<'a> {
        &self.operation
    }

    fn jwt(&self) -> &JwtRequestDetails {
        &self.jwt
    }
}

impl<'exec> MutableClientRequestDetails<'exec> {
    pub fn freeze(self) -> ClientRequestDetails<'exec> {
        ClientRequestDetails {
            method: self.method,
            url: self.url,
            headers: self.headers.into(),
            operation: self.operation,
            jwt: self.jwt,
        }
    }
}

impl From<&MutableClientRequestDetails<'_>> for Value {
    fn from(details: &MutableClientRequestDetails) -> Self {
        request_details_to_vrl_value(details)
    }
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
        request_details_to_vrl_value(details)
    }
}

fn request_details_to_vrl_value(details: &(impl ClientRequestDetailsView + ?Sized)) -> Value {
    // .request.headers
    let headers_value = details.headers().to_vrl_value();

    // .request.url
    let url_value = details.url().to_vrl_value();

    // .request.operation
    let operation_value = Value::Object(BTreeMap::from([
        ("name".into(), details.operation().name.into()),
        ("type".into(), details.operation().kind.into()),
        ("query".into(), details.operation().query.into()),
    ]));

    // .request.jwt
    let jwt_value = match details.jwt() {
        JwtRequestDetails::Authenticated {
            token,
            prefix,
            claims,
            scopes,
        } => Value::Object(BTreeMap::from([
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
        JwtRequestDetails::Unauthenticated => Value::Object(BTreeMap::from([
            ("authenticated".into(), Value::Boolean(false)),
            ("token".into(), Value::Null),
            ("prefix".into(), Value::Null),
            ("claims".into(), Value::Object(BTreeMap::new())),
            ("scopes".into(), Value::Array(vec![])),
        ])),
    };

    Value::Object(BTreeMap::from([
        ("method".into(), details.method().as_str().into()),
        ("headers".into(), headers_value),
        ("url".into(), url_value),
        ("operation".into(), operation_value),
        ("jwt".into(), jwt_value),
    ]))
}
