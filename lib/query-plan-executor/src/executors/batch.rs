use std::{collections::HashMap, hash::Hash};

use dataloader::{cached::Loader, BatchFn};
use graphql_parser::{query, Pos};
use graphql_tools::{
    ast::OperationDefinitionExtension,
    static_graphql::query::{Document, Value},
};
use query_planner::utils::parsing::parse_operation;
use serde_json::Map;

use crate::{executors::common::SubgraphExecutor, ExecutionResult};

struct BatchLoadFn<Executor> {
    subgraph_name: String,
    executor: Executor,
}

impl Hash for crate::ExecutionRequest {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.query.hash(state);
        if let Some(variables) = &self.variables {
            for (key, value) in variables {
                key.hash(state);
                value.hash(state);
            }
        }
    }
}

impl Eq for crate::ExecutionRequest {}

impl PartialEq for crate::ExecutionRequest {
    fn eq(&self, other: &Self) -> bool {
        self.query == other.query
            && self.variables == other.variables
            && self.operation_name == other.operation_name
            && self.extensions == other.extensions
    }
}

impl<Executor> BatchFn<crate::ExecutionRequest, crate::ExecutionResult> for BatchLoadFn<Executor>
    where Executor: SubgraphExecutor
{
    async fn load(
        &mut self,
        keys: &[crate::ExecutionRequest],
    ) -> HashMap<crate::ExecutionRequest, crate::ExecutionResult> {
        if keys.len() == 1 {
            let request = &keys[0];
            let result = self
                .executor
                .execute(&self.subgraph_name, request.clone())
                .await;
            let mut result_map: HashMap<crate::ExecutionRequest, crate::ExecutionResult> =
                HashMap::new();
            result_map.insert(request.clone(), result);
            result_map
        } else {
            let merged_request = merge_requests(keys);
            let result = self
                .executor
                .execute(&self.subgraph_name, merged_request.clone())
                .await;
            let splitted_results = split_result(&result, keys.len());
            let mut result_map: HashMap<crate::ExecutionRequest, crate::ExecutionResult> =
                HashMap::new();
            for (request, result) in keys.iter().zip(splitted_results) {
                result_map.insert(request.clone(), result);
            }
            result_map
        }
    }
}

fn merge_requests(requests: &[crate::ExecutionRequest]) -> crate::ExecutionRequest {
    let mut merged_variables = HashMap::new();
    let mut merged_variable_definitions: Vec<query::VariableDefinition<'static, String>> =
        Vec::new();
    let mut merged_selections = Vec::new();
    let mut merged_fragment_definitions = Vec::new();
    let mut merged_extensions = HashMap::new();

    let mut original_to_prefix_variable_names = HashMap::new();

    for (index, request) in requests.iter().enumerate() {
        let prefix = create_prefix(index);
        if let Some(variables) = &request.variables {
            for (key, _value) in variables {
                let prefixed_key = format!("{}{}", prefix, key);
                original_to_prefix_variable_names.insert(key.clone(), prefixed_key.clone());
            }
        }
        let (prefixed_document, prefixed_variables) =
            prefix_request(&prefix, request);
        for definition in prefixed_document.definitions {
            match definition {
                graphql_tools::static_graphql::query::Definition::Operation(op) => {
                    merged_selections.extend(op.selection_set().items.clone());
                    for variable_def in op.variable_definitions() {
                        merged_variable_definitions.push(variable_def.clone());
                    }
                }
                graphql_tools::static_graphql::query::Definition::Fragment(fragment) => {
                    merged_fragment_definitions.push(fragment);
                }
            }
        }
        if let Some(vars) = prefixed_variables {
            merged_variables.extend(vars);
        }
        if let Some(extensions) = &request.extensions {
            for (key, value) in extensions {
                merged_extensions.insert(key.clone(), value.clone());
            }
        }
    }

    let merged_op = graphql_tools::static_graphql::query::Query {
        name: None,
        variable_definitions: merged_variable_definitions,
        selection_set: graphql_tools::static_graphql::query::SelectionSet {
            items: merged_selections,
            span: (Pos::default(), Pos::default()), // Placeholder span, adjust as needed
        },
        directives: Vec::new(),
        position: Pos::default(), // Placeholder position, adjust as needed
    };

    let mut merged_defs = vec![
        graphql_tools::static_graphql::query::Definition::Operation(
            graphql_parser::query::OperationDefinition::Query(merged_op)
        ),
    ];
    if !merged_fragment_definitions.is_empty() {
        merged_defs.extend(
            merged_fragment_definitions
                .into_iter()
                .map(graphql_tools::static_graphql::query::Definition::Fragment),
        );
    }

    let merged_document = graphql_tools::static_graphql::query::Document {
        definitions: merged_defs,
    };

    crate::ExecutionRequest {
        query: merged_document.to_string(),
        variables: Some(merged_variables),
        extensions: Some(merged_extensions),
        operation_name: None,
    }
}

