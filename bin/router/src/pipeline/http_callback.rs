use std::sync::Arc;

use bytes::Bytes as BytesLib;
use hive_router_plan_executor::executors::active_subscriptions::{
    ActiveSubscriptionsRegistry, BroadcastItem,
};
use hive_router_plan_executor::executors::http_callback::{
    CALLBACK_PROTOCOL_VERSION, SUBSCRIPTION_PROTOCOL_HEADER,
};
use hive_router_plan_executor::response::graphql_error::GraphQLError;
use http::StatusCode;
use ntex::util::Bytes;
use ntex::web::WebResponseError;
use ntex::web::{self, types::Path, HttpRequest, HttpResponse};
use serde::Deserialize;
use strum::EnumString;
use tracing::{debug, error, trace, warn};

#[derive(Debug, Deserialize, EnumString)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
enum CallbackKind {
    Subscription,
}

#[derive(Debug, Deserialize, EnumString)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
enum CallbackAction {
    Check,
    Next,
    Complete,
}

#[derive(Debug, Deserialize)]
struct CallbackPayload<'a> {
    // unused in code, but used for validation
    #[allow(unused)]
    kind: CallbackKind,
    action: CallbackAction,
    id: String,
    verifier: String,
    #[serde(borrow, default)]
    payload: Option<sonic_rs::LazyValue<'a>>,
    #[serde(default)]
    errors: Option<Vec<GraphQLError>>,
}

#[derive(thiserror::Error, Debug)]
pub enum CallbackError {
    #[error(
        "Invalid or missing {} header, expected {}",
        SUBSCRIPTION_PROTOCOL_HEADER,
        CALLBACK_PROTOCOL_VERSION
    )]
    InvalidProtocolHeader,
    #[error("Failed to parse callback payload: {0}")]
    PayloadParseError(#[from] sonic_rs::Error),
    #[error("Subscription ID mismatch: path='{path}', body='{body}'")]
    SubscriptionIdMismatch { path: String, body: String },
    #[error("Missing payload in next message for subscription ID '{subscription_id}'")]
    MissingPayload { subscription_id: String },
    #[error(
        "Subscription not found, may have been terminated for subscription ID '{subscription_id}'"
    )]
    SubscriptionNotFound { subscription_id: String },
    #[error("Invalid verifier for subscription ID '{subscription_id}'")]
    InvalidVerifier { subscription_id: String },
    #[error("Subscription receiver dropped for subscription ID '{subscription_id}'")]
    SubscriptionDropped { subscription_id: String },
    // NOTE: intentionally a different variant from SubscriptionDropped
    #[error(
        "Client consuming too slowly. Event buffer full for subscription ID '{subscription_id}'"
    )]
    ClientTooSlow { subscription_id: String },
}

impl CallbackError {
    fn log(&self) {
        match self {
            CallbackError::InvalidProtocolHeader => warn!("{}", self),
            CallbackError::PayloadParseError(_) => error!("{}", self),
            CallbackError::SubscriptionIdMismatch { .. } => warn!("{}", self),
            CallbackError::MissingPayload { .. } => warn!("{}", self),
            CallbackError::SubscriptionNotFound { .. } => warn!("{}", self),
            CallbackError::InvalidVerifier { .. } => warn!("{}", self),
            CallbackError::SubscriptionDropped { .. } => debug!("{}", self),
            CallbackError::ClientTooSlow { .. } => warn!("{}", self),
        }
    }
}

impl WebResponseError for CallbackError {
    fn status_code(&self) -> StatusCode {
        match self {
            CallbackError::InvalidProtocolHeader
            | CallbackError::PayloadParseError(_)
            | CallbackError::MissingPayload { .. }
            | CallbackError::InvalidVerifier { .. } => StatusCode::BAD_REQUEST,
            CallbackError::SubscriptionNotFound { .. }
            | CallbackError::SubscriptionDropped { .. }
            | CallbackError::SubscriptionIdMismatch { .. } => StatusCode::NOT_FOUND,
            // 503 signals the subgraph that the router is temporarily unable to accept events,
            // the subgraph can decide to retry or close the subscription on its end
            CallbackError::ClientTooSlow { .. } => StatusCode::SERVICE_UNAVAILABLE,
        }
    }
    fn error_response(&self, _: &HttpRequest) -> HttpResponse {
        self.log();
        HttpResponse::build(self.status_code())
            .header(SUBSCRIPTION_PROTOCOL_HEADER, CALLBACK_PROTOCOL_VERSION)
            .finish()
    }
}

