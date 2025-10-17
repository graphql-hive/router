/// Custom serializer for converting Rust AST types to graphql-js v16 JSON format
///
/// This module provides a single serialization function that converts the Rust-idiomatic AST
/// structures to match the exact JSON format expected by graphql-js v16, including:
/// - Converting snake_case field names to camelCase
/// - Adding "kind" discriminator fields with proper string values
/// - Wrapping simple values like names in object structures
/// - Handling optional fields correctly (omitting when None)
///
/// Performance considerations:
/// - Uses pre-sized maps where possible to minimize allocations
/// - Avoids unnecessary cloning of data
/// - Designed for hot-path execution
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

/// Serialize a Document to graphql-js v16 DocumentNode format
///
/// This is the main entry point for serialization. Use this with serde's `serialize_with` attribute:
/// ```ignore
/// #[serde(serialize_with = "graphql_js_serializer::serialize_document")]
/// operation_document_node: Document,
/// ```
pub fn serialize_document<S>(doc: &Document, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut map = serializer.serialize_map(Some(2))?;
    map.serialize_entry("kind", "Document")?;
    map.serialize_entry("definitions", &DefinitionsSeq { doc })?;
    map.end()
}

/// Helper to serialize definitions array
struct DefinitionsSeq<'a> {
    doc: &'a Document,
}

impl<'a> serde::Serialize for DefinitionsSeq<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(1 + self.doc.fragments.len()))?;
        serialize_operation_as_seq_element(&self.doc.operation, &mut seq)?;
        for fragment in &self.doc.fragments {
            serialize_fragment_as_seq_element(fragment, &mut seq)?;
        }
        seq.end()
    }
}

/// Serialize OperationDefinition as a sequence element
fn serialize_operation_as_seq_element<S>(
    op: &OperationDefinition,
    seq: &mut S,
) -> Result<(), S::Error>
where
    S: SerializeSeq,
{
    seq.serialize_element(&OperationDefNode { op })
}

/// Wrapper for OperationDefinition
struct OperationDefNode<'a> {
    op: &'a OperationDefinition,
}

impl<'a> serde::Serialize for OperationDefNode<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let op = self.op;
        let mut len = 2; // kind + selectionSet (always present)
        if op.operation_kind.is_some() {
            len += 1;
        }
        if op.name.is_some() {
            len += 1;
        }
        if op.variable_definitions.as_ref().is_some_and(|v| !v.is_empty()) {
            len += 1;
        }

        let mut map = serializer.serialize_map(Some(len))?;
        map.serialize_entry("kind", "OperationDefinition")?;

        // operation: "query" | "mutation" | "subscription"
        if let Some(kind) = &op.operation_kind {
            map.serialize_entry(
                "operation",
                match kind {
                    OperationKind::Query => "query",
                    OperationKind::Mutation => "mutation",
                    OperationKind::Subscription => "subscription",
                },
            )?;
        }

        // name (optional)
        if let Some(name) = &op.name {
            map.serialize_entry("name", &NameNodeValue { value: name.as_str() })?;
        }

        // variableDefinitions (optional, omit if empty)
        if let Some(var_defs) = &op.variable_definitions {
            if !var_defs.is_empty() {
                map.serialize_entry("variableDefinitions", &VarDefsSeq { defs: var_defs })?;
            }
        }

        // selectionSet (required)
        map.serialize_entry("selectionSet", &SelectionSetNode { set: &op.selection_set })?;

        map.end()
    }
}

/// Serialize FragmentDefinition as a sequence element
fn serialize_fragment_as_seq_element<S>(
    frag: &FragmentDefinition,
    seq: &mut S,
) -> Result<(), S::Error>
where
    S: SerializeSeq,
{
    seq.serialize_element(&FragmentDefNode { frag })
}

/// Wrapper for FragmentDefinition
struct FragmentDefNode<'a> {
    frag: &'a FragmentDefinition,
}

