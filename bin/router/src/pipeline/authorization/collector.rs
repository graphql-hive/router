use crate::pipeline::coerce_variables::CoerceVariablesPayload;
use crate::utils::StrByAddr;
use ahash::HashSet;
use hive_router_plan_executor::introspection::schema::SchemaMetadata;
use hive_router_query_planner::ast::selection_set::{FieldSelection, InlineFragmentSelection};
use hive_router_query_planner::ast::{selection_item::SelectionItem, selection_set::SelectionSet};

use super::metadata::{AuthorizationMetadata, UserAuthContext};
use super::AuthorizationError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PathSegment<'a> {
    Field(&'a str),
    TypeCondition(&'a str),
}

impl<'a> PathSegment<'a> {
    #[inline(always)]
    pub fn as_str(&self) -> &'a str {
        match self {
            PathSegment::Field(name) => name,
            PathSegment::TypeCondition(name) => name,
        }
    }
}

/// Each `CheckIndex` points to a specific `FieldCheck` in the checks vector.
/// This provides type safety to prevent mixing up different kinds of indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct CheckIndex(usize);

impl CheckIndex {
    #[inline(always)]
    pub(super) fn new(index: usize) -> Self {
        Self(index)
    }

    #[inline(always)]
    pub(super) fn get(self) -> usize {
        self.0
    }
}

/// Authorization status for a field, which determines null bubbling behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FieldAuthStatus {
    Authorized,
    UnauthorizedNullable,
    UnauthorizedNonNullable,
}

/// Authorization check for a single field in the operation.
///
/// Stores the authorization result and maintains a link to the parent field
/// for path reconstruction.
///
/// For a query like `{ user { posts { title } } }`, we'd have checks like:
///
/// ```text
/// checks[0]: FieldCheck { parent: None,    path_segment: "user",  status: Authorized }
/// checks[1]: FieldCheck { parent: Some(0), path_segment: "posts", status: Authorized }
/// checks[2]: FieldCheck { parent: Some(1), path_segment: "title", status: UnauthorizedNullable }
/// ```
#[derive(Debug)]
pub(super) struct FieldCheck<'a> {
    pub(super) parent_check_index: Option<CheckIndex>,
    pub(super) path_segment: PathSegment<'a>,
    pub(super) status: FieldAuthStatus,
}

/// Result of collecting authorization statuses for all fields in the operation.
#[derive(Debug)]
pub(super) struct AuthorizationCollectionResult<'a> {
    pub(super) has_non_null_unauthorized: bool,
    pub(super) checks: Vec<FieldCheck<'a>>,
    pub(super) errors: Vec<AuthorizationError>,
}

/// Maps each check index to its child check indices, representing the hierarchical
/// structure of the GraphQL selection set during authorization analysis.
///
/// Used during null bubbling to propagate removal of unauthorized non-nullable fields
/// up to their parents.
///
/// For a query like `{ user { email, posts { title, author } } }`:
///
/// The CheckTree would store:
///
/// ```text
/// tree[0] = [1, 2]     // user has children: email, posts
/// tree[1] = []         // email has no children
/// tree[2] = [3, 4]     // posts has children: title, author
/// tree[3] = []         // title has no children
/// tree[4] = []         // author has no children
/// ```
#[derive(Debug)]
pub(super) struct CheckTree(Vec<Vec<CheckIndex>>);

impl CheckTree {
    fn new() -> Self {
        Self(Vec::with_capacity(64))
    }

    /// Adds a new node to the tree. Must be called for each check in order.
    fn ensure_field(&mut self) {
        self.0.push(Vec::new());
    }

    /// Adds a child to a parent check's children list.
    fn add_child_field(&mut self, parent_index: CheckIndex, child_index: CheckIndex) {
        self.0[parent_index.get()].push(child_index);
    }

    fn get_children(&self, check_index: CheckIndex) -> &[CheckIndex] {
        &self.0[check_index.get()]
    }
}

