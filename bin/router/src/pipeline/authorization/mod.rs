//! Authorization pipeline for GraphQL operations
//!
//! This module implements a three-phase authorization algorithm:
//! 1. **Metadata Phase** - Parse and store authorization rules from the schema
//! 2. **Analysis Phase** - Traverse operations, check authorization, apply null bubbling
//! 3. **Reconstruction Phase** - Rebuild operations/plans with unauthorized fields removed

#[cfg(test)]
mod tests;

mod collector;
mod metadata;
mod rebuilder;
mod tree;

use crate::pipeline::authorization::collector::{
    collect_authorization_statuses, propagate_null_bubbling,
};
use crate::pipeline::authorization::rebuilder::{
    rebuild_authorized_operation, rebuild_authorized_projection_plan,
};
use crate::pipeline::authorization::tree::UnauthorizedPathTrie;
use crate::pipeline::coerce_variables::CoerceVariablesPayload;
use crate::pipeline::normalize::GraphQLNormalizationPayload;

use hive_router_config::authentication::UnauthorizedMode;
use hive_router_config::HiveRouterConfig;
use hive_router_plan_executor::execution::client_request_details::JwtRequestDetails;
use hive_router_plan_executor::introspection::schema::SchemaMetadata;
use hive_router_plan_executor::projection::plan::FieldProjectionPlan;
use hive_router_plan_executor::response::graphql_error::{GraphQLError, GraphQLErrorExtensions};
use hive_router_query_planner::ast::operation::OperationDefinition;

pub use metadata::{
    AuthorizationMetadata, AuthorizationMetadataError, ScopeId, ScopeInterner, UserAuthContext,
};

/// Error representing an unauthorized field access.
///
/// Contains the path from the root of the operation to the unauthorized field,
/// allowing clients to understand exactly which part of their query failed authorization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizationError {
    /// Dot-separated path from root to unauthorized field (e.g., "user.posts.title")
    pub path: String,
}

/// The result of authorization enforcement on a GraphQL operation.
#[derive(Debug)]
pub enum AuthorizationDecision {
    /// The operation is fully authorized. Continue with the original operation.
    NoChange,
    /// The operation was modified to remove unauthorized parts. Continue with the new operation.
    Modified {
        new_operation_definition: OperationDefinition,
        new_projection_plan: Vec<FieldProjectionPlan>,
        errors: Vec<AuthorizationError>,
    },
    /// The operation should be aborted due to unauthorized access and reject mode being enabled.
    Reject { errors: Vec<AuthorizationError> },
}

impl From<&AuthorizationError> for GraphQLError {
    fn from(auth_error: &AuthorizationError) -> Self {
        GraphQLError {
            message: "Unauthorized field or type".into(),
            path: None,
            locations: None,
            extensions: GraphQLErrorExtensions {
                code: Some("UNAUTHORIZED_FIELD_OR_TYPE".into()),
                affected_path: Some(auth_error.path.clone()),
                ..Default::default()
            },
        }
    }
}

/// Main entry point for authorization enforcement.
///
/// Checks if authorization is enabled and delegates to the authorization pipeline
/// if needed. Returns a decision indicating whether the operation should proceed
/// unchanged, be modified, or be rejected.
pub fn enforce_operation_authorization(
    router_config: &HiveRouterConfig,
    normalized_payload: &GraphQLNormalizationPayload,
    auth_metadata: &AuthorizationMetadata,
    schema_metadata: &SchemaMetadata,
    variable_payload: &CoerceVariablesPayload,
    jwt_request_details: &JwtRequestDetails<'_>,
) -> AuthorizationDecision {
    if !router_config.authentication.directives.enabled {
        return AuthorizationDecision::NoChange;
    }

    if !router_config.jwt.enabled {
        return AuthorizationDecision::NoChange;
    }

    let reject_mode =
        router_config.authentication.directives.unauthorized.mode == UnauthorizedMode::Reject;

    apply_authorization_to_operation(
        normalized_payload,
        auth_metadata,
        schema_metadata,
        variable_payload,
        jwt_request_details,
        reject_mode,
    )
}

pub fn apply_authorization_to_operation(
    normalized_payload: &GraphQLNormalizationPayload,
    auth_metadata: &AuthorizationMetadata,
    schema_metadata: &SchemaMetadata,
    variable_payload: &CoerceVariablesPayload,
    jwt_request_details: &JwtRequestDetails<'_>,
    reject_mode: bool,
) -> AuthorizationDecision {
    if auth_metadata.field_rules.is_empty() && auth_metadata.type_rules.is_empty() {
        return AuthorizationDecision::NoChange;
    }

    let user_context = create_user_auth_context(jwt_request_details, auth_metadata);

    // Early exit if authenticated users satisfy all rules
    if user_context.is_authenticated && auth_metadata.scopes.is_empty() {
        return AuthorizationDecision::NoChange;
    }

    // Phase 1: Collect authorization status for all fields

    let collection_result = collect_authorization_statuses(
        &normalized_payload.operation_for_plan.selection_set,
        normalized_payload.root_type_name,
        schema_metadata,
        variable_payload,
        auth_metadata,
        &user_context,
    );

    if collection_result.errors.is_empty() {
        return AuthorizationDecision::NoChange;
    }

    if reject_mode {
        tracing::debug!("Request rejected due to unauthorized fields and reject mode being set");
        return AuthorizationDecision::Reject {
            errors: collection_result.errors,
        };
    }

    // Phase 2: Apply GraphQL null bubbling semantics
    // Unauthorized non-null fields must "bubble up" and nullify their parents

    let removal_flags = if collection_result.has_non_null_unauthorized {
        propagate_null_bubbling(&collection_result.checks)
    } else {
        // No non-null unauthorized fields, so no bubbling needed
        vec![false; collection_result.checks.len()]
    };

    // Phase 3: Reconstruct the operation without unauthorized paths

    let unauthorized_path_trie =
        UnauthorizedPathTrie::from_checks(&collection_result.checks, &removal_flags);

    let new_operation = rebuild_authorized_operation(
        &normalized_payload.operation_for_plan,
        &unauthorized_path_trie,
    );
    let new_projection_plan = rebuild_authorized_projection_plan(
        &normalized_payload.projection_plan,
        &unauthorized_path_trie,
    );

    AuthorizationDecision::Modified {
        new_operation_definition: new_operation,
        new_projection_plan,
        errors: collection_result.errors,
    }
}

/// Creates user authorization context from JWT details.
fn create_user_auth_context(
    jwt_request_details: &JwtRequestDetails<'_>,
    auth_metadata: &AuthorizationMetadata,
) -> UserAuthContext {
    match jwt_request_details {
        JwtRequestDetails::Authenticated { scopes, .. } => {
            UserAuthContext::new(true, scopes.as_deref().unwrap_or(&[]), auth_metadata)
        }
        JwtRequestDetails::Unauthenticated => UserAuthContext::new(false, &[], auth_metadata),
    }
}
