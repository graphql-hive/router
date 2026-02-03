/// Common types and messages for the GraphQL over WebSocket Transport Protocol
/// as per the spec: https://github.com/enisdenjo/graphql-ws/blob/master/PROTOCOL.md
use ntex::ws;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::error;

use crate::response::graphql_error::GraphQLError;

pub const WS_SUBPROTOCOL: &str = "graphql-transport-ws";

pub enum CloseCode {
    SubprotocolNotAcceptable,
    ConnectionInitTimeout,
    ConnectionAcknowledgementTimeout,
    TooManyInitialisationRequests,
    Unauthorized,
    Forbidden(String),
    BadRequest(&'static str),
    BadResponse(&'static str),
    SubscriberAlreadyExists(String),
    InternalServerError(Option<String>),
}

impl From<CloseCode> for ws::Message {
    fn from(msg: CloseCode) -> Self {
        match msg {
            CloseCode::SubprotocolNotAcceptable => ws::Message::Close(Some(ws::CloseReason {
                code: ws::CloseCode::from(4406),
                description: Some("Subprotocol not acceptable".into()),
            })),
            CloseCode::ConnectionInitTimeout => ws::Message::Close(Some(ws::CloseReason {
                code: ws::CloseCode::from(4408),
                description: Some("Connection initialisation timeout".into()),
            })),
            CloseCode::ConnectionAcknowledgementTimeout => {
                ws::Message::Close(Some(ws::CloseReason {
                    code: ws::CloseCode::from(4504),
                    description: Some("Connection acknowledgement timeout".into()),
                }))
            }
            CloseCode::TooManyInitialisationRequests => ws::Message::Close(Some(ws::CloseReason {
                code: ws::CloseCode::from(4429),
                description: Some("Too many initialisation requests".into()),
            })),
            CloseCode::Unauthorized => ws::Message::Close(Some(ws::CloseReason {
                code: ntex::ws::CloseCode::from(4401),
                description: Some("Unauthorized".into()),
            })),
            CloseCode::Forbidden(reason) => ws::Message::Close(Some(ws::CloseReason {
                code: ntex::ws::CloseCode::from(4403),
                description: Some(reason),
            })),
            CloseCode::BadRequest(reason) => ws::Message::Close(Some(ws::CloseReason {
                code: ntex::ws::CloseCode::from(4400),
                description: Some(reason.into()),
            })),
            CloseCode::BadResponse(reason) => ws::Message::Close(Some(ws::CloseReason {
                code: ntex::ws::CloseCode::from(4004),
                description: Some(reason.into()),
            })),
            CloseCode::SubscriberAlreadyExists(id) => ws::Message::Close(Some(ws::CloseReason {
                code: ws::CloseCode::from(4409),
                description: Some(format!("Subscriber for {id} already exists")),
            })),
            CloseCode::InternalServerError(reason) => ws::Message::Close(Some(ws::CloseReason {
                code: ntex::ws::CloseCode::from(4500),
                description: reason.or(Some("Internal Server Error".into())),
            })),
        }
    }
}

// Using serde_json::Value instead of sonic_rs::Value because sonic_rs::Value has
// issues with internally-tagged enum deserialization (#[serde(tag = "type")]).
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SubscribePayload {
    pub query: String,
    pub operation_name: Option<String>,
    pub variables: Option<HashMap<String, serde_json::Value>>,
    pub extensions: Option<HashMap<String, serde_json::Value>>,
}

impl SubscribePayload {
    pub fn new(
        query: String,
        operation_name: Option<String>,
        variables: Option<HashMap<String, serde_json::Value>>,
        extensions: Option<HashMap<String, serde_json::Value>>,
    ) -> Self {
        Self {
            query,
            operation_name,
            variables,
            extensions,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    ConnectionInit {
        payload: Option<ConnectionInitPayload>,
    },
    Ping {},
    Pong {},
    Subscribe {
        id: String,
        payload: SubscribePayload,
    },
    Complete {
        id: String,
    },
}

impl ClientMessage {
    pub fn init(payload: Option<ConnectionInitPayload>) -> ws::Message {
        ClientMessage::ConnectionInit { payload }.into()
    }

    pub fn ping() -> ws::Message {
        ServerMessage::Ping {}.into()
    }

    pub fn pong() -> ws::Message {
        ServerMessage::Pong {}.into()
    }

    pub fn subscribe(id: String, payload: SubscribePayload) -> ws::Message {
        ClientMessage::Subscribe { id, payload }.into()
    }

    pub fn complete(id: String) -> ws::Message {
        ClientMessage::Complete { id }.into()
    }
}

impl From<ClientMessage> for ws::Message {
    fn from(msg: ClientMessage) -> Self {
        match sonic_rs::to_string(&msg) {
            Ok(text) => ws::Message::Text(text.into()),
            Err(e) => {
                error!("Failed to serialize client message to JSON: {}", e);
                CloseCode::InternalServerError(None).into()
            }
        }
    }
}

/// The connection init message payload MUST be a map of string to arbitrary JSON
/// values as per the spec. We represent this as a HashMap<String, Value> and use
/// serde(flatten) to capture all fields for easier parsing to headers later.
//
// Using serde_json::Value instead of sonic_rs::Value because sonic_rs::Value has
// issues with internally-tagged enum deserialization (#[serde(tag = "type")]).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ConnectionInitPayload {
    #[serde(flatten)]
    pub fields: HashMap<String, serde_json::Value>,
}

impl ConnectionInitPayload {
    pub fn new(fields: HashMap<String, serde_json::Value>) -> Self {
        Self { fields }
    }
}

impl From<http::HeaderMap> for ConnectionInitPayload {
    fn from(headers: http::HeaderMap) -> Self {
        let fields: HashMap<String, serde_json::Value> = headers
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|v| (name.to_string(), serde_json::Value::from(v)))
            })
            .collect();
        Self::new(fields)
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    ConnectionAck {
        // NOTE: as per spec there is a "payload" field here, but we don't use it
    },
    Ping {},
    Pong {},
    Next {
        id: String,
        // using serde_json::Value instead of sonic_rs due to compatibility issues
        // with internally-tagged enum deserialization (#[serde(tag = "type")])
        payload: serde_json::Value,
    },
    Error {
        id: String,
        payload: Vec<GraphQLError>,
    },
    Complete {
        id: String,
    },
}

impl ServerMessage {
    pub fn ack() -> ws::Message {
        ServerMessage::ConnectionAck {}.into()
    }

