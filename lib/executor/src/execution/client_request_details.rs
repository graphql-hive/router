use std::collections::BTreeMap;

use bytes::Bytes;
use hive_router_internal::expressions::{lib::ToVrlValue, vrl::core::Value};
use http::Method;
use ntex::http::HeaderMap as NtexHeaderMap;

pub struct OperationDetails<'exec> {
    pub name: Option<&'exec str>,
    pub query: &'exec str,
    pub kind: &'static str,
}

pub struct ClientRequestDetails<'exec, 'req> {
    pub method: &'req Method,
    pub url: &'req http::Uri,
    pub headers: &'req NtexHeaderMap,
    pub operation: OperationDetails<'exec>,
    pub jwt: &'exec JwtRequestDetails<'req>,
}

pub enum JwtRequestDetails<'exec> {
    Authenticated {
        token: &'exec str,
        prefix: Option<&'exec str>,
        claims: &'exec sonic_rs::Value,
        scopes: Option<Vec<String>>,
    },
    Unauthenticated,
}

impl From<&ClientRequestDetails<'_, '_>> for Value {
    fn from(details: &ClientRequestDetails) -> Self {
        // .request.headers
        let headers_value = client_header_map_to_vrl_value(details.headers);

        // .request.url
        let url_value = Self::Object(BTreeMap::from([
            (
                "host".into(),
                details.url.host().unwrap_or("unknown").into(),
            ),
            ("path".into(), details.url.path().into()),
            (
                "port".into(),
                details
                    .url
                    .port_u16()
                    .unwrap_or_else(|| {
                        if details.url.scheme() == Some(&http::uri::Scheme::HTTPS) {
                            443
                        } else {
                            80
                        }
                    })
                    .into(),
            ),
        ]));

        // .request.operation
        let operation_value = Self::Object(BTreeMap::from([
            ("name".into(), details.operation.name.into()),
            ("type".into(), details.operation.kind.into()),
            ("query".into(), details.operation.query.into()),
        ]));

        // .request.jwt
        let jwt_value = match details.jwt {
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
                    prefix.unwrap_or_default().to_string().into(),
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

fn client_header_map_to_vrl_value(headers: &ntex::http::HeaderMap) -> Value {
    let mut obj = BTreeMap::new();
    for (header_name, header_value) in headers.iter() {
        if let Ok(value) = header_value.to_str() {
            obj.insert(
                header_name.as_str().into(),
                Value::Bytes(Bytes::from(value.to_owned())),
            );
        }
    }
    Value::Object(obj)
}
