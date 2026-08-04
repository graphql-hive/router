use ahash::HashMap;
use lasso2::{Rodeo, Spur};

/// Unique identifier for a scope string, interned for fast comparisons.
pub type ScopeId = Spur;

/// String interner for scope values, enabling O(1) comparisons.
pub type ScopeInterner = Rodeo;

/// Unique identifier for a policy string, interned for fast comparisons.
pub type PolicyId = Spur;

/// String interner for policy values, enabling O(1) comparisons.
pub type PolicyInterner = Rodeo;

/// Group of scopes required together (AND logic).
///
/// Example: `["read:posts", "read:users"]` means user needs both scopes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScopeAndGroup(pub Vec<ScopeId>);

/// Full requirements of a `@requiresScopes` directive (OR logic).
///
/// Example: `[["admin"], ["read:posts", "write:posts"]]` means user needs
/// either "admin" OR both "read:posts" and "write:posts".
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RequiredScopes(pub Vec<ScopeAndGroup>);

/// Group of policies required together (AND logic).
///
/// Example: `["read_profile", "read_email"]` means both policies must be granted.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PolicyAndGroup(pub Vec<PolicyId>);

/// Full requirements of a `@policy` directive (OR logic).
///
/// Example: `[["admin"], ["read_profile", "read_email"]]` means either the
/// "admin" policy is granted, or both "read_profile" and "read_email" are.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RequiredPolicies(pub Vec<PolicyAndGroup>);

/// Authorization rule for a field or type.
///
/// A single field or type can carry several authorization directives at once.
/// All the parts present here must be satisfied for access to be granted.
#[derive(Debug, Clone, Default)]
pub struct AuthorizationRule {
    /// `@authenticated` - User must have valid JWT token.
    pub authenticated: bool,
    /// `@requiresScopes` - User must be authenticated with the required scopes.
    pub scopes: Option<RequiredScopes>,
    /// `@policy` - The required policies must have been granted for this request.
    pub policies: Option<RequiredPolicies>,
}

impl AuthorizationRule {
    pub fn is_empty(&self) -> bool {
        !self.authenticated && self.scopes.is_none() && self.policies.is_none()
    }
}

pub type TypeRulesMap = HashMap<String, AuthorizationRule>;
pub type FieldRulesMap = HashMap<String, AuthorizationRule>;
pub type TypeFieldRulesMap = HashMap<String, FieldRulesMap>;

/// Pre-computed authorization metadata built once at router startup.
#[derive(Debug)]
pub struct AuthorizationMetadata {
    /// Type-level authorization rules
    pub type_rules: TypeRulesMap,
    /// Field-level authorization rules
    pub field_rules: TypeFieldRulesMap,
    /// Interner for scope strings
    pub scopes: ScopeInterner,
    /// Interner for policy strings
    pub policies: PolicyInterner,
    /// Type's subtree has any auth rules?
    pub type_has_any_auth: HashMap<String, bool>,
}
