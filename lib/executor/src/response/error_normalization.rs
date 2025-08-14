use std::collections::{HashMap, VecDeque};

use query_planner::planner::plan_nodes::FlattenNodePathSegment;

use crate::response::graphql_error::{GraphQLError, GraphQLErrorPathSegment};

/**
 * Map `[_entities, 0, field]` to `["actual_field", "field"]`;
 *
 * For example if the error location is `[_entities, 0, name]`
 * and flatten path is ['product', 'reviews', 0, 'author']
 * it becomes `["product", "reviews", "0", "author", "name"]`.
 */
pub fn normalize_errors_for_representations(
    subgraph_name: &str,
    normalized_path: &[FlattenNodePathSegment],
    representation_hashes: &[u64],
    hashes_to_indexes: &HashMap<u64, Vec<VecDeque<usize>>>,
    errors: &Vec<GraphQLError>,
) -> Vec<GraphQLError> {
    let mut new_errors: Vec<GraphQLError> = Vec::new();
    'error_loop: for error in errors {
        if let Some(path_in_error) = &error.path {
            if let Some(GraphQLErrorPathSegment::String(first_path)) = path_in_error.first() {
                if first_path == "_entities" {
                    if let Some(GraphQLErrorPathSegment::Index(entity_index)) = path_in_error.get(1)
                    {
                        if let Some(representation_hash) = representation_hashes.get(*entity_index)
                        {
                            if let Some(indexes_in_paths) =
                                hashes_to_indexes.get(representation_hash)
                            {
                                for indexes_in_path in indexes_in_paths {
                                    let mut indexes_in_path = indexes_in_path.clone();
                                    let mut real_path: Vec<GraphQLErrorPathSegment> =
                                        Vec::with_capacity(
                                            normalized_path.len() + path_in_error.len() - 2,
                                        );
                                    for segment in normalized_path {
                                        match segment {
                                            FlattenNodePathSegment::Field(field_name) => {
                                                real_path.push(GraphQLErrorPathSegment::String(
                                                    field_name.to_string(),
                                                ));
                                            }
                                            FlattenNodePathSegment::List => {
                                                if let Some(index_in_path) =
                                                    indexes_in_path.pop_front()
                                                {
                                                    real_path.push(GraphQLErrorPathSegment::Index(
                                                        index_in_path,
                                                    ));
                                                }
                                            }
                                            FlattenNodePathSegment::Cast(_type_condition) => {
                                                // Cast segments are not included in the error path
                                                continue;
                                            }
                                        }
                                    }
                                    if !indexes_in_path.is_empty() {
                                        // If there are still indexes left, we need to traverse them
                                        while let Some(index) = indexes_in_path.pop_front() {
                                            real_path.push(GraphQLErrorPathSegment::Index(index));
                                        }
                                    }
                                    real_path.extend_from_slice(&path_in_error[2..]);
                                    let mut new_error = error.clone();
                                    if !real_path.is_empty() {
                                        new_error.path = Some(real_path);
                                    }
                                    new_error =
                                        add_subgraph_info_to_error(new_error, subgraph_name);
                                    new_errors.push(new_error);
                                }
                                continue 'error_loop;
                            }
                        }
                    }
                }
            }
        }
        // Use the path without indexes in case of unlocated error
        let mut real_path: Vec<GraphQLErrorPathSegment> = Vec::with_capacity(normalized_path.len());
        for segment in normalized_path {
            match segment {
                FlattenNodePathSegment::Field(field_name) => {
                    real_path.push(GraphQLErrorPathSegment::String(field_name.to_string()));
                }
                FlattenNodePathSegment::List => {
                    break;
                }
                FlattenNodePathSegment::Cast(_type_condition) => {
                    // Cast segments are not included in the error path
                    continue;
                }
            }
        }
        let mut new_error = error.clone();
        if !real_path.is_empty() {
            new_error.path = Some(real_path);
        }
        new_error = add_subgraph_info_to_error(new_error, subgraph_name);

        new_errors.push(new_error);
    }

    new_errors
}

pub fn add_subgraph_info_to_error(mut error: GraphQLError, subgraph_name: &str) -> GraphQLError {
    let mut extensions = error.extensions.unwrap_or_default();
    if !extensions.contains_key("serviceName") {
        extensions.insert("serviceName".to_string(), subgraph_name.into());
    }
    if !extensions.contains_key("code") {
        extensions.insert("code".to_string(), "DOWNSTREAM_SERVICE_ERROR".into());
    }
    error.extensions = Some(extensions);
    error
}

#[test]
fn test_normalize_errors_for_representations() {
    // "products", "@", "reviews", "@", "author"
    let normalized_path = vec![
        FlattenNodePathSegment::Field("products".into()),
        FlattenNodePathSegment::List,
        FlattenNodePathSegment::Field("reviews".into()),
        FlattenNodePathSegment::List,
        FlattenNodePathSegment::Field("author".into()),
    ];
    let mut indexes_in_paths: HashMap<u64, Vec<VecDeque<usize>>> = HashMap::new();
    indexes_in_paths.insert(0, vec![VecDeque::from(vec![0, 0])]);
    indexes_in_paths.insert(1, vec![VecDeque::from(vec![0, 1])]);
    indexes_in_paths.insert(2, vec![VecDeque::from(vec![1, 1])]);
    indexes_in_paths.insert(3, vec![VecDeque::from(vec![1, 2])]);
    let representation_hashes: Vec<u64> = vec![0, 1, 2, 3];
    let errors: Vec<GraphQLError> = vec![
        GraphQLError {
            message: "Error 1".to_string(),
            locations: None,
            path: Some(vec![
                GraphQLErrorPathSegment::String("_entities".to_string()),
                GraphQLErrorPathSegment::Index(3),
                GraphQLErrorPathSegment::String("name".to_string()),
            ]),
            extensions: None,
        },
        GraphQLError {
            message: "Error 2".to_string(),
            locations: None,
            path: Some(vec![
                GraphQLErrorPathSegment::String("_entities".to_string()),
                GraphQLErrorPathSegment::Index(2),
                GraphQLErrorPathSegment::String("age".to_string()),
            ]),
            extensions: None,
        },
    ];
    let normalized_errors = normalize_errors_for_representations(
        "products",
        &normalized_path,
        &representation_hashes,
        &indexes_in_paths,
        &errors,
    );
    println!("{:?}", normalized_errors);
    assert_eq!(normalized_errors.len(), 2);
    assert_eq!(
        normalized_errors[0].path,
        Some(vec![
            GraphQLErrorPathSegment::String("products".to_string()),
            GraphQLErrorPathSegment::Index(1),
            GraphQLErrorPathSegment::String("reviews".to_string()),
            GraphQLErrorPathSegment::Index(2),
            GraphQLErrorPathSegment::String("author".to_string()),
            GraphQLErrorPathSegment::String("name".to_string()),
        ])
    );
    assert_eq!(
        normalized_errors[1].path,
        Some(vec![
            GraphQLErrorPathSegment::String("products".to_string()),
            GraphQLErrorPathSegment::Index(1),
            GraphQLErrorPathSegment::String("reviews".to_string()),
            GraphQLErrorPathSegment::Index(1),
            GraphQLErrorPathSegment::String("author".to_string()),
            GraphQLErrorPathSegment::String("age".to_string()),
        ])
    );
}
