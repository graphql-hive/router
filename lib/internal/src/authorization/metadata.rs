use ahash::HashMap;
use lasso2::{Rodeo, Spur};

/// Unique identifier for a scope string, interned for fast comparisons.
pub type ScopeId = Spur;

/// String interner for scope values, enabling O(1) comparisons.
pub type ScopeInterner = Rodeo;

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

/// Authorization rule for a field or type.
#[derive(Debug, Clone)]
pub enum AuthorizationRule {
    /// `@authenticated` - User must have valid JWT token.
    Authenticated,
    /// `@requiresScopes` - User must be authenticated with required scopes.
    RequiresScopes(RequiredScopes),
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
    /// Type's subtree has any auth rules?
    pub type_has_any_auth: HashMap<String, bool>,
}
