use std::borrow::Cow;
use std::sync::Arc;

use async_trait::async_trait;
use hive_console_sdk::circuit_breaker::CircuitBreakerBuilder;
use hive_console_sdk::persisted_documents::{PersistedDocumentsError, PersistedDocumentsManager};
use hive_router_config::persisted_documents::PersistedDocumentsHiveStorageConfig;
use thiserror::Error;

use crate::consts::ROUTER_VERSION;

use super::{
    PersistedDocumentResolveInput, PersistedDocumentResolver, PersistedDocumentResolverError,
    ResolvedDocument,
};

pub struct HiveCDNResolver {
    manager: PersistedDocumentsManager,
}

static CLIENT_INSTRUCTIONS: &str = "Provide both client name and version headers, or send persisted document id in 'appName~appVersion~documentId' format";

#[derive(Debug, Error)]
pub enum HiveResolverError {
    #[error("persisted_documents.storage.hive.endpoint is not configured")]
    MissingEndpoint,
    #[error("persisted_documents.storage.hive.key is not configured")]
    MissingKey,
    #[error("Document id format is invalid. Either 'appName~appVersion~documentId' or 'documentId' is accepted, received: {0}")]
    InvalidDocumentIdFormat(String),
    #[error("Client identity is missing. {CLIENT_INSTRUCTIONS}")]
    ClientIdentityMissing,
    #[error("Client identity is partial. {CLIENT_INSTRUCTIONS}")]
    ClientIdentityPartial,
    #[error("Initialization failed: {0}")]
    ManagerInit(String),
    #[error("SDK error: {0}")]
    SDKError(String),
}

struct AppDocumentId<'a>(Cow<'a, str>);

enum DocumentIdSyntax<'a> {
    App(&'a str),
    Plain(&'a str),
}

impl<'a> TryFrom<&'a str> for DocumentIdSyntax<'a> {
    type Error = HiveResolverError;

    // It uses memchr. I performed a benchmark with a lot of solutions, even byte by byte scanning.
    // It was the best bang for the buck option.
    fn try_from(value: &'a str) -> Result<Self, Self::Error> {
        let bytes = value.as_bytes();

        // First '~' separates app name from app version.
        let Some(first) = memchr::memchr(b'~', bytes) else {
            // If there is no '~', the entire value is a plain document id
            return Ok(Self::Plain(value));
        };

        // We found "~...." - empty app name segment
        if first == 0 {
            return Err(HiveResolverError::InvalidDocumentIdFormat(
                value.to_string(),
            ));
        }

        // Second '~' separates app version from document id
        let Some(second_relative) = memchr::memchr(b'~', &bytes[first + 1..]) else {
            // We found "appName~documentId", so it lacks an app version segment
            return Err(HiveResolverError::InvalidDocumentIdFormat(
                value.to_string(),
            ));
        };
        // If the relative position of the second '~' is 0, it means it's right after the first '~'.
        // Found "appName~~documentId", so it has the app version segment, but it's empty.
        if second_relative == 0 {
            return Err(HiveResolverError::InvalidDocumentIdFormat(
                value.to_string(),
            ));
        }

        // Compute the absolute position of the second '~'
        let second = first + 1 + second_relative;

        // Check if it's not the last character of the string.
        // If it is, we found an empty document id segment.
        if second + 1 >= bytes.len() {
            return Err(HiveResolverError::InvalidDocumentIdFormat(
                value.to_string(),
            ));
        }

        // Syntax with more than 2 separators is invalid.
        if memchr::memchr(b'~', &bytes[second + 1..]).is_some() {
            return Err(HiveResolverError::InvalidDocumentIdFormat(
                value.to_string(),
            ));
        }

        // Syntax is valid, return appName~appVersion~documentId.
        Ok(Self::App(value))
    }
}

impl<'a> TryFrom<PersistedDocumentResolveInput<'a>> for AppDocumentId<'a> {
    type Error = HiveResolverError;

    fn try_from(input: PersistedDocumentResolveInput<'a>) -> Result<Self, Self::Error> {
        let persisted_document_id = input.persisted_document_id.as_ref();

        match DocumentIdSyntax::try_from(persisted_document_id)? {
            DocumentIdSyntax::App(app_document_id) => {
                // Raw app-included id takes precedence over client identity headers.
                Ok(Self(Cow::Borrowed(app_document_id)))
            }
            DocumentIdSyntax::Plain(document_id) => {
                match (input.client_identity.name, input.client_identity.version) {
                    (Some(name), Some(version)) => {
                        Ok(Self(Cow::Owned(format!("{name}~{version}~{document_id}"))))
                    }
                    (Some(_), None) | (None, Some(_)) => {
                        Err(HiveResolverError::ClientIdentityPartial)
                    }
                    (None, None) => Err(HiveResolverError::ClientIdentityMissing),
                }
            }
        }
    }
}

impl AsRef<str> for AppDocumentId<'_> {
    fn as_ref(&self) -> &str {
        self.0.as_ref()
    }
}

