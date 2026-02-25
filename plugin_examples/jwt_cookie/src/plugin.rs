use std::collections::HashMap;

use chrono::{DateTime, Utc};
use hive_router::{
    async_trait,
    ntex::http::{header::HeaderValue, HeaderMap},
    plugins::{
        hooks::{
            on_graphql_params::{OnGraphQLParamsStartHookPayload, OnGraphQLParamsStartHookResult},
            on_http_request::{OnHttpRequestHookPayload, OnHttpRequestHookResult},
            on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
            on_subgraph_execute::{
                OnSubgraphExecuteStartHookPayload, OnSubgraphExecuteStartHookResult,
            },
        },
        plugin_context::RouterHttpRequest,
        plugin_trait::{EndHookPayload, RouterPlugin, StartHookPayload},
    },
    sonic_rs::json,
    GraphQLError,
};
use reqwest::{
    header::{AUTHORIZATION, SET_COOKIE},
    Client, StatusCode,
};
use serde::{Deserialize, Serialize};

pub enum JwtCookieContext {
    ShouldRefresh {
        refresh_token: String,
    },
    Refreshed {
        jwt_token: String,
        refresh_token: String,
        expired_at: DateTime<Utc>,
    },
    Valid {
        jwt_token: String,
    },
}

// Constants for cookie names
pub const JWT_TOKEN_NAME: &str = "jwt_token";
pub const JWT_REFRESH_TOKEN_NAME: &str = "jwt_refresh_token";
pub const JWT_EXPIRED_AT_NAME: &str = "jwt_expired_at";

// Constructor for the plugin context

impl JwtCookieContext {
    pub fn new(router_http_request: &RouterHttpRequest) -> Option<Self> {
        let cookies = parse_cookies(router_http_request.headers);
        if let Some(jwt_token) = cookies.get(JWT_TOKEN_NAME) {
            if let Some(expired_at) = cookies.get(JWT_EXPIRED_AT_NAME) {
                if let Ok(expired_at) = expired_at.parse::<DateTime<Utc>>() {
                    // If expired_at is in the past or within the next minute, consider the token as expired and try to refresh it
                    if expired_at <= Utc::now() + chrono::Duration::days(7) {
                        // Only return ShouldRefresh if there is a refresh token cookie, otherwise consider it as invalid
                        if let Some(refresh_token) = cookies.get(JWT_REFRESH_TOKEN_NAME) {
                            return Some(Self::ShouldRefresh {
                                refresh_token: refresh_token.clone(),
                            });
                        }
                    } else {
                        return Some(Self::Valid {
                            jwt_token: jwt_token.clone(),
                        });
                    }
                }
            }
        }
        None
    }
}

pub struct JwtCookiePlugin {
    client: Client,
    refresh_endpoint: String,
}

// Return type of the token service
#[derive(Serialize, Deserialize)]
pub struct NewTokenResult {
    pub jwt_token: String,
    pub refresh_token: String,
    pub expired_at: DateTime<Utc>,
}

// Helper functions
async fn request_new_token(
    client: &Client,
    refresh_endpoint: &str,
    refresh_token: &str,
) -> Result<JwtCookieContext, reqwest::Error> {
    let result: NewTokenResult = client
        .post(refresh_endpoint)
        .json(&json!({
            "refresh_token": refresh_token,
        }))
        .send()
        .await?
        .json()
        .await?;
    Ok(JwtCookieContext::Refreshed {
        jwt_token: result.jwt_token,
        refresh_token: result.refresh_token,
        expired_at: result.expired_at,
    })
}

fn parse_cookies(headers: &HeaderMap) -> HashMap<String, String> {
    let mut cookies = HashMap::new();
    if let Some(cookie_header) = headers.get("cookie") {
        if let Ok(cookie_str) = cookie_header.to_str() {
            for cookie in cookie_str.split(';') {
                let parts: Vec<&str> = cookie.trim().splitn(2, '=').collect();
                if parts.len() == 2 {
                    let name = parts[0].trim().to_string();
                    let value = parts[1].trim().to_string();
                    cookies.insert(name, value);
                }
            }
        }
    }
    cookies
}

fn create_set_cookie(cookie_name: &str, cookie_value: &str) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{}={}; Path=/; HttpOnly; Secure; SameSite=Lax",
        cookie_name, cookie_value
    ))
    .unwrap()
}

// Configuration Type

#[derive(Deserialize)]
pub struct JwtCookiePluginConfig {
    pub refresh_endpoint: String,
}

