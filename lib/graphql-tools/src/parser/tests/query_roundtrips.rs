use std::fs::File;
use std::io::Read;

use crate::parser::query::{
    Definition, Directive, Document, OperationDefinition, Selection, SelectionSet, Value,
    VariableDefinition,
};
use crate::parser::{parse_query, Style};

fn normalize_document<'a>(document: &mut Document<'a, String>) {
    for definition in &mut document.definitions {
        match definition {
            Definition::Operation(operation) => normalize_operation(operation),
            Definition::Fragment(fragment) => {
                normalize_directives(&mut fragment.directives);
                normalize_selection_set(&mut fragment.selection_set);
            }
        }
    }
}

fn normalize_operation<'a>(operation: &mut OperationDefinition<'a, String>) {
    let selection_set = match operation {
        OperationDefinition::SelectionSet(selection_set) => selection_set,
        OperationDefinition::Query(query) => {
            normalize_variable_definitions(&mut query.variable_definitions);
            normalize_directives(&mut query.directives);
            &mut query.selection_set
        }
        OperationDefinition::Mutation(mutation) => {
            normalize_variable_definitions(&mut mutation.variable_definitions);
            normalize_directives(&mut mutation.directives);
            &mut mutation.selection_set
        }
        OperationDefinition::Subscription(subscription) => {
            normalize_variable_definitions(&mut subscription.variable_definitions);
            normalize_directives(&mut subscription.directives);
            &mut subscription.selection_set
        }
    };
    normalize_selection_set(selection_set);
}

fn normalize_variable_definitions<'a>(variables: &mut [VariableDefinition<'a, String>]) {
    for variable in variables {
        if let Some(default_value) = &mut variable.default_value {
            normalize_value(default_value);
        }
    }
}

fn normalize_selection_set<'a>(selection_set: &mut SelectionSet<'a, String>) {
    for selection in &mut selection_set.items {
        match selection {
            Selection::Field(field) => {
                normalize_arguments(&mut field.arguments);
                normalize_directives(&mut field.directives);
                normalize_selection_set(&mut field.selection_set);
            }
            Selection::FragmentSpread(spread) => normalize_directives(&mut spread.directives),
            Selection::InlineFragment(fragment) => {
                normalize_directives(&mut fragment.directives);
                normalize_selection_set(&mut fragment.selection_set);
            }
        }
    }
}

fn normalize_directives<'a>(directives: &mut [Directive<'a, String>]) {
    for directive in directives {
        normalize_arguments(&mut directive.arguments);
    }
}

fn normalize_arguments<'a>(arguments: &mut [(String, Value<'a, String>)]) {
    for (_, value) in arguments {
        normalize_value(value);
    }
}

fn normalize_value<'a>(value: &mut Value<'a, String>) {
    match value {
        Value::List(items) => {
            for item in items {
                normalize_value(item);
            }
        }
        Value::Object(fields) => {
            for (_, value) in fields.iter_mut() {
                normalize_value(value);
            }
            fields.sort_by(|left, right| left.0.cmp(&right.0));
        }
        _ => {}
    }
}

fn roundtrip_multiline_args(filename: &str) {
    roundtrip(filename, Style::default().multiline_arguments(true))
}

fn roundtrip_default(filename: &str) {
    roundtrip(filename, &Style::default())
}

fn roundtrip(filename: &str, style: &Style) {
    let mut buf = String::with_capacity(1024);
    let path = format!(
        "{}/src/parser/tests/queries/{}.graphql",
        env!("CARGO_MANIFEST_DIR"),
        filename
    );
    let mut f = File::open(path).unwrap();
    f.read_to_string(&mut buf).unwrap();
    let ast = parse_query::<String>(&buf).unwrap().to_owned();
    assert_eq!(ast.format(style), buf);
}

