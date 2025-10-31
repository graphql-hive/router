use crate::pipeline::coerce_variables::CoerceVariablesPayload;
use crate::pipeline::normalize::GraphQLNormalizationPayload;
use crate::utils::StrByAddr;
use ahash::HashMap;

use hive_router_config::authentication::UnauthorizedMode;
use hive_router_config::HiveRouterConfig;
use hive_router_plan_executor::execution::client_request_details::JwtRequestDetails;
use hive_router_plan_executor::introspection::schema::SchemaMetadata;
use hive_router_plan_executor::response::graphql_error::{GraphQLError, GraphQLErrorExtensions};
use hive_router_query_planner::ast::selection_set::{FieldSelection, InlineFragmentSelection};
use hive_router_query_planner::ast::value::Value;
use hive_router_query_planner::ast::{
    operation::OperationDefinition, selection_item::SelectionItem, selection_set::SelectionSet,
};
use hive_router_query_planner::federation_spec::authorization::{
    AuthenticatedDirective, RequiresScopesDirective,
};
use hive_router_query_planner::state::supergraph_state::{SupergraphDefinition, SupergraphState};
use lasso2::{Rodeo, Spur};
use std::collections::HashSet;
use std::hash::Hash;

type ScopeId = Spur;
type ScopeInterner = Rodeo;

/// Contains the authorization details for a single incoming request.
pub struct UserAuthContext {
    pub is_authenticated: bool,
    /// The user's scopes, converted into cheap `ScopeId`s for fast lookups.
    pub scope_ids: HashSet<ScopeId>,
}

impl UserAuthContext {
    /// Creates a context from the request's auth details and the global metadata.
    pub fn new(
        is_authenticated: bool,
        scopes_from_jwt: &[String],
        auth_metadata: &AuthorizationMetadata,
    ) -> Self {
        let scope_ids = scopes_from_jwt
            .iter()
            // Use `get` to convert scope strings into ScopeIds.
            // A scope in the JWT might not exist in the schema, so we safely ignore it.
            .filter_map(|s| auth_metadata.scopes.get(s))
            .collect();

        Self {
            is_authenticated,
            scope_ids,
        }
    }
}

/// Represents a group of scopes that are required together (The AND logic).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScopeAndGroup(Vec<ScopeId>);

/// Represents the full requirements of a `@requiresScopes` directive (The OR logic).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RequiredScopes(Vec<ScopeAndGroup>);

/// An enum representing all possible authorization rules for a given field or type.
#[derive(Debug, Clone)]
pub enum AuthorizationRule {
    /// Represents the `@authenticated` directive.
    Authenticated,
    /// Represents the `@requiresScopes` directive with its normalized scope structure.
    /// Implies authentication as well.
    RequiresScopes(RequiredScopes),
}

type TypeRulesMap = HashMap<String, AuthorizationRule>;
type FieldRulesMap = HashMap<String, AuthorizationRule>;
type TypeFieldRulesMap = HashMap<String, FieldRulesMap>;

/// A container for all pre-computed authorization metadata.
/// This is built once at startup and shared across all requests.
#[derive(Debug)]
pub struct AuthorizationMetadata {
    /// Rules applied to types
    pub type_rules: TypeRulesMap,
    /// Rules applied to specific fields - (TypeName, FieldName) tuple.
    pub field_rules: TypeFieldRulesMap,
    /// The interner that holds all known scope strings from the schema.
    pub scopes: ScopeInterner,
}

#[derive(thiserror::Error, Debug)]
pub enum AuthorizationMetadataError {
    #[error("Invalid scope value: {0}")]
    InvalidScopeValue(String),
    #[error("Invalid @requiresScopes(scope:) argument: {0}")]
    InvalidRequiresScopesArgs(String),
    #[error("Duplicate @requiresScopes directives found")]
    DuplicateRequiresScopesDirective,
}

