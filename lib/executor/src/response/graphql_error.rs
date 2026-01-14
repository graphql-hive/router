use core::fmt;
use graphql_tools::parser::Pos;
use graphql_tools::validation::utils::ValidationError;
use serde::{de, Deserialize, Deserializer, Serialize};
use sonic_rs::Value;
use std::collections::HashMap;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphQLError {
    pub message: String,
    #[serde(default, skip_serializing_if = "is_none_or_empty")]
    pub locations: Option<Vec<GraphQLErrorLocation>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<GraphQLErrorPath>,
    #[serde(default, skip_serializing_if = "GraphQLErrorExtensions::is_empty")]
    pub extensions: GraphQLErrorExtensions,
}

fn is_none_or_empty<T>(opt: &Option<Vec<T>>) -> bool {
    opt.as_ref().is_none_or(|v| v.is_empty())
}

impl From<String> for GraphQLError {
    fn from(message: String) -> Self {
        GraphQLError {
            message,
            locations: None,
            path: None,
            extensions: GraphQLErrorExtensions::default(),
        }
    }
}

impl From<&str> for GraphQLError {
    fn from(message: &str) -> Self {
        GraphQLError {
            message: message.to_string(),
            locations: None,
            path: None,
            extensions: GraphQLErrorExtensions::default(),
        }
    }
}

