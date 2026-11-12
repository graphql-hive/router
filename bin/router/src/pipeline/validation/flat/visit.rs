//! Walks a flat document the way `graphql_tools::ast::visit_document` walks the tree: the same
//! nodes in the same order, with the same type info on the way, so the rules report the same
//! errors in the same order.

use std::collections::HashMap;

use graphql_tools::flat::{
    Argument, Definition, Directive, Document, Fragment, Kind, ListId, Operation, OperationKind,
    Selection, Span, Sym, TypeId, TypeRef, Value, ValueId, VariableDefinition,
};
use graphql_tools::parser::Pos as TreePos;
use graphql_tools::static_graphql::query::Type;
use graphql_tools::static_graphql::schema::{
    self, DirectiveDefinition, InputValue, TypeDefinition,
};
use graphql_tools::validation::utils::{ValidationError, ValidationErrorContext};

pub type Errors = ValidationErrorContext;

/// A flat position as the tree's `Pos`.
#[inline]
pub fn pos(p: graphql_tools::flat::Pos) -> TreePos {
    TreePos {
        line: p.line as usize,
        column: p.column as usize,
    }
}

/// What `OperationVisitorContext` is to the tree.
pub struct Ctx<'a> {
    pub schema: &'a schema::Document,
    pub doc: &'a Document,
    /// `doc.types` as schema types, so a variable's type compares with an argument's.
    types: &'a [Type],
    /// By name, the last one wins, like the tree's.
    pub known_fragments: HashMap<&'a str, &'a Fragment>,
    /// By name, the last one wins, like the tree's.
    pub directives: HashMap<&'a str, &'a DirectiveDefinition>,
    /// By name, the first one wins, like `schema::Document::type_by_name`.
    type_map: HashMap<&'a str, &'a TypeDefinition>,
    directive_map: HashMap<&'a str, &'a DirectiveDefinition>,

    type_stack: Vec<Option<&'a TypeDefinition>>,
    parent_type_stack: Vec<Option<&'a TypeDefinition>>,
    input_type_stack: Vec<Option<&'a TypeDefinition>>,
    type_literal_stack: Vec<Option<&'a Type>>,
    input_type_literal_stack: Vec<Option<&'a Type>>,
    input_type_default_stack: Vec<bool>,
    field_stack: Vec<Option<&'a schema::Field>>,
}

/// `doc.types` as schema types, in the same order.
pub fn schema_types(doc: &Document) -> Vec<Type> {
    fn build(doc: &Document, id: TypeId, done: &mut [Option<Type>]) -> Type {
        if let Some(t) = &done[id as usize] {
            return t.clone();
        }
        let t = match doc.types[id as usize] {
            TypeRef::Named(name) => Type::NamedType(doc.str(name).to_string()),
            TypeRef::List(inner) => Type::ListType(Box::new(build(doc, inner, done))),
            TypeRef::NonNull(inner) => Type::NonNullType(Box::new(build(doc, inner, done))),
        };
        done[id as usize] = Some(t.clone());
        t
    }
    let mut done = vec![None; doc.types.len()];
    (0..doc.types.len() as TypeId)
        .map(|id| build(doc, id, &mut done))
        .collect()
}

impl<'a> Ctx<'a> {
    pub fn new(doc: &'a Document, types: &'a [Type], schema: &'a schema::Document) -> Self {
        let mut type_map = HashMap::new();
        let mut directive_map = HashMap::new();
        let mut directives = HashMap::new();
        for def in &schema.definitions {
            match def {
                schema::Definition::TypeDefinition(t) => {
                    type_map.entry(t.name()).or_insert(t);
                }
                schema::Definition::DirectiveDefinition(d) => {
                    directive_map.entry(d.name.as_str()).or_insert(d);
                    directives.insert(d.name.as_str(), d);
                }
                _ => {}
            }
        }
        Ctx {
            schema,
            doc,
            types,
            known_fragments: doc.fragments().map(|f| (doc.str(f.name), f)).collect(),
            directives,
            type_map,
            directive_map,
            type_stack: vec![],
            parent_type_stack: vec![],
            input_type_stack: vec![],
            type_literal_stack: vec![],
            input_type_literal_stack: vec![],
            input_type_default_stack: vec![],
            field_stack: vec![],
        }
    }

