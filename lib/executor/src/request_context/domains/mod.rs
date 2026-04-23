use super::domains::authentication::AuthenticationContext;
use super::domains::operation::OperationContext;
use super::domains::progressive_override::ProgressiveOverrideContext;
use super::domains::telemetry::TelemetryContext;
use super::RequestContextError;

use crate::response::value::Value as ResponseValue;

use hive_router_config::coprocessor::ContextSelection;
use serde::{ser::SerializeMap, Serialize};
use sonic_rs::Value;
use std::sync::{Arc, Mutex, MutexGuard};

mod authentication;
mod operation;
mod progressive_override;
mod telemetry;

/// The standard prefix used for all Hive-reserved context keys.
pub(crate) const HIVE_PREFIX: &str = "hive::";

/// A trait implemented by internal context domains (operation, authentication and so on).
/// It provides a common interface for key-value mapping for coprocessor patches, and de/serialization.
pub(crate) trait RequestContextDomain {
    /// The prefix for keys belonging to this domain (like `hive::operation::`)
    const DOMAIN_PREFIX: &'static str;

    /// Checks if a key belongs to this domain based on its prefix.
    fn is_applicable(&self, key: &str) -> bool {
        key.starts_with(Self::DOMAIN_PREFIX)
    }

    /// Updates a value in this domain from a coprocessor patch.
    /// This is where per-key mutability policies are enforced.
    fn set_key_value(&mut self, key: &str, value: Value) -> Result<(), RequestContextError>;

    /// Serializes all non-null fields in this domain into the provided map.
    fn serialize_all<S: SerializeMap>(&self, map: &mut S) -> Result<(), S::Error>;

    /// Serializes a specific key from this domain if it exists.
    fn serialize_entry<S: SerializeMap>(&self, key: &str, map: &mut S) -> Result<(), S::Error>;

    /// Returns the number of non-null fields currently stored in this domain.
    /// Some serialization libraries require to define length explicitly,
    /// prior to serializing the map.
    /// We don't really need that for sonic_rs, but I added it just in case we
    /// ever move to a different JSON library.
    fn serialized_len(&self) -> usize;

    fn forbidden_mutation(&self, key: &str) -> Result<(), RequestContextError> {
        Err(RequestContextError::ForbiddenReservedMutation {
            key: key.to_string(),
        })
    }

    fn unknown_key(&self, key: &str) -> Result<(), RequestContextError> {
        Err(RequestContextError::UnknownReservedKey {
            key: key.to_string(),
        })
    }

    /// Utility to serialize an optional field into a map only if it is present.
    fn serialize_optional_entry<S: SerializeMap, T: Serialize>(
        &self,
        map: &mut S,
        key: &str,
        value: Option<&T>,
    ) -> Result<(), S::Error> {
        if let Some(value) = value {
            map.serialize_entry(key, &value)
        } else {
            Ok(())
        }
    }
}

macro_rules! reserved_domains {
    ($($name:ident: $type:ty),* $(,)?) => {
        /// The root request context structure.
        ///
        /// It contains typed reserved domains (hive-owned) and a custom context
        /// for arbitrary plugin/coprocessor data.
        #[derive(Debug, Clone, Default)]
        pub struct RequestContext {
            $(
                /// Reserved context domain for $name.
                pub $name: $type,
            )*
            /// A collection of arbitrary keys and values stored by plugins and coprocessors.
            pub custom: CustomContext,
        }

        impl RequestContext {
            /// Attempts to route a reserved key to the appropriate domain for mutation.
            /// Returns `Ok` if the key was handled by a domain, or `Err`
            /// if the key does not match any reserved domain keys.
            pub(crate) fn try_set_reserved_key(
                &mut self,
                key: &str,
                value: Value,
            ) -> Result<(), RequestContextError> {
                $(
                    if self.$name.is_applicable(key) {
                        self.$name.set_key_value(key, value)?;
                        return Ok(());
                    }
                )*
                Err(RequestContextError::UnknownReservedKey {
                    key: key.to_string(),
                })
            }

            /// Returns the total number of non-null reserved keys across all domains.
            pub(crate) fn reserved_serialized_len(&self) -> usize {
                // 0 is the initial value, then we add the serialized length of each domain
                0 $(+ self.$name.serialized_len())*
            }

            /// Serializes every active reserved key across all domains into a map.
            pub(crate) fn serialize_all_reserved<S: SerializeMap>(&self, map: &mut S) -> Result<(), S::Error> {
                $(self.$name.serialize_all(map)?;)*
                Ok(())
            }

            /// Attempts to serialize a specific reserved key by routing it to its domain.
            /// Returns:
            /// `true` if the key belonged to a domain (even if it was null and nothing was written)
            /// `false` if it matches no domain
            pub(crate) fn try_serialize_reserved_entry<S: SerializeMap>(
                &self,
                key: &str,
                map: &mut S,
            ) -> Result<bool, S::Error> {
                $(
                    if self.$name.is_applicable(key) {
                        self.$name.serialize_entry(key, map)?;
                        return Ok(true);
                    }
                )*
                Ok(false)
            }
        }
    };
}

reserved_domains! {
    operation: OperationContext,
    progressive_override: ProgressiveOverrideContext,
    authentication: AuthenticationContext,
    telemetry: TelemetryContext,
}

#[derive(Debug, Clone, Default)]
pub struct CustomContext(Vec<(String, Value)>);

impl CustomContext {
    pub(crate) fn get(&self, key: &str) -> Option<&Value> {
        self.0
            .iter()
            .find(|(custom_key, _)| custom_key == key)
            .map(|(_, value)| value)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&String, &Value)> {
        self.0.iter().map(|(key, value)| (key, value))
    }

    pub(crate) fn apply(&mut self, key: &str, value: ResponseValue<'_>) {
        if value.is_null() {
            // Remove
            self.0.retain(|(current_key, _)| current_key != key);
            return;
        }

        let value = value.as_ref().into();
        if let Some((_, current)) = self.0.iter_mut().find(|(name, _)| *name == key) {
            // Update existing value
            *current = value;
            return;
        }
        // Insert new value
        self.0.push((key.to_string(), value));
    }

    pub(crate) fn size(&self) -> usize {
        self.0.len()
    }
}

#[derive(Default, Debug, Clone)]
pub struct SharedRequestContext(Arc<Mutex<RequestContext>>);

impl SharedRequestContext {
    pub fn lock(&self) -> Result<MutexGuard<'_, RequestContext>, RequestContextError> {
        self.0.lock().map_err(|_| RequestContextError::LockPoison)
    }

    pub fn snapshot(&self) -> Result<RequestContext, RequestContextError> {
        Ok(self.lock()?.clone())
    }

    pub fn update(&self, f: impl FnOnce(&mut RequestContext)) -> Result<(), RequestContextError> {
        let mut context = self.lock()?;
        f(&mut context);
        Ok(())
    }
}

pub struct SelectedRequestContext<'a> {
    pub(crate) context: &'a RequestContext,
    pub(crate) selection: &'a ContextSelection,
}

impl RequestContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn as_selected<'a>(
        &'a self,
        selection: &'a ContextSelection,
    ) -> SelectedRequestContext<'a> {
        SelectedRequestContext {
            context: self,
            selection,
        }
    }
}