fn validate_protocol(req: &HttpRequest) -> Result<(), CallbackError> {
    let protocol_header = req
        .headers()
        .get(SUBSCRIPTION_PROTOCOL_HEADER)
        .and_then(|v| v.to_str().ok());

    if protocol_header != Some(CALLBACK_PROTOCOL_VERSION) {
        return Err(CallbackError::InvalidProtocolHeader);
    }

    Ok(())
}

fn parse_payload(body: &Bytes) -> Result<CallbackPayload<'_>, CallbackError> {
    Ok(sonic_rs::from_slice(body)?)
}

fn validate_payload(
    payload: &CallbackPayload<'_>,
    subscription_id_from_path: &str,
) -> Result<(), CallbackError> {
    if payload.id != subscription_id_from_path {
        return Err(CallbackError::SubscriptionIdMismatch {
            path: subscription_id_from_path.to_string(),
            body: payload.id.to_string(),
        });
    }

    Ok(())
}

fn handle_check(subscription_id: &str, registry: &ActiveSubscriptionsRegistry) {
    trace!(subscription_id = %subscription_id, "Received check message");
    registry.record_heartbeat(subscription_id);
}

fn handle_next(
    subscription_id: &str,
    payload: &CallbackPayload<'_>,
    registry: &ActiveSubscriptionsRegistry,
) -> Result<(), CallbackError> {
    trace!(subscription_id = %subscription_id, "Received next message");

    let data = match &payload.payload {
        Some(p) => BytesLib::copy_from_slice(p.as_raw_str().as_bytes()),
        None => {
            return Err(CallbackError::MissingPayload {
                subscription_id: subscription_id.to_string(),
            });
        }
    };

    if !registry.send_event(subscription_id, BroadcastItem::Event(data)) {
        debug!(subscription_id = %subscription_id, "Subscription receiver dropped");
        registry.remove(subscription_id);
        return Err(CallbackError::SubscriptionDropped {
            subscription_id: subscription_id.to_string(),
        });
    }

    // TODO: ClientTooSlow

    Ok(())
}

fn handle_complete(
    subscription_id: &str,
    payload: &CallbackPayload<'_>,
    registry: &ActiveSubscriptionsRegistry,
) {
    trace!(subscription_id = %subscription_id, "Received complete message");
    if let Some(errors) = &payload.errors {
        if !errors.is_empty() {
            registry.send_event(subscription_id, BroadcastItem::Error(errors.clone()));
        }
    }
    registry.remove(subscription_id);
}

pub async fn handler(
    req: HttpRequest,
    path: Path<String>,
    body: Bytes,
    active_subscriptions: web::types::State<Arc<ActiveSubscriptionsRegistry>>,
) -> Result<HttpResponse, CallbackError> {
    let subscription_id_from_path = path.into_inner();

    validate_protocol(&req)?;

    let payload = parse_payload(&body)?;

    validate_payload(&payload, &subscription_id_from_path)?;

    if !active_subscriptions.contains(&payload.id) {
        return Err(CallbackError::SubscriptionNotFound {
            subscription_id: payload.id.clone(),
        });
    }

    let verifier = active_subscriptions
        .get_callback_verifier(&payload.id)
        .ok_or_else(|| CallbackError::SubscriptionNotFound {
            subscription_id: payload.id.clone(),
        })?;

    if verifier != payload.verifier {
        return Err(CallbackError::InvalidVerifier {
            subscription_id: payload.id.clone(),
        });
    }

    match payload.action {
        CallbackAction::Check => handle_check(&payload.id, &active_subscriptions),
        CallbackAction::Next => {
            handle_next(&payload.id, &payload, &active_subscriptions)?;
        }
        CallbackAction::Complete => handle_complete(&payload.id, &payload, &active_subscriptions),
    };

    Ok(HttpResponse::NoContent()
        .header(SUBSCRIPTION_PROTOCOL_HEADER, CALLBACK_PROTOCOL_VERSION)
        .finish())
}