impl<'a> serde::Serialize for FragmentDefNode<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(4))?;
        map.serialize_entry("kind", "FragmentDefinition")?;
        map.serialize_entry("name", &NameNodeValue { value: self.frag.name.as_str() })?;
        map.serialize_entry(
            "typeCondition",
            &NamedTypeNodeValue {
                name: self.frag.type_condition.as_str(),
            },
        )?;
        map.serialize_entry("selectionSet", &SelectionSetNode { set: &self.frag.selection_set })?;
        map.end()
    }
}

/// Wrapper for variable definitions array
struct VarDefsSeq<'a> {
    defs: &'a [VariableDefinition],
}

impl<'a> serde::Serialize for VarDefsSeq<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(self.defs.len()))?;
        for def in self.defs {
            seq.serialize_element(&VarDefNode { def })?;
        }
        seq.end()
    }
}

/// Wrapper for VariableDefinition
struct VarDefNode<'a> {
    def: &'a VariableDefinition,
}

impl<'a> serde::Serialize for VarDefNode<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut len = 3; // kind + variable + type
        if self.def.default_value.is_some() {
            len += 1;
        }

        let mut map = serializer.serialize_map(Some(len))?;
        map.serialize_entry("kind", "VariableDefinition")?;
        map.serialize_entry(
            "variable",
            &VariableNodeValue {
                name: self.def.name.as_str(),
            },
        )?;
        map.serialize_entry("type", &TypeNodeValue { ty: &self.def.variable_type })?;

        if let Some(default_val) = &self.def.default_value {
            map.serialize_entry("defaultValue", &ValueNodeValue { val: default_val })?;
        }

        map.end()
    }
}

/// Wrapper for SelectionSet
struct SelectionSetNode<'a> {
    set: &'a SelectionSet,
}

impl<'a> serde::Serialize for SelectionSetNode<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("kind", "SelectionSet")?;
        map.serialize_entry("selections", &SelectionsSeq { items: &self.set.items })?;
        map.end()
    }
}

/// Wrapper for selections array
struct SelectionsSeq<'a> {
    items: &'a [SelectionItem],
}

impl<'a> serde::Serialize for SelectionsSeq<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(self.items.len()))?;
        for item in self.items {
            match item {
                SelectionItem::Field(field) => {
                    seq.serialize_element(&FieldNode { field })?;
                }
                SelectionItem::InlineFragment(frag) => {
                    seq.serialize_element(&InlineFragNode { frag })?;
                }
                SelectionItem::FragmentSpread(name) => {
                    seq.serialize_element(&FragSpreadNode { name: name.as_str() })?;
                }
            }
        }
        seq.end()
    }
}

/// Wrapper for FieldSelection
struct FieldNode<'a> {
    field: &'a FieldSelection,
}

impl<'a> serde::Serialize for FieldNode<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let f = self.field;
        let mut len = 2; // kind + name
        if f.alias.is_some() {
            len += 1;
        }

        // Use the public arguments() method which returns Option<&ArgumentsMap>
        let has_args = f.arguments().is_some();
        if has_args {
            len += 1;
        }

        if f.skip_if.is_some() || f.include_if.is_some() {
            len += 1;
        }
        if !f.selections.is_empty() {
            len += 1;
        }

        let mut map = serializer.serialize_map(Some(len))?;
        map.serialize_entry("kind", "Field")?;

        if let Some(alias) = &f.alias {
            map.serialize_entry("alias", &NameNodeValue { value: alias.as_str() })?;
        }

        map.serialize_entry("name", &NameNodeValue { value: f.name.as_str() })?;

        if let Some(args) = f.arguments() {
            map.serialize_entry("arguments", &ArgumentsSeq { args_map: args })?;
        }

        if f.skip_if.is_some() || f.include_if.is_some() {
            map.serialize_entry(
                "directives",
                &DirectivesSeq {
                    skip_if: &f.skip_if,
                    include_if: &f.include_if,
                },
            )?;
        }

        if !f.selections.is_empty() {
            map.serialize_entry("selectionSet", &SelectionSetNode { set: &f.selections })?;
        }

        map.end()
    }
}

/// Wrapper for InlineFragmentSelection
struct InlineFragNode<'a> {
    frag: &'a InlineFragmentSelection,
}

