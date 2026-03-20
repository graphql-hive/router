use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;

use hive_router_internal::json::MapAccessSerdeExt;
use hive_router_internal::telemetry::metrics::Metrics;
use hive_router_plan_executor::hooks::on_graphql_params::{
    GraphQLParams, OnGraphQLParamsEndHookPayload, OnGraphQLParamsStartHookPayload,
};
use hive_router_plan_executor::plugin_context::PluginRequestState;
use hive_router_plan_executor::plugin_trait::{EndControlFlow, StartControlFlow};
use http::{header::CONTENT_TYPE, Method};
use ntex::util::Bytes;
use ntex::web::types::Query;
use ntex::web::HttpRequest;
use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, Visitor};
use std::sync::Arc;
use tracing::{info, trace, warn};

use crate::pipeline::error::PipelineError;
use crate::pipeline::header::SingleContentType;
use crate::pipeline::persisted_documents::extract::{
    DocumentIdResolver, DocumentIdResolverInput, HttpRequestContext, DOCUMENT_ID_FIELD,
};
use crate::pipeline::persisted_documents::resolve::PersistedDocumentResolveInput;
use crate::pipeline::persisted_documents::types::{ClientIdentity, PersistedDocumentId};
use crate::pipeline::persisted_documents::PersistedDocumentsRuntime;
use crate::shared_state::RouterSharedState;

#[derive(serde::Deserialize, Debug)]
struct GraphQLGetInput {
    pub query: Option<String>,
    #[serde(rename = "operationName")]
    pub operation_name: Option<String>,
    #[serde(rename = "documentId")]
    pub document_id: Option<String>,
    pub variables: Option<String>,
    pub extensions: Option<String>,
}

impl GraphQLGetInput {
    pub fn empty() -> Self {
        Self {
            query: None,
            operation_name: None,
            document_id: None,
            variables: None,
            extensions: None,
        }
    }
}

#[derive(Debug, Default)]
struct GraphQLPostInput {
    query: Option<String>,
    operation_name: Option<String>,
    variables: HashMap<String, sonic_rs::Value>,
    extensions: Option<HashMap<String, sonic_rs::Value>>,
    document_id: Option<String>,
    nonstandard_json_fields: Option<HashMap<String, sonic_rs::OwnedLazyValue>>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(untagged)]
enum GraphQLDocumentIdValue {
    String(String),
    U64(u64),
}

impl GraphQLDocumentIdValue {
    #[inline]
    fn into_string(self) -> String {
        match self {
            Self::String(value) => value,
            Self::U64(value) => value.to_string(),
        }
    }
}

#[derive(Debug)]
pub struct PreparedOperation {
    pub graphql_params: GraphQLParams,
    /// Represents the resolved document ID, if one was found,
    /// according to the document ID resolver plan.
    pub resolved_document_id: Option<PersistedDocumentId>,
}

impl PreparedOperation {
    #[inline]
    fn from_get(
        get_input: GraphQLGetInput,
        document_id_resolver: &DocumentIdResolver,
        request_context: HttpRequestContext<'_>,
    ) -> Result<Self, PipelineError> {
        let document_id = get_input.document_id.clone();
        Ok(Self::from_graphql_params(
            get_input.try_into()?,
            document_id_resolver,
            request_context,
            document_id.as_deref(),
            None,
        ))
    }

    #[inline]
    fn from_post(
        post_input: GraphQLPostInput,
        document_id_resolver: &DocumentIdResolver,
        request_context: HttpRequestContext<'_>,
    ) -> Self {
        let GraphQLPostInput {
            query,
            operation_name,
            variables,
            extensions,
            document_id,
            nonstandard_json_fields,
        } = post_input;

        Self::from_graphql_params(
            GraphQLParams {
                query,
                operation_name,
                variables,
                extensions,
            },
            document_id_resolver,
            request_context,
            document_id.as_deref(),
            nonstandard_json_fields.as_ref(),
        )
    }

    #[inline]
    fn from_graphql_params(
        graphql_params: GraphQLParams,
        document_id_resolver: &DocumentIdResolver,
        request_context: HttpRequestContext<'_>,
        document_id: Option<&str>,
        nonstandard_json_fields: Option<&HashMap<String, sonic_rs::OwnedLazyValue>>,
    ) -> Self {
        let persisted_document_id = if document_id_resolver.is_enabled() {
            document_id_resolver.resolve_document_id(DocumentIdResolverInput {
                graphql_params: &graphql_params,
                document_id,
                nonstandard_json_fields,
                request_context: &request_context,
            })
        } else {
            None
        };

        Self {
            graphql_params,
            resolved_document_id: persisted_document_id,
        }
    }
}

