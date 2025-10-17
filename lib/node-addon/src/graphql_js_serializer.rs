//! GraphQL-JS v16 AST Serializer
//!
//! Zero-allocation serializer converting Rust AST to graphql-js v16 JSON format.
//!
//! ## Transformations
//! - Rust snake_case → JavaScript camelCase
//! - Adds "kind" discriminator fields
//! - Wraps primitives in AST node objects
//! - Omits None fields per GraphQL-JS spec
//!
//! ## Performance
//! - **Zero heap allocations** - all data borrowed from source AST
//! - Pre-sized maps eliminate reallocation
//! - Direct iteration without intermediate collections
//! - Optimized for hot-path query planning

use hive_router_query_planner::ast::{
    document::Document,
    fragment::FragmentDefinition,
    operation::{OperationDefinition, VariableDefinition},
    selection_item::SelectionItem,
    selection_set::{FieldSelection, InlineFragmentSelection, SelectionSet},
    value::Value,
};
use hive_router_query_planner::state::supergraph_state::{OperationKind, TypeNode};
use serde::ser::{SerializeMap, SerializeSeq};
use serde::Serializer;
use std::collections::BTreeMap;

/// Serialize Document to graphql-js DocumentNode.
///
/// Entry point for the custom serializer. Use with serde attribute:
/// ```ignore
/// #[serde(serialize_with = "graphql_js_serializer::serialize_document")]
/// ```
pub fn serialize_document<S>(doc: &Document, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut map = serializer.serialize_map(Some(2))?;
    map.serialize_entry("kind", "Document")?;
    map.serialize_entry("definitions", &DefinitionsSeq(doc))?;
    map.end()
}

// ============================================================================
// Document & Definitions
// ============================================================================

struct DefinitionsSeq<'a>(&'a Document);

impl<'a> serde::Serialize for DefinitionsSeq<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let doc = self.0;
        let mut seq = serializer.serialize_seq(Some(1 + doc.fragments.len()))?;

        seq.serialize_element(&OperationDefNode(&doc.operation))?;

        for fragment in &doc.fragments {
            seq.serialize_element(&FragmentDefNode(fragment))?;
        }

        seq.end()
    }
}

// ============================================================================
// Operation Definition
// ============================================================================

struct OperationDefNode<'a>(&'a OperationDefinition);

impl<'a> serde::Serialize for OperationDefNode<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let op = self.0;

        // Pre-calculate exact size
        let mut len = 2; // kind + selectionSet (always present)
        if op.operation_kind.is_some() { len += 1; }
        if op.name.is_some() { len += 1; }
        if op.variable_definitions.as_ref().is_some_and(|v| !v.is_empty()) { len += 1; }

        let mut map = serializer.serialize_map(Some(len))?;
        map.serialize_entry("kind", "OperationDefinition")?;

        if let Some(kind) = &op.operation_kind {
            map.serialize_entry("operation", operation_kind_str(kind))?;
        }

        if let Some(name) = &op.name {
            map.serialize_entry("name", &NameNode(name))?;
        }

        if let Some(var_defs) = &op.variable_definitions {
            if !var_defs.is_empty() {
                map.serialize_entry("variableDefinitions", &VarDefsSeq(var_defs))?;
            }
        }

        map.serialize_entry("selectionSet", &SelectionSetNode(&op.selection_set))?;
        map.end()
    }
}

#[inline]
fn operation_kind_str(kind: &OperationKind) -> &'static str {
    match kind {
        OperationKind::Query => "query",
        OperationKind::Mutation => "mutation",
        OperationKind::Subscription => "subscription",
    }
}

// ============================================================================
// Fragment Definition
// ============================================================================

struct FragmentDefNode<'a>(&'a FragmentDefinition);

impl<'a> serde::Serialize for FragmentDefNode<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let frag = self.0;
        let mut map = serializer.serialize_map(Some(4))?;

        map.serialize_entry("kind", "FragmentDefinition")?;
        map.serialize_entry("name", &NameNode(&frag.name))?;
        map.serialize_entry("typeCondition", &NamedTypeNode(&frag.type_condition))?;
        map.serialize_entry("selectionSet", &SelectionSetNode(&frag.selection_set))?;

        map.end()
    }
}

// ============================================================================
// Variable Definitions
// ============================================================================

struct VarDefsSeq<'a>(&'a [VariableDefinition]);

impl<'a> serde::Serialize for VarDefsSeq<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for def in self.0 {
            seq.serialize_element(&VarDefNode(def))?;
        }
        seq.end()
    }
}

struct VarDefNode<'a>(&'a VariableDefinition);

impl<'a> serde::Serialize for VarDefNode<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let def = self.0;
        let len = if def.default_value.is_some() { 4 } else { 3 };

        let mut map = serializer.serialize_map(Some(len))?;
        map.serialize_entry("kind", "VariableDefinition")?;
        map.serialize_entry("variable", &VariableNode(&def.name))?;
        map.serialize_entry("type", &TypeNodeValue(&def.variable_type))?;

        if let Some(default_val) = &def.default_value {
            map.serialize_entry("defaultValue", &ValueNode(default_val))?;
        }

        map.end()
    }
}

