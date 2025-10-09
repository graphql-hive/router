use std::collections::HashMap;

use jsonwebtoken::TokenData;
use serde::{Deserialize, Serialize};
use sonic_rs::Value;

pub type TokenPayload = TokenData<JwtClaims>;

#[allow(dead_code)] // TODO: Remove this when we actually use this and integrate with header propagation
pub struct JwtRequestContext {
    pub token: String,
    pub payload: TokenPayload,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum Audience {
    Single(String),
    Multiple(Vec<String>),
}

// Based on https://datatracker.ietf.org/doc/html/rfc7519#section-4.1
#[derive(Debug, Serialize, Deserialize)]
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
