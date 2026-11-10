//! A flat `Document` from the parser's tree, the same one `parse::parse` makes from the text.
//!
//! The pipeline normalizes the parsed document it caches and hands to plugins, which may
//! not be what the query text says, so it's walked rather than the text parsed again.

use graphql_tools::parser::query as tree;

use super::{
    Argument, Definition, Directive, Document, Fragment, Kind, Operation, OperationKind, Selection,
    Span, Sym, TypeRef, Value, ValueId, VariableDefinition, NONE,
};

pub fn from_tree<'a, T: tree::Text<'a>>(document: &tree::Document<'a, T>) -> Document {
    let mut builder = Builder {
        doc: Document::default(),
        stack: Vec::new(),
    };
    for definition in &document.definitions {
        let definition = match definition {
            tree::Definition::Operation(operation) => {
                Definition::Operation(builder.operation(operation))
            }
            tree::Definition::Fragment(fragment) => Definition::Fragment(Fragment {
                name: builder.name(fragment.name.as_ref()),
                type_condition: builder.type_condition(&fragment.type_condition),
                selection_set: builder.selection_set(&fragment.selection_set),
            }),
        };
        builder.doc.definitions.push(definition);
    }
    builder.doc
}

struct Builder {
    doc: Document,
    /// Selection lists being built, a nested one on top.
    stack: Vec<u32>,
}

impl Builder {
    fn name(&mut self, name: &str) -> Sym {
        self.doc.strings.intern(name)
    }

    fn type_condition<'a, T: tree::Text<'a>>(&mut self, tc: &tree::TypeCondition<'a, T>) -> Sym {
        let tree::TypeCondition::On(name) = tc;
        self.name(name.as_ref())
    }

    fn operation<'a, T: tree::Text<'a>>(
        &mut self,
        operation: &tree::OperationDefinition<'a, T>,
    ) -> Operation {
        let (kind, name, variables, selection_set) = match operation {
            tree::OperationDefinition::SelectionSet(selection_set) => {
                (OperationKind::Shorthand, &None, &[][..], selection_set)
            }
            tree::OperationDefinition::Query(q) => (
                OperationKind::Query,
                &q.name,
                &q.variable_definitions[..],
                &q.selection_set,
            ),
            tree::OperationDefinition::Mutation(m) => (
                OperationKind::Mutation,
                &m.name,
                &m.variable_definitions[..],
                &m.selection_set,
            ),
            tree::OperationDefinition::Subscription(s) => (
                OperationKind::Subscription,
                &s.name,
                &s.variable_definitions[..],
                &s.selection_set,
            ),
        };
        let name = match name {
            Some(name) => self.name(name.as_ref()),
            None => NONE,
        };
        let definitions: Vec<VariableDefinition> = variables
            .iter()
            .map(|variable| VariableDefinition {
                name: self.name(variable.name.as_ref()),
                ty: self.type_ref(&variable.var_type),
                default_value: variable.default_value.as_ref().map(|v| self.value(v)),
            })
            .collect();
        let start = self.doc.variables.len() as u32;
        self.doc.variables.extend(definitions);
        Operation {
            kind,
            name,
            variables: Span {
                start,
                len: variables.len() as u32,
            },
            selection_set: self.selection_set(selection_set),
        }
    }

    fn type_ref<'a, T: tree::Text<'a>>(&mut self, ty: &tree::Type<'a, T>) -> u32 {
        let ty = match ty {
            tree::Type::NamedType(name) => TypeRef::Named(self.name(name.as_ref())),
            tree::Type::ListType(inner) => TypeRef::List(self.type_ref(inner)),
            tree::Type::NonNullType(inner) => TypeRef::NonNull(self.type_ref(inner)),
        };
        self.doc.types.push(ty);
        self.doc.types.len() as u32 - 1
    }