/// Context for authorization collection with cached data.
struct AuthorizationCollector<'req, 'auth> {
    schema_metadata: &'req SchemaMetadata,
    variable_payload: &'req CoerceVariablesPayload,
    auth_metadata: &'req AuthorizationMetadata,
    user_context: &'req UserAuthContext,
    validated_types_cache: &'auth mut HashSet<StrByAddr<'req>>,
    errors: &'auth mut Vec<AuthorizationError>,
    checks: &'auth mut Vec<FieldCheck<'req>>,
}

struct TraversalState<'req> {
    selection_set: &'req SelectionSet,
    parent_type_name: &'req str,
    parent_check_index: Option<CheckIndex>,
}

/// Collects authorization status for all fields.
pub(super) fn collect_authorization_statuses<'req>(
    selection_set: &'req SelectionSet,
    root_type_name: &'req str,
    schema_metadata: &'req SchemaMetadata,
    variable_payload: &'req CoerceVariablesPayload,
    auth_metadata: &'req AuthorizationMetadata,
    user_context: &'req UserAuthContext,
) -> AuthorizationCollectionResult<'req> {
    // Check root type (Query/Mutation) authorization once
    // before iterating through all fields.
    // If the root type is unauthorized, all its fields are unauthorized.
    // This optimization avoids unnecessary per-field checks.
    if !auth_metadata.is_type_authorized(root_type_name, user_context) {
        // Root type is unauthorized - mark all top-level fields as errors
        let mut errors = Vec::with_capacity(selection_set.items.len());
        let mut checks = Vec::with_capacity(selection_set.items.len());
        let mut has_non_null_unauthorized = false;

        let type_fields = schema_metadata.get_type_fields(root_type_name);

        for item in &selection_set.items {
            if let SelectionItem::Field(field) = item {
                let field_alias = field.alias.as_ref().unwrap_or(&field.name);

                // Check field nullability to ensure GraphQL spec compliance.
                // Non-nullable root fields must be marked as UnauthorizedNonNullable
                // to trigger null bubbling behavior (which invalidates the entire response).
                let status = if let Some(field_info) = type_fields.and_then(|f| f.get(&field.name))
                {
                    if field_info.is_non_null {
                        has_non_null_unauthorized = true;
                        FieldAuthStatus::UnauthorizedNonNullable
                    } else {
                        FieldAuthStatus::UnauthorizedNullable
                    }
                } else {
                    // Field not found in schema - treat as nullable
                    FieldAuthStatus::UnauthorizedNullable
                };

                // Add check for this field so the rebuilder knows to remove it
                checks.push(FieldCheck {
                    parent_check_index: None,
                    path_segment: PathSegment::Field(field_alias),
                    status,
                });

                errors.push(AuthorizationError {
                    path: field_alias.to_string(),
                });
            }
        }

        return AuthorizationCollectionResult {
            has_non_null_unauthorized,
            checks,
            errors,
        };
    }

    let mut checks = Vec::with_capacity(64);

    let mut validated_types_cache = HashSet::default();
    let mut errors = Vec::new();
    let mut has_non_null_unauthorized = false;

    // Mark root type as validated since we just checked it above
    validated_types_cache.insert(StrByAddr(root_type_name));

    let mut collector_context = AuthorizationCollector {
        schema_metadata,
        variable_payload,
        auth_metadata,
        user_context,
        validated_types_cache: &mut validated_types_cache,
        errors: &mut errors,
        checks: &mut checks,
    };

    collect_authorization_statuses_internal(
        selection_set,
        root_type_name,
        None,
        &mut collector_context,
        &mut has_non_null_unauthorized,
    );

    AuthorizationCollectionResult {
        has_non_null_unauthorized,
        checks,
        errors,
    }
}