    #[inline]
    pub fn str(&self, sym: Sym) -> &'a str {
        self.doc.str(sym)
    }

    /// A type of the document (a variable's) as a schema type.
    #[inline]
    pub fn ty(&self, id: TypeId) -> &'a Type {
        &self.types[id as usize]
    }

    /// `schema::Document::type_by_name`, by a map.
    #[inline]
    pub fn type_by_name(&self, name: &str) -> Option<&'a TypeDefinition> {
        self.type_map.get(name).copied()
    }

    /// `schema::Document::directive_by_name`, by a map.
    #[inline]
    pub fn directive_by_name(&self, name: &str) -> Option<&'a DirectiveDefinition> {
        self.directive_map.get(name).copied()
    }

    pub fn current_input_type_has_default(&self) -> bool {
        *self.input_type_default_stack.last().unwrap_or(&false)
    }

    pub fn current_type(&self) -> Option<&'a TypeDefinition> {
        *self.type_stack.last().unwrap_or(&None)
    }

    pub fn current_input_type(&self) -> Option<&'a TypeDefinition> {
        *self.input_type_stack.last().unwrap_or(&None)
    }

    pub fn current_parent_type(&self) -> Option<&'a TypeDefinition> {
        *self.parent_type_stack.last().unwrap_or(&None)
    }

    /// The field's type in a field. Unlike the tree's, `None` in a definition or an inline
    /// fragment, where no rule reads it.
    pub fn current_type_literal(&self) -> Option<&'a Type> {
        *self.type_literal_stack.last().unwrap_or(&None)
    }

    pub fn parent_input_type(&self) -> Option<&'a TypeDefinition> {
        self.input_type_stack
            .len()
            .checked_sub(2)
            .and_then(|index| self.input_type_stack[index])
    }

    pub fn current_input_type_literal(&self) -> Option<&'a Type> {
        *self.input_type_literal_stack.last().unwrap_or(&None)
    }

    pub fn current_field(&self) -> Option<&'a schema::Field> {
        *self.field_stack.last().unwrap_or(&None)
    }

    fn push_type(&mut self, def: Option<&'a TypeDefinition>, literal: Option<&'a Type>) {
        self.type_stack.push(def);
        self.type_literal_stack.push(literal);
    }

    fn pop_type(&mut self) {
        self.type_stack.pop();
        self.type_literal_stack.pop();
    }

    fn push_input_type(&mut self, t: Option<&'a Type>, has_default: bool) {
        self.input_type_stack
            .push(t.and_then(|t| self.type_by_name(t.inner_type())));
        self.input_type_literal_stack.push(t);
        self.input_type_default_stack.push(has_default);
    }

    fn pop_input_type(&mut self) {
        self.input_type_stack.pop();
        self.input_type_literal_stack.pop();
        self.input_type_default_stack.pop();
    }
}

