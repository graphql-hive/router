use std::time::Duration;
use std::{
    convert::Infallible,
    io,
    task::{Context, Poll},
};

use async_graphql::{Executor, Response as GraphQLResponse};
use async_graphql_axum::GraphQLBatchRequest;
use async_graphql_axum::{rejection::GraphQLRejection, GraphQLRequest};
use axum::{
    body::{Body, HttpBody},
    extract::FromRequest,
    http::{Request as HttpRequest, Response as HttpResponse},
    response::IntoResponse,
    BoxError,
};
use bytes::Bytes;
use futures_util::{future::BoxFuture, stream::BoxStream, Stream, StreamExt};
use hive_router::pipeline::multipart_subscribe::APOLLO_MULTIPART_HTTP_CONTENT_TYPE;
use hive_router::pipeline::sse::SSE_HEADER;
use hive_router::pipeline::{multipart_subscribe, sse};
use serde::{Deserialize, Serialize};
use tower_service::Service;

use crate::SubscriptionProtocol;

const CALLBACK_SPEC_ACCEPT: &str = "application/json;callbackSpec=1.0";
const SUBSCRIPTION_PROTOCOL_HEADER: &str = "subscription-protocol";
const CALLBACK_PROTOCOL_VERSION: &str = "callback/1.0";

#[derive(Clone)]
pub struct GraphQL<E> {
    executor: E,
    subscriptions_protocol: SubscriptionProtocol,
}