struct GraphQLPostBodySeed<'a> {
    document_id_resolver: &'a DocumentIdResolver,
}

impl<'a> GraphQLPostBodySeed<'a> {
    #[inline]
    fn new(document_id_resolver: &'a DocumentIdResolver) -> Self {
        Self {
            document_id_resolver,
        }
    }
}

struct GraphQLPostBodyVisitor {
    // wether to capture extra fields from the POST body
    // besides the query, operation name, variables, extensions and documentId.
    // We only need it when the document ID resolver requires something else than:
    // - documentId
    // - extensions.*
    capture_nonstandard_json_fields: bool,
}

impl<'de> DeserializeSeed<'de> for GraphQLPostBodySeed<'_> {
    type Value = GraphQLPostInput;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(GraphQLPostBodyVisitor {
            capture_nonstandard_json_fields: self
                .document_id_resolver
                .requires_nonstandard_json_fields(),
        })
    }
}

impl<'de> Visitor<'de> for GraphQLPostBodyVisitor {
    type Value = GraphQLPostInput;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a GraphQL POST JSON object")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut query: Option<String> = None;
        let mut operation_name: Option<String> = None;
        let mut variables: Option<HashMap<String, sonic_rs::Value>> = None;
        let mut extensions: Option<HashMap<String, sonic_rs::Value>> = None;
        let mut document_id: Option<GraphQLDocumentIdValue> = None;
        let mut nonstandard_json_fields: Option<HashMap<String, sonic_rs::OwnedLazyValue>> =
            self.capture_nonstandard_json_fields.then(HashMap::new);

        while let Some(key) = map.next_key::<Cow<'de, str>>()? {
            match key.as_ref() {
                "query" => map.deserialize_once_into_option(&mut query, "query")?,
                "operationName" => {
                    map.deserialize_once_into_option(&mut operation_name, "operationName")?
                }
                "variables" => map.deserialize_once_into_option(&mut variables, "variables")?,
                "extensions" => map.deserialize_once_into_option(&mut extensions, "extensions")?,
                DOCUMENT_ID_FIELD => {
                    map.deserialize_once_into_option(&mut document_id, DOCUMENT_ID_FIELD)?
                }
                _ => {
                    if let Some(nonstandard_json_fields) = nonstandard_json_fields.as_mut() {
                        let value = map.next_value::<sonic_rs::OwnedLazyValue>()?;
                        nonstandard_json_fields.insert(key.into_owned(), value);
                    } else {
                        let _ = map.next_value::<IgnoredAny>()?;
                    }
                }
            }
        }

        Ok(GraphQLPostInput {
            query,
            operation_name,
            variables: variables.unwrap_or_default(),
            extensions,
            document_id: document_id.map(GraphQLDocumentIdValue::into_string),
            nonstandard_json_fields,
        })
    }
}

impl TryInto<GraphQLParams> for GraphQLGetInput {
    type Error = PipelineError;

    fn try_into(self) -> Result<GraphQLParams, Self::Error> {
        let variables = match self.variables.as_deref() {
            Some(v_str) if !v_str.is_empty() => match sonic_rs::from_str(v_str) {
                Ok(vars) => vars,
                Err(e) => {
                    return Err(PipelineError::FailedToParseVariables(e));
                }
            },
            _ => HashMap::new(),
        };

        let extensions = match self.extensions.as_deref() {
            Some(e_str) if !e_str.is_empty() => match sonic_rs::from_str(e_str) {
                Ok(exts) => Some(exts),
                Err(e) => {
                    return Err(PipelineError::FailedToParseExtensions(e));
                }
            },
            _ => None,
        };

        let execution_request = GraphQLParams {
            query: self.query,
            operation_name: self.operation_name,
            variables,
            extensions,
        };

        Ok(execution_request)
    }
}

pub trait GetQueryStr {
    fn get_query(&self) -> Result<&str, PipelineError>;
}

impl GetQueryStr for GraphQLParams {
    fn get_query(&self) -> Result<&str, PipelineError> {
        self.query
            .as_deref()
            .ok_or(PipelineError::GetMissingQueryParam("query"))
    }
}

pub enum OperationPreparationResult {
    EarlyResponse(ntex::web::HttpResponse),
    Operation(PreparedOperation),
}

