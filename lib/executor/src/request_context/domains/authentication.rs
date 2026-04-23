use std::collections::HashSet;

use serde::ser::SerializeMap;
use sonic_rs::Value;

use super::super::api::plugin::RequestContextPluginRead;
use super::RequestContextDomain;
use super::RequestContextError;

pub(crate) const JWT_SCOPES_KEY: &str = "hive::authentication::jwt_scopes";
pub(crate) const JWT_STATUS_KEY: &str = "hive::authentication::jwt_status";

/// Context domain for authentication state.
#[derive(Debug, Clone, Default)]
pub struct AuthenticationContext {
    /// Scopes extracted from the current authenticated user's JWT.
    pub jwt_scopes: Option<HashSet<String>>,
    /// Authentication status. If `Some(true)`, the request has been verified as authenticated.
    pub jwt_status: Option<bool>,
}

/// A read-only view of authentication state for plugins.
pub struct RequestContextAuthenticationRead<'a> {
    context: &'a AuthenticationContext,
}

impl RequestContextAuthenticationRead<'_> {
    /// Returns the authenticated user's scopes if present.
    pub fn jwt_scopes(&self) -> Option<&HashSet<String>> {
        self.context.jwt_scopes.as_ref()
    }

    /// Returns the authentication status if known.
    pub fn jwt_status(&self) -> Option<&bool> {
        self.context.jwt_status.as_ref()
    }
}

impl<Hook> RequestContextPluginRead<Hook> {
    /// Returns the authentication read API.
    pub fn authentication(&self) -> RequestContextAuthenticationRead<'_> {
        RequestContextAuthenticationRead {
            context: &self.snapshot.authentication,
        }
    }
}

impl RequestContextDomain for AuthenticationContext {
    const DOMAIN_PREFIX: &'static str = "hive::authentication::";

    fn serialized_len(&self) -> usize {
        usize::from(self.jwt_scopes.is_some()) + usize::from(self.jwt_status.is_some())
    }

    fn set_key_value(&mut self, key: &str, _value: Value) -> Result<(), RequestContextError> {
        match key {
            JWT_SCOPES_KEY => self.forbidden_mutation(key),
            JWT_STATUS_KEY => self.forbidden_mutation(key),
            _ => self.unknown_key(key),
        }
    }

    fn serialize_all<S: SerializeMap>(&self, map: &mut S) -> Result<(), S::Error> {
        self.serialize_optional_entry(map, JWT_SCOPES_KEY, self.jwt_scopes.as_ref())?;
        self.serialize_optional_entry(map, JWT_STATUS_KEY, self.jwt_status.as_ref())?;
        Ok(())
    }

    fn serialize_entry<S: SerializeMap>(&self, key: &str, map: &mut S) -> Result<(), S::Error> {
        match key {
            JWT_SCOPES_KEY => self.serialize_optional_entry(map, key, self.jwt_scopes.as_ref()),
            JWT_STATUS_KEY => self.serialize_optional_entry(map, key, self.jwt_status.as_ref()),
            _ => Ok(()),
        }
    }
}