impl<E> GraphQL<E>
where
    E: Clone,
{
    pub fn new(executor: E, subscriptions_protocol: SubscriptionProtocol) -> Self {
        Self {
            executor,
            subscriptions_protocol,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CallbackSubscriptionExt {
    callback_url: String,
    subscription_id: String,
    verifier: String,
    #[serde(default)]
    heartbeat_interval_ms: u64,
}

#[derive(Serialize)]
struct CallbackMessage<'a> {
    kind: &'a str,
    action: &'a str,
    id: &'a str,
    verifier: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    errors: Option<Vec<GraphQLError>>,
}

// yeah I know there's a graphql error struct, but I want to avoid overengineering this
#[derive(Serialize)]
struct GraphQLError {
    message: String,
}

impl<B, E> Service<HttpRequest<B>> for GraphQL<E>
where
    B: HttpBody<Data = Bytes> + Send + 'static,
    B::Data: Into<Bytes>,
    B::Error: Into<BoxError>,
    E: Executor,
{
    type Response = HttpResponse<Body>;
    type Error = Infallible;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: HttpRequest<B>) -> Self::Future {
        let does_accept_callback = req
            .headers()
            .get("accept")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|accept| accept.contains(CALLBACK_SPEC_ACCEPT));

        let does_accept_multipart_mixed = req
            .headers()
            .get("accept")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|accept| accept.contains(r#"multipart/mixed;subscriptionSpec="1.0""#));

        let does_accept_event_stream = req
            .headers()
            .get("accept")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|accept| accept.contains(SSE_HEADER));

        let break_after_count = req
            .headers()
            .get("x-break-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<usize>().ok());

        let sub_prot = self.subscriptions_protocol.clone();

        let executor = self.executor.clone();
        let req = req.map(Body::new);

        Box::pin(async move {
            if does_accept_callback {
                let req = match GraphQLRequest::<GraphQLRejection>::from_request(req, &()).await {
                    Ok(req) => req,
                    Err(err) => return Ok(err.into_response()),
                };

                let gql_request = req.into_inner();

                let sub_ext = match gql_request.extensions.get("subscription") {
                    Some(val) => {
                        let json = serde_json::to_value(val).unwrap();
                        match serde_json::from_value::<CallbackSubscriptionExt>(json) {
                            Ok(ext) => ext,
                            Err(_) => {
                                return Ok(HttpResponse::builder()
                                    .status(400)
                                    .body(Body::from(
                                        r#"{"errors":[{"message":"Invalid subscription extension"}]}"#,
                                    ))
                                    .unwrap());
                            }
                        }
                    }
                    None => {
                        return Ok(HttpResponse::builder()
                            .status(400)
                            .body(Body::from(
                                r#"{"errors":[{"message":"Missing subscription extension"}]}"#,
                            ))
                            .unwrap());
                    }
                };

                // no hyper, just keeping it simple
                let client = reqwest::Client::new();

                let check_msg = CallbackMessage {
                    kind: "subscription",
                    action: "check",
                    id: &sub_ext.subscription_id,
                    verifier: &sub_ext.verifier,
                    payload: None,
                    errors: None,
                };
                let check_resp = client
                    .post(&sub_ext.callback_url)
                    .header(SUBSCRIPTION_PROTOCOL_HEADER, CALLBACK_PROTOCOL_VERSION)
                    .json(&check_msg)
                    .send()
                    .await;

                match check_resp {
                    Ok(resp) if resp.status() == 204 => {}
                    _ => {
                        // yeah yeah, TODO: explain why it failed
                        return Ok(HttpResponse::builder()
                            .status(400)
                            .body(Body::from(
                                r#"{"errors":[{"message":"Callback check failed for whatever reason"}]}"#,
                            ))
                            .unwrap());
                    }
                }

                let stream = executor.execute_stream(gql_request, None);

                tokio::spawn(emit_subscription_events(
                    client,
                    sub_ext.callback_url,
                    sub_ext.subscription_id,
                    sub_ext.verifier,
                    sub_ext.heartbeat_interval_ms,
                    stream,
                ));

                return Ok(HttpResponse::builder()
                    .status(200)
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"data":null}"#))
                    .unwrap());
            }

            if !does_accept_multipart_mixed && !does_accept_event_stream {
                let req =
                    match GraphQLBatchRequest::<GraphQLRejection>::from_request(req, &()).await {
                        Ok(req) => req,
                        Err(err) => return Ok(err.into_response()),
                    };
                return Ok(async_graphql_axum::GraphQLResponse(
                    executor.execute_batch(req.0).await,
                )
                .into_response());
            }

            let req = match GraphQLRequest::<GraphQLRejection>::from_request(req, &()).await {
                Ok(req) => req,
                Err(err) => return Ok(err.into_response()),
            };
            let stream = executor.execute_stream(req.0, None);

            let use_sse = match sub_prot {
                SubscriptionProtocol::Auto => does_accept_event_stream,
                SubscriptionProtocol::SseOnly => true,
                SubscriptionProtocol::MultipartOnly => false,
            };

            if use_sse {
                let byte_stream =
                    create_sse_stream(stream, Duration::from_secs(10)).map(Ok::<_, io::Error>);
                let body = if let Some(count) = break_after_count {
                    Body::from_stream(abrupt_terminate_after(byte_stream, count))
                } else {
                    Body::from_stream(byte_stream)
                };
                Ok(HttpResponse::builder()
                    .header(http::header::CONTENT_TYPE, SSE_HEADER)
                    .header(http::header::CACHE_CONTROL, "no-cache")
                    .header(http::header::CONNECTION, "keep-alive")
                    .body(body)
                    .unwrap())
            } else {
                let byte_stream =
                    create_multipart_subscribe_stream(stream, Duration::from_secs(30))
                        .map(Ok::<_, io::Error>);
                let body = if let Some(count) = break_after_count {
                    Body::from_stream(abrupt_terminate_after(byte_stream, count))
                } else {
                    Body::from_stream(byte_stream)
                };
                Ok(HttpResponse::builder()
                    .header(
                        http::header::CONTENT_TYPE,
                        APOLLO_MULTIPART_HTTP_CONTENT_TYPE,
                    )
                    .header(http::header::CACHE_CONTROL, "no-cache")
                    .header(http::header::CONNECTION, "keep-alive")
                    .body(body)
                    .unwrap())
            }
        })
    }
}