impl AuthorizationMetadata {
    /// Builds the complete authorization metadata from a given public api schema.
    ///
    /// Called only once at router startup.
    /// It iterates through the entire schema,
    /// finds all authorization directives, normalizes them,
    /// and stores them in a format optimized for fast per-request lookups.
    pub fn build(supergraph: &SupergraphState) -> Result<Self, AuthorizationMetadataError> {
        let mut type_rules = HashMap::default();
        let mut field_rules = HashMap::default();
        let mut scopes = ScopeInterner::new();

        for type_def in supergraph.definitions.values() {
            Self::process_type_definition(
                type_def,
                &mut type_rules,
                &mut field_rules,
                &mut scopes,
            )?;
        }

        Ok(Self {
            type_rules,
            field_rules,
            scopes,
        })
    }

    pub fn is_type_authorized(&self, type_name: &str, user_context: &UserAuthContext) -> bool {
        if let Some(rule) = self.type_rules.get(type_name) {
            if !self.is_rule_satisfied(rule, user_context) {
                // Type rule failed, the type is unauthorized.
                return false;
            }
        }

        // No rule or rule passed
        true
    }

    pub fn is_field_authorized(
        &self,
        type_name: &str,
        field_name: &str,
        user_context: &UserAuthContext,
    ) -> bool {
        if let Some(rule) = self
            .field_rules
            .get(type_name)
            .and_then(|fields| fields.get(field_name))
        {
            if !self.is_rule_satisfied(rule, user_context) {
                // Type rule failed, the type is unauthorized.
                return false;
            }
        }

        // No rule or rule passed
        true
    }

    fn is_rule_satisfied(&self, rule: &AuthorizationRule, user_context: &UserAuthContext) -> bool {
        match rule {
            AuthorizationRule::Authenticated => user_context.is_authenticated,
            AuthorizationRule::RequiresScopes(required) => {
                // A user must be authenticated to have any scopes.
                if !user_context.is_authenticated {
                    return false;
                }

                // The user's scopes must satisfy at least one of the OR groups.
                required.0.iter().any(|and_group| {
                    // The user's scopes must contain ALL of the scopes in this AND group.
                    and_group
                        .0
                        .iter()
                        .all(|scope_id| user_context.scope_ids.contains(scope_id))
                })
            }
        }
    }

    /// Processes a `TypeDefinition`, extracting rules for the type and its fields.
    pub fn process_type_definition(
        type_def: &SupergraphDefinition,
        type_rules: &mut TypeRulesMap,
        field_rules: &mut TypeFieldRulesMap,
        scopes_interner: &mut ScopeInterner,
    ) -> Result<(), AuthorizationMetadataError> {
        let (type_name, authenticated_directives, requires_scopes_directives, maybe_fields) =
            match type_def {
                SupergraphDefinition::Scalar(s) => {
                    (&s.name, &s.authenticated, &s.requires_scopes, None)
                }
                SupergraphDefinition::Object(o) => (
                    &o.name,
                    &o.authenticated,
                    &o.requires_scopes,
                    Some(&o.fields),
                ),
                SupergraphDefinition::Interface(i) => (
                    &i.name,
                    &i.authenticated,
                    &i.requires_scopes,
                    Some(&i.fields),
                ),
                SupergraphDefinition::Enum(e) => {
                    (&e.name, &e.authenticated, &e.requires_scopes, None)
                }
                // Unions and InputObjects do not have output authorization rules applicable here.
                SupergraphDefinition::Union(_) | SupergraphDefinition::InputObject(_) => {
                    return Ok(())
                }
            };

        // Handle rules on the type itself.
        // We're going to use it when checking output types of fields.
        if let Some(rule) = Self::extract_rule_from_directives(
            authenticated_directives,
            requires_scopes_directives,
            scopes_interner,
        )? {
            type_rules.insert(type_name.clone(), rule);
        }

        // Handle rules on the fields of the type.
        // We're going to use it when checking fields under this type.
        if let Some(fields) = maybe_fields {
            let mut type_field_rules = FieldRulesMap::default();
            for (field_name, field_def) in fields {
                if let Some(rule) = Self::extract_rule_from_directives(
                    &field_def.authenticated,
                    &field_def.requires_scopes,
                    scopes_interner,
                )? {
                    type_field_rules.insert(field_name.clone(), rule);
                }
            }

            if !type_field_rules.is_empty() {
                field_rules.insert(type_name.clone(), type_field_rules);
            }
        }
        Ok(())
    }

