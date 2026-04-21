use hive_router_query_planner::state::supergraph_state::OperationKind;
use serde::ser::SerializeMap;
use sonic_rs::{JsonValueTrait, Value};

use crate::request_context::{
    deser::RequestContextValueExt, RequestContextDomain, RequestContextError,
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

    fn set_name_value(&mut self, value: Value) -> Result<(), RequestContextError> {
        if value.is_null() {
            self.name = None;
            return Ok(());
        }

        let value = value.expect_str(OPERATION_NAME_KEY, "string or null")?;
        self.name = Some(value.to_string());

        Ok(())
    }

    fn set_kind_value(&mut self, value: Value) -> Result<(), RequestContextError> {
        if value.is_null() {
            self.kind = None;
            return Ok(());
        }

        let value = value.expect_str(OPERATION_KIND_KEY, "string or null")?;
        self.kind =
            Some(
                value
                    .try_into()
                    .map_err(|_| RequestContextError::InvalidOperationKind {
                        value: value.to_string(),
                    })?,
            );

        Ok(())
    }
}

impl RequestContextDomain for OperationContext {
    const PREFIX: &'static str = "hive::operation::";

    fn set_key_value(&mut self, key: &str, value: Value) -> Result<(), RequestContextError> {
        match key {
            OPERATION_NAME_KEY => {
                self.set_name_value(value)?;
                Ok(())
            }
            OPERATION_KIND_KEY => {
                self.set_kind_value(value)?;
                Ok(())
            }
            _ => Err(RequestContextError::UnknownReservedKey {
                key: key.to_string(),
            }),
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

    fn is_mutable(key: &str) -> Option<bool> {
        match key {
            OPERATION_NAME_KEY | OPERATION_KIND_KEY => Some(false),
            _ => None,
        }
    }
}