/// Internal traversal that populates the field checks array.
fn collect_authorization_statuses_internal<'req, 'auth>(
    selection_set: &'req SelectionSet,
    parent_type_name: &'req str,
    parent_check_index: Option<CheckIndex>,
    context: &mut AuthorizationCollector<'req, 'auth>,
    has_non_null_unauthorized: &mut bool,
) {
    let mut stack = Vec::with_capacity(32);
    stack.push(TraversalState {
        selection_set,
        parent_type_name,
        parent_check_index,
    });

    while let Some(current_state) = stack.pop() {
        let type_fields = context
            .schema_metadata
            .get_type_fields(current_state.parent_type_name);

        // Check once per selection set
        let parent_has_auth = context
            .auth_metadata
            .type_has_any_auth
            .get(current_state.parent_type_name)
            .copied()
            .unwrap_or(true);

        for selection in &current_state.selection_set.items {
            match selection {
                SelectionItem::Field(field) => {
                    if is_field_ignored(field, context.variable_payload) {
                        continue;
                    }
                    let Some(field_info) = type_fields.and_then(|f| f.get(&field.name)) else {
                        continue;
                    };

                    let is_authorized = if parent_has_auth {
                        check_authorization_for_field(
                            current_state.parent_type_name,
                            &field.name,
                            &field_info.output_type_name,
                            context.auth_metadata,
                            context.user_context,
                            context.validated_types_cache,
                        )
                    } else {
                        true
                    };

                    let status = if is_authorized {
                        FieldAuthStatus::Authorized
                    } else if field_info.is_non_null {
                        FieldAuthStatus::UnauthorizedNonNullable
                    } else {
                        FieldAuthStatus::UnauthorizedNullable
                    };

                    if status == FieldAuthStatus::UnauthorizedNonNullable
                        && has_non_null_unauthorized == &false
                    {
                        *has_non_null_unauthorized = true;
                    }

                    let field_alias = field.alias.as_ref().unwrap_or(&field.name);
                    let current_check_index = CheckIndex::new(context.checks.len());

                    context.checks.push(FieldCheck {
                        parent_check_index: current_state.parent_check_index,
                        path_segment: PathSegment::Field(field_alias),
                        status,
                    });

                    // Skip traversing unauthorized field children
                    if status == FieldAuthStatus::Authorized {
                        stack.push(TraversalState {
                            selection_set: &field.selections,
                            parent_type_name: &field_info.output_type_name,
                            parent_check_index: Some(current_check_index),
                        });
                    } else {
                        context.errors.push(AuthorizationError {
                            path: build_error_path(
                                context.checks,
                                current_state.parent_check_index,
                                Some(field_alias),
                            ),
                        });
                    }
                }
                SelectionItem::InlineFragment(fragment) => {
                    if is_fragment_ignored(fragment, context.variable_payload) {
                        continue;
                    }

                    // Check if the concrete type is authorized
                    let is_type_authorized = if parent_has_auth {
                        check_authorization_for_type_condition(
                            &fragment.type_condition,
                            context.auth_metadata,
                            context.user_context,
                            context.validated_types_cache,
                        )
                    } else {
                        true
                    };

                    let status = if is_type_authorized {
                        FieldAuthStatus::Authorized
                    } else {
                        FieldAuthStatus::UnauthorizedNullable
                    };

                    let type_condition_check_index = CheckIndex::new(context.checks.len());
                    context.checks.push(FieldCheck {
                        parent_check_index: current_state.parent_check_index,
                        path_segment: PathSegment::TypeCondition(&fragment.type_condition),
                        status,
                    });

                    if status == FieldAuthStatus::Authorized {
                        stack.push(TraversalState {
                            selection_set: &fragment.selections,
                            parent_type_name: &fragment.type_condition,
                            parent_check_index: Some(type_condition_check_index),
                        });
                    } else {
                        context.errors.push(AuthorizationError {
                            // Create an error for the parent field.
                            path: build_error_path(
                                context.checks,
                                current_state.parent_check_index,
                                None,
                            ),
                        });
                    }
                }
                SelectionItem::FragmentSpread(_) => {
                    // Fragment spreads are inlined during normalization, so we can skip them here.
                }
            }
        }
    }
}