    /// Extracts the highest priority authorization rule from a set of directives.
    pub fn extract_rule_from_directives(
        authenticated_directives: &[AuthenticatedDirective],
        requires_scopes_directives: &[RequiresScopesDirective],
        interner: &mut ScopeInterner,
    ) -> Result<Option<AuthorizationRule>, AuthorizationMetadataError> {
        // If multiple @requiresScopes directives are present, we consider it an error.
        // The composition should have merged them into one.
        if requires_scopes_directives.len() > 1 {
            return Err(AuthorizationMetadataError::DuplicateRequiresScopesDirective);
        }

        // Check for `@requiresScopes`.
        // If `@authenticated` is also present, `@requiresScopes` takes precedence,
        // as it is a stricter requirement and implies authentication.
        if let Some(directive) = requires_scopes_directives.first() {
            let scopes = Self::normalize_scopes_arg(&directive.scopes, interner)?;
            return Ok(Some(AuthorizationRule::RequiresScopes(scopes)));
        }

        // Check for `@authenticated`.
        if !authenticated_directives.is_empty() {
            return Ok(Some(AuthorizationRule::Authenticated));
        }

        Ok(None)
    }

    /// Parses and normalizes the `scopes` argument `Value` from a `@requiresScopes` directive.
    /// This represents the top-level OR group.
    pub fn normalize_scopes_arg(
        value: &Value,
        interner: &mut ScopeInterner,
    ) -> Result<RequiredScopes, AuthorizationMetadataError> {
        // The top-level value must be a list (the OR group).
        let or_groups_val = if let Value::List(list) = value {
            list
        } else {
            return Err(AuthorizationMetadataError::InvalidRequiresScopesArgs(
                format!("expected a list, got '{}'", value),
            ));
        };

        let mut or_groups: Vec<_> = or_groups_val
            .iter()
            .map(|and_group_val| Self::normalize_and_group(and_group_val, interner))
            .collect::<Result<_, _>>()?;

        if or_groups.is_empty() {
            return Err(AuthorizationMetadataError::InvalidRequiresScopesArgs(
                "expected at least one AND group, got none".to_string(),
            ));
        }

        // Sort the outer list for a fully canonical representation.
        or_groups.sort();
        Ok(RequiredScopes(or_groups))
    }

    /// Parses and normalizes a single AND group from within the `scopes` argument.
    pub fn normalize_and_group(
        value: &Value,
        interner: &mut ScopeInterner,
    ) -> Result<ScopeAndGroup, AuthorizationMetadataError> {
        // Each AND group must also be a list.
        let and_group_val = if let Value::List(list) = value {
            list
        } else {
            return Err(AuthorizationMetadataError::InvalidRequiresScopesArgs(
                "expected a list for AND group".to_string(),
            ));
        };

        let mut and_group: Vec<ScopeId> = and_group_val
            .iter()
            .map(|scope_val| {
                if let Value::String(s) = scope_val {
                    Ok(interner.get_or_intern(s))
                } else {
                    Err(AuthorizationMetadataError::InvalidScopeValue(format!(
                        "expected scope to be a string, got: '{}'",
                        scope_val
                    )))
                }
            })
            .collect::<Result<_, _>>()?;

        if and_group.is_empty() {
            // TODO: how should the router act when the @requiresScopes(scopes:) is malformed and incorrect? We could warn and ignore wrong values, but user's intent was to enforce access and now there's are partial requirements or no requirements at all.
            return Err(AuthorizationMetadataError::InvalidRequiresScopesArgs(
                "empty AND group, expected at least one scope".to_string(),
            ));
        }

        // Sort for canonical representation, essential for the binary search lookups
        and_group.sort();
        Ok(ScopeAndGroup(and_group))
    }
}

#[derive(Debug, Default)]
struct PathNode {
    /// Mapping from path segment (field name/alias) to child node index in the `nodes` arena.
    children: HashMap<Spur, usize>,
    /// If true, the field corresponding to this path is denied.
    is_denied: bool,
}

