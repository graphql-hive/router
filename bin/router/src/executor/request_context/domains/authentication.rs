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

    fn set_key_value(&mut self, key: &str, _value: Value) -> Result<(), RequestContextError> {
        match key {
            JWT_SCOPES_KEY => self.forbidden_mutation(key),
            JWT_STATUS_KEY => self.forbidden_mutation(key),
            _ => self.unknown_key(key),
        }
    }

    super::impl_domain_serde!(
        JWT_SCOPES_KEY => jwt_scopes,
        JWT_STATUS_KEY => jwt_status,
    );
}