// ============================================================================
// Selection Set
// ============================================================================

struct SelectionSetNode<'a>(&'a SelectionSet);

impl<'a> serde::Serialize for SelectionSetNode<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("kind", "SelectionSet")?;
        map.serialize_entry("selections", &SelectionsSeq(&self.0.items))?;
        map.end()
    }
}

struct SelectionsSeq<'a>(&'a [SelectionItem]);

impl<'a> serde::Serialize for SelectionsSeq<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for item in self.0 {
            match item {
                SelectionItem::Field(field) => seq.serialize_element(&FieldNode(field))?,
                SelectionItem::InlineFragment(frag) => seq.serialize_element(&InlineFragNode(frag))?,
                SelectionItem::FragmentSpread(name) => seq.serialize_element(&FragSpreadNode(name))?,
            }
        }
        seq.end()
    }
}

// ============================================================================
// Field Selection
// ============================================================================

struct FieldNode<'a>(&'a FieldSelection);

impl<'a> serde::Serialize for FieldNode<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let field = self.0;

        // Pre-calculate exact map size
        let mut len = 2; // kind + name
        if field.alias.is_some() { len += 1; }
        if field.arguments().is_some() { len += 1; }
        if field.skip_if.is_some() || field.include_if.is_some() { len += 1; }
        if !field.selections.is_empty() { len += 1; }

        let mut map = serializer.serialize_map(Some(len))?;
        map.serialize_entry("kind", "Field")?;

        if let Some(alias) = &field.alias {
            map.serialize_entry("alias", &NameNode(alias))?;
        }

        map.serialize_entry("name", &NameNode(&field.name))?;

        if let Some(args) = field.arguments() {
            map.serialize_entry("arguments", &ArgumentsSeq(args))?;
        }

        if field.skip_if.is_some() || field.include_if.is_some() {
            map.serialize_entry("directives", &DirectivesSeq(&field.skip_if, &field.include_if))?;
        }

        if !field.selections.is_empty() {
            map.serialize_entry("selectionSet", &SelectionSetNode(&field.selections))?;
        }

        map.end()
    }
}

// ============================================================================
// Inline Fragment
// ============================================================================

struct InlineFragNode<'a>(&'a InlineFragmentSelection);

impl<'a> serde::Serialize for InlineFragNode<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let frag = self.0;
        let has_directives = frag.skip_if.is_some() || frag.include_if.is_some();
        let len = if has_directives { 4 } else { 3 };

        let mut map = serializer.serialize_map(Some(len))?;
        map.serialize_entry("kind", "InlineFragment")?;
        map.serialize_entry("typeCondition", &NamedTypeNode(&frag.type_condition))?;

        if has_directives {
            map.serialize_entry("directives", &DirectivesSeq(&frag.skip_if, &frag.include_if))?;
        }

        map.serialize_entry("selectionSet", &SelectionSetNode(&frag.selections))?;
        map.end()
    }
}

// ============================================================================
// Fragment Spread
// ============================================================================

struct FragSpreadNode<'a>(&'a str);

impl<'a> serde::Serialize for FragSpreadNode<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("kind", "FragmentSpread")?;
        map.serialize_entry("name", &NameNode(self.0))?;
        map.end()
    }
}

// ============================================================================
// Arguments
// ============================================================================

struct ArgumentsSeq<'a, T: 'a>(&'a T);

impl<'a, T> serde::Serialize for ArgumentsSeq<'a, T>
where
    for<'b> &'b T: IntoIterator<Item = (&'b String, &'b Value)>,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // ZERO-ALLOCATION: Iterate directly, serde handles the buffering internally
        // We can't know the exact size without collecting, but IntoIterator provides
        // a size_hint that serde uses for pre-allocation
        let iter = self.0.into_iter();
        let (lower, _) = iter.size_hint();
        let mut seq = serializer.serialize_seq(Some(lower))?;

        for (name, value) in iter {
            seq.serialize_element(&ArgumentNode(name, value))?;
        }

        seq.end()
    }
}

struct ArgumentNode<'a>(&'a str, &'a Value);

impl<'a> serde::Serialize for ArgumentNode<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(3))?;
        map.serialize_entry("kind", "Argument")?;
        map.serialize_entry("name", &NameNode(self.0))?;
        map.serialize_entry("value", &ValueNode(self.1))?;
        map.end()
    }
}

// ============================================================================
// Directives
// ============================================================================

struct DirectivesSeq<'a>(&'a Option<String>, &'a Option<String>);

impl<'a> serde::Serialize for DirectivesSeq<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Efficient count using boolean coercion
        let count = self.0.is_some() as usize + self.1.is_some() as usize;
        let mut seq = serializer.serialize_seq(Some(count))?;

        if let Some(var) = self.0 {
            seq.serialize_element(&DirectiveNode("skip", var))?;
        }

        if let Some(var) = self.1 {
            seq.serialize_element(&DirectiveNode("include", var))?;
        }

        seq.end()
    }
}

struct DirectiveNode<'a>(&'a str, &'a str);