impl<'a> serde::Serialize for InlineFragNode<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let f = self.frag;
        let mut len = 3; // kind + typeCondition + selectionSet
        if f.skip_if.is_some() || f.include_if.is_some() {
            len += 1;
        }

        let mut map = serializer.serialize_map(Some(len))?;
        map.serialize_entry("kind", "InlineFragment")?;
        map.serialize_entry(
            "typeCondition",
            &NamedTypeNodeValue {
                name: f.type_condition.as_str(),
            },
        )?;

        if f.skip_if.is_some() || f.include_if.is_some() {
            map.serialize_entry(
                "directives",
                &DirectivesSeq {
                    skip_if: &f.skip_if,
                    include_if: &f.include_if,
                },
            )?;
        }

        map.serialize_entry("selectionSet", &SelectionSetNode { set: &f.selections })?;
        map.end()
    }
}

/// Wrapper for FragmentSpread
struct FragSpreadNode<'a> {
    name: &'a str,
}

impl<'a> serde::Serialize for FragSpreadNode<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("kind", "FragmentSpread")?;
        map.serialize_entry("name", &NameNodeValue { value: self.name })?;
        map.end()
    }
}

/// Wrapper for arguments array
/// We need to work with ArgumentsMap through its public interface
struct ArgumentsSeq<'a, T: 'a> {
    args_map: &'a T,
}

// We need to be able to iterate over arguments
// ArgumentsMap has keys() and get_argument() methods that are public
// But we can't directly import ArgumentsMap, so we'll use a trait bound
impl<'a, T> serde::Serialize for ArgumentsSeq<'a, T>
where
    for<'b> &'b T: IntoIterator<Item = (&'b String, &'b Value)>,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Collect entries to get the count
        let entries: Vec<_> = self.args_map.into_iter().collect();
        let mut seq = serializer.serialize_seq(Some(entries.len()))?;

        for (name, value) in entries {
            seq.serialize_element(&ArgumentNode {
                name: name.as_str(),
                value,
            })?;
        }

        seq.end()
    }
}

/// Wrapper for Argument
struct ArgumentNode<'a> {
    name: &'a str,
    value: &'a Value,
}

impl<'a> serde::Serialize for ArgumentNode<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(3))?;
        map.serialize_entry("kind", "Argument")?;
        map.serialize_entry("name", &NameNodeValue { value: self.name })?;
        map.serialize_entry("value", &ValueNodeValue { val: self.value })?;
        map.end()
    }
}

/// Wrapper for directives array
struct DirectivesSeq<'a> {
    skip_if: &'a Option<String>,
    include_if: &'a Option<String>,
}

impl<'a> serde::Serialize for DirectivesSeq<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut count = 0;
        if self.skip_if.is_some() {
            count += 1;
        }
        if self.include_if.is_some() {
            count += 1;
        }

        let mut seq = serializer.serialize_seq(Some(count))?;

        if let Some(var) = self.skip_if {
            seq.serialize_element(&DirectiveNode {
                name: "skip",
                arg_name: "if",
                var_name: var.as_str(),
            })?;
        }

        if let Some(var) = self.include_if {
            seq.serialize_element(&DirectiveNode {
                name: "include",
                arg_name: "if",
                var_name: var.as_str(),
            })?;
        }

        seq.end()
    }
}

/// Wrapper for Directive
struct DirectiveNode<'a> {
    name: &'a str,
    arg_name: &'a str,
    var_name: &'a str,
}

impl<'a> serde::Serialize for DirectiveNode<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(3))?;
        map.serialize_entry("kind", "Directive")?;
        map.serialize_entry("name", &NameNodeValue { value: self.name })?;
        map.serialize_entry("arguments", &DirectiveArgs {
            arg_name: self.arg_name,
            var_name: self.var_name,
        })?;
        map.end()
    }
}

/// Wrapper for directive arguments
struct DirectiveArgs<'a> {
    arg_name: &'a str,
    var_name: &'a str,
}

impl<'a> serde::Serialize for DirectiveArgs<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(1))?;
        seq.serialize_element(&DirectiveArgNode {
            name: self.arg_name,
            var_name: self.var_name,
        })?;
        seq.end()
    }
}

/// Wrapper for directive argument
struct DirectiveArgNode<'a> {
    name: &'a str,
    var_name: &'a str,
}