pub struct OperationPreparation<'a> {
    req: &'a HttpRequest,
    persisted_documents_runtime: &'a PersistedDocumentsRuntime,
    plugin_req_state: &'a Option<PluginRequestState<'a>>,
    body: Bytes,
    require_id: bool,
    persisted_documents_enabled: bool,
    log_missing_id_requests: bool,
    client_identity: ClientIdentity<'a>,
    metrics: Arc<Metrics>,
}

impl<'a> OperationPreparation<'a> {
    #[inline]
    pub async fn prepare(
        req: &'a HttpRequest,
        shared_state: &'a Arc<RouterSharedState>,
        plugin_req_state: &'a Option<PluginRequestState<'a>>,
        body: Bytes,
        client_name: Option<&'a str>,
        client_version: Option<&'a str>,
    ) -> Result<OperationPreparationResult, PipelineError> {
        Self {
            req,
            persisted_documents_runtime: &shared_state.persisted_documents_runtime,
            plugin_req_state,
            body,
            require_id: shared_state.router_config.persisted_documents.require_id,
            persisted_documents_enabled: shared_state.router_config.persisted_documents.enabled,
            log_missing_id_requests: shared_state
                .router_config
                .persisted_documents
                .log_missing_id,
            client_identity: ClientIdentity {
                name: client_name,
                version: client_version,
            },
            metrics: shared_state.telemetry_context.metrics.clone(),
        }
        .extract_and_resolve()
        .await
    }

    async fn extract_and_resolve(mut self) -> Result<OperationPreparationResult, PipelineError> {
        let mut graphql_params_from_plugins = None;
        let mut graphql_params_end_callbacks = Vec::new();

        if let Some(plugin_req_state) = self.plugin_req_state.as_ref() {
            let mut deserialization_payload: OnGraphQLParamsStartHookPayload =
                OnGraphQLParamsStartHookPayload {
                    router_http_request: &plugin_req_state.router_http_request,
                    context: &plugin_req_state.context,
                    body: self.body.clone(),
                    graphql_params: None,
                };

            for plugin in plugin_req_state.plugins.as_ref() {
                let result = plugin.on_graphql_params(deserialization_payload).await;
                deserialization_payload = result.payload;
                match result.control_flow {
                    StartControlFlow::Proceed => {}
                    StartControlFlow::EndWithResponse(response) => {
                        return Ok(OperationPreparationResult::EarlyResponse(response));
                    }
                    StartControlFlow::OnEnd(callback) => {
                        graphql_params_end_callbacks.push(callback);
                    }
                }
            }

            graphql_params_from_plugins = deserialization_payload.graphql_params;
            self.body = deserialization_payload.body;
        }

        let mut operation = self.decode_or_use_plugin_override(graphql_params_from_plugins)?;

        if self.persisted_documents_enabled && operation.resolved_document_id.is_none() {
            self.metrics.persisted_documents.record_missing_id();
        }

        if self.persisted_documents_enabled
            && self.log_missing_id_requests
            && operation.resolved_document_id.is_none()
        {
            info!(
                event = "persisted_documents.missing_id_request",
                method = %self.req.method(),
                path = %self.req.uri().path(),
                require_id = self.require_id,
                operation_name = operation.graphql_params.operation_name.as_deref().unwrap_or(""),
                operation_body = operation.graphql_params.query.as_deref().unwrap_or(""),
                client_name = self.client_identity.name.unwrap_or(""),
                client_version = self.client_identity.version.unwrap_or(""),
                "request without document id"
            );
        }

        self.enforce_require_id_policy(&mut operation)?;

        // Apollo's APQ requests may include both `query` and a persisted id/hash in `extensions`.
        // The require-id policy above normalizes query/id precedence before this branch.
        if self.persisted_documents_enabled && operation.graphql_params.query.is_none() {
            self.resolve_query_from_document_id(&mut operation).await?;
        }

        if let Some(plugin_req_state) = self.plugin_req_state.as_ref() {
            let mut payload = OnGraphQLParamsEndHookPayload {
                graphql_params: operation.graphql_params,
                context: &plugin_req_state.context,
            };

            for callback in graphql_params_end_callbacks {
                let result = callback(payload);
                payload = result.payload;
                match result.control_flow {
                    EndControlFlow::Proceed => {}
                    EndControlFlow::EndWithResponse(response) => {
                        return Ok(OperationPreparationResult::EarlyResponse(response));
                    }
                }
            }

            operation.graphql_params = payload.graphql_params;
        }

        Ok(OperationPreparationResult::Operation(operation))
    }

