use crate::{
    request_context::telemetry_context::TelemetryContext, response::value::Value as ResponseValue,
};
use authentication_context::AuthenticationContext;
use hive_router_config::coprocessor::ContextSelection;
use operation_context::OperationContext;
use progressive_override_context::ProgressiveOverrideContext;
use serde::ser::SerializeMap;
use sonic_rs::Value;
use std::sync::{Arc, Mutex, MutexGuard};

mod authentication_context;
mod coprocessor_api;
mod deser;
mod error;
mod operation_context;
mod plugin_api;
mod progressive_override_context;
mod telemetry_context;
mod web;

pub use coprocessor_api::RequestContextPatch;
pub use error::RequestContextError;
pub use plugin_api::RequestContextPluginApi;
pub use web::RequestContextExt;

const HIVE_PREFIX: &str = "hive::";

pub(crate) trait RequestContextDomain {
    const DOMAIN_PREFIX: &'static str;
    fn is_applicable(&self, key: &str) -> bool;
    fn set_key_value(&mut self, key: &str, value: Value) -> Result<(), RequestContextError>;
    fn serialize_all<S: SerializeMap>(&self, map: &mut S) -> Result<(), S::Error>;
    fn serialize_entry<S: SerializeMap>(&self, key: &str, map: &mut S) -> Result<(), S::Error>;
    fn serialized_len(&self) -> usize;

    fn forbidden_mutation(&self, key: &str) -> Result<(), RequestContextError> {
        Err(RequestContextError::ForbiddenReservedMutation {
            key: key.to_string(),
        })
    }

    fn unknown_key(&self, key: &str) -> Result<(), RequestContextError> {
        Err(RequestContextError::UnknownReservedKey {
            key: key.to_string(),
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct RequestContext {
    pub operation: OperationContext,
    pub progressive_override: ProgressiveOverrideContext,
    pub authentication: AuthenticationContext,
    pub telemetry: TelemetryContext,
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

pub struct SelectedRequestContext<'a> {
    context: &'a RequestContext,
    selection: &'a ContextSelection,
}

impl RequestContext {
    pub fn new() -> Self {
        Self::default()
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
}