/// `OperationVisitor`, over flat nodes.
#[allow(unused_variables)]
pub trait Visitor<'a> {
    fn enter_document(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors) {}
    fn leave_document(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors) {}
    fn enter_operation(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, op: &'a Operation) {}
    fn leave_operation(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, op: &'a Operation) {}
    fn enter_fragment_definition(
        &mut self,
        ctx: &mut Ctx<'a>,
        errors: &mut Errors,
        fragment: &'a Fragment,
    ) {
    }
    fn leave_fragment_definition(
        &mut self,
        ctx: &mut Ctx<'a>,
        errors: &mut Errors,
        fragment: &'a Fragment,
    ) {
    }
    fn enter_variable_definition(
        &mut self,
        ctx: &mut Ctx<'a>,
        errors: &mut Errors,
        variable: &'a VariableDefinition,
    ) {
    }
    fn leave_variable_definition(
        &mut self,
        ctx: &mut Ctx<'a>,
        errors: &mut Errors,
        variable: &'a VariableDefinition,
    ) {
    }
    fn enter_directive(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, d: &'a Directive) {}
    fn leave_directive(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, d: &'a Directive) {}
    fn enter_argument(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, arg: &'a Argument) {}
    fn leave_argument(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, arg: &'a Argument) {}
    fn enter_selection_set(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, list: ListId) {}
    fn leave_selection_set(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, list: ListId) {}
    fn enter_field(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, field: &'a Selection) {}
    fn leave_field(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, field: &'a Selection) {}
    fn enter_fragment_spread(
        &mut self,
        ctx: &mut Ctx<'a>,
        errors: &mut Errors,
        spread: &'a Selection,
    ) {
    }
    fn leave_fragment_spread(
        &mut self,
        ctx: &mut Ctx<'a>,
        errors: &mut Errors,
        spread: &'a Selection,
    ) {
    }
    fn enter_inline_fragment(
        &mut self,
        ctx: &mut Ctx<'a>,
        errors: &mut Errors,
        fragment: &'a Selection,
    ) {
    }
    fn leave_inline_fragment(
        &mut self,
        ctx: &mut Ctx<'a>,
        errors: &mut Errors,
        fragment: &'a Selection,
    ) {
    }
    fn enter_null_value(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors) {}
    fn leave_null_value(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors) {}
    /// Boolean, Int, Float or String.
    fn enter_scalar_value(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, value: ValueId) {}
    fn leave_scalar_value(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, value: ValueId) {}
    fn enter_enum_value(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, name: Sym) {}
    fn leave_enum_value(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, name: Sym) {}
    fn enter_variable_value(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, name: Sym) {}
    fn leave_variable_value(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, name: Sym) {}
    /// Into `doc.value_items`
    fn enter_list_value(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, items: Span) {}
    fn leave_list_value(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, items: Span) {}
    /// Into `doc.object_fields`
    fn enter_object_value(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, fields: Span) {}
    fn leave_object_value(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, fields: Span) {}
    fn enter_object_field(
        &mut self,
        ctx: &mut Ctx<'a>,
        errors: &mut Errors,
        field: &'a (Sym, ValueId),
    ) {
    }
    fn leave_object_field(
        &mut self,
        ctx: &mut Ctx<'a>,
        errors: &mut Errors,
        field: &'a (Sym, ValueId),
    ) {
    }
}

pub type BoxedVisitor<'a> = Box<dyn Visitor<'a> + 'a>;

/// A validation rule on the flat document: what `ValidationRule` is to the tree.
pub trait FlatRule: Send + Sync {
    fn visitor<'a>(&self) -> BoxedVisitor<'a>;
    fn error_code(&self) -> &'static str;
}

pub(super) struct Walk<'a> {
    pub visitors: Vec<BoxedVisitor<'a>>,
    pub ctx: Ctx<'a>,
    pub errors: Errors,
}

/// Every visitor's hook, in plan order.
macro_rules! each {
    ($w:ident . $hook:ident ( $($arg:expr),* )) => {
        for visitor in &mut $w.visitors {
            visitor.$hook(&mut $w.ctx, &mut $w.errors, $($arg),*);
        }
    };
}

impl<'a> Walk<'a> {
    pub fn document(&mut self) {
        let doc = self.ctx.doc;
        each!(self.enter_document());
        for definition in &doc.definitions {
            match definition {
                Definition::Fragment(fragment) => {
                    let def = self.ctx.type_by_name(doc.str(fragment.type_condition));
                    self.ctx.push_type(def, None);
                    each!(self.enter_fragment_definition(fragment));
                    self.directives(fragment.directives);
                    self.selection_set(fragment.selection_set);
                    each!(self.leave_fragment_definition(fragment));
                    self.ctx.pop_type();
                }
                Definition::Operation(op) => {
                    let schema = self.ctx.schema;
                    let root = match op.kind {
                        OperationKind::Query | OperationKind::Shorthand => {
                            Some(&schema.query_type().name)
                        }
                        OperationKind::Mutation => schema.mutation_type().map(|t| &t.name),
                        OperationKind::Subscription => schema.subscription_type().map(|t| &t.name),
                    };
                    let def = root.and_then(|name| self.ctx.type_by_name(name));
                    self.ctx.push_type(def, None);
                    each!(self.enter_operation(op));
                    self.directives(op.directives);
                    for variable in &doc.variables[op.variables.range()] {
                        self.ctx
                            .push_input_type(Some(self.ctx.ty(variable.ty)), false);
                        each!(self.enter_variable_definition(variable));
                        if let Some(default_value) = variable.default_value {
                            self.value(default_value);
                        }
                        each!(self.leave_variable_definition(variable));
                        self.ctx.pop_input_type();
                    }
                    self.selection_set(op.selection_set);
                    each!(self.leave_operation(op));
                    self.ctx.pop_type();
                }
            }
        }
        each!(self.leave_document());
    }

    fn directives(&mut self, directives: Span) {
        let doc = self.ctx.doc;
        for directive in doc.directives_of(directives) {
            let defs = self
                .ctx
                .directive_by_name(doc.str(directive.name))
                .map(|def| &def.arguments);
            each!(self.enter_directive(directive));
            self.arguments(defs, directive.args);
            each!(self.leave_directive(directive));
        }
    }

    fn arguments(&mut self, defs: Option<&'a Vec<InputValue>>, args: Span) {
        let doc = self.ctx.doc;
        for argument in doc.args_of(args) {
            let name = doc.str(argument.name);
            let def = defs.and_then(|defs| defs.iter().find(|a| a.name == name));
            let has_default = def.is_some_and(|a| a.default_value.is_some());
            self.ctx
                .push_input_type(def.map(|a| &a.value_type), has_default);
            each!(self.enter_argument(argument));
            self.value(argument.value);
            each!(self.leave_argument(argument));
            self.ctx.pop_input_type();
        }
    }

    fn value(&mut self, id: ValueId) {
        let doc = self.ctx.doc;
        match *doc.value(id) {
            Value::Boolean(_) | Value::Float(_) | Value::Int(_) | Value::String(_) => {
                each!(self.enter_scalar_value(id));
                each!(self.leave_scalar_value(id));
            }
            Value::Null => {
                each!(self.enter_null_value());
                each!(self.leave_null_value());
            }
            Value::Enum(name) => {
                each!(self.enter_enum_value(name));
                each!(self.leave_enum_value(name));
            }
            Value::Variable(name) => {
                each!(self.enter_variable_value(name));
                each!(self.leave_variable_value(name));
            }
            Value::List(items) => {
                each!(self.enter_list_value(items));
                let item_type = self.ctx.current_input_type_literal().and_then(|t| {
                    let nullable = match t {
                        Type::NonNullType(inner) => inner.as_ref(),
                        t => t,
                    };
                    match nullable {
                        Type::ListType(inner) => Some(inner.as_ref()),
                        _ => None,
                    }
                });
                self.ctx.push_input_type(item_type, false);
                for &item in &doc.value_items[items.range()] {
                    self.value(item);
                }
                self.ctx.pop_input_type();
                each!(self.leave_list_value(items));
            }
            Value::Object(fields) => {
                each!(self.enter_object_value(fields));
                for pair in &doc.object_fields[fields.range()] {
                    let input_field = self
                        .ctx
                        .current_input_type_literal()
                        .and_then(|t| self.ctx.type_by_name(t.inner_type()))
                        .and_then(|t| t.input_field_by_name(doc.str(pair.0)));
                    let has_default = input_field.is_some_and(|f| f.default_value.is_some());
                    self.ctx
                        .push_input_type(input_field.map(|f| &f.value_type), has_default);
                    each!(self.enter_object_field(pair));
                    self.value(pair.1);
                    each!(self.leave_object_field(pair));
                    self.ctx.pop_input_type();
                }
                each!(self.leave_object_value(fields));
            }
        }
    }

    fn selection_set(&mut self, list: ListId) {
        let doc = self.ctx.doc;
        let parent = self.ctx.current_type();
        self.ctx.parent_type_stack.push(parent);
        each!(self.enter_selection_set(list));
        for &id in doc.items(list) {
            self.selection(doc.selection(id));
        }
        each!(self.leave_selection_set(list));
        self.ctx.parent_type_stack.pop();
    }

    fn selection(&mut self, s: &'a Selection) {
        let doc = self.ctx.doc;
        match s.kind {
            Kind::Field => {
                let field_def = self
                    .ctx
                    .current_parent_type()
                    .and_then(|t| t.field_by_name(doc.str(s.name)));
                let field_type = field_def.map(|f| &f.field_type);
                let def = field_type.and_then(|t| self.ctx.type_by_name(t.inner_type()));
                self.ctx.push_type(def, field_type);
                each!(self.enter_field(s));
                self.ctx.field_stack.push(field_def);
                self.arguments(field_def.map(|f| &f.arguments), s.args);
                self.directives(s.directives);
                self.selection_set(s.children);
                self.ctx.field_stack.pop();
                each!(self.leave_field(s));
                self.ctx.pop_type();
            }
            Kind::FragmentSpread => {
                each!(self.enter_fragment_spread(s));
                self.directives(s.directives);
                each!(self.leave_fragment_spread(s));
            }
            Kind::InlineFragment => {
                let typed = doc.opt_str(s.type_condition);
                if let Some(name) = typed {
                    let def = self.ctx.type_by_name(name);
                    self.ctx.push_type(def, None);
                }
                each!(self.enter_inline_fragment(s));
                self.directives(s.directives);
                self.selection_set(s.children);
                each!(self.leave_inline_fragment(s));
                if typed.is_some() {
                    self.ctx.pop_type();
                }
            }
        }
    }
}

/// A value printed like the tree's `Display` prints it: `[a, b]`, `{a: 1}`, strings quoted.
pub fn display_value(doc: &Document, id: ValueId) -> String {
    let mut out = String::new();
    write_value(doc, id, &mut out);
    out
}

fn write_value(doc: &Document, id: ValueId, out: &mut String) {
    use std::fmt::Write;
    match *doc.value(id) {
        Value::Variable(name) => {
            out.push('$');
            out.push_str(doc.str(name));
        }
        Value::Int(i) => write!(out, "{i}").unwrap(),
        Value::Float(f) => write!(out, "{f}").unwrap(),
        Value::String(s) => write_quoted(doc.str(s), out),
        Value::Boolean(b) => out.push_str(if b { "true" } else { "false" }),
        Value::Null => out.push_str("null"),
        Value::Enum(name) => out.push_str(doc.str(name)),
        Value::List(items) => {
            out.push('[');
            for (i, &item) in doc.value_items[items.range()].iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_value(doc, item, out);
            }
            out.push(']');
        }
        Value::Object(fields) => {
            out.push('{');
            for (i, &(name, value)) in doc.object_fields[fields.range()].iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(doc.str(name));
                out.push_str(": ");
                write_value(doc, value, out);
            }
            out.push('}');
        }
    }
}

/// `Formatter::write_quoted` at indent 0.
fn write_quoted(s: &str, out: &mut String) {
    use std::fmt::Write;
    let has_newline = s.contains('\n');
    let has_nonprintable = s
        .chars()
        .any(|c| !matches!(c, '\n' | '\r' | '\t' | '\u{0020}'..='\u{FFFF}'));
    if !has_newline || has_nonprintable {
        out.push('"');
        for c in s.chars() {
            match c {
                '\r' => out.push_str(r"\r"),
                '\n' => out.push_str(r"\n"),
                '\t' => out.push_str(r"\t"),
                '"' => out.push_str("\\\""),
                '\\' => out.push_str(r"\\"),
                '\u{0020}'..='\u{FFFF}' => out.push(c),
                // decimal, like the tree's
                _ => write!(out, "\\u{:04}", c as u32).unwrap(),
            }
        }
        out.push('"');
    } else {
        out.push_str("\"\"\"\n");
        for line in s.lines() {
            if !line.trim().is_empty() {
                out.push_str("  ");
                out.push_str(&line.replace(r#"""""#, r#"\""""#));
            }
            out.push('\n');
        }
        out.push_str("\"\"\"");
    }
}

/// `Value::compare` of the tree, zip quirk included: lists of different lengths compare by
/// the shorter one.
pub fn values_equal(doc: &Document, a: ValueId, b: ValueId) -> bool {
    match (*doc.value(a), *doc.value(b)) {
        (Value::Null, Value::Null) => true,
        (Value::Boolean(a), Value::Boolean(b)) => a == b,
        (Value::Int(a), Value::Int(b)) => a == b,
        (Value::Float(a), Value::Float(b)) => a == b,
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Enum(a), Value::Enum(b)) => a == b,
        (Value::Variable(a), Value::Variable(b)) => a == b,
        (Value::List(a), Value::List(b)) => doc.value_items[a.range()]
            .iter()
            .zip(&doc.value_items[b.range()])
            .all(|(&a, &b)| values_equal(doc, a, b)),
        (Value::Object(a), Value::Object(b)) => {
            if a.len != b.len {
                return false;
            }
            let b = &doc.object_fields[b.range()];
            let mut matched = vec![false; b.len()];
            for &(k_a, v_a) in &doc.object_fields[a.range()] {
                let found = b.iter().enumerate().position(|(idx, &(k_b, v_b))| {
                    !matched[idx] && k_a == k_b && values_equal(doc, v_a, v_b)
                });
                match found {
                    Some(idx) => matched[idx] = true,
                    None => return false,
                }
            }
            true
        }
        _ => false,
    }
}

/// The error with these fields; saves a few lines in every rule.
#[inline]
pub fn error(
    error_code: &'static str,
    message: String,
    locations: Vec<TreePos>,
) -> ValidationError {
    ValidationError {
        locations,
        message,
        error_code,
    }
}