    #[inline]
    fn decode_or_use_plugin_override(
        &self,
        graphql_params_override: Option<GraphQLParams>,
    ) -> Result<PreparedOperation, PipelineError> {
        if let Some(graphql_params) = graphql_params_override {
            return Ok(PreparedOperation::from_graphql_params(
                graphql_params,
                &self.persisted_documents_runtime.document_id_resolver,
                self.req.into(),
                None,
                None,
            ));
        }

        match *self.req.method() {
            Method::GET => self.decode_get(),
            Method::POST => self.decode_post(),
            _ => {
                warn!("unsupported HTTP method: {}", self.req.method());
                Err(PipelineError::UnsupportedHttpMethod(
                    self.req.method().to_owned(),
                ))
            }
        }
    }

    #[inline]
    fn decode_get(&self) -> Result<PreparedOperation, PipelineError> {
        let query_params_str = self.req.uri().query();
        let query_params = if let Some(q) = query_params_str {
            Query::<GraphQLGetInput>::from_query(q)?.0
        } else {
            // We need it to be able to use Persisted Documents in `GET /graphql/:id` format
            GraphQLGetInput::empty()
        };

        PreparedOperation::from_get(
            query_params,
            &self.persisted_documents_runtime.document_id_resolver,
            self.req.into(),
        )
    }

    #[inline]
    fn decode_post(&self) -> Result<PreparedOperation, PipelineError> {
        match self.req.headers().get(CONTENT_TYPE) {
            Some(value) => {
                let content_type_str = value
                    .to_str()
                    .map_err(|_| PipelineError::InvalidHeaderValue(CONTENT_TYPE))?;
                if !content_type_str.contains(SingleContentType::JSON.as_ref()) {
                    warn!(
                        "Invalid content type on a POST request: {}",
                        content_type_str
                    );
                    return Err(PipelineError::UnsupportedContentType);
                }
            }
            None => {
                trace!("POST without content type detected");
                return Err(PipelineError::MissingContentTypeHeader);
            }
        }

        let mut deserializer = sonic_rs::Deserializer::from_slice(&self.body);

        let post_input =
            GraphQLPostBodySeed::new(&self.persisted_documents_runtime.document_id_resolver)
                .deserialize(&mut deserializer)
                .map_err(PipelineError::FailedToParseBody)?;

        // Calling end() is important to ensure there is no trailing garbage after the JSON payload.
        // Without calling it, this might be accepted:
        // {"query":"{ me { id } }"} garbage
        // or even:
        // {"query":"{ me { id } }"}{"another":"object"}
        deserializer
            .end()
            .map_err(PipelineError::FailedToParseBody)?;

        Ok(PreparedOperation::from_post(
            post_input,
            &self.persisted_documents_runtime.document_id_resolver,
            self.req.into(),
        ))
    }

    #[inline]
    fn enforce_require_id_policy(
        &self,
        prepared_operation: &mut PreparedOperation,
    ) -> Result<(), PipelineError> {
        if !self.persisted_documents_enabled {
            // If persisted documents are disabled, clear the resolved document ID,
            // as it's not meant to be used in that case.
            prepared_operation.resolved_document_id = None;
            return Ok(());
        }

        if self.require_id {
            // If require_id is set, clear the query to make the document ID-based resolution mandatory.
            prepared_operation.graphql_params.query = None;
            if prepared_operation.resolved_document_id.is_none() {
                return Err(PipelineError::PersistedDocumentIdRequired);
            }
            return Ok(());
        }

        if prepared_operation.graphql_params.query.is_some() {
            // if a query is present, clear the resolved document ID,
            // as the query takes precedence over the document ID.
            prepared_operation.resolved_document_id = None;
        }

        Ok(())
    }

