use ahash::{HashMap, HashMapExt};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use graphql_tools::parser::minify_query;
use graphql_tools::parser::query::{
    Definition, Document, OperationDefinition, Selection, SelectionSet,
};
use hive_console_sdk::agent::utils::normalize_operation as hive_sdk_normalize_operation;
use hive_router_internal::telemetry::traces::spans::graphql::{
    GraphQLParseSpan, GraphQLSpanOperationIdentity,
};
use hive_router_query_planner::utils::parsing::safe_parse_operation;
use xxhash_rust::xxh3::Xxh3;

use crate::pipeline::error::PipelineError;
use crate::pipeline::execution_request::ExecutionRequest;
use crate::shared_state::RouterSharedState;
use tracing::{error, trace, Instrument};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FragmentAnalysisStatus {
    InProgress,
    IsIntrospectionOnly,
    IsNotIntrospection,
}

#[derive(Clone)]
pub struct ParseCacheEntry {
    document: Arc<Document<'static, String>>,
    document_minified_string: Arc<String>,
    hive_operation_hash: Arc<String>,
    is_introspection_only: bool,
}

#[derive(Debug, Clone)]
pub struct GraphQLParserPayload {
    pub parsed_operation: Arc<Document<'static, String>>,
    /// Whether the operation is a pure introspection query,
    /// meaning it does not require a query plan
    /// or communication with subgraphs.
    pub is_introspection_only: bool,
    pub minified_document: Arc<String>,
    pub operation_name: Option<String>,
    pub operation_type: String,
    pub cache_key: u64,
    pub cache_key_string: String,
    pub hive_operation_hash: Arc<String>,
}

impl<'a> From<&'a GraphQLParserPayload> for GraphQLSpanOperationIdentity<'a> {
    fn from(op_id: &'a GraphQLParserPayload) -> Self {
        GraphQLSpanOperationIdentity {
            name: op_id.operation_name.as_deref(),
            operation_type: &op_id.operation_type,
            client_document_hash: &op_id.cache_key_string,
        }
    }
}

#[inline]
pub async fn parse_operation_with_cache(
    app_state: &RouterSharedState,
    execution_params: &ExecutionRequest,
) -> Result<GraphQLParserPayload, PipelineError> {
    let parse_span = GraphQLParseSpan::new();

    async {
        let cache_key = {
            let mut hasher = Xxh3::new();
            execution_params.query.hash(&mut hasher);
            hasher.finish()
        };

        let parse_cache_item = if let Some(cached) = app_state.parse_cache.get(&cache_key).await {
            trace!("Found cached parsed operation for query");
            parse_span.record_cache_hit(true);
            cached
        } else {
            parse_span.record_cache_hit(false);
            let parsed = safe_parse_operation(&execution_params.query).map_err(|err| {
                error!("Failed to parse GraphQL operation: {}", err);
                PipelineError::FailedToParseOperation(err)
            })?;
            trace!("sucessfully parsed GraphQL operation");

            let is_pure_introspection = is_introspection_query_only(
                &parsed,
                execution_params.operation_name.as_ref().map(|n| n.as_str()),
            );

            let parsed_arc = Arc::new(parsed);
            let minified_arc = {
                Arc::new(
                    minify_query(execution_params.query.as_str()).map_err(|err| {
                        error!("Failed to minify parsed GraphQL operation: {}", err);
                        PipelineError::FailedToMinifyParsedOperation(err.to_string())
                    })?,
                )
            };

            let hive_normalized_operation = hive_sdk_normalize_operation(&parsed_arc);
            let hive_minified = minify_query(hive_normalized_operation.to_string().as_ref())
                .map_err(|err| {
                    error!(
                        "Failed to minify GraphQL operation normalized for Hive SDK: {}",
                        err
                    );
                    PipelineError::FailedToMinifyParsedOperation(err.to_string())
                })?;

            let entry = ParseCacheEntry {
                document: parsed_arc,
                document_minified_string: minified_arc,
                hive_operation_hash: Arc::new(format!("{:x}", md5::compute(hive_minified))),
                is_introspection_only: is_pure_introspection,
            };

            app_state.parse_cache.insert(cache_key, entry.clone()).await;
            entry
        };

        let parsed_operation = parse_cache_item.document;

        let cache_key_string = cache_key.to_string();

        let (operation_type, operation_name) =
            match parsed_operation
                .definitions
                .iter()
                .find_map(|def| match def {
                    Definition::Operation(op) => Some(op),
                    _ => None,
                }) {
                Some(OperationDefinition::Query(def)) => {
                    ("query", def.name.as_ref().map(|s| s.to_string()))
                }
                Some(OperationDefinition::Mutation(def)) => {
                    ("mutation", def.name.as_ref().map(|s| s.to_string()))
                }
                Some(OperationDefinition::Subscription(def)) => {
                    ("subscription", def.name.as_ref().map(|s| s.to_string()))
                }
                Some(OperationDefinition::SelectionSet(_)) => ("query", None),
                None => {
                    // This should not happen as we must have at least one operation definition
                    // but just in case, we handle it gracefully,
                    // the error will be caught later in the pipeline, specifically in the validation stage
                    ("query", None)
                }
            };

        let payload = GraphQLParserPayload {
            parsed_operation,
            is_introspection_only: parse_cache_item.is_introspection_only,
            minified_document: parse_cache_item.document_minified_string,
            operation_name,
            operation_type: operation_type.to_string(),
            cache_key,
            cache_key_string,
            hive_operation_hash: parse_cache_item.hive_operation_hash.clone(),
        };

        parse_span.record_operation_identity((&payload).into());

        Ok(payload)
    }
    .instrument(parse_span.clone())
    .await
}

