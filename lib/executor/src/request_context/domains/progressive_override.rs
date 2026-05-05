use std::collections::HashSet;

use serde::ser::SerializeMap;
use sonic_rs::{JsonValueTrait, Value};

use super::super::api::plugin::{RequestContextPluginRead, RequestContextPluginWrite};
use super::super::deser::RequestContextValueExt;
use super::RequestContextDomain;
use super::RequestContextError;
use crate::hooks;

pub trait CanWriteProgressiveOverride {}
impl CanWriteProgressiveOverride for hooks::OnQueryPlan {}
impl CanWriteProgressiveOverride for hooks::OnHttpRequest {}
impl CanWriteProgressiveOverride for hooks::OnGraphqlParams {}
impl CanWriteProgressiveOverride for hooks::OnGraphqlParse {}
impl CanWriteProgressiveOverride for hooks::OnGraphqlValidation {}

pub(crate) const UNRESOLVED_LABELS_KEY: &str = "hive::progressive_override::unresolved_labels";
pub(crate) const LABELS_TO_OVERRIDE_KEY: &str = "hive::progressive_override::labels_to_override";

/// Context domain for progressive overrides.
#[derive(Debug, Clone, Default)]
pub struct ProgressiveOverrideContext {
    /// The set of labels that require an external decision
    pub unresolved_labels: Option<HashSet<String>>,
    /// The set of labels that should be overridden for this request
    pub labels_to_override: Option<HashSet<String>>,
}

impl ProgressiveOverrideContext {
    fn set_labels_to_override_value(&mut self, value: Value) -> Result<(), RequestContextError> {
        if value.is_null() {
            self.labels_to_override = None;
            return Ok(());
        }

        let array = value.expect_array(LABELS_TO_OVERRIDE_KEY, "array of strings or null")?;
        let mut labels = HashSet::with_capacity(array.len());
        for item in array {
            let label = item.expect_str(LABELS_TO_OVERRIDE_KEY, "array of strings or null")?;
            labels.insert(label.to_string());
        }

        self.labels_to_override = Some(labels);
        Ok(())
    }
}

/// A read-only view of progressive override state for plugins.
pub struct RequestContextProgressiveOverrideRead<'a> {
    context: &'a ProgressiveOverrideContext,
}

impl RequestContextProgressiveOverrideRead<'_> {
    /// Returns the set of unresolved labels that require a decision.
    pub fn unresolved_labels(&self) -> Option<&HashSet<String>> {
        self.context.unresolved_labels.as_ref()
    }

    /// Returns the set of labels currently marked to be overridden.
    pub fn labels_to_override(&self) -> Option<&HashSet<String>> {
        self.context.labels_to_override.as_ref()
    }
}

/// A writable interface for progressive override state for plugins.
pub struct RequestContextProgressiveOverrideWrite<'a> {
    context: &'a mut ProgressiveOverrideContext,
}

impl RequestContextProgressiveOverrideWrite<'_> {
    /// Sets the labels that should be overridden for the current request.
    /// Providing `None` is equivalent to an empty set, so no overrides.
    pub fn set_labels_to_override(&mut self, labels: Option<HashSet<String>>) -> &mut Self {
        self.context.labels_to_override = labels;
        self
    }
}

impl<Hook> RequestContextPluginRead<Hook> {
    /// Returns the progressive override read API.
    pub fn progressive_override(&self) -> RequestContextProgressiveOverrideRead<'_> {
        RequestContextProgressiveOverrideRead {
            context: &self.snapshot.progressive_override,
        }
    }
}

impl<Hook: CanWriteProgressiveOverride> RequestContextPluginWrite<'_, Hook> {
    /// Returns the progressive override write API.
    /// Only available in hooks that implement `CanWriteProgressiveOverride`.
    pub fn progressive_override(&mut self) -> RequestContextProgressiveOverrideWrite<'_> {
        RequestContextProgressiveOverrideWrite {
            context: &mut self.context.progressive_override,
        }
    }
}

impl RequestContextDomain for ProgressiveOverrideContext {
    const DOMAIN_PREFIX: &'static str = "hive::progressive_override::";

    fn set_key_value(&mut self, key: &str, value: Value) -> Result<(), RequestContextError> {
        match key {
            UNRESOLVED_LABELS_KEY => self.forbidden_mutation(key),
            LABELS_TO_OVERRIDE_KEY => self.set_labels_to_override_value(value),
            _ => self.unknown_key(key),
        }
    }

    super::impl_domain_serde!(
        UNRESOLVED_LABELS_KEY => unresolved_labels,
        LABELS_TO_OVERRIDE_KEY => labels_to_override,
    );
}