impl<'a> serde::Serialize for DirectiveArgNode<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(3))?;
        map.serialize_entry("kind", "Argument")?;
        map.serialize_entry("name", &NameNodeValue { value: self.name })?;
        map.serialize_entry(
            "value",
            &VariableNodeValue {
                name: self.var_name,
            },
        )?;
        map.end()
    }
}

/// Wrapper for Value
struct ValueNodeValue<'a> {
    val: &'a Value,
}

impl<'a> serde::Serialize for ValueNodeValue<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self.val {
            Value::Variable(name) => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("kind", "Variable")?;
                map.serialize_entry("name", &NameNodeValue { value: name.as_str() })?;
                map.end()
            }
            Value::Int(i) => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("kind", "IntValue")?;
                map.serialize_entry("value", &i.to_string())?;
                map.end()
            }
            Value::Float(f) => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("kind", "FloatValue")?;
                map.serialize_entry("value", &f.to_string())?;
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
                map.serialize_entry("values", &ValuesSeq { vals: list })?;
                map.end()
            }
            Value::Object(obj) => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("kind", "ObjectValue")?;
                map.serialize_entry("fields", &ObjectFieldsSeq { obj })?;
                map.end()
            }
        }
    }
}

/// Wrapper for list values
struct ValuesSeq<'a> {
    vals: &'a [Value],
}

impl<'a> serde::Serialize for ValuesSeq<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(self.vals.len()))?;
        for val in self.vals {
            seq.serialize_element(&ValueNodeValue { val })?;
        }
        seq.end()
    }
}

/// Wrapper for object fields
struct ObjectFieldsSeq<'a> {
    obj: &'a BTreeMap<String, Value>,
}

impl<'a> serde::Serialize for ObjectFieldsSeq<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(self.obj.len()))?;
        for (key, val) in self.obj {
            seq.serialize_element(&ObjectFieldNode {
                name: key.as_str(),
                value: val,
            })?;
        }
        seq.end()
    }
}

/// Wrapper for object field
struct ObjectFieldNode<'a> {
    name: &'a str,
    value: &'a Value,
}

impl<'a> serde::Serialize for ObjectFieldNode<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(3))?;
        map.serialize_entry("kind", "ObjectField")?;
        map.serialize_entry("name", &NameNodeValue { value: self.name })?;
        map.serialize_entry("value", &ValueNodeValue { val: self.value })?;
        map.end()
    }
}

/// Wrapper for TypeNode
struct TypeNodeValue<'a> {
    ty: &'a TypeNode,
}

impl<'a> serde::Serialize for TypeNodeValue<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self.ty {
            TypeNode::Named(name) => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("kind", "NamedType")?;
                map.serialize_entry("name", &NameNodeValue { value: name.as_str() })?;
                map.end()
            }
            TypeNode::List(inner) => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("kind", "ListType")?;
                map.serialize_entry("type", &TypeNodeValue { ty: inner.as_ref() })?;
                map.end()
            }
            TypeNode::NonNull(inner) => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("kind", "NonNullType")?;
                map.serialize_entry("type", &TypeNodeValue { ty: inner.as_ref() })?;
                map.end()
            }
        }
    }
}

/// Simple wrapper for NameNode
struct NameNodeValue<'a> {
    value: &'a str,
}

impl<'a> serde::Serialize for NameNodeValue<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("kind", "Name")?;
        map.serialize_entry("value", self.value)?;
        map.end()
    }
}

/// Simple wrapper for VariableNode
struct VariableNodeValue<'a> {
    name: &'a str,
}

impl<'a> serde::Serialize for VariableNodeValue<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("kind", "Variable")?;
        map.serialize_entry("name", &NameNodeValue { value: self.name })?;
        map.end()
    }
}

/// Simple wrapper for NamedTypeNode
struct NamedTypeNodeValue<'a> {
    name: &'a str,
}

impl<'a> serde::Serialize for NamedTypeNodeValue<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("kind", "NamedType")?;
        map.serialize_entry("name", &NameNodeValue { value: self.name })?;
        map.end()
    }
}