/// An index-based (flattened) Trie for storing unauthorized field paths efficiently.
#[derive(Debug)]
pub struct UnauthorizedPathTree {
    /// The "arena" where all nodes are stored.
    /// The root node is always at index 0.
    nodes: Vec<PathNode>,
    /// The string interner for path segments (field names/aliases).
    interner: Rodeo,
}

impl UnauthorizedPathTree {
    fn new() -> Self {
        Self {
            nodes: vec![PathNode::default()], // Root node at index 0
            interner: Rodeo::new(),
        }
    }

    /// Records a path to an unauthorized field
    fn mark_path_as_unauthorized(&mut self, path: &[&str]) {
        let mut current_node_idx = 0;
        for segment in path {
            let symbol = self.interner.get_or_intern(segment);

            // Check if the child exists without holding a mutable borrow for too long.
            if let Some(&child_node_idx) = self.nodes[current_node_idx].children.get(&symbol) {
                // If the child exists, we just move to it.
                current_node_idx = child_node_idx;
            } else {
                // The child does not exist. We need to create it.

                // Create the new node and get its index.
                let new_node_idx = self.nodes.len();
                self.nodes.push(PathNode::default());

                let parent_node = &mut self.nodes[current_node_idx];
                parent_node.children.insert(symbol, new_node_idx);

                // Update the position to the newly created node
                current_node_idx = new_node_idx;
            }
        }

        // Mark the final node as the end of an unauthorized path.
        self.nodes[current_node_idx].is_denied = true;
    }

    fn has_unauthorized_paths(&self) -> bool {
        // The Trie is empty if the root node has no children.
        !self.nodes[0].children.is_empty()
    }