/// Checks if all fields (including those in fragments) are introspection fields
pub fn is_introspection_query_only<'a>(
    query: &Document<'a, String>,
    operation_name: Option<&str>,
) -> bool {
    let operation = query.definitions.iter().find_map(|def| match def {
        Definition::Operation(op) => {
            if operation_name.is_none() {
                return Some(op);
            }

            let bingo = match op {
                OperationDefinition::Query(q) => q.name.as_deref() == operation_name,
                OperationDefinition::Mutation(m) => m.name.as_deref() == operation_name,
                OperationDefinition::Subscription(s) => s.name.as_deref() == operation_name,
                // If we're looking for named, anonymous should be dropped,
                // If we're looking for anonymous, first operation will be used.
                // That's why we drop anonymous here.
                OperationDefinition::SelectionSet(_) => false,
            };

            if bingo {
                return Some(op);
            }

            None
        }
        _ => None,
    });

    let selection_set = match operation {
        Some(OperationDefinition::Query(def)) => &def.selection_set,
        Some(OperationDefinition::SelectionSet(sel)) => sel,
        // Mutations and Subscriptions are never pure introspection
        _ => return false,
    };

    // Early escape for queries with no fragment spreads
    if !contains_fragment_spreads(selection_set) {
        // TODO: when having 100000000000 inline fragmnents, it will iterate over them all, potentiall vulnerability
        return check_root_fields_without_fragments(selection_set);
    }

    // Builds fragments HashMap only if we have fragment spreads
    let fragments: HashMap<_, _> = query
        .definitions
        .iter()
        .filter_map(|def| match def {
            Definition::Fragment(frag) => Some((frag.name.as_str(), &frag.selection_set)),
            _ => None,
        })
        .collect();

    let mut fragment_cache: HashMap<&str, FragmentAnalysisStatus> =
        HashMap::with_capacity(fragments.len());
    check_root_fields_with_fragments(selection_set, &fragments, &mut fragment_cache)
}

fn contains_fragment_spreads(selection_set: &SelectionSet<'_, String>) -> bool {
    selection_set.items.iter().any(|sel| match sel {
        Selection::FragmentSpread(_) => true,
        Selection::InlineFragment(inf) => contains_fragment_spreads(&inf.selection_set),
        _ => false,
    })
}

fn check_root_fields_without_fragments(selection_set: &SelectionSet<'_, String>) -> bool {
    for selection in &selection_set.items {
        match selection {
            Selection::Field(field) => {
                if !field.name.as_str().starts_with("__") {
                    return false;
                }
            }
            Selection::InlineFragment(inline_frag) => {
                if !check_root_fields_without_fragments(&inline_frag.selection_set) {
                    return false;
                }
            }
            Selection::FragmentSpread(_) => {
                unreachable!(
                    r"
                    FragmentSpread found in check_root_fields_without_fragments.
                    This function should only be called after contains_fragment_spreads() returns false.
                    If you're seeing this panic, the early escape logic in is_introspection_query_only() is broken.
                  "
                )
            }
        }
    }
    true
}

