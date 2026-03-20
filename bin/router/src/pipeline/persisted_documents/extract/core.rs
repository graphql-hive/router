use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Deref;

use hive_router_config::persisted_documents::{
    PersistedDocumentExtractorConfig, PersistedDocumentUrlTemplate, PersistedDocumentsConfig,
};
use hive_router_plan_executor::hooks::on_graphql_params::GraphQLParams;
use ntex::web::HttpRequest;
use sonic_rs::OwnedLazyValue;
use thiserror::Error;

use crate::pipeline::persisted_documents::extract::extractors::apollo::{
    ApolloExtractor, APOLLO_HASH_PATH,
};
use crate::pipeline::persisted_documents::extract::extractors::document_id::{
    DocumentIdExtractor, DOCUMENT_ID_FIELD,
};
use crate::pipeline::persisted_documents::extract::extractors::json_path::JsonPathExtractor;
use crate::pipeline::persisted_documents::extract::extractors::url_path_param::UrlPathParamExtractor;
use crate::pipeline::persisted_documents::extract::extractors::url_query_param::{
    QueryParams, UrlQueryParamExtractor,
};

use super::super::types::PersistedDocumentId;

pub struct HttpRequestContext<'a> {
    pub(crate) path: &'a str,
    pub(crate) query: Option<QueryParams<'a>>,
}

pub struct DocumentIdResolverInput<'a> {
    pub graphql_params: &'a GraphQLParams,
    pub document_id: Option<&'a str>,
    pub nonstandard_json_fields: Option<&'a HashMap<String, OwnedLazyValue>>,
    pub request_context: &'a HttpRequestContext<'a>,
}

impl<'a> From<&'a HttpRequest> for HttpRequestContext<'a> {
    fn from(req: &'a HttpRequest) -> Self {
        Self::from_parts(req.uri().path(), req.uri().query())
    }
}

impl<'a> HttpRequestContext<'a> {
    pub fn from_parts(path: &'a str, query: Option<&'a str>) -> Self {
        Self {
            path,
            query: query.map(QueryParams::new),
        }
    }
}

pub struct DocumentIdResolver {
    graphql_endpoint: GraphQLEndpointPath,
    state: ResolverState,
}

#[derive(Debug, Error)]
pub enum PersistedDocumentExtractError {
    #[error("url_path_param.template must contain ':id' segment: {template}")]
    MissingIdParam { template: String },
    #[error("failed to compile url_path_param.template: {0}")]
    MatcherCompile(String),
}

enum ResolverState {
    Disabled,
    Enabled(ActivePlan),
}

struct ActivePlan {
    extractors: Vec<Box<dyn DocumentIdSourceExtractor>>,
    requires_nonstandard_json_fields: bool,
    depends_on_graphql_path: bool,
}

pub(super) trait DocumentIdSourceExtractor: Send + Sync {
    fn extract(&self, ctx: &ExtractionContext<'_>) -> Option<PersistedDocumentId>;
}

#[derive(Debug)]
struct GraphQLEndpointPath(String);

impl Deref for GraphQLEndpointPath {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<str> for GraphQLEndpointPath {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

pub(super) struct ExtractionContext<'a> {
    pub(crate) graphql_params: &'a GraphQLParams,
    document_id: Option<&'a str>,
    pub(crate) nonstandard_json_fields: Option<&'a HashMap<String, OwnedLazyValue>>,
    relative_path: Option<&'a str>,
    pub(crate) request_context: &'a HttpRequestContext<'a>,
}

impl<'a> ExtractionContext<'a> {
    fn new(input: DocumentIdResolverInput<'a>, graphql_endpoint: &GraphQLEndpointPath) -> Self {
        Self {
            graphql_params: input.graphql_params,
            document_id: input.document_id,
            nonstandard_json_fields: input.nonstandard_json_fields,
            relative_path: graphql_endpoint.relative_path(input.request_context.path()),
            request_context: input.request_context,
        }
    }

    pub(super) fn document_id(&self) -> Option<&'a str> {
        self.document_id
    }

    pub(super) fn relative_path(&self) -> Option<&'a str> {
        self.relative_path
    }

    pub(super) fn query_param(&self, name: &str) -> Option<Cow<'a, str>> {
        self.request_context.query_param(name)
    }
}

