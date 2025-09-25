use std::collections::HashMap;

use hive_router_plan_executor::execution::jwt_forward::JwtAuthForwardingPlan;
use jsonwebtoken::TokenData;
use serde::{Deserialize, Serialize};
use sonic_rs::Value;

use crate::jwt::errors::JwtForwardingError;

pub type JwtTokenPayload = TokenData<JwtClaims>;

#[derive(Debug, Clone)]
pub struct JwtRequestContext {
    // The payload extracted from the JWT token, and the extensions key to inject it into the request
    pub token_payload: Option<(String, JwtTokenPayload)>,
}

impl TryInto<Option<JwtAuthForwardingPlan>> for JwtRequestContext {
    type Error = JwtForwardingError;

    fn try_into(self) -> Result<Option<JwtAuthForwardingPlan>, Self::Error> {
        if let Some((extension_field_name, payload)) = &self.token_payload {
            return Ok(Some(JwtAuthForwardingPlan {
                extension_field_name: extension_field_name.clone(),
                extension_field_value: sonic_rs::to_value(&payload.claims)?,
            }));
        }

        Ok(None)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum Audience {
    Single(String),
    Multiple(Vec<String>),
}

// Based on https://datatracker.ietf.org/doc/html/rfc7519#section-4.1
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JwtClaims {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iss: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub aud: Option<Audience>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub exp: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub nbf: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub iat: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub jti: Option<String>,

    #[serde(flatten)]
    pub additional_claims: HashMap<String, Value>,
}
