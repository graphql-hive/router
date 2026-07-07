use serde::ser::SerializeMap;
use sonic_rs::{JsonValueTrait, Value};

use super::super::api::plugin::{RequestContextPluginRead, RequestContextPluginWrite};
use super::super::deser::RequestContextValueExt;
use super::RequestContextDomain;
use super::RequestContextError;
use crate::hooks;

pub trait CanWritePersistedDocuments {}
impl CanWritePersistedDocuments for hooks::OnHttpRequest {}
impl CanWritePersistedDocuments for hooks::OnGraphqlParams {}

pub(crate) const SKIP_ENFORCEMENT_KEY: &str = "hive::persisted_documents::skip_enforcement";

#[derive(Debug, Clone, Default)]
pub struct PersistedDocumentsContext {
    pub skip_enforcement: Option<bool>,
}

impl PersistedDocumentsContext {
    fn set_skip_enforcement_value(&mut self, value: Value) -> Result<(), RequestContextError> {
        if value.is_null() {
            self.skip_enforcement = None;
            return Ok(());
        }

        let flag = value.expect_bool(SKIP_ENFORCEMENT_KEY, "boolean or null")?;
        self.skip_enforcement = Some(flag);
        Ok(())
    }
}

pub struct RequestContextPersistedDocumentsRead<'a> {
    context: &'a PersistedDocumentsContext,
}

impl RequestContextPersistedDocumentsRead<'_> {
    pub fn skip_enforcement(&self) -> Option<bool> {
        self.context.skip_enforcement
    }
}

pub struct RequestContextPersistedDocumentsWrite<'a> {
    context: &'a mut PersistedDocumentsContext,
}

impl RequestContextPersistedDocumentsWrite<'_> {
    pub fn set_skip_enforcement(&mut self, value: bool) -> &mut Self {
        self.context.skip_enforcement = Some(value);
        self
    }
}

impl<Hook> RequestContextPluginRead<Hook> {
    pub fn persisted_documents(&self) -> RequestContextPersistedDocumentsRead<'_> {
        RequestContextPersistedDocumentsRead {
            context: &self.snapshot.persisted_documents,
        }
    }
}

impl<Hook: CanWritePersistedDocuments> RequestContextPluginWrite<'_, Hook> {
    pub fn persisted_documents(&mut self) -> RequestContextPersistedDocumentsWrite<'_> {
        RequestContextPersistedDocumentsWrite {
            context: &mut self.context.persisted_documents,
        }
    }
}

impl RequestContextDomain for PersistedDocumentsContext {
    const DOMAIN_PREFIX: &'static str = "hive::persisted_documents::";

    fn set_key_value(&mut self, key: &str, value: Value) -> Result<(), RequestContextError> {
        match key {
            SKIP_ENFORCEMENT_KEY => self.set_skip_enforcement_value(value),
            _ => self.unknown_key(key),
        }
    }

    super::impl_domain_serde!(
        SKIP_ENFORCEMENT_KEY => skip_enforcement,
    );
}
