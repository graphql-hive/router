use ahash::{HashMap, HashSet};
use hive_router_plan_executor::introspection::schema::SchemaMetadata;
use hive_router_query_planner::ast::value::Value;
use hive_router_query_planner::federation_spec::authorization::{
    AuthenticatedDirective, RequiresScopesDirective,
};
use hive_router_query_planner::state::supergraph_state::{SupergraphDefinition, SupergraphState};
use lasso2::{Rodeo, Spur};

/// Unique identifier for a scope string, interned for fast comparisons.
pub type ScopeId = Spur;

/// String interner for scope values, enabling O(1) comparisons.
pub type ScopeInterner = Rodeo;

/// Authorization context for a single incoming request.
#[derive(Debug)]
pub struct UserAuthContext {
    pub is_authenticated: bool,
    pub scope_ids: HashSet<ScopeId>,
}

impl UserAuthContext {
    /// Creates a context from JWT details. Unknown scopes are silently ignored.
    pub fn new(
        is_authenticated: bool,
        scopes_from_jwt: &[String],
        auth_metadata: &AuthorizationMetadata,
    ) -> Self {
        Self {
            is_authenticated,
            scope_ids: scopes_from_jwt
                .iter()
                .filter_map(|s| auth_metadata.scopes.get(s))
                .collect(),
        }
    }
}

/// Group of scopes required together (AND logic).
///
/// Example: `["read:posts", "read:users"]` means user needs both scopes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScopeAndGroup(pub(crate) Vec<ScopeId>);

/// Full requirements of a `@requiresScopes` directive (OR logic).
///
/// Example: `[["admin"], ["read:posts", "write:posts"]]` means user needs
/// either "admin" OR both "read:posts" and "write:posts".
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RequiredScopes(pub(crate) Vec<ScopeAndGroup>);

/// Authorization rule for a field or type.
#[derive(Debug, Clone)]
pub enum AuthorizationRule {
    /// `@authenticated` - User must have valid JWT token.
    Authenticated,
    /// `@requiresScopes` - User must be authenticated with required scopes.
    RequiresScopes(RequiredScopes),
}

type TypeRulesMap = HashMap<String, AuthorizationRule>;
type FieldRulesMap = HashMap<String, AuthorizationRule>;
type TypeFieldRulesMap = HashMap<String, FieldRulesMap>;

/// Pre-computed authorization metadata built once at router startup.
#[derive(Debug)]
pub struct AuthorizationMetadata {
    /// Type-level authorization rules
    pub type_rules: TypeRulesMap,
    /// Field-level authorization rules
    pub field_rules: TypeFieldRulesMap,
    /// Interner for scope strings
    pub scopes: ScopeInterner,
    /// Type's subtree has any auth rules?
    pub type_has_any_auth: HashMap<String, bool>,
}

