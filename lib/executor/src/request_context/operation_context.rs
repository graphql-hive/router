use hive_router_query_planner::state::supergraph_state::OperationKind;
use serde::ser::SerializeMap;
use sonic_rs::Value;

use crate::request_context::{
    plugin_api::RequestContextPluginRead, RequestContextDomain, RequestContextError,
};

pub(crate) const OPERATION_NAME_KEY: &str = "hive::operation::name";
pub(crate) const OPERATION_KIND_KEY: &str = "hive::operation::kind";

#[derive(Debug, Clone, Default)]
pub struct OperationContext {
    pub name: Option<String>,
    pub kind: Option<OperationKind>,
}

impl OperationContext {
    pub fn update(&mut self, name: Option<String>, kind: Option<OperationKind>) {
        self.name = name;
        self.kind = kind;
    }
}

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

impl<Caps> RequestContextPluginRead<Caps> {
    pub fn operation(&self) -> RequestContextOperationRead<'_> {
        RequestContextOperationRead {
            context: &self.snapshot.operation,
        }
    }
}

impl RequestContextDomain for OperationContext {
    const DOMAIN_PREFIX: &'static str = "hive::operation::";

    fn is_applicable(&self, key: &str) -> bool {
        key.starts_with(Self::DOMAIN_PREFIX)
    }

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
        if let Some(name) = &self.name {
            map.serialize_entry(OPERATION_NAME_KEY, name)?;
        }
        if let Some(kind) = &self.kind {
            map.serialize_entry(OPERATION_KIND_KEY, &kind.to_string())?;
        }
        Ok(())
    }

    fn serialize_entry<S: SerializeMap>(&self, key: &str, map: &mut S) -> Result<(), S::Error> {
        match key {
            OPERATION_NAME_KEY => {
                if let Some(name) = &self.name {
                    map.serialize_entry(OPERATION_NAME_KEY, name)?;
                }
                Ok(())
            }
            OPERATION_KIND_KEY => {
                if let Some(kind) = &self.kind {
                    map.serialize_entry(OPERATION_KIND_KEY, &kind.to_string())?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}