    fn selection_set<'a, T: tree::Text<'a>>(&mut self, set: &tree::SelectionSet<'a, T>) -> Span {
        let mark = self.stack.len();
        for item in &set.items {
            let selection = match item {
                tree::Selection::Field(field) => {
                    let name = self.name(field.name.as_ref());
                    let alias = match &field.alias {
                        Some(alias) => self.name(alias.as_ref()),
                        None => NONE,
                    };
                    let args = self.arguments(&field.arguments);
                    let directives = self.directives(&field.directives);
                    Selection {
                        kind: Kind::Field,
                        name,
                        alias,
                        type_condition: NONE,
                        args,
                        directives,
                        children: self.selection_set(&field.selection_set),
                    }
                }
                tree::Selection::InlineFragment(fragment) => {
                    let type_condition = match &fragment.type_condition {
                        Some(tc) => self.type_condition(tc),
                        None => NONE,
                    };
                    let directives = self.directives(&fragment.directives);
                    Selection {
                        kind: Kind::InlineFragment,
                        name: NONE,
                        alias: NONE,
                        type_condition,
                        args: Span::EMPTY,
                        directives,
                        children: self.selection_set(&fragment.selection_set),
                    }
                }
                tree::Selection::FragmentSpread(spread) => Selection {
                    kind: Kind::FragmentSpread,
                    name: self.name(spread.fragment_name.as_ref()),
                    alias: NONE,
                    type_condition: NONE,
                    args: Span::EMPTY,
                    directives: self.directives(&spread.directives),
                    children: Span::EMPTY,
                },
            };
            self.doc.selections.push(selection);
            self.stack.push(self.doc.selections.len() as u32 - 1);
        }
        let start = self.doc.children.len() as u32;
        self.doc.children.extend(self.stack.drain(mark..));
        Span {
            start,
            len: self.doc.children.len() as u32 - start,
        }
    }

    /// The values go in their own arrays, so one list's arguments stay together.
    fn arguments<'a, T: tree::Text<'a>>(
        &mut self,
        args: &[(T::Value, tree::Value<'a, T>)],
    ) -> Span {
        let start = self.doc.args.len() as u32;
        for (name, value) in args {
            let name = self.name(name.as_ref());
            let value = self.value(value);
            self.doc.args.push(Argument { name, value });
        }
        Span {
            start,
            len: args.len() as u32,
        }
    }

    /// A directive's arguments go in `args`, so the directives stay together.
    fn directives<'a, T: tree::Text<'a>>(&mut self, directives: &[tree::Directive<'a, T>]) -> Span {
        let start = self.doc.directives.len() as u32;
        for directive in directives {
            let name = self.name(directive.name.as_ref());
            let args = self.arguments(&directive.arguments);
            self.doc.directives.push(Directive { name, args });
        }
        Span {
            start,
            len: directives.len() as u32,
        }
    }

    fn value<'a, T: tree::Text<'a>>(&mut self, value: &tree::Value<'a, T>) -> ValueId {
        let value = match value {
            tree::Value::Variable(name) => Value::Variable(self.name(name.as_ref())),
            tree::Value::Int(int) => Value::Int(
                int.as_i64()
                    .expect("GraphQL integer value out of i64 range"),
            ),
            tree::Value::Float(float) => Value::Float(*float),
            tree::Value::String(s) => Value::String(self.name(s)),
            tree::Value::Boolean(b) => Value::Boolean(*b),
            tree::Value::Null => Value::Null,
            tree::Value::Enum(name) => Value::Enum(self.name(name.as_ref())),
            tree::Value::List(items) => {
                let items: Vec<ValueId> = items.iter().map(|item| self.value(item)).collect();
                let start = self.doc.value_items.len() as u32;
                self.doc.value_items.extend(items);
                Value::List(Span {
                    start,
                    len: self.doc.value_items.len() as u32 - start,
                })
            }
            // as written, repeats too
            tree::Value::Object(fields) => {
                let fields: Vec<(Sym, ValueId)> = fields
                    .iter()
                    .map(|(name, value)| (self.name(name.as_ref()), self.value(value)))
                    .collect();
                let start = self.doc.object_fields.len() as u32;
                self.doc.object_fields.extend(fields);
                Value::Object(Span {
                    start,
                    len: self.doc.object_fields.len() as u32 - start,
                })
            }
        };
        self.doc.values.push(value);
        self.doc.values.len() as u32 - 1
    }
}
