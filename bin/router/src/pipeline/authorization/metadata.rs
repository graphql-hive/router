use ahash::{HashMap, HashSet};
use hive_router_internal::authorization::metadata::{
    AuthorizationMetadata, AuthorizationRule, FieldRulesMap, PolicyAndGroup, PolicyId,
    PolicyInterner, RequiredPolicies, RequiredScopes, ScopeAndGroup, ScopeId, ScopeInterner,
    TypeFieldRulesMap, TypeRulesMap,
};
use hive_router_plan_executor::execution::client_request_details::JwtRequestDetails;
use hive_router_plan_executor::introspection::schema::SchemaMetadata;
use hive_router_query_planner::ast::value::Value;
use hive_router_query_planner::federation_spec::authorization::{
    AuthenticatedDirective, PolicyDirective, RequiresScopesDirective,
};
use hive_router_query_planner::state::supergraph_state::{SupergraphDefinition, SupergraphState};

/// Authorization context for a single incoming request.
#[derive(Debug)]
pub struct UserAuthContext {
    pub is_authenticated: bool,
    pub scope_ids: HashSet<ScopeId>,
    /// Policies granted for this request, as decided by a coprocessor or a plugin.
    pub granted_policy_ids: HashSet<PolicyId>,
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
            granted_policy_ids: HashSet::default(),
        }
    }

    pub fn from_jwt(
        jwt_request_details: &JwtRequestDetails,
        auth_metadata: &AuthorizationMetadata,
    ) -> Self {
        match jwt_request_details {
            JwtRequestDetails::Authenticated { scopes, .. } => {
                Self::new(true, scopes.as_deref().unwrap_or(&[]), auth_metadata)
            }
            JwtRequestDetails::Unauthenticated => Self::new(false, &[], auth_metadata),
        }
    }

    /// Records the policies that were granted for this request.
    /// Policies unknown to the schema are silently ignored.
    pub fn with_granted_policies<'a>(
        mut self,
        granted_policies: impl IntoIterator<Item = &'a str>,
        auth_metadata: &AuthorizationMetadata,
    ) -> Self {
        self.granted_policy_ids = granted_policies
            .into_iter()
            .filter_map(|policy| auth_metadata.policies.get(policy))
            .collect();

        self
    }
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
    #[error("Invalid policy value: {0}")]
    InvalidPolicyValue(String),
    #[error("Invalid @policy(policies:) argument: {0}")]
    InvalidPolicyArgs(String),
    #[error("Duplicate @policy directives found")]
    DuplicatePolicyDirective,
}

pub trait AuthorizationMetadataExt
where
    Self: Sized,
{
    /// Builds authorization metadata from the supergraph schema.
    /// Called once at router startup to extract and normalize all authorization directives.
    fn build(
        supergraph: &SupergraphState,
        schema_metadata: &SchemaMetadata,
    ) -> Result<Self, AuthorizationMetadataError>;

    fn is_empty(&self) -> bool;

    /// Whether the schema declares any `@policy` requirement.
    fn has_policies(&self) -> bool;

    /// Computes whether each type has auth rules in its subtree.
    fn compute_type_auth_metadata(
        definitions: &std::collections::HashMap<String, SupergraphDefinition>,
        schema_metadata: &SchemaMetadata,
        type_rules: &TypeRulesMap,
        field_rules: &TypeFieldRulesMap,
    ) -> HashMap<String, bool>;

    fn type_has_any_auth_recursive(
        type_name: &str,
        schema_metadata: &SchemaMetadata,
        type_rules: &TypeRulesMap,
        field_rules: &TypeFieldRulesMap,
        visited: &mut HashSet<String>,
    ) -> bool;

    /// Computes and adds authorization rules for union types based on their members.
    /// For each union, combines the authorization requirements of all member types.
    fn compute_union_type_rules(schema_metadata: &SchemaMetadata, type_rules: &mut TypeRulesMap);

    /// Computes the combined authorization rule for a union from its members.
    /// Combines member requirements with AND logic (user must have access to all members).
    fn compute_union_authorization_rule(
        member_names: &HashSet<String>,
        type_rules: &TypeRulesMap,
    ) -> Option<AuthorizationRule>;

    /// Combines multiple RequiredScopes using AND logic via cross product.
    /// Example: [["a"], ["b"]] AND [["c"], ["d"]] = [["a", "c"], ["a", "d"], ["b", "c"], ["b", "d"]]
    fn cross_product_required_scopes(member_scopes: &[&RequiredScopes]) -> RequiredScopes;

    /// Combines multiple RequiredPolicies using AND logic via cross product.
    /// Follows the same rules as [`Self::cross_product_required_scopes`].
    fn cross_product_required_policies(member_policies: &[&RequiredPolicies]) -> RequiredPolicies;

    /// Processes a type definition, extracting authorization rules for the type and its fields.
    fn process_type_definition(
        type_def: &SupergraphDefinition,
        type_rules: &mut TypeRulesMap,
        field_rules: &mut TypeFieldRulesMap,
        scopes_interner: &mut ScopeInterner,
        policies_interner: &mut PolicyInterner,
    ) -> Result<(), AuthorizationMetadataError>;

    /// Extracts authorization rule from directives.
    fn extract_rule_from_directives(
        authenticated_directives: &[AuthenticatedDirective],
        requires_scopes_directives: &[RequiresScopesDirective],
        policy_directives: &[PolicyDirective],
        scopes_interner: &mut ScopeInterner,
        policies_interner: &mut PolicyInterner,
    ) -> Result<Option<AuthorizationRule>, AuthorizationMetadataError>;

    /// Parses and normalizes the `scopes` argument from a `@requiresScopes` directive.
    fn normalize_scopes_arg(
        value: &Value,
        interner: &mut ScopeInterner,
    ) -> Result<RequiredScopes, AuthorizationMetadataError>;

    fn normalize_and_group(
        value: &Value,
        interner: &mut ScopeInterner,
    ) -> Result<ScopeAndGroup, AuthorizationMetadataError>;

    /// Parses and normalizes the `policies` argument from a `@policy` directive.
    fn normalize_policies_arg(
        value: &Value,
        interner: &mut PolicyInterner,
    ) -> Result<RequiredPolicies, AuthorizationMetadataError>;

    fn normalize_policy_and_group(
        value: &Value,
        interner: &mut PolicyInterner,
    ) -> Result<PolicyAndGroup, AuthorizationMetadataError>;
}