    /// Finds the child of a given node corresponding to a path segment.
    fn find_child_segment(&self, parent_node_idx: usize, segment: &str) -> Option<(usize, bool)> {
        let symbol = self.interner.get(segment)?;
        let parent_node = &self.nodes[parent_node_idx];

        parent_node.children.get(&symbol).map(|&child_idx| {
            let is_end = self.nodes[child_idx].is_denied;
            (child_idx, is_end)
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizationError {
    pub path: Vec<String>,
}

pub enum AuthorizationDecision {
    /// The operation is fully authorized. Continue with the original operation.
    NoChange,
    /// The operation was modified to remove unauthorized parts. Continue with the new operation.
    Modified {
        new_operation_definition: OperationDefinition,
        errors: Vec<AuthorizationError>,
    },
    /// The operation should be aborted due to unauthorized access and mode: rejected.
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
                affected_path: Some(auth_error.path.join(".")),
                ..Default::default()
            },
        }
    }
}

pub fn apply_authorization_to_operation(
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

    if auth_metadata.field_rules.is_empty() && auth_metadata.type_rules.is_empty() {
        return AuthorizationDecision::NoChange;
    }

    // Create the user-specific context.
    let user_context = match jwt_request_details {
        JwtRequestDetails::Authenticated { scopes, .. } => {
            UserAuthContext::new(true, scopes.as_deref().unwrap_or(&[]), auth_metadata)
        }
        JwtRequestDetails::Unauthenticated => UserAuthContext::new(false, &[], auth_metadata),
    };

    // Scan the operation for unauthorized fields.
    let mut unauthorized_paths_trie = UnauthorizedPathTree::new();
    let mut validated_types_cache = HashSet::new();
    let mut errors = Vec::new();
    let mut current_path = Vec::with_capacity(32);
    let mut context = CollectorContext {
        schema_metadata,
        variable_payload,
        auth_metadata,
        user_context: &user_context,
        unauthorized_paths: &mut unauthorized_paths_trie,
        validated_types_cache: &mut validated_types_cache,
        current_path: &mut current_path,
        errors: &mut errors,
    };
    collect_unauthorized_paths(
        &normalized_payload.operation_for_plan.selection_set,
        normalized_payload.root_type_name,
        &mut context,
    );

    // If the trie is empty, no fields were unauthorized. Return the original payload.
    if !unauthorized_paths_trie.has_unauthorized_paths() {
        return AuthorizationDecision::NoChange;
    }

    // If the mode is "reject", abort the operation.
    if router_config.authentication.directives.unauthorized.mode == UnauthorizedMode::Reject {
        tracing::debug!("Request rejected due to unauthorized fields and reject mode being set",);
        return AuthorizationDecision::Reject { errors };
    }

    // Rebuild a new, authorized operation.
    let new_operation = rebuild_authorized_operation(
        &normalized_payload.operation_for_plan,
        &unauthorized_paths_trie,
    );

    AuthorizationDecision::Modified {
        new_operation_definition: new_operation,
        errors,
    }
}

/// Checks if a field should be ignored based on `@skip` and `@include` directives, and variable payload.
fn is_field_ignored(field: &FieldSelection, variable_payload: &CoerceVariablesPayload) -> bool {
    is_selection_ignored(&field.skip_if, &field.include_if, variable_payload)
}

/// Checks if a fragment should be ignored based on `@skip` and `@include` directives, and variable payload.
fn is_fragment_ignored(
    fragment: &InlineFragmentSelection,
    variable_payload: &CoerceVariablesPayload,
) -> bool {
    is_selection_ignored(&fragment.skip_if, &fragment.include_if, variable_payload)
}

fn is_selection_ignored(
    skip_if: &Option<String>,
    include_if: &Option<String>,
    variable_payload: &CoerceVariablesPayload,
) -> bool {
    // A selection is ignored if the @skip(if: true) directive is present and the `if` argument is true.
    if let Some(variable_name) = skip_if {
        if variable_payload.variable_equals_true(variable_name) {
            return true;
        }
    }

    // A selection is ignored if the @include(if: false) directive is present and the `if` argument is false.
    if let Some(variable_name) = include_if {
        if !variable_payload.variable_equals_true(variable_name) {
            return true;
        }
    }

    false
}

struct CollectorContext<'req, 'auth> {
    schema_metadata: &'req SchemaMetadata,
    variable_payload: &'req CoerceVariablesPayload,
    auth_metadata: &'req AuthorizationMetadata,
    user_context: &'req UserAuthContext,
    unauthorized_paths: &'auth mut UnauthorizedPathTree,
    validated_types_cache: &'auth mut HashSet<StrByAddr<'req>>,
    current_path: &'auth mut Vec<&'req str>,
    errors: &'auth mut Vec<AuthorizationError>,
}

fn collect_unauthorized_paths<'req, 'auth>(
    selection_set: &'req SelectionSet,
    parent_type_name: &str,
    context: &mut CollectorContext<'req, 'auth>,
) {
    let Some(type_fields) = context.schema_metadata.get_type_fields(parent_type_name) else {
        return;
    };

    for selection in &selection_set.items {
        match selection {
            SelectionItem::Field(field) => {
                if is_field_ignored(field, context.variable_payload) {
                    // Field is skipped due to conditional directives, so we ignore it for authorization.
                    // No need to check further or traverse its children.
                    continue;
                }

                let Some(output_type_name) = type_fields.get(&field.name) else {
                    continue;
                };
                let path_segment = field.alias.as_ref().unwrap_or(&field.name);

                context.current_path.push(path_segment);

                let is_authorized = check_authorization_for_field(
                    parent_type_name,
                    &field.name,
                    output_type_name,
                    context.auth_metadata,
                    context.user_context,
                    context.validated_types_cache,
                );

                if is_authorized {
                    collect_unauthorized_paths(&field.selections, output_type_name, context);
                } else {
                    context.errors.push(AuthorizationError {
                        path: context.current_path.iter().map(|s| s.to_string()).collect(),
                    });
                    context
                        .unauthorized_paths
                        .mark_path_as_unauthorized(context.current_path);
                }

                context.current_path.pop();
            }
            SelectionItem::InlineFragment(fragment) => {
                if is_fragment_ignored(fragment, context.variable_payload) {
                    // Fragment is skipped due to conditional directives, so we ignore it for authorization.
                    // No need to check further or traverse its children.
                    continue;
                }

                collect_unauthorized_paths(&fragment.selections, &fragment.type_condition, context);
            }
            SelectionItem::FragmentSpread(_) => {
                // Fragments spreads are inlined during normalization, so we can skip them here.
            }
        }
    }
}

/// Performs the authorization check for a single field
fn check_authorization_for_field<'auth>(
    parent_type_name: &str,
    field_name: &str,
    output_type_name: &'auth str,
    auth_metadata: &AuthorizationMetadata,
    user_context: &UserAuthContext,
    validated_types_cache: &mut HashSet<StrByAddr<'auth>>,
) -> bool {
    let output_type_key = StrByAddr(output_type_name);
    // Check the output type rule first. This is a critical optimization.
    // We only perform the expensive check if the type is not already in our cache.
    if !validated_types_cache.contains(&output_type_key) {
        if !auth_metadata.is_type_authorized(output_type_name, user_context) {
            // Type rule failed, the field is unauthorized.
            return false;
        }
        // Whether the rule passed or there was no rule, we cache the type name
        // to prevent looking it up again for this request.
        validated_types_cache.insert(output_type_key);
    }

    // If the type rule passed (or didn't exist), check the field-specific rule.
    if !auth_metadata.is_field_authorized(parent_type_name, field_name, user_context) {
        // Field-specific rule failed.
        return false;
    }

    // All applicable checks passed, so the field is authorized.
    true
}

