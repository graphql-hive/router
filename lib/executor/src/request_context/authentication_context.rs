use std::collections::HashSet;

use serde::ser::SerializeMap;
use sonic_rs::Value;

use crate::request_context::{
    plugin_api::RequestContextPluginRead, RequestContextDomain, RequestContextError,
};

pub(crate) const JWT_SCOPES_KEY: &str = "hive::authentication::jwt_scopes";
pub(crate) const JWT_STATUS_KEY: &str = "hive::authentication::jwt_status";

#[derive(Debug, Clone, Default)]
pub struct AuthenticationContext {
    pub jwt_scopes: Option<HashSet<String>>,
    pub jwt_status: Option<bool>,
}

pub struct RequestContextAuthenticationRead<'a> {
    context: &'a AuthenticationContext,
}

impl RequestContextAuthenticationRead<'_> {
    pub fn jwt_scopes(&self) -> Option<&HashSet<String>> {
        self.context.jwt_scopes.as_ref()
    }

    pub fn jwt_status(&self) -> Option<&bool> {
        self.context.jwt_status.as_ref()
    }
}

impl<Caps> RequestContextPluginRead<Caps> {
    pub fn authentication(&self) -> RequestContextAuthenticationRead<'_> {
        RequestContextAuthenticationRead {
            context: &self.snapshot.authentication,
        }
    }
}

impl RequestContextDomain for AuthenticationContext {
    const DOMAIN_PREFIX: &'static str = "hive::authentication::";

    fn is_applicable(&self, key: &str) -> bool {
        key.starts_with(Self::DOMAIN_PREFIX)
    }

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
        if let Some(value) = &self.jwt_scopes {
            map.serialize_entry(JWT_SCOPES_KEY, value)?;
        }
        if let Some(value) = &self.jwt_status {
            map.serialize_entry(JWT_STATUS_KEY, value)?;
        }
        Ok(())
    }

    fn serialize_entry<S: SerializeMap>(&self, key: &str, map: &mut S) -> Result<(), S::Error> {
        match key {
            JWT_SCOPES_KEY => {
                if let Some(value) = &self.jwt_scopes {
                    map.serialize_entry(JWT_SCOPES_KEY, value)?;
                }
                Ok(())
            }
            JWT_STATUS_KEY => {
                if let Some(value) = &self.jwt_status {
                    map.serialize_entry(JWT_STATUS_KEY, value)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}
