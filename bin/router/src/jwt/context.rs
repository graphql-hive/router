use std::{collections::HashMap, sync::Arc};

use hive_router_plan_executor::execution::jwt_forward::JwtForwardingError;
use jsonwebtoken::TokenData;
use serde::{Deserialize, Serialize};

pub type JwtTokenPayload = TokenData<JwtClaims>;

#[derive(Debug, Clone)]
pub struct JwtRequestContext {
    pub token_prefix: Option<String>,
    pub token_raw: String,
    pub token_payload: Arc<JwtTokenPayload>,
    /// The name of the claim to read authorization scopes from. Defaults to "scope"/"scopes" when `None`.
    pub scopes_claim: Option<String>,
}

impl JwtRequestContext {
    pub fn get_claims_value(&self) -> Result<sonic_rs::Value, JwtForwardingError> {
        Ok(sonic_rs::to_value(&self.token_payload.claims)?)
    }

    /// Extracts an optional scopes field from the token's payload, read from the claim named by
    /// `scopes_claim`, or from "scope" (falling back to "scopes") when `scopes_claim` is `None`.
    /// Supports both space-delimited and array formats.
    pub fn extract_scopes(&self) -> Option<Vec<String>> {
        let map = &self.token_payload.claims.additional_claims;
        let maybe_scopes = match &self.scopes_claim {
            Some(claim_name) => map.get(claim_name.as_str()),
            None => map.get("scope").or_else(|| map.get("scopes")),
        };

        if let Some(serde_json::Value::String(scopes_str)) = maybe_scopes {
            return Some(scopes_str.split(' ').map(String::from).collect());
        }

        if let Some(serde_json::Value::Array(scopes_arr)) = maybe_scopes {
            return Some(
                scopes_arr
                    .iter()
                    .filter_map(|s| s.as_str())
                    .map(String::from)
                    .collect::<Vec<_>>(),
            );
        }

        None
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

    // we are using serde to deserialize the additional claims
    // because the jsonwebtoken crate is using `serde_json` internally, and the `sonic_rs::Value` is not recognized as valid type
    #[serde(flatten)]
    pub additional_claims: HashMap<String, serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use jsonwebtoken::Header;
    use serde_json::json;

    use super::*;

    fn context_with_claims(
        additional_claims: HashMap<String, serde_json::Value>,
        scopes_claim: Option<String>,
    ) -> JwtRequestContext {
        JwtRequestContext {
            token_prefix: None,
            token_raw: "token".to_string(),
            scopes_claim,
            token_payload: Arc::new(TokenData {
                header: Header::default(),
                claims: JwtClaims {
                    iss: None,
                    sub: None,
                    aud: None,
                    exp: None,
                    nbf: None,
                    iat: None,
                    jti: None,
                    additional_claims,
                },
            }),
        }
    }

    #[test]
    fn extracts_default_scope_claim_when_scopes_claim_not_configured() {
        let claims = HashMap::from([("scope".to_string(), json!("read:foo write:bar"))]);
        let ctx = context_with_claims(claims, None);

        assert_eq!(
            ctx.extract_scopes(),
            Some(vec!["read:foo".to_string(), "write:bar".to_string()])
        );
    }

    #[test]
    fn extracts_configured_claim_name() {
        let claims = HashMap::from([("roles".to_string(), json!(["Admin", "Reader"]))]);
        let ctx = context_with_claims(claims, Some("roles".to_string()));

        assert_eq!(
            ctx.extract_scopes(),
            Some(vec!["Admin".to_string(), "Reader".to_string()])
        );
    }

    #[test]
    fn ignores_default_scope_claim_when_a_different_claim_is_configured() {
        let claims = HashMap::from([("scope".to_string(), json!("read:foo"))]);
        let ctx = context_with_claims(claims, Some("roles".to_string()));

        assert_eq!(ctx.extract_scopes(), None);
    }
}