impl<'a> serde::Serialize for DirectiveNode<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(3))?;
        map.serialize_entry("kind", "Directive")?;
        map.serialize_entry("name", &NameNode(self.0))?;
        map.serialize_entry("arguments", &DirectiveArgsSeq(self.1))?;
        map.end()
    }
}

struct DirectiveArgsSeq<'a>(&'a str);

impl<'a> serde::Serialize for DirectiveArgsSeq<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(1))?;
        seq.serialize_element(&DirectiveArgNode(self.0))?;
        seq.end()
    }
}

struct DirectiveArgNode<'a>(&'a str);

impl<'a> serde::Serialize for DirectiveArgNode<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(3))?;
        map.serialize_entry("kind", "Argument")?;
        map.serialize_entry("name", &NameNode("if"))?;
        map.serialize_entry("value", &VariableNode(self.0))?;
        map.end()
    }
}

// ============================================================================
// Value Nodes
// ============================================================================

struct ValueNode<'a>(&'a Value);

impl<'a> serde::Serialize for ValueNode<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self.0 {
            Value::Variable(name) => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("kind", "Variable")?;
                map.serialize_entry("name", &NameNode(name))?;
                map.end()
            }
            Value::Int(i) => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("kind", "IntValue")?;
                // Serialize as string per GraphQL spec
                map.serialize_entry("value", &IntAsString(*i))?;
                map.end()
            }
            Value::Float(f) => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("kind", "FloatValue")?;
                // Serialize as string per GraphQL spec
                map.serialize_entry("value", &FloatAsString(*f))?;
                map.end()
            }
            Value::String(s) => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("kind", "StringValue")?;
                map.serialize_entry("value", s)?;
                map.end()
            }
            Value::Boolean(b) => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("kind", "BooleanValue")?;
                map.serialize_entry("value", b)?;
                map.end()
            }
            Value::Null => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("kind", "NullValue")?;
                map.end()
            }
            Value::Enum(e) => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("kind", "EnumValue")?;
                map.serialize_entry("value", e)?;
                map.end()
            }
            Value::List(list) => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("kind", "ListValue")?;
                map.serialize_entry("values", &ValuesSeq(list))?;
                map.end()
            }
            Value::Object(obj) => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("kind", "ObjectValue")?;
                map.serialize_entry("fields", &ObjectFieldsSeq(obj))?;
                map.end()
            }
        }
    }
}

/// Integer serialized as string per GraphQL spec
struct IntAsString(i64);

impl serde::Serialize for IntAsString {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // serde_json uses itoa internally for optimal string conversion
        serializer.collect_str(&self.0)
    }
}

/// Float serialized as string per GraphQL spec
struct FloatAsString(f64);

impl serde::Serialize for FloatAsString {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // serde_json uses ryu internally for optimal string conversion
        serializer.collect_str(&self.0)
    }
}

struct ValuesSeq<'a>(&'a [Value]);

impl<'a> serde::Serialize for ValuesSeq<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for val in self.0 {
            seq.serialize_element(&ValueNode(val))?;
        }
        seq.end()
    }
}

struct ObjectFieldsSeq<'a>(&'a BTreeMap<String, Value>);

impl<'a> serde::Serialize for ObjectFieldsSeq<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for (key, val) in self.0 {
            seq.serialize_element(&ObjectFieldNode(key, val))?;
        }
        seq.end()
    }
}

struct ObjectFieldNode<'a>(&'a str, &'a Value);

impl<'a> serde::Serialize for ObjectFieldNode<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(3))?;
        map.serialize_entry("kind", "ObjectField")?;
        map.serialize_entry("name", &NameNode(self.0))?;
        map.serialize_entry("value", &ValueNode(self.1))?;
        map.end()
    }
}

// ============================================================================
// Type Nodes
// ============================================================================

struct TypeNodeValue<'a>(&'a TypeNode);

impl<'a> serde::Serialize for TypeNodeValue<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self.0 {
            TypeNode::Named(name) => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("kind", "NamedType")?;
                map.serialize_entry("name", &NameNode(name))?;
                map.end()
            }
            TypeNode::List(inner) => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("kind", "ListType")?;
                map.serialize_entry("type", &TypeNodeValue(inner))?;
                map.end()
            }
            TypeNode::NonNull(inner) => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("kind", "NonNullType")?;
                map.serialize_entry("type", &TypeNodeValue(inner))?;
                map.end()
            }
        }
    }
}

// ============================================================================
// Primitive Wrapper Nodes
// ============================================================================

struct NameNode<'a>(&'a str);

impl<'a> serde::Serialize for NameNode<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("kind", "Name")?;
        map.serialize_entry("value", self.0)?;
        map.end()
    }
}

struct VariableNode<'a>(&'a str);

impl<'a> serde::Serialize for VariableNode<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("kind", "Variable")?;
        map.serialize_entry("name", &NameNode(self.0))?;
        map.end()
    }
}

struct NamedTypeNode<'a>(&'a str);

impl<'a> serde::Serialize for NamedTypeNode<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("kind", "NamedType")?;
        map.serialize_entry("name", &NameNode(self.0))?;
        map.end()
    }
}