/// Errors that can occur during authorization metadata construction.
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
    /// Builds authorization metadata from the supergraph schema.
    /// Called once at router startup to extract and normalize all authorization directives.
    pub fn build(
        supergraph: &SupergraphState,
        schema_metadata: &SchemaMetadata,
    ) -> Result<Self, AuthorizationMetadataError> {
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

        // Compute authorization for union types based on their members
        Self::compute_union_type_rules(schema_metadata, &mut type_rules);

        // Compute which types have any auth rules in their subtree
        let type_has_any_auth = Self::compute_type_auth_metadata(
            &supergraph.definitions,
            schema_metadata,
            &type_rules,
            &field_rules,
        );

        Ok(Self {
            type_rules,
            field_rules,
            scopes,
            type_has_any_auth,
        })
    }

    /// Computes whether each type has auth rules in its subtree.
    fn compute_type_auth_metadata(
        definitions: &std::collections::HashMap<String, SupergraphDefinition>,
        schema_metadata: &SchemaMetadata,
        type_rules: &TypeRulesMap,
        field_rules: &TypeFieldRulesMap,
    ) -> HashMap<String, bool> {
        let mut result = HashMap::default();

        for type_name in definitions.keys() {
            let mut visited = HashSet::default();
            let has_auth = Self::type_has_any_auth_recursive(
                type_name,
                schema_metadata,
                type_rules,
                field_rules,
                &mut visited,
            );
            result.insert(type_name.clone(), has_auth);
        }

        result
    }

    fn type_has_any_auth_recursive(
        type_name: &str,
        schema_metadata: &SchemaMetadata,
        type_rules: &TypeRulesMap,
        field_rules: &TypeFieldRulesMap,
        visited: &mut HashSet<String>,
    ) -> bool {
        if visited.contains(type_name) {
            return false;
        }
        visited.insert(type_name.to_string());

        if type_rules.contains_key(type_name) {
            return true;
        }

        if field_rules
            .get(type_name)
            .is_some_and(|fields_map| !fields_map.is_empty())
        {
            return true;
        }

        // look for implementing types (for interfaces and unions)
        if let Some(implementing_types) = schema_metadata.get_possible_types(type_name) {
            for implementing_type in implementing_types {
                if Self::type_has_any_auth_recursive(
                    implementing_type,
                    schema_metadata,
                    type_rules,
                    field_rules,
                    visited,
                ) {
                    return true;
                }
            }
        }

        if let Some(type_fields) = schema_metadata.get_type_fields(type_name) {
            for field_info in type_fields.values() {
                if Self::type_has_any_auth_recursive(
                    &field_info.output_type_name,
                    schema_metadata,
                    type_rules,
                    field_rules,
                    visited,
                ) {
                    return true;
                }
            }
        }

        false
    }

    pub fn is_type_authorized(&self, type_name: &str, user_context: &UserAuthContext) -> bool {
        if let Some(rule) = self.type_rules.get(type_name) {
            return self.is_rule_satisfied(rule, user_context);
        }

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
            return self.is_rule_satisfied(rule, user_context);
        }
        true
    }

    /// Computes and adds authorization rules for union types based on their members.
    /// For each union, combines the authorization requirements of all member types.
    fn compute_union_type_rules(schema_metadata: &SchemaMetadata, type_rules: &mut TypeRulesMap) {
        for union_name in &schema_metadata.union_types {
            // Skip if union already has explicit authorization
            if type_rules.contains_key(union_name) {
                continue;
            }

            if let Some(members) = schema_metadata.get_possible_types(union_name) {
                if let Some(rule) = Self::compute_union_authorization_rule(members, type_rules) {
                    type_rules.insert(union_name.clone(), rule);
                }
            }
        }
    }

    /// Computes the combined authorization rule for a union from its members.
    /// Combines member requirements with AND logic (user must have access to all members).
    fn compute_union_authorization_rule(
        member_names: &HashSet<String>,
        type_rules: &TypeRulesMap,
    ) -> Option<AuthorizationRule> {
        let mut member_scopes: Vec<&RequiredScopes> = Vec::new();
        let mut needs_authenticated = false;

        // Collect rules from all members
        for member_name in member_names {
            if let Some(rule) = type_rules.get(member_name) {
                match rule {
                    AuthorizationRule::Authenticated => {
                        needs_authenticated = true;
                    }
                    AuthorizationRule::RequiresScopes(scopes) => {
                        needs_authenticated = true; // scopes implies authenticated
                        member_scopes.push(scopes);
                    }
                }
            }
        }

        if !needs_authenticated {
            return None;
        }

        // Some members have @authenticated but no scopes
        if member_scopes.is_empty() {
            return Some(AuthorizationRule::Authenticated);
        }

        Some(AuthorizationRule::RequiresScopes(
            Self::cross_product_required_scopes(&member_scopes),
        ))
    }

    /// Combines multiple RequiredScopes using AND logic via cross product.
    /// Example: [["a"], ["b"]] AND [["c"], ["d"]] = [["a", "c"], ["a", "d"], ["b", "c"], ["b", "d"]]
    fn cross_product_required_scopes(member_scopes: &[&RequiredScopes]) -> RequiredScopes {
        let mut result: Vec<ScopeAndGroup> = vec![ScopeAndGroup(vec![])];

        for member_scope in member_scopes {
            let mut new_result = Vec::new();

            for existing_and_group in &result {
                for member_and_group in &member_scope.0 {
                    // Combine existing AND group with member's AND group
                    let mut combined = existing_and_group.0.clone();
                    combined.extend(member_and_group.0.iter().copied());
                    combined.sort();
                    combined.dedup();
                    new_result.push(ScopeAndGroup(combined));
                }
            }

            result = new_result;
        }

        RequiredScopes(result)
    }

    fn is_rule_satisfied(&self, rule: &AuthorizationRule, user_context: &UserAuthContext) -> bool {
        match rule {
            AuthorizationRule::Authenticated => user_context.is_authenticated,
            AuthorizationRule::RequiresScopes(scopes) => {
                user_context.is_authenticated
                    && scopes.0.iter().any(|and_group| {
                        and_group
                            .0
                            .iter()
                            .all(|scope_id| user_context.scope_ids.contains(scope_id))
                    })
            }
        }
    }

    /// Processes a type definition, extracting authorization rules for the type and its fields.
    fn process_type_definition(
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

        // Extract type-level rules
        if let Some(rule) = Self::extract_rule_from_directives(
            authenticated_directives,
            requires_scopes_directives,
            scopes_interner,
        )? {
            type_rules.insert(type_name.clone(), rule);
        }

        // Extract field-level rules
        if let Some(fields) = maybe_fields {
            let mut type_field_rules = FieldRulesMap::default();
            for (field_name, field_def) in fields {
                let maybe_field_rules = Self::extract_rule_from_directives(
                    &field_def.authenticated,
                    &field_def.requires_scopes,
                    scopes_interner,
                )?;
                if let Some(rule) = maybe_field_rules {
                    type_field_rules.insert(field_name.clone(), rule);
                }
            }

            if !type_field_rules.is_empty() {
                field_rules.insert(type_name.clone(), type_field_rules);
            }
        }
        Ok(())
    }

    /// Extracts authorization rule from directives.
    fn extract_rule_from_directives(
        authenticated_directives: &[AuthenticatedDirective],
        requires_scopes_directives: &[RequiresScopesDirective],
        interner: &mut ScopeInterner,
    ) -> Result<Option<AuthorizationRule>, AuthorizationMetadataError> {
        if requires_scopes_directives.len() > 1 {
            return Err(AuthorizationMetadataError::DuplicateRequiresScopesDirective);
        }

        if let Some(directive) = requires_scopes_directives.first() {
            let scopes = Self::normalize_scopes_arg(&directive.scopes, interner)?;
            return Ok(Some(AuthorizationRule::RequiresScopes(scopes)));
        }

        if !authenticated_directives.is_empty() {
            return Ok(Some(AuthorizationRule::Authenticated));
        }

        Ok(None)
    }

    /// Parses and normalizes the `scopes` argument from a `@requiresScopes` directive.
    fn normalize_scopes_arg(
        value: &Value,
        interner: &mut ScopeInterner,
    ) -> Result<RequiredScopes, AuthorizationMetadataError> {
        let Value::List(or_groups_val) = value else {
            return Err(AuthorizationMetadataError::InvalidRequiresScopesArgs(
                format!("expected a list, got '{}'", value),
            ));
        };

        let mut or_groups: Vec<_> = or_groups_val
            .iter()
            .map(|v| Self::normalize_and_group(v, interner))
            .collect::<Result<_, _>>()?;

        if or_groups.is_empty() {
            return Err(AuthorizationMetadataError::InvalidRequiresScopesArgs(
                "expected at least one AND group, got none".to_string(),
            ));
        }

        or_groups.sort();
        Ok(RequiredScopes(or_groups))
    }

    fn normalize_and_group(
        value: &Value,
        interner: &mut ScopeInterner,
    ) -> Result<ScopeAndGroup, AuthorizationMetadataError> {
        let Value::List(and_group_val) = value else {
            return Err(AuthorizationMetadataError::InvalidRequiresScopesArgs(
                "expected a list for AND group".to_string(),
            ));
        };

        let mut and_group: Vec<ScopeId> = and_group_val
            .iter()
            .map(|v| match v {
                Value::String(s) => Ok(interner.get_or_intern(s)),
                _ => Err(AuthorizationMetadataError::InvalidScopeValue(format!(
                    "expected scope to be a string, got: '{}'",
                    v
                ))),
            })
            .collect::<Result<_, _>>()?;

        if and_group.is_empty() {
            return Err(AuthorizationMetadataError::InvalidRequiresScopesArgs(
                "empty AND group, expected at least one scope".to_string(),
            ));
        }

        and_group.sort();
        Ok(ScopeAndGroup(and_group))
    }
}
