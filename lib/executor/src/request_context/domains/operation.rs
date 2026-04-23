use hive_router_query_planner::state::supergraph_state::OperationKind;
use serde::ser::SerializeMap;
use sonic_rs::Value;

use super::super::api::plugin::RequestContextPluginRead;
use super::RequestContextDomain;
use super::RequestContextError;

pub(crate) const OPERATION_NAME_KEY: &str = "hive::operation::name";
pub(crate) const OPERATION_KIND_KEY: &str = "hive::operation::kind";

/// Context domain for GraphQL operation metadata.
///
/// This domain stores details about the operation extracted from the request.
#[derive(Debug, Clone, Default)]
pub struct OperationContext {
    /// The name of the GraphQL operation.
    pub name: Option<String>,
    /// The kind of the GraphQL operation ("query", "mutation", "subscription").
    pub kind: Option<OperationKind>,
}

impl OperationContext {
    /// Updates the operation metadata in a single call.
    pub fn update(&mut self, name: Option<String>, kind: Option<OperationKind>) {
        self.name = name;
        self.kind = kind;
    }
}

/// A read-only view of the operation metadata for plugins.
pub struct RequestContextOperationRead<'a> {
    context: &'a OperationContext,
}

impl RequestContextOperationRead<'_> {
    pub fn name(&self) -> Option<&String> {
        self.context.name.as_ref()
    }

    pub fn kind(&self) -> Option<&OperationKind> {
        self.context.kind.as_ref()
    }
}

impl<Hook> RequestContextPluginRead<Hook> {
    /// Returns the operation metadata for reads.
    pub fn operation(&self) -> RequestContextOperationRead<'_> {
        RequestContextOperationRead {
            context: &self.snapshot.operation,
        }
    }
}

impl RequestContextDomain for OperationContext {
    const DOMAIN_PREFIX: &'static str = "hive::operation::";

    fn serialized_len(&self) -> usize {
        usize::from(self.name.is_some()) + usize::from(self.kind.is_some())
    }

    fn set_key_value(&mut self, key: &str, _value: Value) -> Result<(), RequestContextError> {
        match key {
            OPERATION_NAME_KEY => self.forbidden_mutation(key),
            OPERATION_KIND_KEY => self.forbidden_mutation(key),
            _ => self.unknown_key(key),
        }
    }

    fn serialize_all<S: SerializeMap>(&self, map: &mut S) -> Result<(), S::Error> {
        self.serialize_optional_entry(map, OPERATION_NAME_KEY, self.name.as_ref())?;
        self.serialize_optional_entry(map, OPERATION_KIND_KEY, self.kind.as_ref())?;
        Ok(())
    }

    fn serialize_entry<S: SerializeMap>(&self, key: &str, map: &mut S) -> Result<(), S::Error> {
        match key {
            OPERATION_NAME_KEY => self.serialize_optional_entry(map, key, self.name.as_ref()),
            OPERATION_KIND_KEY => self.serialize_optional_entry(map, key, self.kind.as_ref()),
            _ => Ok(()),
        }
    }
}
