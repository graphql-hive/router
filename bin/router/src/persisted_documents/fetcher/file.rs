use std::{collections::HashMap, fs::read_to_string};

use hive_router_config::primitives::file_path::FilePath;

use crate::persisted_documents::PersistedDocumentsError;

pub struct FilePersistedDocumentsManager {
    operations: HashMap<String, String>,
}

impl FilePersistedDocumentsManager {
    pub fn try_new(file_path: &FilePath) -> Result<Self, PersistedDocumentsError> {
        let content =
            read_to_string(&file_path.absolute).map_err(PersistedDocumentsError::FileReadError)?;

        let operations: HashMap<String, String> =
            serde_json::from_str(&content).map_err(PersistedDocumentsError::ParseError)?;

        Ok(Self { operations })
    }

    pub fn resolve_document(&self, document_id: &str) -> Result<String, PersistedDocumentsError> {
        match self.operations.get(document_id) {
            Some(document) => Ok(document.clone()),
            None => Err(PersistedDocumentsError::NotFound(document_id.to_string())),
        }
    }
}