/// Checks field authorization (parent type rule + field rule + output type rule).
fn check_authorization_for_field<'auth>(
    parent_type_name: &'auth str,
    field_name: &str,
    output_type_name: &'auth str,
    auth_metadata: &AuthorizationMetadata,
    user_context: &UserAuthContext,
    validated_types_cache: &mut HashSet<StrByAddr<'auth>>,
) -> bool {
    let output_type_key = StrByAddr(output_type_name);
    // Cache type authorization checks
    if !validated_types_cache.contains(&output_type_key) {
        if !auth_metadata.is_type_authorized(output_type_name, user_context) {
            return false;
        }
        validated_types_cache.insert(output_type_key);
    }

    auth_metadata.is_field_authorized(parent_type_name, field_name, user_context)
}

/// Checks type authorization (type rule)
fn check_authorization_for_type_condition<'auth>(
    type_condition: &'auth str,
    auth_metadata: &AuthorizationMetadata,
    user_context: &UserAuthContext,
    validated_types_cache: &mut HashSet<StrByAddr<'auth>>,
) -> bool {
    let type_key = StrByAddr(type_condition);

    // Cache type authorization checks
    if !validated_types_cache.contains(&type_key) {
        if !auth_metadata.is_type_authorized(type_condition, user_context) {
            return false;
        }
        validated_types_cache.insert(type_key);
    }

    auth_metadata.is_type_authorized(type_condition, user_context)
}

/// Builds dot-separated path to unauthorized field.
fn build_error_path(
    checks: &[FieldCheck],
    parent_check_index: Option<CheckIndex>,
    field_alias: Option<&str>, // Changed to Option<&str>
) -> String {
    let mut segments = Vec::with_capacity(24);
    let mut current_index = parent_check_index;
    while let Some(index) = current_index {
        let check = &checks[index.get()];
        if let PathSegment::Field(response_key) = check.path_segment {
            segments.push(response_key);
        }
        current_index = check.parent_check_index;
    }
    segments.reverse();

    // Add the final segment only if it's provided.
    if let Some(alias) = field_alias {
        segments.push(alias);
    }

    segments.join(".")
}

#[inline]
fn is_field_ignored(field: &FieldSelection, variable_payload: &CoerceVariablesPayload) -> bool {
    is_selection_ignored(&field.skip_if, &field.include_if, variable_payload)
}

#[inline]
fn is_fragment_ignored(
    fragment: &InlineFragmentSelection,
    variable_payload: &CoerceVariablesPayload,
) -> bool {
    is_selection_ignored(&fragment.skip_if, &fragment.include_if, variable_payload)
}

#[inline]
fn is_selection_ignored(
    skip_if: &Option<String>,
    include_if: &Option<String>,
    variable_payload: &CoerceVariablesPayload,
) -> bool {
    if let Some(variable_name) = skip_if {
        if variable_payload.variable_equals_true(variable_name) {
            return true;
        }
    }

    if let Some(variable_name) = include_if {
        if !variable_payload.variable_equals_true(variable_name) {
            return true;
        }
    }

    false
}

/// Builds CheckTree from authorization checks.
///
/// Constructs parent→children relationships by iterating through checks
/// and using their parent_check_index links.
fn build_check_tree(checks: &[FieldCheck]) -> CheckTree {
    let mut tree = CheckTree::new();

    for (i, check) in checks.iter().enumerate() {
        tree.ensure_field();
        if let Some(parent_idx) = check.parent_check_index {
            tree.add_child_field(parent_idx, CheckIndex::new(i));
        }
    }

    tree
}

/// Applies GraphQL null bubbling semantics to authorization results.
pub(super) fn propagate_null_bubbling(checks: &[FieldCheck]) -> Vec<bool> {
    let check_tree = build_check_tree(checks);
    let mut removal_flags = vec![false; checks.len()];

    // Process bottom-up: leaves first, then parents
    for check_index in (0..checks.len()).rev() {
        let check = &checks[check_index];

        // Check if this field itself is unauthorized non-nullable
        if matches!(check.status, FieldAuthStatus::UnauthorizedNonNullable) {
            removal_flags[check_index] = true;
            continue;
        }

        // Check if any children are being removed
        let any_child_removed = check_tree
            .get_children(CheckIndex::new(check_index))
            .iter()
            .any(|&child_idx| removal_flags[child_idx.get()]);

        removal_flags[check_index] = any_child_removed;
    }

    removal_flags
}
