/// Common types and messages for the GraphQL over WebSocket Transport Protocol
/// as per the spec: https://github.com/enisdenjo/graphql-ws/blob/master/PROTOCOL.md
use ntex::web::ws;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;
use tracing::error;

use crate::response::graphql_error::GraphQLError;

pub enum CloseCode {
    ConnectionInitTimeout,
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
            CloseCode::ConnectionInitTimeout => ws::Message::Close(Some(ws::CloseReason {
                code: ws::CloseCode::from(4408),
                description: Some("Connection initialisation timeout".into()),
            })),
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

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    ConnectionInit {
        payload: Option<ConnectionInitPayload>,
    },
    Ping {},
    Subscribe {
        id: String,
        payload: ExecutionRequest,
    },
    Complete {
        id: String,
    },
}

/// The connection init message payload MUST be a map of string to arbitrary JSON
/// values as per the spec. We represent this as a HashMap<String, Value> and use
/// serde(flatten) to capture all fields for easier parsing to headers later.
#[derive(Serialize, Deserialize, Debug)]
pub struct ConnectionInitPayload {
    #[serde(flatten)]
    pub fields: HashMap<String, sonic_rs::Value>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    ConnectionAck {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        payload: Option<sonic_rs::Value>,
    },
    Pong {},
    Next {
        id: String,
        payload: sonic_rs::Value,
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
    pub fn ack(payload: Option<sonic_rs::Value>) -> ws::Message {
        ServerMessage::ConnectionAck { payload }.into()
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

// copied from bin/router/src/pipeline/execution_request.rs

// we cant import it directly due to cyclic dependency issues
// TODO: refactor to share the execution request accordingly

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionRequest {
    pub query: String,
    pub operation_name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub variables: HashMap<String, sonic_rs::Value>,
    pub extensions: Option<HashMap<String, sonic_rs::Value>>,
}

fn deserialize_null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    T: Default + Deserialize<'de>,
    D: Deserializer<'de>,
{
    let opt = Option::<T>::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}

// end copy from bin/router/src/pipeline/execution_request.rs