fn create_prefix(index: usize) -> String {
    format!("_v{}_", index)
}

fn prefix_request(
    prefix: &str,
    request: &crate::ExecutionRequest,
) -> (
    graphql_tools::static_graphql::query::Document,
    Option<HashMap<String, serde_json::Value>>,
) {
    let document = parse_operation(&request.query);
    let mut prefixed_document = alias_top_level_fields(prefix, &document);
    let mut renamed_variables: HashMap<String, String> = HashMap::new();
    let prefixed_variables: Option<HashMap<String, serde_json::Value>> =
        request.variables.as_ref().map(|vars| {
            vars.into_iter()
                .map(|(k, v)| {
                    let prefixed_key = format!("{}{}", prefix, k);
                    renamed_variables.insert(k.clone(), prefixed_key.clone());
                    (prefixed_key, v.clone())
                })
                .collect()
        });
    let mut prefix_doc_str = prefixed_document.to_string();
    for (variable_name, prefixed_variable_name) in renamed_variables {
        prefix_doc_str = prefix_doc_str.replace(
            &format!("${}", variable_name),
            &format!("${}", prefixed_variable_name),
        );
    }
    (parse_operation(&prefix_doc_str), prefixed_variables)
}

fn alias_top_level_fields(
    prefix: &str,
    document: &Document,
) -> graphql_tools::static_graphql::query::Document {
    let new_definitions: Vec<graphql_tools::static_graphql::query::Definition> = document
        .definitions
        .iter()
        .map(|def| match def {
            graphql_tools::static_graphql::query::Definition::Operation(op) => match op {
                graphql_parser::query::OperationDefinition::Query(query) => {
                    graphql_tools::static_graphql::query::Definition::Operation(
                        graphql_parser::query::OperationDefinition::Query(
                            graphql_tools::static_graphql::query::Query {
                                name: query.name.clone(),
                                variable_definitions: query.variable_definitions.iter().map(|vd| {
                                    graphql_tools::static_graphql::query::VariableDefinition {
                                        name: format!("{}{}", prefix, vd.name),
                                        var_type: vd.var_type.clone(),
                                        default_value: vd.default_value.clone(),
                                        position: vd.position.clone(),
                                    }
                                }).collect(),
                                directives: query.directives.clone(),
                                selection_set: graphql_tools::static_graphql::query::SelectionSet {
                                    span: query.selection_set.span.clone(),
                                    items: alias_fields_in_selection(
                                        prefix,
                                        &query.selection_set.items,
                                        document,
                                    ),
                                },
                                position: query.position.clone(),
                            },
                        ),
                    )
                }
                graphql_parser::query::OperationDefinition::Mutation(mutation) => {
                    graphql_tools::static_graphql::query::Definition::Operation(
                        graphql_parser::query::OperationDefinition::Mutation(
                            graphql_tools::static_graphql::query::Mutation {
                                name: mutation.name.clone(),
                                variable_definitions: mutation.variable_definitions.clone(),
                                directives: mutation.directives.clone(),
                                selection_set: graphql_tools::static_graphql::query::SelectionSet {
                                    span: mutation.selection_set.span.clone(),
                                    items: alias_fields_in_selection(
                                        prefix,
                                        &mutation.selection_set.items,
                                        document,
                                    ),
                                },
                                position: mutation.position.clone(),
                            },
                        ),
                    )
                }
                graphql_parser::query::OperationDefinition::Subscription(subscription) => {
                    graphql_tools::static_graphql::query::Definition::Operation(
                        graphql_parser::query::OperationDefinition::Subscription(
                            graphql_tools::static_graphql::query::Subscription {
                                name: subscription.name.clone(),
                                variable_definitions: subscription.variable_definitions.clone(),
                                directives: subscription.directives.clone(),
                                selection_set: graphql_tools::static_graphql::query::SelectionSet {
                                    span: subscription.selection_set.span.clone(),
                                    items: alias_fields_in_selection(
                                        prefix,
                                        &subscription.selection_set.items,
                                        document,
                                    ),
                                },
                                position: subscription.position.clone(),
                            },
                        ),
                    )
                }
                graphql_parser::query::OperationDefinition::SelectionSet(selection_set) => {
                    graphql_tools::static_graphql::query::Definition::Operation(
                        graphql_parser::query::OperationDefinition::SelectionSet(
                            graphql_tools::static_graphql::query::SelectionSet {
                                span: selection_set.span.clone(),
                                items: alias_fields_in_selection(
                                    prefix,
                                    &selection_set.items,
                                    document,
                                ),
                            },
                        ),
                    )
                }
            },
            graphql_tools::static_graphql::query::Definition::Fragment(fragment) => {
                graphql_tools::static_graphql::query::Definition::Fragment(
                    graphql_tools::static_graphql::query::FragmentDefinition {
                        name: format!("{}{}", prefix, fragment.name),
                        type_condition: fragment.type_condition.clone(),
                        directives: fragment.directives.clone(),
                        selection_set: fragment.selection_set.clone(),
                        position: fragment.position.clone(),
                    },
                )
            }
        })
        .collect();
    graphql_tools::static_graphql::query::Document {
        definitions: new_definitions,
    }
}

