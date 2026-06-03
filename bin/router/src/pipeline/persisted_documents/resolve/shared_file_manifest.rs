use std::{borrow::Cow, collections::HashMap, ops::Deref, sync::Arc};

use serde::Deserialize;

use crate::pipeline::persisted_documents::resolve::PersistedDocumentResolverError;

// In-memory map used by the file/storage manifest resolver.
// Values are Arc-backed so lookups only clone cheap references.
pub struct DocumentsById(HashMap<String, Arc<str>>);

#[derive(Debug, thiserror::Error)]
pub enum FileManifestError {
    #[error("failed to parse persisted documents manifest at '{path}': {message}")]
    ParseManifest { path: String, message: String },
    #[error("unsupported apollo manifest format. Expected 'apollo-persisted-query-manifest', received '{format}'")]
    UnsupportedApolloManifestFormat { format: String },
    #[error("unsupported apollo manifest version. Expected '1', received '{version}'")]
    UnsupportedApolloManifestVersion { version: u8 },
}

impl Deref for DocumentsById {
    type Target = HashMap<String, Arc<str>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub type KeyValueManifest<'a> = HashMap<Cow<'a, str>, Cow<'a, str>>;

#[derive(Deserialize)]
#[serde(untagged)]
#[serde(bound(deserialize = "'de: 'a"))]
pub enum PersistedDocumentsManifest<'a> {
    Apollo(ApolloPersistedQueryManifest<'a>),
    KeyValue(KeyValueManifest<'a>),
}

impl<'a> TryFrom<PersistedDocumentsManifest<'a>> for DocumentsById {
    type Error = PersistedDocumentResolverError;

    fn try_from(value: PersistedDocumentsManifest<'a>) -> Result<Self, Self::Error> {
        match value {
            PersistedDocumentsManifest::Apollo(manifest) => manifest.try_into(),
            PersistedDocumentsManifest::KeyValue(manifest) => Ok(manifest.into()),
        }
    }
}

impl<'a> TryFrom<ApolloPersistedQueryManifest<'a>> for DocumentsById {
    type Error = PersistedDocumentResolverError;

    fn try_from(manifest: ApolloPersistedQueryManifest<'a>) -> Result<Self, Self::Error> {
        if manifest.format != "apollo-persisted-query-manifest" {
            return Err(FileManifestError::UnsupportedApolloManifestFormat {
                format: manifest.format.into_owned(),
            }
            .into());
        }

        if manifest.version != 1 {
            return Err(FileManifestError::UnsupportedApolloManifestVersion {
                version: manifest.version,
            }
            .into());
        }

        Ok(DocumentsById(
            manifest
                .operations
                .into_iter()
                .map(|op| (op.id.into_owned(), Arc::<str>::from(op.body)))
                .collect::<HashMap<_, _>>(),
        ))
    }
}

impl<'a> From<KeyValueManifest<'a>> for DocumentsById {
    fn from(manifest: KeyValueManifest<'a>) -> Self {
        DocumentsById(
            manifest
                .into_iter()
                .map(|(id, text)| (id.into_owned(), Arc::<str>::from(text)))
                .collect(),
        )
    }
}

#[derive(Deserialize)]
pub struct ApolloPersistedQueryManifest<'a> {
    #[serde(borrow)]
    format: Cow<'a, str>,
    version: u8,
    #[serde(borrow)]
    operations: Vec<ApolloPersistedQueryOperation<'a>>,
}

#[derive(Deserialize)]
pub struct ApolloPersistedQueryOperation<'a> {
    #[serde(borrow)]
    id: Cow<'a, str>,
    #[serde(borrow)]
    body: Cow<'a, str>,
}

pub fn parse_manifest<'a>(
    manifest_path: &str,
    raw_manifest: &'a [u8],
) -> Result<PersistedDocumentsManifest<'a>, FileManifestError> {
    sonic_rs::from_slice(raw_manifest).map_err(|err| FileManifestError::ParseManifest {
        path: manifest_path.to_string(),
        message: err.to_string(),
    })
}