impl HiveCDNResolver {
    pub fn from_storage_config(
        config: &PersistedDocumentsHiveStorageConfig,
    ) -> Result<Self, HiveResolverError> {
        let endpoints: Vec<String> = config
            .endpoint
            .clone()
            .ok_or(HiveResolverError::MissingEndpoint)?
            .into();
        let key = config.key.clone().ok_or(HiveResolverError::MissingKey)?;

        let circuit_breaker = CircuitBreakerBuilder::default()
            .error_threshold(config.circuit_breaker.error_threshold)
            .volume_threshold(config.circuit_breaker.volume_threshold)
            .reset_timeout(config.circuit_breaker.reset_timeout);

        let mut builder = PersistedDocumentsManager::builder()
            .key(key)
            .accept_invalid_certs(config.accept_invalid_certs)
            .connect_timeout(config.connect_timeout)
            .request_timeout(config.request_timeout)
            .max_retries(config.retry_policy.max_retries)
            .cache_size(config.cache_size)
            .circuit_breaker(circuit_breaker)
            .user_agent(format!("hive-router/{ROUTER_VERSION}"));

        if let Some(negative_cache) = config.negative_cache.enabled_config() {
            builder = builder.negative_cache_ttl(negative_cache.ttl);
        }

        for endpoint in endpoints {
            builder = builder.add_endpoint(endpoint);
        }

        let manager = builder
            .build()
            .map_err(|err| HiveResolverError::ManagerInit(err.to_string()))?;
        Ok(Self { manager })
    }
}

#[async_trait]
impl PersistedDocumentResolver for HiveCDNResolver {
    // TODO: Consider implementing stale-while-revalidate (SWR).
    //
    // Requirements:
    // - We should not spawn a task/thread per request when an entry becomes stale.
    // - Revalidation must be bounded (queue + capped worker concurrency) to avoid overload.
    // - Requests should keep serving stale entries during the grace window while refresh runs.
    // - Refreshes should be de-duplicated per document id (avoid N concurrent refreshes for same key).
    // - Queue overflow and cancellation/shutdown behavior must be defined explicitly.
    // - Interaction with SDK cache and negative-cache semantics needs careful handling.
    //
    // Suggested:
    // - Add a background task (similar to file resolver) with "notify" + bounded queue.
    // - Keep per-entry freshness metadata: fresh until / stale until.
    // - On stale hit: return stale + enqueue refresh
    // - On miss/expired: fetch
    // - Add observability counters for stale-served, refresh-enqueued, refresh-failed, queue-dropped.
    async fn resolve(
        &self,
        input: PersistedDocumentResolveInput<'_>,
    ) -> Result<ResolvedDocument, PersistedDocumentResolverError> {
        let app_document_id = AppDocumentId::try_from(input)?;
        let text = self
            .manager
            .resolve_document(app_document_id.as_ref())
            .await
            .map_err(|err| match err {
                PersistedDocumentsError::DocumentNotFound => {
                    PersistedDocumentResolverError::NotFound(app_document_id.as_ref().to_string())
                }
                other => HiveResolverError::SDKError(other.to_string()).into(),
            })?;

        Ok(ResolvedDocument {
            text: Arc::<str>::from(text),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{AppDocumentId, PersistedDocumentResolveInput};
    use crate::pipeline::persisted_documents::types::{ClientIdentity, PersistedDocumentId};

    struct Case {
        raw_id: &'static str,
        client_name: Option<&'static str>,
        client_version: Option<&'static str>,
        expected: Result<&'static str, &'static str>,
    }

    #[test]
    fn app_document_id_conversion_matrix() {
        let cases = [
            Case {
                raw_id: "documentId",
                client_name: Some("app"),
                client_version: Some("1.0.0"),
                expected: Ok("app~1.0.0~documentId"),
            },
            Case {
                raw_id: "app~1.0.0~documentId",
                client_name: None,
                client_version: None,
                expected: Ok("app~1.0.0~documentId"),
            },
            Case {
                raw_id: "app~1.0.0~documentId",
                client_name: Some("app"),
                client_version: Some("1.2.3"),
                expected: Ok("app~1.0.0~documentId"),
            },
            Case {
                raw_id: "documentId",
                client_name: None,
                client_version: None,
                expected: Err("missing"),
            },
            Case {
                raw_id: "documentId",
                client_name: Some("app"),
                client_version: None,
                expected: Err("partial"),
            },
            Case {
                raw_id: "documentId",
                client_name: None,
                client_version: Some("1.0.0"),
                expected: Err("partial"),
            },
            Case {
                raw_id: "app~documentId",
                client_name: None,
                client_version: None,
                expected: Err("invalid"),
            },
            Case {
                raw_id: "app~~documentId",
                client_name: None,
                client_version: None,
                expected: Err("invalid"),
            },
            Case {
                raw_id: "~1.0.0~documentId",
                client_name: None,
                client_version: None,
                expected: Err("invalid"),
            },
            Case {
                raw_id: "app~1.0.0~",
                client_name: None,
                client_version: None,
                expected: Err("invalid"),
            },
            Case {
                raw_id: "a~b~c~d",
                client_name: None,
                client_version: None,
                expected: Err("invalid"),
            },
        ];

        for (idx, case) in cases.into_iter().enumerate() {
            let persisted_document_id =
                PersistedDocumentId::try_from(case.raw_id).expect("fixture id should parse");
            let input = PersistedDocumentResolveInput {
                persisted_document_id: &persisted_document_id,
                client_identity: ClientIdentity {
                    name: case.client_name,
                    version: case.client_version,
                },
            };

            match (AppDocumentId::try_from(input), case.expected) {
                (Ok(actual), Ok(expected)) => {
                    assert_eq!(actual.as_ref(), expected, "case_index={idx}")
                }
                (Err(err), Err(expected)) => {
                    assert!(
                        err.to_string().contains(expected),
                        "case_index={}, err={}",
                        idx,
                        err
                    );
                }
                (Ok(actual), Err(expected)) => panic!(
                    "case_index={} expected err containing '{}' but got Ok({})",
                    idx,
                    expected,
                    actual.as_ref()
                ),
                (Err(err), Ok(expected)) => {
                    panic!(
                        "case_index={} expected Ok({}) but got Err({})",
                        idx, expected, err
                    )
                }
            }
        }
    }
}
