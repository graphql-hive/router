use ahash::{HashMap, HashMapExt};

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

/// Context domain for custom authorization policies (`@policy`).
///
/// The router seeds `required_policies` with every policy the current operation
/// depends on, each mapped to `null`. A coprocessor (or plugin) decides by
/// overwriting entries with `true`/`false`; anything left `null`, or missing
/// entirely, is treated as denied. Mirrors Apollo Router's
/// `apollo::authorization::required_policies` contract.
#[derive(Debug, Clone, Default)]
pub struct AuthorizationContext {
    pub required_policies: Option<HashMap<String, Option<bool>>>,
}

impl AuthorizationContext {
    fn set_required_policies_value(&mut self, value: Value) -> Result<(), RequestContextError> {
        if value.is_null() {
            self.required_policies = None;
            return Ok(());
        }

        let object = value.expect_object(
            REQUIRED_POLICIES_KEY,
            "object mapping policy names to booleans or null",
        )?;
        let mut policies = HashMap::with_capacity(object.len());
        for (policy, decision) in object.iter() {
            let decision = if decision.is_null() {
                None
            } else {
                Some(decision.expect_bool(
                    REQUIRED_POLICIES_KEY,
                    "object mapping policy names to booleans or null",
                )?)
            };
            policies.insert(policy.to_string(), decision);
        }

        self.required_policies = Some(policies);
        Ok(())
    }
}

/// A read-only view of the authorization policy state for plugins.
pub struct RequestContextAuthorizationRead<'a> {
    context: &'a AuthorizationContext,
}

impl RequestContextAuthorizationRead<'_> {
    /// Returns the policies the current operation requires a decision on, each
    /// mapped to its current decision. `None` means undecided, and is denied
    /// by default just like an entry that was never granted.
    pub fn required_policies(&self) -> Option<&HashMap<String, Option<bool>>> {
        self.context.required_policies.as_ref()
    }
}

/// A writable interface for the authorization policy state for plugins.
pub struct RequestContextAuthorizationWrite<'a> {
    context: &'a mut AuthorizationContext,
}

impl RequestContextAuthorizationWrite<'_> {
    /// Grants or denies a single policy for this request.
    pub fn set_policy_decision(&mut self, policy: impl Into<String>, granted: bool) -> &mut Self {
        self.context
            .required_policies
            .get_or_insert_with(HashMap::default)
            .insert(policy.into(), Some(granted));
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
            REQUIRED_POLICIES_KEY => self.set_required_policies_value(value),
            _ => self.unknown_key(key),
        }
    }

    super::impl_domain_serde!(
        REQUIRED_POLICIES_KEY => required_policies,
    );
}