impl DocumentIdResolver {
    pub fn from_config(
        config: &PersistedDocumentsConfig,
        graphql_endpoint: &str,
    ) -> Result<Self, PersistedDocumentExtractError> {
        let graphql_endpoint = GraphQLEndpointPath::from(graphql_endpoint);

        if !config.enabled {
            return Ok(Self {
                graphql_endpoint,
                state: ResolverState::Disabled,
            });
        }

        let configured_extractors = match config.extractors.as_ref() {
            Some(extractors) => extractors.clone(),
            None => PersistedDocumentsConfig::default_extractors(),
        };

        let mut extractors = Vec::with_capacity(configured_extractors.len());
        let mut requires_nonstandard_json_fields = false;
        let mut depends_on_graphql_path = false;

        for extractor_config in &configured_extractors {
            let (extractor, requires_nonstandard_fields, depends_on_url_path) =
                build_extractor(extractor_config)?;
            requires_nonstandard_json_fields |= requires_nonstandard_fields;
            depends_on_graphql_path |= depends_on_url_path;
            extractors.push(extractor);
        }

        Ok(Self {
            graphql_endpoint,
            state: ResolverState::Enabled(ActivePlan {
                extractors,
                requires_nonstandard_json_fields,
                depends_on_graphql_path,
            }),
        })
    }

    #[inline]
    pub fn is_enabled(&self) -> bool {
        matches!(self.state, ResolverState::Enabled(_))
    }

    #[inline]
    pub fn requires_nonstandard_json_fields(&self) -> bool {
        match &self.state {
            ResolverState::Disabled => false,
            ResolverState::Enabled(active_plan) => active_plan.requires_nonstandard_json_fields,
        }
    }

    pub fn depends_on_graphql_path(&self) -> bool {
        match &self.state {
            ResolverState::Disabled => false,
            ResolverState::Enabled(active_plan) => active_plan.depends_on_graphql_path,
        }
    }

    pub fn resolve_document_id(
        &self,
        input: DocumentIdResolverInput<'_>,
    ) -> Option<PersistedDocumentId> {
        let active_plan = match &self.state {
            ResolverState::Disabled => return None,
            ResolverState::Enabled(active_plan) => active_plan,
        };

        let ctx = ExtractionContext::new(input, &self.graphql_endpoint);

        for extractor in &active_plan.extractors {
            if let Some(persisted_document_id) = extractor.extract(&ctx) {
                return Some(persisted_document_id);
            }
        }

        None
    }
}

fn build_extractor(
    extractor_config: &PersistedDocumentExtractorConfig,
) -> Result<(Box<dyn DocumentIdSourceExtractor>, bool, bool), PersistedDocumentExtractError> {
    match extractor_config {
        PersistedDocumentExtractorConfig::JsonPath { path } => {
            if path.as_str() == DOCUMENT_ID_FIELD {
                return Ok((Box::new(DocumentIdExtractor), false, false));
            }

            if path.as_str() == APOLLO_HASH_PATH {
                return Ok((Box::new(ApolloExtractor), false, false));
            }

            let segments = path
                .as_str()
                .split('.')
                .map(|s| s.to_string())
                .collect::<Vec<_>>();

            let requires_extra = JsonPathExtractor::requires_nonstandard_json_fields(&segments);

            Ok((
                Box::new(JsonPathExtractor { segments }),
                requires_extra,
                false,
            ))
        }
        PersistedDocumentExtractorConfig::UrlQueryParam { name } => Ok((
            Box::new(UrlQueryParamExtractor {
                name: name.as_str().to_string(),
            }),
            false,
            false,
        )),
        PersistedDocumentExtractorConfig::UrlPathParam { template } => {
            let extractor: UrlPathParamExtractor = template.try_into()?;
            Ok((Box::new(extractor), false, true))
        }
    }
}

impl From<&str> for GraphQLEndpointPath {
    fn from(endpoint: &str) -> Self {
        if endpoint.is_empty() || endpoint == "/" {
            return Self("/".to_string());
        }

        let with_leading_slash = if endpoint.starts_with('/') {
            endpoint.to_string()
        } else {
            format!("/{endpoint}")
        };

        Self(with_leading_slash.trim_end_matches('/').to_string())
    }
}

impl GraphQLEndpointPath {
    fn relative_path<'a>(&self, request_path: &'a str) -> Option<&'a str> {
        let suffix = if self.as_ref() == "/" {
            request_path
        } else {
            let suffix = request_path.strip_prefix(self.as_ref())?;
            if !suffix.is_empty() && !suffix.starts_with('/') {
                return None;
            }
            suffix
        };

        Some(suffix)
    }
}

impl TryFrom<&PersistedDocumentUrlTemplate> for UrlPathParamExtractor {
    type Error = PersistedDocumentExtractError;

    fn try_from(template: &PersistedDocumentUrlTemplate) -> Result<Self, Self::Error> {
        UrlPathParamExtractor::try_from_template(template)
    }
}
