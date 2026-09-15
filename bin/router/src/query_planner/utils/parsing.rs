use graphql_tools::parser::query;
use graphql_tools::parser::query::Text;
use graphql_tools::parser::schema;

use crate::query_planner::ast::shrink::ShrinkMemory;

#[inline]
pub fn parse_schema(sdl: &str) -> schema::Document<'static, String> {
    graphql_tools::parser::parse_schema(sdl)
        .unwrap()
        .into_static()
}

#[inline]
pub fn safe_parse_schema(
    sdl: &str,
) -> Result<schema::Document<'static, String>, schema::ParseError> {
    graphql_tools::parser::parse_schema(sdl).map(|schema| schema.into_static())
}

#[inline]
pub fn parse_operation(operation: &str) -> query::Document<'static, String> {
    graphql_tools::parser::parse_query(operation)
        .unwrap()
        .into_static()
}

#[inline]
pub fn safe_parse_operation(
    operation: &str,
) -> Result<query::Document<'static, String>, query::ParseError> {
    graphql_tools::parser::parse_query(operation).map(|op| op.into_static())
}

#[inline]
pub fn safe_parse_operation_with_token_limit(
    operation: &str,
    token_limit: usize,
) -> Result<query::Document<'static, String>, query::ParseError> {
    graphql_tools::parser::parse_query_with_token_limit(operation, token_limit)
        .map(|op| op.into_static())
}

#[inline]
pub fn shrink_parsed_operation(document: &mut query::Document<'static, String>) {
    document.shrink_memory();
}

impl<'a, T: Text<'a>> ShrinkMemory for query::Document<'a, T> {
    fn shrink_memory(&mut self) {
        self.definitions.shrink_memory();
    }
}

impl<'a, T: Text<'a>> ShrinkMemory for query::Definition<'a, T> {
    fn shrink_memory(&mut self) {
        match self {
            query::Definition::Operation(operation) => operation.shrink_memory(),
            query::Definition::Fragment(fragment) => fragment.shrink_memory(),
        }
    }
}

impl<'a, T: Text<'a>> ShrinkMemory for query::OperationDefinition<'a, T> {
    fn shrink_memory(&mut self) {
        match self {
            query::OperationDefinition::SelectionSet(set) => set.shrink_memory(),
            query::OperationDefinition::Query(q) => {
                q.variable_definitions.shrink_memory();
                shrink_directives(&mut q.directives);
                q.selection_set.shrink_memory();
            }
            query::OperationDefinition::Mutation(m) => {
                m.variable_definitions.shrink_memory();
                shrink_directives(&mut m.directives);
                m.selection_set.shrink_memory();
            }
            query::OperationDefinition::Subscription(s) => {
                s.variable_definitions.shrink_memory();
                shrink_directives(&mut s.directives);
                s.selection_set.shrink_memory();
            }
        }
    }
}

impl<'a, T: Text<'a>> ShrinkMemory for query::FragmentDefinition<'a, T> {
    fn shrink_memory(&mut self) {
        shrink_directives(&mut self.directives);
        self.selection_set.shrink_memory();
    }
}

impl<'a, T: Text<'a>> ShrinkMemory for query::SelectionSet<'a, T> {
    fn shrink_memory(&mut self) {
        self.items.shrink_memory();
    }
}

impl<'a, T: Text<'a>> ShrinkMemory for query::Selection<'a, T> {
    fn shrink_memory(&mut self) {
        match self {
            query::Selection::Field(field) => {
                shrink_arguments(&mut field.arguments);
                shrink_directives(&mut field.directives);
                field.selection_set.shrink_memory();
            }
            query::Selection::FragmentSpread(spread) => {
                shrink_directives(&mut spread.directives);
            }
            query::Selection::InlineFragment(fragment) => {
                shrink_directives(&mut fragment.directives);
                fragment.selection_set.shrink_memory();
            }
        }
    }
}

impl<'a, T: Text<'a>> ShrinkMemory for query::VariableDefinition<'a, T> {
    fn shrink_memory(&mut self) {
        self.default_value.shrink_memory();
    }
}

impl<'a, T: Text<'a>> ShrinkMemory for query::Directive<'a, T> {
    fn shrink_memory(&mut self) {
        shrink_arguments(&mut self.arguments);
    }
}

impl<'a, T: Text<'a>> ShrinkMemory for query::Value<'a, T> {
    fn shrink_memory(&mut self) {
        match self {
            query::Value::List(items) => items.shrink_memory(),
            query::Value::Object(fields) => {
                for (_, value) in fields.iter_mut() {
                    value.shrink_memory();
                }
                fields.shrink_to_fit();
            }
            _ => {}
        }
    }
}

fn shrink_directives<'a, T: Text<'a>>(directives: &mut Vec<query::Directive<'a, T>>) {
    directives.shrink_memory();
}

fn shrink_arguments<'a, T: Text<'a>>(arguments: &mut Vec<(T::Value, query::Value<'a, T>)>) {
    for (_, value) in arguments.iter_mut() {
        value.shrink_memory();
    }
    arguments.shrink_to_fit();
}
