use crate::response::value::Value as ResponseValue;
use hive_router_config::coprocessor::ContextSelection;
use operation::OperationContext;
use serde::ser::SerializeMap;
use sonic_rs::Value;
use std::sync::{Arc, Mutex, MutexGuard};

mod deser;
mod error;
mod operation;
mod web;

pub use error::RequestContextError;
pub use web::RequestContextExt;

const HIVE_PREFIX: &str = "hive::";

pub(crate) trait RequestContextDomain {
    const PREFIX: &'static str;
    fn set_key_value(&mut self, key: &str, value: Value) -> Result<(), RequestContextError>;
    fn serialize_all<S: SerializeMap>(&self, map: &mut S) -> Result<(), S::Error>;
    fn serialize_entry<S: SerializeMap>(&self, key: &str, map: &mut S) -> Result<(), S::Error>;
    fn is_mutable(key: &str) -> Option<bool>
    where
        Self: Sized;
}

#[derive(Debug, Clone, Default)]
pub struct RequestContext {
    pub operation: OperationContext,
    pub custom: CustomContext,
}

#[derive(Debug, Clone, Default)]
pub struct CustomContext(Vec<(String, Value)>);

impl CustomContext {
    fn get(&self, key: &str) -> Option<&Value> {
        self.0
            .iter()
            .find(|(custom_key, _)| custom_key == key)
            .map(|(_, value)| value)
    }

    fn iter(&self) -> impl Iterator<Item = (&String, &Value)> {
        self.0.iter().map(|(key, value)| (key, value))
    }

    fn apply(&mut self, key: &str, value: ResponseValue<'_>) {
        if value.is_null() {
            // Remove
            self.0.retain(|(current_key, _)| current_key != key);
            return;
        }

        let value = value.as_ref().into();
        if let Some((_, current)) = self.0.iter_mut().find(|(name, _)| *name == key) {
            // Update existing value
            *current = value;
            return;
        }
        // Insert new value
        self.0.push((key.to_string(), value));
    }

    fn size(&self) -> usize {
        self.0.len()
    }
}

#[derive(Default, Debug, Clone)]
pub struct SharedRequestContext(Arc<Mutex<RequestContext>>);

impl SharedRequestContext {
    pub fn lock(&self) -> Result<MutexGuard<'_, RequestContext>, RequestContextError> {
        self.0.lock().map_err(|_| RequestContextError::LockPoison)
    }
    pub fn snapshot(&self) -> Result<RequestContext, RequestContextError> {
        Ok(self.lock()?.clone())
    }

    pub fn update(&self, f: impl FnOnce(&mut RequestContext)) -> Result<(), RequestContextError> {
        let mut context = self.lock()?;
        f(&mut context);
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct RequestContextPatch<'a> {
    entries: Vec<(&'a str, ResponseValue<'a>)>,
}

pub struct RequestContextFacade<'a> {
    context: &'a mut RequestContext,
}

pub struct SelectedRequestContext<'a> {
    context: &'a RequestContext,
    selection: &'a ContextSelection,
}

impl RequestContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn facade(&mut self) -> RequestContextFacade<'_> {
        RequestContextFacade::new(self)
    }

    pub fn as_selected<'a>(
        &'a self,
        selection: &'a ContextSelection,
    ) -> SelectedRequestContext<'a> {
        SelectedRequestContext {
            context: self,
            selection,
        }
    }

    fn set_reserved_internal(
        &mut self,
        key: &str,
        value: ResponseValue,
    ) -> Result<(), RequestContextError> {
        if key.starts_with(OperationContext::PREFIX) {
            self.operation.set_key_value(key, value.as_ref().into())?;
            return Ok(());
        }

        Err(RequestContextError::UnknownReservedKey {
            key: key.to_string(),
        })
    }
}

impl RequestContextFacade<'_> {
    pub fn new(context: &mut RequestContext) -> RequestContextFacade<'_> {
        RequestContextFacade { context }
    }

    pub fn set(&mut self, key: &str, value: ResponseValue<'_>) -> Result<(), RequestContextError> {
        if key.starts_with(HIVE_PREFIX) {
            let Some(mutable) = OperationContext::is_mutable(key) else {
                return Err(RequestContextError::UnknownReservedKey {
                    key: key.to_string(),
                });
            };

            if !mutable {
                return Err(RequestContextError::ForbiddenReservedMutation {
                    key: key.to_string(),
                });
            }

            return self.context.set_reserved_internal(key, value);
        }

        self.context.custom.apply(key, value);

        Ok(())
    }

    pub fn unset(&mut self, key: &str) -> Result<(), RequestContextError> {
        self.set(key, ResponseValue::Null)
    }

    pub fn apply_patch(&mut self, patch: RequestContextPatch) -> Result<(), RequestContextError> {
        for (key, value) in patch.entries {
            self.set(key, value)?;
        }

        Ok(())
    }
}
