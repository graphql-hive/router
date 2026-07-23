//! Authorization pipeline for GraphQL operations
//!
//! This module implements a three-phase authorization algorithm:
//! 1. **Metadata Phase** - Parse and store authorization rules from the schema
//! 2. **Analysis Phase** - Traverse operations, check authorization, apply null bubbling
//! 3. **Reconstruction Phase** - Rebuild operations/plans with unauthorized fields removed

#[cfg(test)]
mod tests;

pub mod metadata;

use std::sync::Arc;

use crate::pipeline::error::{ClientPipelineError, PipelineError};
use crate::pipeline::normalize::GraphQLNormalizationPayload;
use crate::pipeline::nullify::rebuilder::{
    rebuild_nulled_operation, rebuild_nulled_projection_plan,
};
use crate::pipeline::trie::Trie;
use crate::utils::StrByAddr;

use ahash::HashMap;
use hive_router_config::authorization::UnauthorizedMode;
use hive_router_config::HiveRouterConfig;
use hive_router_internal::authorization::metadata::{AuthorizationMetadata, AuthorizationRule};
use hive_router_plan_executor::execution::client_request_details::JwtRequestDetails;
use hive_router_plan_executor::execution::plan::CoerceVariablesPayload;
use hive_router_plan_executor::introspection::schema::SchemaMetadata;
use hive_router_plan_executor::operation_filter::{OperationFilter, Selection};
use hive_router_plan_executor::projection::plan::FieldProjectionPlan;
use hive_router_plan_executor::response::graphql_error::GraphQLError;
use hive_router_query_planner::ast::operation::OperationDefinition;

use hive_router_internal::telemetry::traces::spans::graphql::GraphQLAuthorizeSpan;
pub use metadata::{AuthorizationMetadataError, AuthorizationMetadataExt, UserAuthContext};

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
        GraphQLError::from_message_and_code(
            "Unauthorized field or type",
            "UNAUTHORIZED_FIELD_OR_TYPE",
        )
        .add_affected_path(&auth_error.path)
    }
}

impl From<&GraphQLError> for AuthorizationError {
    fn from(error: &GraphQLError) -> Self {
        AuthorizationError {
            path: error.extensions.affected_path.clone().unwrap_or_default(),
        }
    }
}

fn unauthorized_error() -> GraphQLError {
    GraphQLError::from_message_and_code("Unauthorized field or type", "UNAUTHORIZED_FIELD_OR_TYPE")
}

struct AuthorizationChecker<'a, 'op> {
    auth_metadata: &'a AuthorizationMetadata,
    user_context: &'a UserAuthContext,
    cache: HashMap<StrByAddr<'op>, bool>,
}

