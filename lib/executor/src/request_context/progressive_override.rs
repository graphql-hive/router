use std::collections::HashSet;

use serde::ser::SerializeMap;
use sonic_rs::{JsonValueTrait, Value};

use crate::request_context::{
    deser::RequestContextValueExt, RequestContextDomain, RequestContextError,
};

pub(crate) const UNRESOLVED_LABELS_KEY: &str = "hive::progressive_override::unresolved_labels";
pub(crate) const LABELS_TO_OVERRIDE_KEY: &str = "hive::progressive_override::labels_to_override";

#[derive(Debug, Clone, Default)]
pub struct ProgressiveOverrideContext {
    pub unresolved_labels: Option<HashSet<String>>,
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

impl RequestContextDomain for ProgressiveOverrideContext {
    const DOMAIN_PREFIX: &'static str = "hive::progressive_override::";

    fn is_applicable(&self, key: &str) -> bool {
        key.starts_with(Self::DOMAIN_PREFIX)
    }

    fn serialized_len(&self) -> usize {
        usize::from(self.unresolved_labels.is_some())
            + usize::from(self.labels_to_override.is_some())
    }

    fn set_key_value(&mut self, key: &str, value: Value) -> Result<(), RequestContextError> {
        match key {
            UNRESOLVED_LABELS_KEY => self.forbidden_mutation(key),
            LABELS_TO_OVERRIDE_KEY => self.set_labels_to_override_value(value),
            _ => self.unknown_key(key),
        }
    }

    fn serialize_all<S: SerializeMap>(&self, map: &mut S) -> Result<(), S::Error> {
        if let Some(unresolved_labels) = &self.unresolved_labels {
            map.serialize_entry(UNRESOLVED_LABELS_KEY, unresolved_labels)?;
        }
        if let Some(labels_to_override) = &self.labels_to_override {
            map.serialize_entry(LABELS_TO_OVERRIDE_KEY, labels_to_override)?;
        }
        Ok(())
    }

    fn serialize_entry<S: SerializeMap>(&self, key: &str, map: &mut S) -> Result<(), S::Error> {
        match key {
            UNRESOLVED_LABELS_KEY => {
                if let Some(unresolved_labels) = &self.unresolved_labels {
                    map.serialize_entry(UNRESOLVED_LABELS_KEY, unresolved_labels)?;
                }
                Ok(())
            }
            LABELS_TO_OVERRIDE_KEY => {
                if let Some(labels_to_override) = &self.labels_to_override {
                    map.serialize_entry(LABELS_TO_OVERRIDE_KEY, labels_to_override)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}
