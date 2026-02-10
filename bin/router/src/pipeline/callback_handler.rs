use bytes::Bytes as BytesLib;
use hive_router_plan_executor::executors::http_callback::{
    ActiveSubscriptionsMap, CallbackMessage, CALLBACK_PROTOCOL_VERSION, SUBSCRIPTION_PROTOCOL_HEADER,
};
use hive_router_plan_executor::response::graphql_error::GraphQLError;
use ntex::util::Bytes;
use ntex::web::{self, types::Path, HttpRequest, HttpResponse};
use serde::Deserialize;
use tracing::{debug, trace, warn};

#[derive(Debug, Deserialize)]
struct CallbackPayload {
    kind: String,
    action: String,
    id: String,
    verifier: String,
    #[serde(default)]
    payload: Option<sonic_rs::Value>,
    #[serde(default)]
    errors: Option<Vec<GraphQLError>>,
}

pub async fn callback_handler(
    req: HttpRequest,
    path: Path<String>,
    body: Bytes,
    active_subscriptions: web::types::State<ActiveSubscriptionsMap>,
) -> HttpResponse {
    let subscription_id_from_path = path.into_inner();

    let protocol_header = req
        .headers()
        .get(SUBSCRIPTION_PROTOCOL_HEADER)
        .and_then(|v| v.to_str().ok());

    if protocol_header != Some(CALLBACK_PROTOCOL_VERSION) {
        warn!(
            "Invalid or missing {} header, expected {}",
            SUBSCRIPTION_PROTOCOL_HEADER, CALLBACK_PROTOCOL_VERSION
        );
        return HttpResponse::BadRequest().finish();
    }

    let payload: CallbackPayload = match sonic_rs::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            warn!("Failed to parse callback payload: {}", e);
            return HttpResponse::BadRequest().finish();
        }
    };

    if payload.kind != "subscription" {
        warn!("Invalid callback kind: {}, expected 'subscription'", payload.kind);
        return HttpResponse::BadRequest().finish();
    }

    if payload.id != subscription_id_from_path {
        warn!(
            "Subscription ID mismatch: path='{}', body='{}'",
            subscription_id_from_path, payload.id
        );
        return HttpResponse::BadRequest().finish();
    }

    let subscription = match active_subscriptions.get(&payload.id) {
        Some(sub) => sub,
        None => {
            debug!(
                subscription_id = %payload.id,
                "Subscription not found, may have been terminated"
            );
            return HttpResponse::NotFound().finish();
        }
    };

    if subscription.verifier != payload.verifier {
        warn!(
            subscription_id = %payload.id,
            "Invalid verifier for subscription"
        );
        return HttpResponse::Unauthorized().finish();
    }

    match payload.action.as_str() {
        "check" => {
            trace!(subscription_id = %payload.id, "Received check message");
            HttpResponse::NoContent()
                .header(SUBSCRIPTION_PROTOCOL_HEADER, CALLBACK_PROTOCOL_VERSION)
                .finish()
        }
        "next" => {
            trace!(subscription_id = %payload.id, "Received next message");
            let data = match payload.payload {
                Some(p) => match sonic_rs::to_vec(&p) {
                    Ok(bytes) => BytesLib::from(bytes),
                    Err(e) => {
                        warn!(
                            subscription_id = %payload.id,
                            "Failed to serialize payload: {}", e
                        );
                        return HttpResponse::BadRequest().finish();
                    }
                },
                None => {
                    warn!(
                        subscription_id = %payload.id,
                        "Missing payload in next message"
                    );
                    return HttpResponse::BadRequest().finish();
                }
            };

            if subscription.sender.send(CallbackMessage::Next { payload: data }).is_err() {
                debug!(
                    subscription_id = %payload.id,
                    "Subscription receiver dropped"
                );
                drop(subscription);
                active_subscriptions.remove(&payload.id);
                return HttpResponse::NotFound().finish();
            }

            HttpResponse::NoContent()
                .header(SUBSCRIPTION_PROTOCOL_HEADER, CALLBACK_PROTOCOL_VERSION)
                .finish()
        }
        "complete" => {
            trace!(subscription_id = %payload.id, "Received complete message");
            let _ = subscription.sender.send(CallbackMessage::Complete {
                errors: payload.errors,
            });
            drop(subscription);
            active_subscriptions.remove(&payload.id);

            HttpResponse::NoContent()
                .header(SUBSCRIPTION_PROTOCOL_HEADER, CALLBACK_PROTOCOL_VERSION)
                .finish()
        }
        _ => {
            warn!(
                subscription_id = %payload.id,
                action = %payload.action,
                "Unknown callback action"
            );
            HttpResponse::BadRequest().finish()
        }
    }
}
