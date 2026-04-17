use std::collections::BTreeMap;

use bytes::Bytes;
use hive_router_internal::expressions::{
    lib::{ToVrlValue, VrlObjectBuilder},
    vrl::core::Value,
    VrlView,
};
use http::Method;
use ntex::http::HeaderMap as NtexHeaderMap;

pub struct OperationDetails<'exec> {
    pub name: Option<&'exec str>,
    pub query: &'exec str,
    pub kind: &'static str,
}

pub struct ClientRequestDetails<'exec> {
    pub method: &'exec Method,
    pub url: &'exec http::Uri,
    pub headers: NtexHeaderMap,
    pub operation: OperationDetails<'exec>,
    pub jwt: JwtRequestDetails,
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

pub struct OperationDetailsView<'a> {
    pub source: &'a OperationDetails<'a>,
}

impl<'a> OperationDetailsView<'a> {
    pub fn new(source: &'a OperationDetails<'a>) -> Self {
        Self { source }
    }
}

impl VrlView for OperationDetailsView<'_> {
    fn write<'a>(&self, out: &mut VrlObjectBuilder<'a, '_>) {
        out.insert_lazy("name", || self.source.name.map(str::to_owned).into())
            .insert_lazy("type", || self.source.kind.into())
            .insert_lazy("query", || self.source.query.into());
    }
}

pub struct JwtRequestDetailsView<'a> {
    pub source: &'a JwtRequestDetails,
}

impl<'a> JwtRequestDetailsView<'a> {
    pub fn new(source: &'a JwtRequestDetails) -> Self {
        Self { source }
    }
}

impl VrlView for JwtRequestDetailsView<'_> {
    fn write<'a>(&self, out: &mut VrlObjectBuilder<'a, '_>) {
        match self.source {
            JwtRequestDetails::Authenticated {
                token,
                prefix,
                claims,
                scopes,
            } => {
                out.insert_lazy("authenticated", || Value::Boolean(true))
                    .insert_lazy("token", || token.to_string().into())
                    .insert_lazy("prefix", || prefix.as_deref().unwrap_or_default().into())
                    .insert_lazy("claims", || claims.to_vrl_value())
                    .insert_lazy("scopes", || match scopes {
                        Some(scopes) => Value::Array(
                            scopes
                                .iter()
                                .map(|v| Value::Bytes(Bytes::from(v.clone())))
                                .collect(),
                        ),
                        None => Value::Array(vec![]),
                    });
            }
            JwtRequestDetails::Unauthenticated => {
                out.insert_lazy("authenticated", || Value::Boolean(false))
                    .insert_lazy("token", || Value::Null)
                    .insert_lazy("prefix", || Value::Null)
                    .insert_lazy("claims", || Value::Object(BTreeMap::new()))
                    .insert_lazy("scopes", || Value::Array(vec![]));
            }
        }
    }
}

pub struct ClientRequestView<'a> {
    pub source: &'a ClientRequestDetails<'a>,
}

impl<'a> ClientRequestView<'a> {
    pub fn new(source: &'a ClientRequestDetails<'a>) -> Self {
        Self { source }
    }
}

impl VrlView for ClientRequestView<'_> {
    fn write<'a>(&self, out: &mut VrlObjectBuilder<'a, '_>) {
        out.insert_lazy("method", || self.source.method.as_str().into())
            .insert_lazy("headers", || self.source.headers.to_vrl_value())
            .insert_lazy("url", || self.source.url.to_vrl_value())
            .insert_object("operation", |op| {
                OperationDetailsView::new(&self.source.operation).write(op);
            })
            .insert_object("jwt", |jwt| {
                JwtRequestDetailsView::new(&self.source.jwt).write(jwt);
            });
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
        let jwt_value = match &details.jwt {
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