// Actual Plugin Implementation

#[async_trait]
impl RouterPlugin for JwtCookiePlugin {
    type Config = JwtCookiePluginConfig;
    fn plugin_name() -> &'static str {
        "jwt_cookie"
    }
    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        let config = payload.config()?;
        payload.initialize_plugin(Self {
            client: Client::new(),
            refresh_endpoint: config.refresh_endpoint,
        })
    }
    // First handle incoming cookies
    async fn on_graphql_params<'exec>(
        &'exec self,
        payload: OnGraphQLParamsStartHookPayload<'exec>,
    ) -> OnGraphQLParamsStartHookResult<'exec> {
        let ctx = match JwtCookieContext::new(payload.router_http_request) {
            None => {
                // No valid cookie found
                return payload.end_with_graphql_error(
                    GraphQLError::from_message_and_code(
                        "Expected JWT token in the cookies",
                        "NO_JWT_COOKIE",
                    ),
                    StatusCode::UNAUTHORIZED,
                );
            }
            Some(JwtCookieContext::ShouldRefresh { refresh_token }) => {
                // Try to refresh the token
                match request_new_token(&self.client, &self.refresh_endpoint, &refresh_token).await
                {
                    Ok(new_ctx) => new_ctx,
                    Err(err) => {
                        return payload.end_with_graphql_error(
                            GraphQLError::from_message_and_code(
                                format!("Failed to refresh the token: {}", err),
                                "FAILED_TO_REFRESH_JWT",
                            ),
                            StatusCode::UNAUTHORIZED,
                        );
                    }
                }
            }
            Some(ctx) => ctx,
        };
        payload.context.insert(ctx);

        payload.proceed()
    }
    // Then set the JWT token in the Authorization header for subgraph requests
    async fn on_subgraph_execute<'exec>(
        &'exec self,
        mut payload: OnSubgraphExecuteStartHookPayload<'exec>,
    ) -> OnSubgraphExecuteStartHookResult<'exec> {
        if let Some(ctx) = payload.context.get_ref::<JwtCookieContext>() {
            let jwt_token = match &*ctx {
                JwtCookieContext::ShouldRefresh { .. } => {
                    // This case should not happen because we refresh the token in the on_graphql_params hook, but handle it just in case
                    return payload.end_with_graphql_error(
                        GraphQLError::from_message_and_code(
                            "Unexpected state: JWT token should have been refreshed but it's not",
                            "JWT_NOT_REFRESHED",
                        ),
                        StatusCode::INTERNAL_SERVER_ERROR,
                    );
                }
                JwtCookieContext::Refreshed { ref jwt_token, .. } => jwt_token,
                JwtCookieContext::Valid { ref jwt_token } => jwt_token,
            };
            // Set the JWT token in the Authorization header for subgraph requests
            payload.execution_request.headers.insert(
                AUTHORIZATION,
                (&format!("Bearer {}", jwt_token)).try_into().unwrap(),
            );
        }
        payload.proceed()
    }
    // Then set the new cookies in the response if the token was refreshed or newly obtained
    fn on_http_request<'exec>(
        &'exec self,
        payload: OnHttpRequestHookPayload<'exec>,
    ) -> OnHttpRequestHookResult<'exec> {
        payload.on_end(|payload| {
            if let Some(ctx) = payload.context.get_ref::<JwtCookieContext>() {
                // If the token was refreshed or newly obtained, set the new JWT token and refresh token in the response cookies
                if let JwtCookieContext::Refreshed {
                    ref jwt_token,
                    ref refresh_token,
                    ref expired_at,
                } = *ctx
                {
                    // Cookie expiry is different than JWT expiry
                    // Set the new JWT token and refresh token in the response cookies
                    let set_jwt_cookie = create_set_cookie(JWT_TOKEN_NAME, jwt_token);
                    let set_refresh_cookie =
                        create_set_cookie(JWT_REFRESH_TOKEN_NAME, refresh_token);
                    let expired_at = create_set_cookie(
                        JWT_EXPIRED_AT_NAME,
                        &expired_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    );
                    return payload
                        .map_response(|mut response| {
                            let headers = response.headers_mut();
                            headers.append(SET_COOKIE, set_jwt_cookie);
                            headers.append(SET_COOKIE, set_refresh_cookie);
                            headers.append(SET_COOKIE, expired_at);
                            response
                        })
                        .proceed();
                }
            }
            payload.proceed()
        })
    }
}