fn alias_fields_in_selection(
    prefix: &str,
    selections: &Vec<graphql_tools::static_graphql::query::Selection>,
    document: &Document,
) -> Vec<graphql_tools::static_graphql::query::Selection> {
    selections
        .iter()
        .map(|selection| match selection {
            graphql_tools::static_graphql::query::Selection::Field(field) => {
                graphql_tools::static_graphql::query::Selection::Field(alias_field(field, prefix))
            }
            graphql_tools::static_graphql::query::Selection::InlineFragment(inline_fragment) => {
                graphql_tools::static_graphql::query::Selection::InlineFragment(
                    alias_fields_in_inline_fragment(prefix, inline_fragment, document),
                )
            }
            graphql_tools::static_graphql::query::Selection::FragmentSpread(spread) => {
                let inline_fragment = inline_fragment_spread(spread, document);
                alias_fields_in_inline_fragment(prefix, &inline_fragment, document);
                graphql_tools::static_graphql::query::Selection::InlineFragment(inline_fragment)
            }
        })
        .collect()
}

fn alias_field(
    field: &graphql_tools::static_graphql::query::Field,
    prefix: &str,
) -> graphql_tools::static_graphql::query::Field {
    let alias = field.alias.as_ref().map_or_else(
        || format!("{}{}", prefix, field.name),
        |alias| format!("{}{}", prefix, alias),
    );
    graphql_tools::static_graphql::query::Field {
        alias: Some(alias),
        name: field.name.clone(),
        arguments: field.arguments.clone(),
        directives: field.directives.clone(),
        selection_set: field.selection_set.clone(),
        position: field.position.clone(),
    }
}

fn alias_fields_in_inline_fragment(
    prefix: &str,
    inline_fragment: &graphql_tools::static_graphql::query::InlineFragment,
    document: &Document,
) -> graphql_tools::static_graphql::query::InlineFragment {
    let selections = &inline_fragment.selection_set.items;
    let new_selections = alias_fields_in_selection(prefix, selections, document);
    graphql_tools::static_graphql::query::InlineFragment {
        type_condition: inline_fragment.type_condition.clone(),
        position: inline_fragment.position.clone(),
        directives: inline_fragment.directives.clone(),
        selection_set: graphql_tools::static_graphql::query::SelectionSet {
            span: inline_fragment.selection_set.span.clone(),
            items: new_selections,
        },
    }
}

