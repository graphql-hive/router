/// Common types and messages for the GraphQL over WebSocket Transport Protocol
/// as per the spec: https://github.com/enisdenjo/graphql-ws/blob/master/PROTOCOL.md
use ntex::ws;
use serde::{Deserialize, Serialize};
use sonic_rs::{JsonContainerTrait, JsonValueTrait, Value};
use std::collections::HashMap;
use strum::AsRefStr;
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

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SubscribePayload {
    pub query: String,
    pub operation_name: Option<String>,
    pub variables: Option<HashMap<String, Value>>,
    pub extensions: Option<HashMap<String, Value>>,
}

impl SubscribePayload {
    pub fn new(
        query: String,
        operation_name: Option<String>,
        variables: Option<HashMap<String, Value>>,
        extensions: Option<HashMap<String, Value>>,
    ) -> Self {
        Self {
            query,
            operation_name,
            variables,
            extensions,
        }
    }
}

#[derive(
    Serialize,
    Debug,
    AsRefStr, // for logging the enum variant type as a string without the fields
)]
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

// using a custom deserializer due to compatibility issues
// with internally-tagged enum deserialization #[serde(tag = "type")]
impl<'de> Deserialize<'de> for ClientMessage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let obj = value
            .as_object()
            .ok_or_else(|| serde::de::Error::custom("expected object"))?;

        let type_key = "type".to_string();
        let msg_type = obj
            .get(&type_key)
            .and_then(|v| v.as_str())
            .ok_or_else(|| serde::de::Error::missing_field("type"))?;

        match msg_type {
            "connection_init" => {
                let payload_key = "payload".to_string();
                let payload = obj
                    .get(&payload_key)
                    .filter(|v| !v.is_null())
                    .map(|v| {
                        sonic_rs::from_str(&v.to_string())
                            .map_err(|e| serde::de::Error::custom(e.to_string()))
                    })
                    .transpose()?;
                Ok(ClientMessage::ConnectionInit { payload })
            }
            "ping" => Ok(ClientMessage::Ping {}),
            "pong" => Ok(ClientMessage::Pong {}),
            "subscribe" => {
                let id_key = "id".to_string();
                let payload_key = "payload".to_string();
                let id = obj
                    .get(&id_key)
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| serde::de::Error::missing_field("id"))?
                    .to_string();
                let payload_value = obj
                    .get(&payload_key)
                    .ok_or_else(|| serde::de::Error::missing_field("payload"))?;
                let payload: SubscribePayload = sonic_rs::from_str(&payload_value.to_string())
                    .map_err(|e| serde::de::Error::custom(e.to_string()))?;
                Ok(ClientMessage::Subscribe { id, payload })
            }
            "complete" => {
                let id_key = "id".to_string();
                let id = obj
                    .get(&id_key)
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| serde::de::Error::missing_field("id"))?
                    .to_string();
                Ok(ClientMessage::Complete { id })
            }
            other => Err(serde::de::Error::unknown_variant(
                other,
                &["connection_init", "ping", "pong", "subscribe", "complete"],
            )),
        }
    }
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
/// values as per the spec. We represent this as a HashMap<String, Value>.
#[derive(Serialize, Debug, Clone)]
pub struct ConnectionInitPayload {
    #[serde(flatten)]
    pub fields: HashMap<String, Value>,
}

impl<'de> Deserialize<'de> for ConnectionInitPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let obj = value
            .as_object()
            .ok_or_else(|| serde::de::Error::custom("expected object"))?;

        let mut fields = HashMap::new();
        for (k, v) in obj.iter() {
            fields.insert(k.to_string(), v.clone());
        }

        Ok(ConnectionInitPayload { fields })
    }
}

impl ConnectionInitPayload {
    pub fn new(fields: HashMap<String, Value>) -> Self {
        Self { fields }
    }
}

impl From<http::HeaderMap> for ConnectionInitPayload {
    fn from(headers: http::HeaderMap) -> Self {
        let fields: HashMap<String, Value> = headers
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|v| (name.to_string(), Value::from(v)))
            })
            .collect();
        Self::new(fields)
    }
}

#[derive(
    Serialize,
    Debug,
    AsRefStr, // for logging the enum variant type as a string without the fields
)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    ConnectionAck {
        // NOTE: as per spec there is a "payload" field here, but we don't use it
    },
    Ping {},
    Pong {},
    Next {
        id: String,
        payload: Value,
    },
    Error {
        id: String,
        payload: Vec<GraphQLError>,
    },
    Complete {
        id: String,
    },
}

// using a custom deserializer due to compatibility issues
// with internally-tagged enum deserialization #[serde(tag = "type")]
impl<'de> Deserialize<'de> for ServerMessage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let obj = value
            .as_object()
            .ok_or_else(|| serde::de::Error::custom("expected object"))?;

        let type_key = "type".to_string();
        let msg_type = obj
            .get(&type_key)
            .and_then(|v| v.as_str())
            .ok_or_else(|| serde::de::Error::missing_field("type"))?;

        match msg_type {
            "connection_ack" => Ok(ServerMessage::ConnectionAck {}),
            "ping" => Ok(ServerMessage::Ping {}),
            "pong" => Ok(ServerMessage::Pong {}),
            "next" => {
                let id_key = "id".to_string();
                let payload_key = "payload".to_string();
                let id = obj
                    .get(&id_key)
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| serde::de::Error::missing_field("id"))?
                    .to_string();
                let payload = obj.get(&payload_key).cloned().unwrap_or_else(Value::new);
                Ok(ServerMessage::Next { id, payload })
            }
            "error" => {
                let id_key = "id".to_string();
                let payload_key = "payload".to_string();
                let id = obj
                    .get(&id_key)
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| serde::de::Error::missing_field("id"))?
                    .to_string();
                let payload_value = obj
                    .get(&payload_key)
                    .ok_or_else(|| serde::de::Error::missing_field("payload"))?;
                let payload: Vec<GraphQLError> = sonic_rs::from_str(&payload_value.to_string())
                    .map_err(|e| serde::de::Error::custom(e.to_string()))?;
                Ok(ServerMessage::Error { id, payload })
            }
            "complete" => {
                let id_key = "id".to_string();
                let id = obj
                    .get(&id_key)
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| serde::de::Error::missing_field("id"))?
                    .to_string();
                Ok(ServerMessage::Complete { id })
            }
            other => Err(serde::de::Error::unknown_variant(
                other,
                &[
                    "connection_ack",
                    "ping",
                    "pong",
                    "next",
                    "error",
                    "complete",
                ],
            )),
        }
    }
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