    #[inline]
    async fn resolve_query_from_document_id(
        &self,
        prepared_operation: &mut PreparedOperation,
    ) -> Result<(), PipelineError> {
        if let Some(document_id) = prepared_operation.resolved_document_id.as_ref() {
            let resolver = self
                .persisted_documents_runtime
                .persisted_document_resolver
                .as_ref()
                .ok_or_else(|| {
                    PipelineError::PersistedDocumentResolution(
                        "Persisted documents storage is not configured".to_string(),
                    )
                })?;

            let resolved = resolver
                .resolve(PersistedDocumentResolveInput {
                    persisted_document_id: document_id,
                    client_identity: self.client_identity,
                })
                .await
                .map_err(|error| {
                    self.metrics.persisted_documents.record_resolution_failure();
                    PipelineError::from(error)
                })?;

            prepared_operation.graphql_params.query = Some(resolved.text.to_string());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use async_trait::async_trait;
    use hive_router_config::persisted_documents::PersistedDocumentsConfig;
    use hive_router_internal::telemetry::metrics::Metrics;
    use hive_router_plan_executor::hooks::on_graphql_params::GraphQLParams;
    use hive_router_plan_executor::plugin_context::PluginRequestState;
    use ntex::util::Bytes;
    use ntex::web::test::TestRequest;
    use ntex::web::HttpRequest;

    use super::{OperationPreparation, PreparedOperation};
    use crate::pipeline::error::PipelineError;
    use crate::pipeline::persisted_documents::extract::DocumentIdResolver;
    use crate::pipeline::persisted_documents::resolve::{
        PersistedDocumentResolveInput, PersistedDocumentResolver, PersistedDocumentResolverError,
        ResolvedDocument,
    };
    use crate::pipeline::persisted_documents::types::{ClientIdentity, PersistedDocumentId};
    use crate::pipeline::persisted_documents::PersistedDocumentsRuntime;

    struct StaticResolver {
        document: Arc<str>,
    }

    #[async_trait]
    impl PersistedDocumentResolver for StaticResolver {
        async fn resolve(
            &self,
            _input: PersistedDocumentResolveInput<'_>,
        ) -> Result<ResolvedDocument, PersistedDocumentResolverError> {
            Ok(ResolvedDocument {
                text: Arc::clone(&self.document),
            })
        }
    }

    fn document_id_resolver() -> DocumentIdResolver {
        DocumentIdResolver::from_config(&PersistedDocumentsConfig::default(), "/graphql")
            .expect("resolver config should compile")
    }

    fn request() -> HttpRequest {
        TestRequest::with_uri("/graphql").to_http_request()
    }

    fn operation(query: Option<&str>, persisted_id: Option<&str>) -> PreparedOperation {
        PreparedOperation {
            graphql_params: GraphQLParams {
                query: query.map(ToString::to_string),
                operation_name: None,
                variables: HashMap::new(),
                extensions: None,
            },
            resolved_document_id: PersistedDocumentId::from_option(persisted_id),
        }
    }

    #[ntex::test]
    async fn resolves_query_from_persisted_document_id() {
        let req = request();
        let resolver = Arc::new(document_id_resolver());
        let persisted_resolver: Arc<dyn PersistedDocumentResolver> = Arc::new(StaticResolver {
            document: Arc::<str>::from("query { me { id } }"),
        });
        let persisted_documents_runtime = PersistedDocumentsRuntime {
            document_id_resolver: resolver,
            persisted_document_resolver: Some(persisted_resolver.clone()),
        };
        let plugin_req_state: Option<PluginRequestState<'_>> = None;
        let prep = OperationPreparation {
            req: &req,
            persisted_documents_runtime: &persisted_documents_runtime,
            plugin_req_state: &plugin_req_state,
            body: Bytes::new(),
            require_id: false,
            persisted_documents_enabled: true,
            log_missing_id_requests: false,
            client_identity: ClientIdentity::default(),
            metrics: Arc::new(Metrics::new(None)),
        };
        let mut op = PreparedOperation {
            graphql_params: GraphQLParams {
                query: None,
                operation_name: None,
                variables: HashMap::new(),
                extensions: None,
            },
            resolved_document_id: Some(PersistedDocumentId::try_from("sha256:abc").unwrap()),
        };

        prep.resolve_query_from_document_id(&mut op)
            .await
            .expect("query should resolve");

        assert_eq!(
            op.graphql_params.query.as_deref(),
            Some("query { me { id } }")
        );
    }

    #[test]
    fn require_id_enabled_drops_query_and_keeps_id() {
        let req = request();
        let resolver = Arc::new(document_id_resolver());
        let persisted_documents_runtime = PersistedDocumentsRuntime {
            document_id_resolver: resolver,
            persisted_document_resolver: None,
        };
        let plugin_req_state: Option<PluginRequestState<'_>> = None;
        let prep = OperationPreparation {
            req: &req,
            persisted_documents_runtime: &persisted_documents_runtime,
            plugin_req_state: &plugin_req_state,
            body: Bytes::new(),
            require_id: true,
            persisted_documents_enabled: true,
            log_missing_id_requests: false,
            client_identity: ClientIdentity::default(),
            metrics: Arc::new(Metrics::new(None)),
        };
        let mut op = operation(Some("query { me { id } }"), Some("sha256:abc"));

        prep.enforce_require_id_policy(&mut op)
            .expect("require_id policy should pass");

        assert!(op.graphql_params.query.is_none());
        assert!(op.resolved_document_id.is_some());
    }

    #[test]
    fn require_id_enabled_without_id_returns_required_error() {
        let req = request();
        let resolver = Arc::new(document_id_resolver());
        let persisted_documents_runtime = PersistedDocumentsRuntime {
            document_id_resolver: resolver,
            persisted_document_resolver: None,
        };
        let plugin_req_state: Option<PluginRequestState<'_>> = None;
        let prep = OperationPreparation {
            req: &req,
            persisted_documents_runtime: &persisted_documents_runtime,
            plugin_req_state: &plugin_req_state,
            body: Bytes::new(),
            require_id: true,
            persisted_documents_enabled: true,
            log_missing_id_requests: false,
            client_identity: ClientIdentity::default(),
            metrics: Arc::new(Metrics::new(None)),
        };
        let mut op = operation(Some("query { me { id } }"), None);

        let err = prep
            .enforce_require_id_policy(&mut op)
            .expect_err("missing id should fail");

        assert!(matches!(err, PipelineError::PersistedDocumentIdRequired));
    }

    #[test]
    fn require_id_disabled_query_wins_and_drops_id() {
        let req = request();
        let resolver = Arc::new(document_id_resolver());
        let persisted_documents_runtime = PersistedDocumentsRuntime {
            document_id_resolver: resolver,
            persisted_document_resolver: None,
        };
        let plugin_req_state: Option<PluginRequestState<'_>> = None;
        let prep = OperationPreparation {
            req: &req,
            persisted_documents_runtime: &persisted_documents_runtime,
            plugin_req_state: &plugin_req_state,
            body: Bytes::new(),
            require_id: false,
            persisted_documents_enabled: true,
            log_missing_id_requests: false,
            client_identity: ClientIdentity::default(),
            metrics: Arc::new(Metrics::new(None)),
        };
        let mut op = operation(Some("query { me { id } }"), Some("sha256:abc"));

        prep.enforce_require_id_policy(&mut op)
            .expect("policy should pass");

        assert!(op.graphql_params.query.is_some());
        assert!(op.resolved_document_id.is_none());
    }

    #[test]
    fn persisted_documents_disabled_always_drops_id() {
        let req = request();
        let resolver = Arc::new(document_id_resolver());
        let persisted_documents_runtime = PersistedDocumentsRuntime {
            document_id_resolver: resolver,
            persisted_document_resolver: None,
        };
        let plugin_req_state: Option<PluginRequestState<'_>> = None;
        let prep = OperationPreparation {
            req: &req,
            persisted_documents_runtime: &persisted_documents_runtime,
            plugin_req_state: &plugin_req_state,
            body: Bytes::new(),
            require_id: true,
            persisted_documents_enabled: false,
            log_missing_id_requests: false,
            client_identity: ClientIdentity::default(),
            metrics: Arc::new(Metrics::new(None)),
        };
        let mut op = operation(Some("query { me { id } }"), Some("sha256:abc"));

        prep.enforce_require_id_policy(&mut op)
            .expect("policy should pass");

        assert!(op.graphql_params.query.is_some());
        assert!(op.resolved_document_id.is_none());
    }

    #[test]
    fn query_missing_with_require_id_disabled_keeps_persisted_id() {
        let req = request();
        let resolver = Arc::new(document_id_resolver());
        let persisted_documents_runtime = PersistedDocumentsRuntime {
            document_id_resolver: resolver,
            persisted_document_resolver: None,
        };
        let plugin_req_state: Option<PluginRequestState<'_>> = None;
        let prep = OperationPreparation {
            req: &req,
            persisted_documents_runtime: &persisted_documents_runtime,
            plugin_req_state: &plugin_req_state,
            body: Bytes::new(),
            require_id: false,
            persisted_documents_enabled: true,
            log_missing_id_requests: false,
            client_identity: ClientIdentity::default(),
            metrics: Arc::new(Metrics::new(None)),
        };
        let mut op = operation(None, Some("sha256:abc"));

        prep.enforce_require_id_policy(&mut op)
            .expect("policy should pass");

        assert!(op.graphql_params.query.is_none());
        assert!(op.resolved_document_id.is_some());
    }
}
