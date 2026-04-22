use serde::ser::SerializeMap;
use sonic_rs::Value;

use crate::request_context::{
    plugin_api::RequestContextPluginRead, RequestContextDomain, RequestContextError,
};

pub(crate) const CLIENT_NAME_KEY: &str = "hive::telemetry::client_name";
pub(crate) const CLIENT_VERSION_KEY: &str = "hive::telemetry::client_version";

#[derive(Debug, Clone, Default)]
pub struct TelemetryContext {
    pub client_name: Option<String>,
    pub client_version: Option<String>,
}

pub struct RequestContextTelemetryRead<'a> {
    context: &'a TelemetryContext,
}

impl RequestContextTelemetryRead<'_> {
    pub fn client_name(&self) -> Option<&String> {
        self.context.client_name.as_ref()
    }

    pub fn client_version(&self) -> Option<&String> {
        self.context.client_version.as_ref()
    }
}

impl RequestContextPluginRead {
    pub fn telemetry(&self) -> RequestContextTelemetryRead<'_> {
        RequestContextTelemetryRead {
            context: &self.snapshot.telemetry,
        }
    }
}

impl RequestContextDomain for TelemetryContext {
    const DOMAIN_PREFIX: &'static str = "hive::telemetry::";

    fn is_applicable(&self, key: &str) -> bool {
        key.starts_with(Self::DOMAIN_PREFIX)
    }

    fn serialized_len(&self) -> usize {
        usize::from(self.client_name.is_some()) + usize::from(self.client_version.is_some())
    }

    fn set_key_value(&mut self, key: &str, _value: Value) -> Result<(), RequestContextError> {
        match key {
            CLIENT_NAME_KEY => self.forbidden_mutation(key),
            CLIENT_VERSION_KEY => self.forbidden_mutation(key),
            _ => self.unknown_key(key),
        }
    }

    fn serialize_all<S: SerializeMap>(&self, map: &mut S) -> Result<(), S::Error> {
        if let Some(value) = &self.client_name {
            map.serialize_entry(CLIENT_NAME_KEY, value)?;
        }
        if let Some(value) = &self.client_version {
            map.serialize_entry(CLIENT_VERSION_KEY, value)?;
        }
        Ok(())
    }

    fn serialize_entry<S: SerializeMap>(&self, key: &str, map: &mut S) -> Result<(), S::Error> {
        match key {
            CLIENT_NAME_KEY => {
                if let Some(value) = &self.client_name {
                    map.serialize_entry(CLIENT_NAME_KEY, value)?;
                }
                Ok(())
            }
            CLIENT_VERSION_KEY => {
                if let Some(value) = &self.client_version {
                    map.serialize_entry(CLIENT_VERSION_KEY, value)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}
