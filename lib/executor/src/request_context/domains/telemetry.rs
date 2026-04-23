use serde::ser::SerializeMap;
use sonic_rs::Value;

use super::super::api::plugin::RequestContextPluginRead;
use super::RequestContextDomain;
use super::RequestContextError;

pub(crate) const CLIENT_NAME_KEY: &str = "hive::telemetry::client_name";
pub(crate) const CLIENT_VERSION_KEY: &str = "hive::telemetry::client_version";

/// Context domain for telemetry metadata.
/// This domain stores client identification.
#[derive(Debug, Clone, Default)]
pub struct TelemetryContext {
    /// The name of the client application
    pub client_name: Option<String>,
    /// The version of the client application
    pub client_version: Option<String>,
}

/// A read-only view of telemetry metadata for plugins.
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

impl<Hook> RequestContextPluginRead<Hook> {
    /// Returns the telemetry metadata for reads.
    pub fn telemetry(&self) -> RequestContextTelemetryRead<'_> {
        RequestContextTelemetryRead {
            context: &self.snapshot.telemetry,
        }
    }
}

impl RequestContextDomain for TelemetryContext {
    const DOMAIN_PREFIX: &'static str = "hive::telemetry::";

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
        self.serialize_optional_entry(map, CLIENT_NAME_KEY, self.client_name.as_ref())?;
        self.serialize_optional_entry(map, CLIENT_VERSION_KEY, self.client_version.as_ref())?;
        Ok(())
    }

    fn serialize_entry<S: SerializeMap>(&self, key: &str, map: &mut S) -> Result<(), S::Error> {
        match key {
            CLIENT_NAME_KEY => self.serialize_optional_entry(map, key, self.client_name.as_ref()),
            CLIENT_VERSION_KEY => {
                self.serialize_optional_entry(map, key, self.client_version.as_ref())
            }
            _ => Ok(()),
        }
    }
}
