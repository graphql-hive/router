use sonic_rs::Value;

use crate::execution::client_request_details::JwtRequestDetails;

#[derive(Default, Clone)]
pub struct JwtAuthForwardingPlan {
    pub extension_field_name: String,
    pub extension_field_value: Value,
}

impl JwtRequestDetails {
    pub fn build_forwarding_plan(
        &self,
        extension_field_name: &str,
    ) -> Result<Option<JwtAuthForwardingPlan>, JwtForwardingError> {
        Ok(match self {
            JwtRequestDetails::Authenticated { claims, .. } => Some(JwtAuthForwardingPlan {
                extension_field_name: extension_field_name.to_string(),
                extension_field_value: sonic_rs::to_value(&claims)?,
            }),
            _ => None,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum JwtForwardingError {
    #[error("failed to serialized jwt claims")]
    ClaimsSerializeError(#[from] sonic_rs::Error),
    #[error("failed to parse  as valid header value")]
    ValueIsNotValidHeader(#[from] http::header::InvalidHeaderValue),
}