fn inline_fragment_spread(
    spread: &graphql_tools::static_graphql::query::FragmentSpread,
    document: &Document,
) -> graphql_tools::static_graphql::query::InlineFragment {
    let fragment = document
        .definitions
        .iter()
        .find_map(|def| {
            if let graphql_tools::static_graphql::query::Definition::Fragment(fragment) = def {
                if fragment.name == spread.fragment_name {
                    Some(fragment)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .unwrap();
    graphql_tools::static_graphql::query::InlineFragment {
        type_condition: Some(fragment.type_condition.clone()),
        position: spread.position,
        directives: spread.directives.clone(),
        selection_set: fragment.selection_set.clone(),
    }
}

fn split_result(result: &crate::ExecutionResult, num_results: usize) -> Vec<crate::ExecutionResult> {
    let mut split_results: Vec<ExecutionResult> = Vec::with_capacity(num_results);

    for i in 0..num_results {
        split_results.push(ExecutionResult {
            data: Some(serde_json::Value::Object(Map::new())),
            errors: result.errors.clone(),
            extensions: result.extensions.clone(),
        });
    }

    if let Some(serde_json::Value::Object(data)) = &result.data {
        for (key, value) in data.iter() {
            let (index, original_key) = parse_key(key);
            let result = split_results
                .get_mut(index).unwrap();
            let result_data= result.data.as_mut().unwrap();
            let data = result_data.as_object_mut().unwrap();
            data.insert(original_key, value.clone());
        }
    };

    split_results
}

// Parses a key that has been prefixed with an index, e.g., "_v0_fieldName".
// Original key is fieldName, index is 0.
fn parse_key(prefixed_key: &str) -> (usize, String) {
    let parts: Vec<&str> = prefixed_key.split('_').collect();
    if parts.len() < 3 {
        panic!("Invalid prefixed key format: {}", prefixed_key);
    }
    //Remove the "_v" prefix and parse the index
    let index = parts[1].strip_prefix("v").expect("Invalid prefix format").parse::<usize>().expect("Failed to parse index");
    let original_key = parts[2..].join("_"); // Join the rest as the original key
    (index, original_key)
}

pub struct BatchExecutor<Executor> 
    where Executor: SubgraphExecutor
{
    subgraph_loader_map: HashMap<
        String,
        dataloader::cached::Loader<
            crate::ExecutionRequest,
            crate::ExecutionResult,
            BatchLoadFn<Executor>,
            HashMap<crate::ExecutionRequest, crate::ExecutionResult>,
        >,
    >,
}

impl<Executor> BatchExecutor<Executor> 
    where Executor: SubgraphExecutor + Send + Sync
{
    pub fn new(subgraph_executor_map: HashMap<String, Executor>) -> Self {
        let subgraph_loader_map = subgraph_executor_map
            .into_iter()
            .map(|(name, executor)| {
                let loader = Loader::new(BatchLoadFn {
                    subgraph_name: name.clone(),
                    executor,
                });
                (name, loader)
            })
            .collect();

        BatchExecutor {
            subgraph_loader_map,
        }
    }
}

#[async_trait::async_trait]
impl<Executor> SubgraphExecutor for BatchExecutor<Executor> 
    where Executor: SubgraphExecutor + Send + Sync
{
    async fn execute(
        &self,
        subgraph_name: &str,
        execution_request: crate::ExecutionRequest,
    ) -> crate::ExecutionResult {
        match self.subgraph_loader_map.get(subgraph_name) {
            Some(loader) => {
                loader.load(execution_request).await
            }
            None => ExecutionResult::from_error_message(format!(
                "Subgraph {} not found in loader map",
                subgraph_name
            )),
        }
    }
}