impl<'op> AuthorizationChecker<'_, 'op> {
    /// Returns `true` if this type (or any field/nested type under it) has authorization
    /// rules that need to be checked.
    /// When `false`, everything under this type can be skipped.
    fn parent_has_auth(&self, parent_type_name: &str) -> bool {
        self.auth_metadata
            .type_has_any_auth
            .get(parent_type_name)
            .copied()
            .unwrap_or(true)
    }

    fn is_type_authorized(&mut self, type_name: &'op str) -> bool {
        let key = StrByAddr(type_name);

        if let Some(&authorized) = self.cache.get(&key) {
            return authorized;
        }

        let authorized = self
            .auth_metadata
            .type_rules
            .get(type_name)
            .is_none_or(|rule| self.is_rule_satisfied(rule));

        self.cache.insert(key, authorized);

        authorized
    }

    fn is_field_authorized(&self, type_name: &str, field_name: &str) -> bool {
        let Some(rule) = self
            .auth_metadata
            .field_rules
            .get(type_name)
            .and_then(|fields| fields.get(field_name))
        else {
            return true;
        };

        self.is_rule_satisfied(rule)
    }

    fn is_rule_satisfied(&self, rule: &AuthorizationRule) -> bool {
        match rule {
            AuthorizationRule::Authenticated => self.user_context.is_authenticated,
            AuthorizationRule::RequiresScopes(scopes) => {
                self.user_context.is_authenticated
                    && scopes.0.iter().any(|and_group| {
                        and_group
                            .0
                            .iter()
                            .all(|scope_id| self.user_context.scope_ids.contains(scope_id))
                    })
            }
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
    normalized_payload: &Arc<GraphQLNormalizationPayload>,
    auth_metadata: &AuthorizationMetadata,
    schema_metadata: &SchemaMetadata,
    variable_payload: &CoerceVariablesPayload,
    jwt_request_details: &JwtRequestDetails,
) -> Result<(Arc<GraphQLNormalizationPayload>, Vec<AuthorizationError>), PipelineError> {
    if !router_config.authorization.directives.enabled {
        return Ok((normalized_payload.clone(), vec![]));
    }

    if !router_config.jwt.enabled {
        return Ok((normalized_payload.clone(), vec![]));
    }

    let span = GraphQLAuthorizeSpan::new();
    let _guard = span.span.enter();

    let reject_mode =
        router_config.authorization.directives.unauthorized.mode == UnauthorizedMode::Reject;

    let decision = apply_authorization_to_operation(
        normalized_payload,
        auth_metadata,
        schema_metadata,
        variable_payload,
        jwt_request_details,
        reject_mode,
    )?;

    Ok(match decision {
        AuthorizationDecision::NoChange => (normalized_payload.clone(), vec![]),
        AuthorizationDecision::Modified {
            new_operation_definition,
            new_projection_plan,
            errors,
        } => (
            normalized_payload.with_operation(new_operation_definition, new_projection_plan),
            errors,
        ),
        AuthorizationDecision::Reject { errors } => {
            return Err(ClientPipelineError::AuthorizationFailed(errors).into());
        }
    })
}

pub fn apply_authorization_to_operation(
    normalized_payload: &GraphQLNormalizationPayload,
    auth_metadata: &AuthorizationMetadata,
    schema_metadata: &SchemaMetadata,
    variable_payload: &CoerceVariablesPayload,
    jwt_request_details: &JwtRequestDetails,
    reject_mode: bool,
) -> Result<AuthorizationDecision, PipelineError> {
    if auth_metadata.is_empty() {
        return Ok(AuthorizationDecision::NoChange);
    }

    let user_context = UserAuthContext::from_jwt(jwt_request_details, auth_metadata);

    // Early exit if authenticated users satisfy all rules
    if user_context.is_authenticated && auth_metadata.scopes.is_empty() {
        return Ok(AuthorizationDecision::NoChange);
    }

    // Phase 1 & 2: Walk the operation, decide per-field/per-fragment
    // authorization, and let `OperationFilter` bubble non-null rejections up to
    // the nearest nullable ancestor.

    let mut checker = AuthorizationChecker {
        auth_metadata,
        user_context: &user_context,
        cache: HashMap::default(),
    };

    let operation_filter_output = OperationFilter::new(schema_metadata).filter(
        &normalized_payload.root_type_name,
        &normalized_payload.operation_for_plan.selection_set,
        variable_payload,
        |selection| match selection {
            Selection::Field(field) => {
                if !checker.parent_has_auth(field.parent_type_name) {
                    return selection.keep();
                }

                let is_authorized = checker.is_type_authorized(field.parent_type_name)
                    && checker.is_field_authorized(field.parent_type_name, field.field_name)
                    && checker.is_type_authorized(field.output_type_name);

                if is_authorized {
                    selection.keep()
                } else {
                    selection.reject(unauthorized_error())
                }
            }
            Selection::Fragment(fragment) => {
                if !checker.parent_has_auth(fragment.parent_type_name) {
                    return selection.keep();
                }

                if checker.is_type_authorized(fragment.type_condition) {
                    selection.keep()
                } else {
                    selection.reject(unauthorized_error())
                }
            }
        },
    )?;

    if operation_filter_output.errors.is_empty() {
        return Ok(AuthorizationDecision::NoChange);
    }

    let errors: Vec<AuthorizationError> = operation_filter_output
        .errors
        .iter()
        .map(AuthorizationError::from)
        .collect();

    if reject_mode {
        tracing::debug!("Request rejected due to unauthorized fields and reject mode being set");
        return Ok(AuthorizationDecision::Reject { errors });
    }

    // Phase 3: Reconstruct the operation without unauthorized paths

    let nulled_field_trie = Trie::from_paths(&operation_filter_output.rejected_paths);

    let new_operation =
        rebuild_nulled_operation(&normalized_payload.operation_for_plan, &nulled_field_trie);
    let new_projection_plan =
        rebuild_nulled_projection_plan(&normalized_payload.projection_plan, &nulled_field_trie);

    Ok(AuthorizationDecision::Modified {
        new_operation_definition: new_operation,
        new_projection_plan,
        errors,
    })
}