fn check_root_fields_with_fragments<'a>(
    selection_set: &'a SelectionSet<'_, String>,
    fragments: &'a HashMap<&str, &SelectionSet<'_, String>>,
    fragment_cache: &mut HashMap<&'a str, FragmentAnalysisStatus>,
) -> bool {
    for selection in &selection_set.items {
        match selection {
            Selection::Field(field) => {
                if !field.name.as_str().starts_with("__") {
                    return false;
                }
            }
            Selection::InlineFragment(inline_frag) => {
                if !check_root_fields_with_fragments(
                    &inline_frag.selection_set,
                    fragments,
                    fragment_cache,
                ) {
                    return false;
                }
            }
            Selection::FragmentSpread(frag_spread) => {
                let fragment_name = frag_spread.fragment_name.as_str();

                // Check the cache first
                match fragment_cache.get(fragment_name) {
                    Some(FragmentAnalysisStatus::InProgress) => {
                        // Cycle detected - we're already analyzing this fragment
                        // This is safe to skip since if it had non-introspection fields,
                        // we would have detected them when we first started analyzing it
                        continue;
                    }
                    Some(FragmentAnalysisStatus::IsIntrospectionOnly) => {
                        // Already cached and confirmed to be introspection-only
                        continue;
                    }
                    Some(FragmentAnalysisStatus::IsNotIntrospection) => {
                        // Already cached and confirmed to not be introspection-only
                        return false;
                    }
                    None => {
                        // Not in cache, so analyze it
                    }
                }

                let Some(frag_sel_set) = fragments.get(fragment_name) else {
                    // Fragment not found - conservative approach -> not pure introspection
                    return false;
                };

                // Mark as in-progress to detect cycles
                fragment_cache.insert(fragment_name, FragmentAnalysisStatus::InProgress);
                let result =
                    check_root_fields_with_fragments(frag_sel_set, fragments, fragment_cache);

                // Update cache with final result
                let final_status = if result {
                    FragmentAnalysisStatus::IsIntrospectionOnly
                } else {
                    FragmentAnalysisStatus::IsNotIntrospection
                };
                fragment_cache.insert(fragment_name, final_status);

                if !result {
                    return false;
                }
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_query(query: &str) -> Document<'static, String> {
        safe_parse_operation(query).expect("Failed to parse query")
    }

    // List of queries that ARE introspection-only
    const INTROSPECTION_ONLY_QUERIES: &[&str] = &[
        r#"
            {
              __typename
            }
        "#,
        r#"
            {
              __schema {
                types {
                  name
                }
              }
            }
        "#,
        r#"
            {
              __type(name: "Query") {
                name
              }
            }
        "#,
        r#"
            {
              __typename
              __schema {
                types {
                  name
                }
              }
            }
        "#,
        r#"
            query {
              ...SchemaFields
            }
            fragment SchemaFields on Query {
              __schema {
                types {
                  name
                }
              }
            }
        "#,
        r#"
            query {
              ...SchemaFields
              ...TypeFields
            }
            fragment SchemaFields on Query {
              __schema {
                types {
                  name
                }
              }
            }
            fragment TypeFields on Query {
              __type(name: "Query") {
                name
              }
            }
        "#,
        r#"
            query {
              ... on Query {
                __schema {
                  types {
                    name
                  }
                }
              }
            }
        "#,
        r#"
            {
              __type(name: "Query") {
                __typename
              }
            }
        "#,
        r#"
            {
              __type(name: "Query") {
                fields {
                  name
                }
              }
            }
        "#,
        r#"
            {
              __schema {
                types {
                  __typename
                  fields {
                    __typename
                  }
                }
              }
            }
        "#,
        r#"
            query GetSchema {
              __schema {
                types {
                  name
                }
              }
            }
        "#,
        r#"
            {
              __schema {
                user {
                  id
                }
              }
            }
        "#,
        r#"
            query {
              ... on Query {
                __typename
              }
            }
        "#,
        r#"
            query {
              ...SchemaFields
            }
            fragment SchemaFields on Query {
              __schema {
                user {
                  id
                }
              }
            }
        "#,
        r#"
            query {
              ... on Query {
                __typename
              }
              ...SchemaFields
            }
            fragment SchemaFields on Query {
              __schema {
                types {
                  name
                }
              }
            }
        "#,
        r#"
            query {
              ... on Query {
                ... on Query {
                  __typename
                }
              }
            }
        "#,
        r#"
            query {
              ...FragA
            }
            fragment FragA on Query {
              ...FragB
            }
            fragment FragB on Query {
              __typename
            }
        "#,
        r#"
            query {
              ...FragA
            }
            fragment FragA on Query {
              ...FragB
            }
            fragment FragB on Query {
              ...FragC
            }
            fragment FragC on Query {
              __schema {
                types {
                  name
                }
              }
            }
        "#,
        r#"
            query {
              ... on Query {
                __typename
                ...FragA
              }
            }
            fragment FragA on Query {
              ... on Query {
                __schema {
                  types {
                    name
                  }
                }
              }
              ...FragB
            }
            fragment FragB on Query {
              __type(name: "Query") {
                name
              }
            }
        "#,
    ];

    // List of queries that are NOT introspection-only
    const NON_INTROSPECTION_QUERIES: &[&str] = &[
        r#"
            {
              user {
                id
              }
            }
        "#,
        r#"
            {
              __typename
              user {
                id
              }
            }
        "#,
        r#"
            mutation {
              createUser(name: "John") {
                id
              }
            }
        "#,
        r#"
            subscription {
              userCreated {
                id
              }
            }
        "#,
        r#"
            query {
              ...UserFields
            }
            fragment UserFields on Query {
              user {
                id
              }
            }
        "#,
        r#"
            query {
              ...MixedFields
            }
            fragment MixedFields on Query {
              __typename
              user {
                id
              }
            }
        "#,
        r#"
            query {
              ... on Query {
                user {
                  id
                }
              }
            }
        "#,
        r#"
            query {
              ...UndefinedFragment
            }
        "#,
        r#"
            query {
              ... on Query {
                user {
                  id
                }
              }
            }
        "#,
        r#"
            query {
              ... on Query {
                user {
                  id
                }
                ... on Query {
                  __typename
                }
              }
            }
        "#,
        r#"
            query {
              ... on Query {
                __typename
              }
              ...UserFields
            }
            fragment UserFields on Query {
              user {
                id
              }
            }
        "#,
        r#"
            query {
              ... on Query {
                ... on Query {
                  user {
                    id
                  }
                }
              }
            }
        "#,
        r#"
            query {
              ...FragA
            }
            fragment FragA on Query {
              ...FragB
            }
            fragment FragB on Query {
              user {
                id
              }
            }
        "#,
        r#"
            query {
              ...FragA
            }
            fragment FragA on Query {
              ...FragB
            }
            fragment FragB on Query {
              ...FragC
            }
            fragment FragC on Query {
              user {
                id
              }
            }
        "#,
        r#"
            query {
              ... on Query {
                __typename
                ...FragA
              }
            }
            fragment FragA on Query {
              ... on Query {
                __schema {
                  types {
                    name
                  }
                }
              }
              ...FragB
            }
            fragment FragB on Query {
              user {
                id
              }
            }
        "#,
    ];

    #[test]
    fn test_all_introspection_only_queries_return_true() {
        for (index, query_str) in INTROSPECTION_ONLY_QUERIES.iter().enumerate() {
            let query = parse_query(query_str);
            assert!(
                is_introspection_query_only(&query, None),
                "Query at index {} incorrectly identified as mixed: {}",
                index,
                query_str
            );
        }
    }

    #[test]
    fn test_all_non_introspection_queries_return_false() {
        for (index, query_str) in NON_INTROSPECTION_QUERIES.iter().enumerate() {
            let query = parse_query(query_str);
            assert!(
                !is_introspection_query_only(&query, None),
                "Query at index {} incorrectly identified as introspection-only: {}",
                index,
                query_str
            );
        }
    }

    #[test]
    fn test_self_referencing() {
        let query = parse_query(
            r#"
              query { ...FragA }
              fragment FragA on Query {
                ...FragA
                __typename
              }
            "#,
        );
        assert!(is_introspection_query_only(&query, None));

        let query = parse_query(
            r#"
              query { ...FragA }
              fragment FragA on Query {
                ...FragA
                __typename
                foo
              }
            "#,
        );
        assert!(!is_introspection_query_only(&query, None));

        let query = parse_query(
            r#"
              query { ...FragA }
              fragment FragA on Query { ...FragB __typename }
              fragment FragB on Query { ...FragA __typename }
          "#,
        );
        assert!(is_introspection_query_only(&query, None));

        let query = parse_query(
            r#"
              query { ...FragA }
              fragment FragA on Query { ...FragB __typename }
              fragment FragB on Query { ...FragA foo }
          "#,
        );
        assert!(!is_introspection_query_only(&query, None));

        let query = parse_query(
            r#"
            query { ...FragA }
              fragment FragA on Query {
                __typename ...FragA
              }
          "#,
        );
        assert!(is_introspection_query_only(&query, None));

        let query = parse_query(
            r#"
            query { ...FragA }
              fragment FragA on Query {
                __typename foo ...FragA
              }
          "#,
        );
        assert!(!is_introspection_query_only(&query, None));
    }
}
