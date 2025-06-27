use std::collections::VecDeque;

use query_planner::planner::plan_nodes::FlattenNodePathSegment;
use serde_json::{Number, Value};

use crate::GraphQLError;

/**
 * Map `[_entities, 0, field]` to `["actual_field", "field"]`;
 *
 * For example if the error location is `[_entities, 0, name]`
 * and flatten path is ['product', 'reviews', 0, 'author']
 * it becomes `["product", "reviews", "0", "author", "name"]`.
 */
pub fn normalize_errors_for_representations(
    indexes_in_paths: &mut [VecDeque<usize>],
    normalized_path: &[FlattenNodePathSegment],
    errors: Vec<GraphQLError>,
) -> Vec<GraphQLError> {
    errors
        .into_iter()
        .map(|mut error| {
            if let Some(path_in_error) = &error.path {
                if let Some(entity_index) = path_in_error.get(1).and_then(|v| v.as_i64()) {
                    if let Some(indexes_in_path) = indexes_in_paths.get_mut(entity_index as usize) {
                        let mut real_path: Vec<Value> =
                            Vec::with_capacity(normalized_path.len() + path_in_error.len() - 2);
                        for segment in normalized_path {
                            match segment {
                                FlattenNodePathSegment::Field(field_name) => {
                                    real_path.push(Value::String(field_name.to_string()));
                                }
                                FlattenNodePathSegment::List => {
                                    if let Some(index_in_path) = indexes_in_path.pop_front() {
                                        real_path.push(Value::Number(Number::from(index_in_path)));
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
                                real_path.push(Value::Number(Number::from(index)));
                            }
                        }
                        real_path.extend_from_slice(&path_in_error[2..]);
                        if !real_path.is_empty() {
                            error.path = Some(real_path);
                        }
                        return error;
                    }
                }
            }
            // Use the path without indexes in case of unlocated error
            let mut real_path: Vec<Value> = Vec::with_capacity(normalized_path.len());
            for segment in normalized_path {
                match segment {
                    FlattenNodePathSegment::Field(field_name) => {
                        real_path.push(Value::String(field_name.to_string()));
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
            if !real_path.is_empty() {
                error.path = Some(real_path);
            }
            error
        })
        .collect()
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
    let indexes_in_paths = vec![
        VecDeque::from(vec![0, 0]),
        VecDeque::from(vec![0, 1]),
        VecDeque::from(vec![1, 1]),
        VecDeque::from(vec![1, 2]),
    ];
    let errors: Vec<GraphQLError> = vec![
        GraphQLError {
            message: "Error 1".to_string(),
            locations: None,
            path: Some(vec![
                Value::String("_entities".to_string()),
                Value::Number(Number::from(3)),
                Value::String("name".to_string()),
            ]),
            extensions: None,
        },
        GraphQLError {
            message: "Error 2".to_string(),
            locations: None,
            path: Some(vec![
                Value::String("_entities".to_string()),
                Value::Number(Number::from(2)),
                Value::String("age".to_string()),
            ]),
            extensions: None,
        },
    ];
    let normalized_errors = normalize_errors_for_representations(
        &mut indexes_in_paths.clone(),
        &normalized_path,
        errors,
    );
    assert_eq!(normalized_errors.len(), 2);
    assert_eq!(
        normalized_errors[0].path,
        Some(vec![
            Value::String("products".to_string()),
            Value::Number(Number::from(1)),
            Value::String("reviews".to_string()),
            Value::Number(Number::from(2)),
            Value::String("author".to_string()),
            Value::String("name".to_string()),
        ])
    );
    assert_eq!(
        normalized_errors[1].path,
        Some(vec![
            Value::String("products".to_string()),
            Value::Number(Number::from(1)),
            Value::String("reviews".to_string()),
            Value::Number(Number::from(1)),
            Value::String("author".to_string()),
            Value::String("age".to_string()),
        ])
    );
}