    pub fn ping() -> ws::Message {
        ServerMessage::Ping {}.into()
    }

    pub fn pong() -> ws::Message {
        ServerMessage::Pong {}.into()
    }

    pub fn next(id: &str, body: &[u8]) -> ws::Message {
        let payload = match sonic_rs::from_slice(body) {
            Ok(value) => value,
            Err(err) => {
                error!("Failed to serialize plan execution output body: {}", err);
                return CloseCode::InternalServerError(None).into();
            }
        };
        ServerMessage::Next {
            id: id.to_string(),
            payload,
        }
        .into()
    }

    pub fn error(id: &str, errors: &[GraphQLError]) -> ws::Message {
        ServerMessage::Error {
            id: id.to_string(),
            payload: errors.to_vec(),
        }
        .into()
    }

    pub fn complete(id: &str) -> ws::Message {
        ServerMessage::Complete { id: id.to_string() }.into()
    }
}

impl From<ServerMessage> for ws::Message {
    fn from(msg: ServerMessage) -> Self {
        match sonic_rs::to_string(&msg) {
            Ok(text) => ws::Message::Text(text.into()),
            Err(e) => {
                error!("Failed to serialize server message to JSON: {}", e);
                CloseCode::InternalServerError(None).into()
            }
        }
    }
}

/// A utility function that helps convert serde_json::Value to sonic_rs::Value.
// We need this because we use serde_json::Value in some places due to
// compatibility issues with internally-tagged enum deserialization (#[serde(tag = "type")]).
pub fn serde_to_sonic(value: serde_json::Value) -> sonic_rs::Value {
    match value {
        serde_json::Value::Null => sonic_rs::Value::new(),
        serde_json::Value::Bool(b) => sonic_rs::Value::from(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                sonic_rs::Value::from(i)
            } else if let Some(u) = n.as_u64() {
                sonic_rs::Value::from(u)
            } else if let Some(f) = n.as_f64() {
                sonic_rs::Value::new_f64(f).unwrap_or_else(|| sonic_rs::Value::new())
            } else {
                sonic_rs::Value::new()
            }
        }
        serde_json::Value::String(s) => sonic_rs::Value::from(s.as_str()),
        serde_json::Value::Array(arr) => {
            sonic_rs::Value::from(arr.into_iter().map(serde_to_sonic).collect::<Vec<_>>())
        }
        serde_json::Value::Object(obj) => {
            let mut sonic_obj = sonic_rs::Object::new();
            for (k, v) in obj {
                sonic_obj.insert(&k, serde_to_sonic(v));
            }
            sonic_rs::Value::from(sonic_obj)
        }
    }
}

/// A utility function that helps convert sonic_rs::Value to serde_json::Value.
// We need this because we use serde_json::Value in some places due to
// compatibility issues with internally-tagged enum deserialization (#[serde(tag = "type")]).
pub fn sonic_to_serde(value: sonic_rs::Value) -> serde_json::Value {
    use sonic_rs::{JsonContainerTrait, JsonValueTrait};

    if value.is_null() {
        serde_json::Value::Null
    } else if let Some(b) = value.as_bool() {
        serde_json::Value::Bool(b)
    } else if let Some(s) = value.as_str() {
        serde_json::Value::String(s.to_string())
    } else if let Some(i) = value.as_i64() {
        serde_json::Value::Number(serde_json::Number::from(i))
    } else if let Some(u) = value.as_u64() {
        serde_json::Value::Number(serde_json::Number::from(u))
    } else if let Some(f) = value.as_f64() {
        serde_json::Number::from_f64(f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null)
    } else if let Some(arr) = value.as_array() {
        serde_json::Value::Array(arr.iter().map(|v| sonic_to_serde(v.clone())).collect())
    } else if let Some(obj) = value.as_object() {
        let mut serde_obj = serde_json::Map::new();
        for (k, v) in obj.iter() {
            serde_obj.insert(k.to_string(), sonic_to_serde(v.clone()));
        }
        serde_json::Value::Object(serde_obj)
    } else {
        serde_json::Value::Null
    }
}
