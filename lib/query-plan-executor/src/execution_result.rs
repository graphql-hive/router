use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::GraphQLError;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionResultExtensions {
    code: Option<String>,
    http: Option<HTTPErrorExtensions>,
    service_name: Option<String>,
    #[serde(flatten)]
    extensions: Option<Map<String, Value>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HTTPErrorExtensions {
    status: Option<u16>,
    headers: Option<HashMap<String, String>>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ExecutionResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<GraphQLError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Map<String, Value>>,
}

impl ExecutionResult {
    pub fn from_error_message(message: String) -> ExecutionResult {
        ExecutionResult {
            data: None,
            errors: Some(vec![GraphQLError {
                message,
                locations: None,
                path: None,
                extensions: None,
            }]),
            extensions: None,
        }
    }
    pub fn new(
        data: Option<Value>,
        errors: Option<Vec<GraphQLError>>,
        extensions: Option<Map<String, Value>>,
    ) -> ExecutionResult {
        let final_data = match data {
            Some(data) if data.is_null() => None,
            _ => data,
        };
        let final_errors = match errors {
            Some(errors) if errors.is_empty() => None,
            _ => errors,
        };
        let final_extensions = match extensions {
            Some(extensions) if extensions.is_empty() => None,
            _ => extensions,
        };
        ExecutionResult {
            data: final_data,
            errors: final_errors,
            extensions: final_extensions,
        }
    }
}