/// Creates a new OperationDefinition, filtering out unauthorized paths.
fn rebuild_authorized_operation(
    original_operation: &OperationDefinition,
    unauthorized_paths: &UnauthorizedPathTree,
) -> OperationDefinition {
    OperationDefinition {
        name: original_operation.name.clone(),
        operation_kind: original_operation.operation_kind.clone(),
        selection_set: rebuild_authorized_selection_set(
            &original_operation.selection_set,
            unauthorized_paths,
            0, // The root of the trie is always at index 0.
        ),
        variable_definitions: original_operation.variable_definitions.clone(),
    }
}

/// Recursively traverses an original selection set and returns a new, filtered
/// selection set containing only authorized nodes.
fn rebuild_authorized_selection_set(
    original_selection_set: &SelectionSet,
    unauthorized_paths: &UnauthorizedPathTree,
    current_trie_node_idx: usize,
) -> SelectionSet {
    let mut authorized_items = Vec::with_capacity(original_selection_set.items.len());

    for selection in &original_selection_set.items {
        match selection {
            SelectionItem::Field(field) => {
                let path_segment = field.alias.as_ref().unwrap_or(&field.name);

                if let Some((child_node_idx, is_unauthorized)) =
                    unauthorized_paths.find_child_segment(current_trie_node_idx, path_segment)
                {
                    // This field's path exists in the trie.
                    if is_unauthorized {
                        // This exact field is unauthorized, so we drop it and do not recurse.
                        continue;
                    }

                    // The field itself is allowed, but has unauthorized children. Recurse.
                    let new_selections = rebuild_authorized_selection_set(
                        &field.selections,
                        unauthorized_paths,
                        child_node_idx,
                    );

                    if new_selections.is_empty() && !field.selections.is_empty() {
                        // All children were unauthorized, so we drop this field as well.
                        continue;
                    }

                    let mut new_field = field.clone();
                    new_field.selections = new_selections;
                    authorized_items.push(SelectionItem::Field(new_field));
                } else {
                    // This field is not on any unauthorized path, so we can keep it and its children as is.
                    authorized_items.push(selection.clone());
                }
            }
            SelectionItem::InlineFragment(fragment) => {
                // Recurse into the fragment's selection set. The trie context (node index)
                // remains the same because the fields inside are still direct children of the parent type.
                let new_selections = rebuild_authorized_selection_set(
                    &fragment.selections,
                    unauthorized_paths,
                    current_trie_node_idx,
                );

                // Only keep the fragment if it still contains any fields after filtering.
                if !new_selections.items.is_empty() {
                    let mut new_fragment = fragment.clone();
                    new_fragment.selections = new_selections;
                    authorized_items.push(SelectionItem::InlineFragment(new_fragment));
                }
            }
            SelectionItem::FragmentSpread(_) => {
                // Normalized operation has no fragment spreads as they are inlined
            }
        }
    }

    SelectionSet {
        items: authorized_items,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl UnauthorizedPathTree {
        /// Checks if a full path is marked as denied in the tree.
        fn is_path_denied(&self, path: &[&str]) -> bool {
            let mut current_node_idx = 0;
            for segment in path {
                if let Some((next_node_idx, _)) = self.find_child_segment(current_node_idx, segment)
                {
                    current_node_idx = next_node_idx;
                } else {
                    // If any segment of the path doesn't exist in the tree,
                    // then the full path cannot have been marked as denied.
                    return false;
                }
            }
            // After successfully traversing the whole path, check the `is_denied`
            // flag of the final node we landed on.
            self.nodes[current_node_idx].is_denied
        }
    }

    #[test]
    fn test_simple_path_denial() {
        let mut tree = UnauthorizedPathTree::new();
        tree.mark_path_as_unauthorized(&["users", "friends", "name"]);

        // The exact path that was marked should be denied.
        assert!(tree.is_path_denied(&["users", "friends", "name"]));

        // Paths that are prefixes of a denied path should NOT be denied themselves.
        assert!(!tree.is_path_denied(&["users", "friends"]));
        assert!(!tree.is_path_denied(&["users"]));

        // A path that doesn't exist in the tree should not be denied.
        assert!(!tree.is_path_denied(&["users", "favFriend", "name"]));
    }

    #[test]
    fn test_branching_paths() {
        let mut tree = UnauthorizedPathTree::new();
        tree.mark_path_as_unauthorized(&["me", "email"]);
        tree.mark_path_as_unauthorized(&["me", "address"]);

        // Both paths should be denied.
        assert!(tree.is_path_denied(&["me", "email"]));
        assert!(tree.is_path_denied(&["me", "address"]));

        // The common prefix should not be denied.
        assert!(false == tree.is_path_denied(&["me"]));

        // A different path from the same root should not be denied.
        assert!(false == tree.is_path_denied(&["me", "name"]));

        // Internally, the node for "me" (at index 1) should now have two children.
        // We can verify this by checking the number of nodes.
        // 1 (root) + 1 (me) + 1 (email) + 1 (address) = 4 nodes
        assert_eq!(tree.nodes.len(), 4);
        let me_node = &tree.nodes[1];
        assert_eq!(me_node.children.len(), 2);
    }

    #[test]
    fn test_nested_and_overlapping_paths() {
        let mut tree = UnauthorizedPathTree::new();
        tree.mark_path_as_unauthorized(&["a", "b", "c"]);

        // Mark a path that is a prefix of the already-denied path.
        tree.mark_path_as_unauthorized(&["a", "b"]);

        // Both the longer and shorter paths should now be denied.
        assert!(tree.is_path_denied(&["a", "b"]));
        assert!(tree.is_path_denied(&["a", "b", "c"]));

        // The intermediate path is not denied.
        assert!(false == tree.is_path_denied(&["a"]));
    }

    #[test]
    fn test_deeply_nested_recursive_like_path() {
        let mut tree = UnauthorizedPathTree::new();
        let path = &["me", "friends", "friends", "friends", "name"];
        tree.mark_path_as_unauthorized(path);

        // The full path should be denied.
        assert!(tree.is_path_denied(path));

        // No prefix of the path should be denied.
        assert!(false == tree.is_path_denied(&["me", "friends", "friends", "friends"]));
        assert!(false == tree.is_path_denied(&["me", "friends", "friends"]));
        assert!(false == tree.is_path_denied(&["me", "friends"]));
        assert!(false == tree.is_path_denied(&["me"]));
    }

    #[test]
    fn test_has_unauthorized_paths() {
        let mut tree = UnauthorizedPathTree::new();
        // Initially, there are no unauthorized paths.
        assert!(false == tree.has_unauthorized_paths());

        tree.mark_path_as_unauthorized(&["a"]);
        // After marking one path, it should report true.
        assert!(tree.has_unauthorized_paths());
    }
}
