use sonic_rs::{JsonValueTrait, Object, Value};

use crate::executor::execution::client_request_details::JwtRequestDetails;

#[derive(Default)]
pub struct JwtAuthForwardingPlan {
    pub extension_field_name: String,
    pub extension_field_value: Value,
}

impl JwtRequestDetails {
    pub fn build_forwarding_plan(
        &self,
        extension_field_name: &str,
        include_claims: Option<&[String]>,
    ) -> Result<Option<JwtAuthForwardingPlan>, JwtForwardingError> {
        Ok(match self {
            JwtRequestDetails::Authenticated { claims, .. } => Some(JwtAuthForwardingPlan {
                extension_field_name: extension_field_name.to_string(),
                extension_field_value: filter_claims(claims, include_claims),
            }),
            _ => None,
        })
    }
}

fn filter_claims(claims: &Value, include_claims: Option<&[String]>) -> Value {
    match include_claims {
        Some(keys) => keys
            .iter()
            .filter_map(|key| claims.get(key.as_str()).map(|value| (key.clone(), value)))
            .collect::<Object>()
            .into(),
        None => claims.clone(),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum JwtForwardingError {
    #[error("failed to serialized jwt claims")]
    ClaimsSerializeError(#[from] sonic_rs::Error),
    #[error("failed to parse  as valid header value")]
    ValueIsNotValidHeader(#[from] http::header::InvalidHeaderValue),
}
