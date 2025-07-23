use std::collections::HashMap;

use graphql_parser::Pos;
use graphql_tools::validation::utils::ValidationError;

use crate::{GraphQLError, GraphQLErrorLocation};

impl From<&ValidationError> for GraphQLError {
    fn from(val: &ValidationError) -> Self {
        GraphQLError {
            message: val.message.to_string(),
            locations: Some(val.locations.iter().map(|pos| pos.into()).collect()),
            path: None,
            extensions: Some(HashMap::from([(
                "code".to_string(),
                serde_json::Value::String(val.error_code.to_string()),
            )])),
        }
    }
}

impl From<&Pos> for GraphQLErrorLocation {
    fn from(val: &Pos) -> Self {
        GraphQLErrorLocation {
            line: val.line,
            column: val.column,
        }
    }
}