async fn emit_subscription_events(
    client: reqwest::Client,
    callback_url: String,
    subscription_id: String,
    verifier: String,
    heartbeat_interval_ms: u64,
    stream: impl Stream<Item = GraphQLResponse> + Send + Unpin + 'static,
) {
    let mut stream = std::pin::pin!(stream);

    let heartbeat_enabled = heartbeat_interval_ms > 0;
    let mut heartbeat_interval = if heartbeat_enabled {
        Some(tokio::time::interval(Duration::from_millis(
            heartbeat_interval_ms,
        )))
    } else {
        None
    };

    // skip the first immediate tick
    if let Some(ref mut interval) = heartbeat_interval {
        interval.tick().await;
    }

    loop {
        let next_event = if let Some(ref mut interval) = heartbeat_interval {
            tokio::select! {
                item = stream.next() => item.map(CallbackEvent::Next),
                _ = interval.tick() => Some(CallbackEvent::Heartbeat),
            }
        } else {
            stream.next().await.map(CallbackEvent::Next)
        };

        match next_event {
            Some(CallbackEvent::Next(response)) => {
                let payload =
                    serde_json::to_value(&response).expect("Failed to serialize GraphQLResponse");
                let msg = CallbackMessage {
                    kind: "subscription",
                    action: "next",
                    id: &subscription_id,
                    verifier: &verifier,
                    payload: Some(payload),
                    errors: None,
                };
                match send_callback(&client, &callback_url, &msg).await {
                    Ok(status) if status.is_success() => {}
                    // 404, or any other non-success status or error terminates the
                    // subscription. this subgraph is for testing, we dont dwell
                    // too much about learning the exact reason
                    _ => return,
                }
            }
            Some(CallbackEvent::Heartbeat) => {
                let msg = CallbackMessage {
                    kind: "subscription",
                    action: "check",
                    id: &subscription_id,
                    verifier: &verifier,
                    payload: None,
                    errors: None,
                };
                match send_callback(&client, &callback_url, &msg).await {
                    Ok(status) if status.is_success() => {}
                    // 404, or any other non-success status or error terminates the
                    // subscription. this subgraph is for testing, we dont dwell
                    // too much about learning the exact reason
                    _ => return,
                }
            }
            None => {
                let msg = CallbackMessage {
                    kind: "subscription",
                    action: "complete",
                    id: &subscription_id,
                    verifier: &verifier,
                    payload: None,
                    errors: None,
                };
                let _ = send_callback(&client, &callback_url, &msg).await;
                return;
            }
        }
    }
}

enum CallbackEvent {
    Next(GraphQLResponse),
    Heartbeat,
}

async fn send_callback(
    client: &reqwest::Client,
    url: &str,
    msg: &CallbackMessage<'_>,
) -> Result<reqwest::StatusCode, reqwest::Error> {
    let resp = client
        .post(url)
        .header(SUBSCRIPTION_PROTOCOL_HEADER, CALLBACK_PROTOCOL_VERSION)
        .json(msg)
        .send()
        .await?;
    Ok(resp.status())
}

fn abrupt_terminate_after<S>(
    stream: S,
    count: usize,
) -> impl Stream<Item = Result<Bytes, io::Error>>
where
    S: Stream<Item = Result<Bytes, io::Error>> + Send + 'static,
{
    async_stream::stream! {
        let mut stream = std::pin::pin!(stream);
        let mut emitted = 0;

        while let Some(item) = stream.next().await {
            yield item;
            emitted += 1;
            if emitted > count {
                // error abruptly killing the connection
                yield Err(io::Error::new(io::ErrorKind::ConnectionReset, "connection abruptly terminated"));
                break;
            }
        }
    }
}

pub fn create_sse_stream(
    input: impl Stream<Item = GraphQLResponse> + Send + Unpin + 'static,
    heartbeat_interval: Duration,
) -> BoxStream<'static, Bytes> {
    // GraphQLResponse stream to Vec<u8> stream
    let byte_stream =
        input.map(|resp| serde_json::to_vec(&resp).expect("Failed to serialize GraphQLResponse"));
    sse::create_stream(byte_stream, heartbeat_interval)
        .map(|result| {
            // Convert Result<ntex::util::Bytes, std::io::Error> to bytes::Bytes
            // Unwrap is safe here as we control the serialization above
            Bytes::copy_from_slice(&result.expect("SSE stream error"))
        })
        .boxed()
}

pub fn create_multipart_subscribe_stream(
    input: impl Stream<Item = GraphQLResponse> + Send + Unpin + 'static,
    heartbeat_interval: Duration,
) -> BoxStream<'static, Bytes> {
    // GraphQLResponse stream to Vec<u8> stream
    let byte_stream =
        input.map(|resp| serde_json::to_vec(&resp).expect("Failed to serialize GraphQLResponse"));
    multipart_subscribe::create_apollo_multipart_http_stream(byte_stream, heartbeat_interval)
        .map(|result| {
            // Convert Result<ntex::util::Bytes, std::io::Error> to bytes::Bytes
            // Unwrap is safe here as we control the serialization above
            Bytes::copy_from_slice(&result.expect("Multipart stream error"))
        })
        .boxed()
}