impl From<&ValidationError> for GraphQLError {
    fn from(val: &ValidationError) -> Self {
        GraphQLError {
            message: val.message.to_string(),
            locations: Some(val.locations.iter().map(|pos| pos.into()).collect()),
            path: None,
            extensions: GraphQLErrorExtensions::new_from_code("GRAPHQL_VALIDATION_FAILED"),
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

impl GraphQLError {
    pub fn entity_index_and_path<'a>(&'a self) -> Option<EntityIndexAndPath<'a>> {
        self.path.as_ref().and_then(|p| p.entity_index_and_path())
    }

    pub fn normalize_entity_error(
        self,
        entity_index_error_map: &HashMap<&usize, Vec<GraphQLErrorPath>>,
    ) -> Vec<GraphQLError> {
        if let Some(entity_index_and_path) = &self.entity_index_and_path() {
            if let Some(entity_error_paths) =
                entity_index_error_map.get(&entity_index_and_path.entity_index)
            {
                return entity_error_paths
                    .iter()
                    .map(|error_path| {
                        let mut new_error_path = error_path.clone();
                        new_error_path.extend_from_slice(entity_index_and_path.rest_of_path);
                        GraphQLError {
                            path: Some(new_error_path),
                            ..self.clone()
                        }
                    })
                    .collect();
            }
        }
        vec![self]
    }
    /// Creates a GraphQLError with the given message and extensions.
    /// Example:
    /// ```rust
    /// use hive_router_plan_executor::response::graphql_error::GraphQLError;
    /// use hive_router_plan_executor::response::graphql_error::GraphQLErrorExtensions;
    /// use sonic_rs::json;
    ///
    /// let extensions = GraphQLErrorExtensions {
    ///     code: Some("SOME_ERROR_CODE".to_string()),
    ///     service_name: None,
    ///     affected_path: None,
    ///     extensions: std::collections::HashMap::new(),
    /// };
    ///
    /// let error = GraphQLError::from_message_and_extensions("An error occurred", extensions);
    ///
    /// assert_eq!(json!(error), json!({
    ///     "message": "An error occurred",
    ///     "extensions": {
    ///         "code": "SOME_ERROR_CODE"
    ///     }
    /// }));
    /// ```
    pub fn from_message_and_extensions<TMessage: Into<String>>(
        message: TMessage,
        extensions: GraphQLErrorExtensions,
    ) -> Self {
        GraphQLError {
            message: message.into(),
            locations: None,
            path: None,
            extensions,
        }
    }
    /// Creates a GraphQLError with the given message and code in extensions.
    /// Example:
    /// ```rust
    /// use hive_router_plan_executor::response::graphql_error::GraphQLError;
    /// use sonic_rs::json;
    ///
    /// let error = GraphQLError::from_message_and_code("An error occurred", "SOME_ERROR_CODE");
    ///
    /// assert_eq!(json!(error), json!({
    ///     "message": "An error occurred",
    ///     "extensions": {
    ///         "code": "SOME_ERROR_CODE"
    ///     }
    /// }));
    /// ```
    pub fn from_message_and_code<TMessage: Into<String>, TCode: Into<String>>(
        message: TMessage,
        code: TCode,
    ) -> Self {
        GraphQLError {
            message: message.into(),
            locations: None,
            path: None,
            extensions: GraphQLErrorExtensions::new_from_code(code),
        }
    }

    /// Adds subgraph name and error code `DOWNSTREAM_SERVICE_ERROR` to the extensions.
    /// Example:
    /// ```rust
    /// use hive_router_plan_executor::response::graphql_error::GraphQLError;
    /// use sonic_rs::json;
    ///
    /// let error = GraphQLError::from("An error occurred")
    ///     .add_subgraph_name("users");
    ///
    /// assert_eq!(json!(error), json!({
    ///     "message": "An error occurred",
    ///     "extensions": {
    ///         "serviceName": "users",
    ///         "code": "DOWNSTREAM_SERVICE_ERROR"
    ///     }
    /// }));
    /// ```
    pub fn add_subgraph_name<TStr: Into<String>>(mut self, subgraph_name: TStr) -> Self {
        self.extensions
            .service_name
            .get_or_insert(subgraph_name.into());
        self.extensions
            .code
            .get_or_insert("DOWNSTREAM_SERVICE_ERROR".to_string());
        self
    }

    /// Adds affected path to the extensions.
    /// Example:
    /// ```rust
    /// use hive_router_plan_executor::response::graphql_error::GraphQLError;
    /// use sonic_rs::json;
    ///
    /// let error = GraphQLError::from("An error occurred")
    ///     .add_affected_path("user.friends[0].name");
    ///
    /// assert_eq!(json!(error), json!({
    ///     "message": "An error occurred",
    ///     "extensions": {
    ///         "affectedPath": "user.friends[0].name"
    ///     }
    /// }));
    /// ```
    pub fn add_affected_path<TStr: Into<String>>(mut self, affected_path: TStr) -> Self {
        self.extensions.affected_path = Some(affected_path.into());
        self
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GraphQLErrorLocation {
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum GraphQLErrorPathSegment {
    String(String),
    Index(usize),
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(transparent)]
pub struct GraphQLErrorPath {
    pub segments: Vec<GraphQLErrorPathSegment>,
}

pub struct EntityIndexAndPath<'a> {
    pub entity_index: usize,
    pub rest_of_path: &'a [GraphQLErrorPathSegment],
}

impl GraphQLErrorPath {
    pub fn with_capacity(capacity: usize) -> Self {
        GraphQLErrorPath {
            segments: Vec::with_capacity(capacity),
        }
    }
    pub fn concat(&self, segment: GraphQLErrorPathSegment) -> Self {
        let mut new_path = self.segments.clone();
        new_path.push(segment);
        GraphQLErrorPath { segments: new_path }
    }

    pub fn concat_index(&self, index: usize) -> Self {
        self.concat(GraphQLErrorPathSegment::Index(index))
    }

    pub fn concat_str(&self, field: String) -> Self {
        self.concat(GraphQLErrorPathSegment::String(field))
    }

    pub fn extend_from_slice(&mut self, other: &[GraphQLErrorPathSegment]) {
        self.segments.extend_from_slice(other);
    }

    pub fn entity_index_and_path<'a>(&'a self) -> Option<EntityIndexAndPath<'a>> {
        match &self.segments.as_slice() {
            [GraphQLErrorPathSegment::String(maybe_entities), GraphQLErrorPathSegment::Index(entity_index), rest_of_path @ ..]
                if maybe_entities == "_entities" =>
            {
                Some(EntityIndexAndPath {
                    entity_index: *entity_index,
                    rest_of_path,
                })
            }
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GraphQLErrorExtensions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_name: Option<String>,
    /// Corresponds to a path of a Flatten(Fetch) node that caused the error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affected_path: Option<String>,
    #[serde(flatten)]
    pub extensions: HashMap<String, Value>,
}

// Workaround for https://github.com/cloudwego/sonic-rs/issues/114

impl<'de> Deserialize<'de> for GraphQLErrorExtensions {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct GraphQLErrorExtensionsVisitor;

        impl<'de> de::Visitor<'de> for GraphQLErrorExtensionsVisitor {
            type Value = GraphQLErrorExtensions;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a map for GraphQLErrorExtensions")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut code = None;
                let mut service_name = None;
                let mut affected_path = None;
                let mut extensions = HashMap::new();

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "code" => {
                            if code.is_some() {
                                return Err(de::Error::duplicate_field("code"));
                            }
                            code = Some(map.next_value()?);
                        }
                        "serviceName" => {
                            if service_name.is_some() {
                                return Err(de::Error::duplicate_field("serviceName"));
                            }
                            service_name = Some(map.next_value()?);
                        }
                        "affectedPath" => {
                            if affected_path.is_some() {
                                return Err(de::Error::duplicate_field("affectedPath"));
                            }
                            affected_path = map.next_value()?;
                        }
                        other_key => {
                            let value: Value = map.next_value()?;
                            extensions.insert(other_key.to_string(), value);
                        }
                    }
                }

                Ok(GraphQLErrorExtensions {
                    code,
                    service_name,
                    affected_path,
                    extensions,
                })
            }
        }

        deserializer.deserialize_map(GraphQLErrorExtensionsVisitor)
    }
}

impl GraphQLErrorExtensions {
    pub fn new_from_code<TCode: Into<String>>(code: TCode) -> Self {
        GraphQLErrorExtensions {
            code: Some(code.into()),
            service_name: None,
            affected_path: None,
            extensions: HashMap::new(),
        }
    }

    pub fn new_from_code_and_service_name<TCode: Into<String>, TServiceName: Into<String>>(
        code: TCode,
        service_name: TServiceName,
    ) -> Self {
        GraphQLErrorExtensions {
            code: Some(code.into()),
            service_name: Some(service_name.into()),
            affected_path: None,
            extensions: HashMap::new(),
        }
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.extensions.get(key)
    }

    pub fn set(&mut self, key: String, value: Value) {
        self.extensions.insert(key, value);
    }

    pub fn is_empty(&self) -> bool {
        self.code.is_none()
            && self.service_name.is_none()
            && self.affected_path.is_none()
            && self.extensions.is_empty()
    }
}
