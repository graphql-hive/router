use std::collections::HashSet;

use serde::ser::SerializeMap;
use sonic_rs::{JsonValueTrait, Value};

use super::super::api::plugin::{RequestContextPluginRead, RequestContextPluginWrite};
use super::super::deser::RequestContextValueExt;
use super::RequestContextDomain;
use super::RequestContextError;
use crate::hooks;

pub trait CanWriteAuthorization {}
impl CanWriteAuthorization for hooks::OnGraphqlAnalysis {}

pub(crate) const REQUIRED_POLICIES_KEY: &str = "hive::authorization::required_policies";
pub(crate) const GRANTED_POLICIES_KEY: &str = "hive::authorization::granted_policies";

/// Context domain for custom authorization policies (`@policy`).
///
/// The router publishes the policies the current operation depends on in
/// `required_policies`, and a coprocessor (or plugin) answers with the subset it
/// grants in `granted_policies`. Anything not granted is treated as denied.
#[derive(Debug, Clone, Default)]
pub struct AuthorizationContext {
    /// The policies the current operation requires a decision on.
    pub required_policies: Option<HashSet<String>>,
    /// The policies that were granted for this request.
    pub granted_policies: Option<HashSet<String>>,
}

impl AuthorizationContext {
    fn set_granted_policies_value(&mut self, value: Value) -> Result<(), RequestContextError> {
        if value.is_null() {
            self.granted_policies = None;
            return Ok(());
        }

        let array = value.expect_array(GRANTED_POLICIES_KEY, "array of strings or null")?;
        let mut policies = HashSet::with_capacity(array.len());
        for item in array {
            let policy = item.expect_str(GRANTED_POLICIES_KEY, "array of strings or null")?;
            policies.insert(policy.to_string());
        }

        self.granted_policies = Some(policies);
        Ok(())
    }
}

/// A read-only view of the authorization policy state for plugins.
pub struct RequestContextAuthorizationRead<'a> {
    context: &'a AuthorizationContext,
}

impl RequestContextAuthorizationRead<'_> {
    /// Returns the policies the current operation requires a decision on.
    pub fn required_policies(&self) -> Option<&HashSet<String>> {
        self.context.required_policies.as_ref()
    }

    /// Returns the policies currently granted for this request.
    pub fn granted_policies(&self) -> Option<&HashSet<String>> {
        self.context.granted_policies.as_ref()
    }
}

/// A writable interface for the authorization policy state for plugins.
pub struct RequestContextAuthorizationWrite<'a> {
    context: &'a mut AuthorizationContext,
}

impl RequestContextAuthorizationWrite<'_> {
    /// Sets the policies granted for the current request.
    /// Providing `None` is equivalent to an empty set, so nothing is granted.
    pub fn set_granted_policies(&mut self, policies: Option<HashSet<String>>) -> &mut Self {
        self.context.granted_policies = policies;
        self
    }
}

impl<Hook> RequestContextPluginRead<Hook> {
    /// Returns the authorization read API.
    pub fn authorization(&self) -> RequestContextAuthorizationRead<'_> {
        RequestContextAuthorizationRead {
            context: &self.snapshot.authorization,
        }
    }
}

impl<Hook: CanWriteAuthorization> RequestContextPluginWrite<'_, Hook> {
    /// Returns the authorization write API.
    /// Only available in hooks that implement `CanWriteAuthorization`.
    pub fn authorization(&mut self) -> RequestContextAuthorizationWrite<'_> {
        RequestContextAuthorizationWrite {
            context: &mut self.context.authorization,
        }
    }
}

impl RequestContextDomain for AuthorizationContext {
    const DOMAIN_PREFIX: &'static str = "hive::authorization::";

    fn set_key_value(&mut self, key: &str, value: Value) -> Result<(), RequestContextError> {
        match key {
            REQUIRED_POLICIES_KEY => self.forbidden_mutation(key),
            GRANTED_POLICIES_KEY => self.set_granted_policies_value(value),
            _ => self.unknown_key(key),
        }
    }

    super::impl_domain_serde!(
        REQUIRED_POLICIES_KEY => required_policies,
        GRANTED_POLICIES_KEY => granted_policies,
    );
}