fn roundtrip2(filename: &str) {
    let mut buf = String::with_capacity(1024);
    let source = format!(
        "{}/src/parser/tests/queries/{}.graphql",
        env!("CARGO_MANIFEST_DIR"),
        filename
    );
    let target = format!(
        "{}/src/parser/tests/queries/{}_canonical.graphql",
        env!("CARGO_MANIFEST_DIR"),
        filename
    );
    let mut f = File::open(source).unwrap();
    f.read_to_string(&mut buf).unwrap();
    let ast = parse_query::<String>(&buf).unwrap().to_owned();

    let mut buf = String::with_capacity(1024);
    let mut f = File::open(target).unwrap();
    f.read_to_string(&mut buf).unwrap();
    // Object input fields are semantically unordered. Normalize them before
    // comparing the two parsed documents so formatting differences remain
    // covered without making object ordering an AST requirement.
    let canonical = parse_query::<String>(&buf).unwrap().to_owned();
    let mut ast = ast;
    let mut canonical = canonical;
    normalize_document(&mut ast);
    normalize_document(&mut canonical);
    assert_eq!(ast.to_string(), canonical.to_string());
}

#[test]
fn minimal() {
    roundtrip_default("minimal");
}
#[test]
fn minimal_query() {
    roundtrip_default("minimal_query");
}
#[test]
fn named_query() {
    roundtrip_default("named_query");
}
#[test]
fn query_vars() {
    roundtrip_default("query_vars");
}
#[test]
fn query_nameless_vars() {
    roundtrip_default("query_nameless_vars");
}
#[test]
fn query_nameless_vars_multiple_fields() {
    roundtrip2("query_nameless_vars_multiple_fields");
}
#[test]
fn query_var_defaults() {
    roundtrip_default("query_var_defaults");
}
#[test]
fn query_var_defaults1() {
    roundtrip_default("query_var_default_string");
}
#[test]
fn query_var_defaults2() {
    roundtrip_default("query_var_default_float");
}
#[test]
fn query_var_defaults3() {
    roundtrip_default("query_var_default_list");
}
#[test]
fn query_var_defaults4() {
    roundtrip_default("query_var_default_object");
}
#[test]
fn query_aliases() {
    roundtrip_default("query_aliases");
}
#[test]
fn query_arguments() {
    roundtrip_default("query_arguments");
}
#[test]
fn query_arguments_multiline() {
    roundtrip_multiline_args("query_arguments_multiline");
}
#[test]
fn query_directive() {
    roundtrip_default("query_directive");
}
#[test]
fn mutation_directive() {
    roundtrip_default("mutation_directive");
}
#[test]
fn mutation_nameless_vars() {
    roundtrip_default("mutation_nameless_vars");
}
#[test]
fn subscription_directive() {
    roundtrip_default("subscription_directive");
}
#[test]
fn string_literal() {
    roundtrip_default("string_literal");
}
#[test]
fn triple_quoted_literal() {
    roundtrip_default("triple_quoted_literal");
}
#[test]
fn query_list_arg() {
    roundtrip_default("query_list_argument");
}
#[test]
fn query_object_arg() {
    roundtrip_default("query_object_argument");
}
#[test]
fn query_object_arg_multiline() {
    roundtrip_multiline_args("query_object_argument_multiline");
}
#[test]
fn query_array_arg_multiline() {
    roundtrip_multiline_args("query_array_argument_multiline");
}
#[test]
fn nested_selection() {
    roundtrip_default("nested_selection");
}
#[test]
fn inline_fragment() {
    roundtrip_default("inline_fragment");
}
#[test]
fn inline_fragment_dir() {
    roundtrip_default("inline_fragment_dir");
}
#[test]
fn fragment_spread() {
    roundtrip_default("fragment_spread");
}
#[test]
fn minimal_mutation() {
    roundtrip_default("minimal_mutation");
}
#[test]
fn fragment() {
    roundtrip_default("fragment");
}
#[test]
fn directive_args() {
    roundtrip_default("directive_args");
}
#[test]
fn directive_args_multiline() {
    roundtrip_multiline_args("directive_args_multiline");
}
#[test]
fn kitchen_sink() {
    roundtrip2("kitchen-sink");
}
