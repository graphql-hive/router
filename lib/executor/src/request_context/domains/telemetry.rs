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

    fn set_key_value(&mut self, key: &str, _value: Value) -> Result<(), RequestContextError> {
        match key {
            CLIENT_NAME_KEY => self.forbidden_mutation(key),
            CLIENT_VERSION_KEY => self.forbidden_mutation(key),
            _ => self.unknown_key(key),
        }
    }

    super::impl_domain_serde!(
        CLIENT_NAME_KEY => client_name,
        CLIENT_VERSION_KEY => client_version,
    );
}