impl AuthorizationMetadataExt for AuthorizationMetadata {
    /// Builds authorization metadata from the supergraph schema.
    /// Called once at router startup to extract and normalize all authorization directives.
    fn build(
        supergraph: &SupergraphState,
        schema_metadata: &SchemaMetadata,
    ) -> Result<Self, AuthorizationMetadataError> {
        let mut type_rules = HashMap::default();
        let mut field_rules = HashMap::default();
        let mut scopes = ScopeInterner::new();
        let mut policies = PolicyInterner::new();

        for type_def in supergraph.definitions.values() {
            Self::process_type_definition(
                type_def,
                &mut type_rules,
                &mut field_rules,
                &mut scopes,
                &mut policies,
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
            policies,
            type_has_any_auth,
        })
    }

    fn is_empty(&self) -> bool {
        self.type_rules.is_empty() && self.field_rules.is_empty()
    }

    fn has_policies(&self) -> bool {
        !self.policies.is_empty()
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
        let mut member_policies: Vec<&RequiredPolicies> = Vec::new();
        let mut needs_authenticated = false;

        // Collect rules from all members
        for member_name in member_names {
            let Some(rule) = type_rules.get(member_name) else {
                continue;
            };

            // scopes imply authenticated, policies do not
            needs_authenticated |= rule.authenticated || rule.scopes.is_some();

            if let Some(scopes) = &rule.scopes {
                member_scopes.push(scopes);
            }

            if let Some(policies) = &rule.policies {
                member_policies.push(policies);
            }
        }

        let rule = AuthorizationRule {
            authenticated: needs_authenticated,
            scopes: (!member_scopes.is_empty())
                .then(|| Self::cross_product_required_scopes(&member_scopes)),
            policies: (!member_policies.is_empty())
                .then(|| Self::cross_product_required_policies(&member_policies)),
        };

        (!rule.is_empty()).then_some(rule)
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

    fn cross_product_required_policies(member_policies: &[&RequiredPolicies]) -> RequiredPolicies {
        let mut result: Vec<PolicyAndGroup> = vec![PolicyAndGroup(vec![])];

        for member_policy in member_policies {
            let mut new_result = Vec::new();

            for existing_and_group in &result {
                for member_and_group in &member_policy.0 {
                    let mut combined = existing_and_group.0.clone();
                    combined.extend(member_and_group.0.iter().copied());
                    combined.sort();
                    combined.dedup();
                    new_result.push(PolicyAndGroup(combined));
                }
            }

            result = new_result;
        }

        RequiredPolicies(result)
    }

    /// Processes a type definition, extracting authorization rules for the type and its fields.
    fn process_type_definition(
        type_def: &SupergraphDefinition,
        type_rules: &mut TypeRulesMap,
        field_rules: &mut TypeFieldRulesMap,
        scopes_interner: &mut ScopeInterner,
        policies_interner: &mut PolicyInterner,
    ) -> Result<(), AuthorizationMetadataError> {
        let (
            type_name,
            authenticated_directives,
            requires_scopes_directives,
            policy_directives,
            maybe_fields,
        ) = match type_def {
            SupergraphDefinition::Scalar(s) => (
                &s.name,
                &s.authenticated,
                &s.requires_scopes,
                &s.policy,
                None,
            ),
            SupergraphDefinition::Object(o) => (
                &o.name,
                &o.authenticated,
                &o.requires_scopes,
                &o.policy,
                Some(&o.fields),
            ),
            SupergraphDefinition::Interface(i) => (
                &i.name,
                &i.authenticated,
                &i.requires_scopes,
                &i.policy,
                Some(&i.fields),
            ),
            SupergraphDefinition::Enum(e) => (
                &e.name,
                &e.authenticated,
                &e.requires_scopes,
                &e.policy,
                None,
            ),
            // Unions and InputObjects do not have output authorization rules applicable here.
            SupergraphDefinition::Union(_) | SupergraphDefinition::InputObject(_) => return Ok(()),
        };

        // Extract type-level rules
        if let Some(rule) = Self::extract_rule_from_directives(
            authenticated_directives,
            requires_scopes_directives,
            policy_directives,
            scopes_interner,
            policies_interner,
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
                    &field_def.policy,
                    scopes_interner,
                    policies_interner,
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
        policy_directives: &[PolicyDirective],
        scopes_interner: &mut ScopeInterner,
        policies_interner: &mut PolicyInterner,
    ) -> Result<Option<AuthorizationRule>, AuthorizationMetadataError> {
        if requires_scopes_directives.len() > 1 {
            return Err(AuthorizationMetadataError::DuplicateRequiresScopesDirective);
        }

        if policy_directives.len() > 1 {
            return Err(AuthorizationMetadataError::DuplicatePolicyDirective);
        }

        let scopes = requires_scopes_directives
            .first()
            .map(|directive| Self::normalize_scopes_arg(&directive.scopes, scopes_interner))
            .transpose()?;

        let policies = policy_directives
            .first()
            .map(|directive| Self::normalize_policies_arg(&directive.policies, policies_interner))
            .transpose()?;

        let rule = AuthorizationRule {
            // `@requiresScopes` implies `@authenticated`, `@policy` does not.
            authenticated: !authenticated_directives.is_empty() || scopes.is_some(),
            scopes,
            policies,
        };

        Ok((!rule.is_empty()).then_some(rule))
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

    /// Parses and normalizes the `policies` argument from a `@policy` directive.
    fn normalize_policies_arg(
        value: &Value,
        interner: &mut PolicyInterner,
    ) -> Result<RequiredPolicies, AuthorizationMetadataError> {
        let Value::List(or_groups_val) = value else {
            return Err(AuthorizationMetadataError::InvalidPolicyArgs(format!(
                "expected a list, got '{}'",
                value
            )));
        };

        let mut or_groups: Vec<_> = or_groups_val
            .iter()
            .map(|v| Self::normalize_policy_and_group(v, interner))
            .collect::<Result<_, _>>()?;

        if or_groups.is_empty() {
            return Err(AuthorizationMetadataError::InvalidPolicyArgs(
                "expected at least one AND group, got none".to_string(),
            ));
        }

        or_groups.sort();
        Ok(RequiredPolicies(or_groups))
    }

    fn normalize_policy_and_group(
        value: &Value,
        interner: &mut PolicyInterner,
    ) -> Result<PolicyAndGroup, AuthorizationMetadataError> {
        let Value::List(and_group_val) = value else {
            return Err(AuthorizationMetadataError::InvalidPolicyArgs(
                "expected a list for AND group".to_string(),
            ));
        };

        let mut and_group: Vec<PolicyId> = and_group_val
            .iter()
            .map(|v| match v {
                Value::String(s) => Ok(interner.get_or_intern(s)),
                _ => Err(AuthorizationMetadataError::InvalidPolicyValue(format!(
                    "expected policy to be a string, got: '{}'",
                    v
                ))),
            })
            .collect::<Result<_, _>>()?;

        if and_group.is_empty() {
            return Err(AuthorizationMetadataError::InvalidPolicyArgs(
                "empty AND group, expected at least one policy".to_string(),
            ));
        }

        and_group.sort();
        Ok(PolicyAndGroup(and_group))
    }
}
