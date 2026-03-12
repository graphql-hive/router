use bytes::Bytes as BytesLib;
use dashmap::mapref::one::Ref;
use hive_router_plan_executor::executors::http_callback::{
    ActiveSubscription, ActiveSubscriptionsMap, CallbackMessage, CALLBACK_PROTOCOL_VERSION,
    SUBSCRIPTION_PROTOCOL_HEADER,
};
use hive_router_plan_executor::response::graphql_error::GraphQLError;
use ntex::util::Bytes;
use ntex::web::{self, types::Path, HttpRequest, HttpResponse};
use serde::Deserialize;
use tracing::{debug, trace, warn};

#[derive(Debug, Deserialize)]
struct CallbackPayload<'a> {
    kind: String,
    action: String,
    id: String,
    verifier: String,
    #[serde(borrow, default)]
    payload: Option<&'a serde_json::value::RawValue>,
    #[serde(default)]
    errors: Option<Vec<GraphQLError>>,
}

fn bad_request() -> HttpResponse {
    HttpResponse::BadRequest()
        .header(SUBSCRIPTION_PROTOCOL_HEADER, CALLBACK_PROTOCOL_VERSION)
        .finish()
}

fn not_found() -> HttpResponse {
    HttpResponse::NotFound()
        .header(SUBSCRIPTION_PROTOCOL_HEADER, CALLBACK_PROTOCOL_VERSION)
        .finish()
}

fn no_content() -> HttpResponse {
    HttpResponse::NoContent()
        .header(SUBSCRIPTION_PROTOCOL_HEADER, CALLBACK_PROTOCOL_VERSION)
        .finish()
}

fn validate_protocol(req: &HttpRequest) -> Result<(), HttpResponse> {
    let protocol_header = req
        .headers()
        .get(SUBSCRIPTION_PROTOCOL_HEADER)
        .and_then(|v| v.to_str().ok());

    if protocol_header != Some(CALLBACK_PROTOCOL_VERSION) {
        warn!(
            "Invalid or missing {} header, expected {}",
            SUBSCRIPTION_PROTOCOL_HEADER, CALLBACK_PROTOCOL_VERSION
        );
        return Err(bad_request());
    }

    Ok(())
}

fn parse_payload(body: &Bytes) -> Result<CallbackPayload<'_>, HttpResponse> {
    serde_json::from_slice(body).map_err(|e| {
        warn!("Failed to parse callback payload: {}", e);
        bad_request()
    })
}

fn validate_payload(
    payload: &CallbackPayload<'_>,
    subscription_id_from_path: &str,
) -> Result<(), HttpResponse> {
    if payload.kind != "subscription" {
        warn!(
            "Invalid callback kind: {}, expected 'subscription'",
            payload.kind
        );
        return Err(bad_request());
    }

    if payload.id != subscription_id_from_path {
        warn!(
            "Subscription ID mismatch: path='{}', body='{}'",
            subscription_id_from_path, payload.id
        );
        return Err(bad_request());
    }

    Ok(())
}

fn handle_check(
    subscription_id: &str,
    subscription: &Ref<'_, String, ActiveSubscription>,
) -> HttpResponse {
    trace!(subscription_id = %subscription_id, "Received check message");
    subscription.record_heartbeat();
    no_content()
}

fn handle_next(
    subscription_id: &str,
    payload: &CallbackPayload<'_>,
    subscription: Ref<'_, String, ActiveSubscription>,
    active_subscriptions: &ActiveSubscriptionsMap,
) -> HttpResponse {
    trace!(subscription_id = %subscription_id, "Received next message");

    let data = match payload.payload {
        Some(p) => BytesLib::copy_from_slice(p.get().as_bytes()),
        None => {
            warn!(subscription_id = %subscription_id, "Missing payload in next message");
            return bad_request();
        }
    };

    if subscription
        .sender
        .send(CallbackMessage::Next { payload: data })
        .is_err()
    {
        debug!(subscription_id = %subscription_id, "Subscription receiver dropped");
        drop(subscription);
        active_subscriptions.remove(subscription_id);
        return not_found();
    }

    no_content()
}

fn handle_complete(
    subscription_id: &str,
    payload: &CallbackPayload<'_>,
    subscription: Ref<'_, String, ActiveSubscription>,
    active_subscriptions: &ActiveSubscriptionsMap,
) -> HttpResponse {
    trace!(subscription_id = %subscription_id, "Received complete message");
    let _ = subscription.sender.send(CallbackMessage::Complete {
        errors: payload.errors.clone(),
    });
    drop(subscription);
    active_subscriptions.remove(subscription_id);
    no_content()
}

pub async fn handler(
    req: HttpRequest,
    path: Path<String>,
    body: Bytes,
    active_subscriptions: web::types::State<ActiveSubscriptionsMap>,
) -> HttpResponse {
    let subscription_id_from_path = path.into_inner();

    if let Err(response) = validate_protocol(&req) {
        return response;
    }

    let payload = match parse_payload(&body) {
        Ok(p) => p,
        Err(response) => return response,
    };

    if let Err(response) = validate_payload(&payload, &subscription_id_from_path) {
        return response;
    }

    let subscription = match active_subscriptions.get(&payload.id) {
        Some(sub) => sub,
        None => {
            debug!(
                subscription_id = %payload.id,
                "Subscription not found, may have been terminated"
            );
            return not_found();
        }
    };

    if subscription.verifier != payload.verifier {
        warn!(subscription_id = %payload.id, "Invalid verifier for subscription");
        return bad_request();
    }

    match payload.action.as_str() {
        "check" => handle_check(&payload.id, &subscription),
        "next" => handle_next(&payload.id, &payload, subscription, &active_subscriptions),
        "complete" => handle_complete(&payload.id, &payload, subscription, &active_subscriptions),
        _ => {
            warn!(
                subscription_id = %payload.id,
                action = %payload.action,
                "Unknown callback action"
            );
            bad_request()
        }
    }
}
