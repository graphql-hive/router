use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use http::{HeaderMap, Uri};
use sonic_rs::{JsonNumberTrait, Value, ValueRef};

use crate::{
    executors::error::SubgraphExecutorError, plugin_context::PluginRequestState,
    response::subgraph_response::SubgraphResponse,
};

#[async_trait]
pub trait SubgraphExecutor {
    fn endpoint(&self) -> &Uri;
    async fn execute<'a>(
        &self,
        execution_request: SubgraphExecutionRequest<'a>,
        timeout: Option<Duration>,
        plugin_req_state: &'a Option<PluginRequestState<'a>>,
    ) -> Result<SubgraphResponse<'a>, SubgraphExecutorError>;

    fn to_boxed_arc<'a>(self) -> Arc<Box<dyn SubgraphExecutor + Send + Sync + 'a>>
    where
        Self: Sized + Send + Sync + 'a,
    {
        Arc::new(Box::new(self))
    }
}

pub type SubgraphExecutorType = dyn crate::executors::common::SubgraphExecutor + Send + Sync;

pub type SubgraphExecutorBoxedArc = Arc<Box<SubgraphExecutorType>>;

pub type SubgraphRequestExtensions = HashMap<String, Value>;

pub struct SubgraphExecutionRequest<'a> {
    pub query: &'a str,
    pub dedupe: bool,
    pub operation_name: Option<&'a str>,
    // TODO: variables could be stringified before even executing the request
    pub variables: Option<Vec<(&'a str, &'a sonic_rs::Value)>>,
    pub headers: HeaderMap,
    pub representations: Option<Vec<u8>>,
    pub extensions: Option<SubgraphRequestExtensions>,
}

impl Hash for SubgraphExecutionRequest<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.query.hash(state);
        self.operation_name.hash(state);

        hash_variables(&self.variables, state);

        self.representations.hash(state);
    }
}

fn hash_json_value<H: Hasher>(value: &sonic_rs::Value, state: &mut H) {
    hash_json_value_ref(value.as_ref(), state);
}

fn hash_json_value_ref<H: Hasher>(value: ValueRef<'_>, state: &mut H) {
    match value {
        ValueRef::Null => {
            "Null".hash(state);
        }
        ValueRef::Bool(value) => {
            "Bool".hash(state);
            value.hash(state);
        }
        ValueRef::String(value) => {
            "String".hash(state);
            value.hash(state);
        }
        ValueRef::Number(value) => {
            "Number".hash(state);
            if let Some(value) = value.as_u64() {
                "u64".hash(state);
                value.hash(state);
            } else if let Some(value) = value.as_i64() {
                "i64".hash(state);
                value.hash(state);
            } else if let Some(value) = value.as_f64() {
                "f64".hash(state);
                value.to_bits().hash(state);
            } else {
                "else".hash(state);
            }
        }
        ValueRef::Array(values) => {
            "Array".hash(state);
            values.len().hash(state);
            for value in values.iter() {
                hash_json_value_ref(value.as_ref(), state);
            }
        }
        ValueRef::Object(object) => {
            "Object".hash(state);
            object.len().hash(state);
            for (key, value) in object.iter() {
                key.hash(state);
                hash_json_value_ref(value.as_ref(), state);
            }
        }
    }
}

fn hash_variables<H: Hasher>(variables: &Option<Vec<(&str, &sonic_rs::Value)>>, state: &mut H) {
    match variables {
        None => "Variables::None".hash(state),
        Some(variables) => {
            "Variables::Some".hash(state);

            variables.len().hash(state);
            for &(name, value) in variables {
                name.hash(state);
                hash_json_value(value, state);
            }
        }
    }
}

impl SubgraphExecutionRequest<'_> {
    pub fn add_request_extensions_field(&mut self, key: String, value: Value) {
        self.extensions
            .get_or_insert_with(HashMap::new)
            .insert(key, value);
    }
}
