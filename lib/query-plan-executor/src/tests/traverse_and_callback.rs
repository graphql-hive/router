use query_planner::planner::plan_nodes::FlattenNodePathSegment;
use serde_json::json;

use crate::{schema_metadata::SchemaMetadata, traverse_and_callback};

#[test]
fn array_cast_test() {
    let path = [
        FlattenNodePathSegment::Field("magazine".into()),
        FlattenNodePathSegment::List,
        FlattenNodePathSegment::Cast("Magazine".into()),
    ];
    let mut data = json!({
      "book": [
        {
          "id": "p3",
          "__typename": "Book",
          "sku": "sku-3",
          "dimensions": {
            "weight": 0.6,
            "size": "small"
          }
        }
      ],
      "magazine": [
        {
          "id": "p4",
          "__typename": "Magazine",
          "sku": "sku-4",
          "dimensions": {
            "weight": 0.3,
            "size": "small"
          }
        }
      ]
    });

    let mut result = vec![];
    traverse_and_callback(&mut data, &path, &SchemaMetadata::default(), &mut |value| {
        result.push(value);
    });
    let expected = json!([{
        "id": "p4",
        "__typename": "Magazine",
        "sku": "sku-4",
        "dimensions": {
            "weight": 0.3,
            "size": "small"
        }
    }]);

    assert_eq!(
        serde_json::to_string_pretty(&result).unwrap_or_default(),
        serde_json::to_string_pretty(&expected).unwrap_or_default()
    );
}

#[test]
fn simple_field_access() {
    let path = [FlattenNodePathSegment::Field("a".into())];
    let mut data = json!({"a": 1, "b": 2});

    let mut result = vec![];
    traverse_and_callback(&mut data, &path, &SchemaMetadata::default(), &mut |value| {
        result.push(value);
    });
    assert_eq!(
        serde_json::to_string_pretty(&result).unwrap_or_default(),
        serde_json::to_string_pretty(&json!([1])).unwrap_or_default()
    );
}

#[test]
fn nested_field_access() {
    let path = [
        FlattenNodePathSegment::Field("a".into()),
        FlattenNodePathSegment::Field("b".into()),
    ];
    let mut data = json!({"a": {"b": 3}});

    let mut result = vec![];
    traverse_and_callback(&mut data, &path, &SchemaMetadata::default(), &mut |value| {
        result.push(value);
    });
    assert_eq!(
        serde_json::to_string_pretty(&result).unwrap_or_default(),
        serde_json::to_string_pretty(&json!([3])).unwrap_or_default()
    );
}

#[test]
fn simple_list_access() {
    let path = [
        FlattenNodePathSegment::Field("a".into()),
        FlattenNodePathSegment::List,
    ];
    let mut data = json!({"a": [1, 2, 3]});

    let mut result = vec![];
    traverse_and_callback(&mut data, &path, &SchemaMetadata::default(), &mut |value| {
        result.push(value);
    });
    assert_eq!(
        serde_json::to_string_pretty(&result).unwrap_or_default(),
        serde_json::to_string_pretty(&json!([1, 2, 3])).unwrap_or_default()
    );
}

#[test]
fn field_access_in_list() {
    let path = [
        FlattenNodePathSegment::Field("a".into()),
        FlattenNodePathSegment::List,
        FlattenNodePathSegment::Field("b".into()),
    ];
    let mut data = json!({"a": [{"b": 1}, {"b": 2}]});

    let mut result = vec![];
    traverse_and_callback(&mut data, &path, &SchemaMetadata::default(), &mut |value| {
        result.push(value);
    });
    assert_eq!(
        serde_json::to_string_pretty(&result).unwrap_or_default(),
        serde_json::to_string_pretty(&json!([1, 2])).unwrap_or_default()
    );
}

#[test]
fn cast_in_list_with_field_access() {
    let path = [
        FlattenNodePathSegment::Field("a".into()),
        FlattenNodePathSegment::List,
        FlattenNodePathSegment::Cast("TypeA".into()),
        FlattenNodePathSegment::Field("b".into()),
    ];
    let mut data = json!({"a": [
        {"__typename": "TypeA", "b": 1},
        {"__typename": "TypeB", "b": 2},
        {"__typename": "TypeA", "b": 3}
    ]});

    let mut result = vec![];
    traverse_and_callback(&mut data, &path, &SchemaMetadata::default(), &mut |value| {
        result.push(value);
    });
    assert_eq!(
        serde_json::to_string_pretty(&result).unwrap_or_default(),
        serde_json::to_string_pretty(&json!([1, 3])).unwrap_or_default()
    );
}

#[test]
fn filter_list_by_cast() {
    let path = [
        FlattenNodePathSegment::Field("media".into()),
        FlattenNodePathSegment::List,
        FlattenNodePathSegment::Cast("Movie".into()),
    ];
    let mut data = json!({
        "media": [
          {
            "__typename": "Book",
            "title": "Book 1",
            "id": "m1"
          },
          {
            "__typename": "Movie",
            "id": "m2"
          }
        ]
    });

    let mut result = vec![];
    traverse_and_callback(&mut data, &path, &SchemaMetadata::default(), &mut |value| {
        result.push(value);
    });
    let expected = json!([
        {
            "__typename": "Movie",
            "id": "m2"
        }
    ]);
    assert_eq!(
        serde_json::to_string_pretty(&result).unwrap_or_default(),
        serde_json::to_string_pretty(&expected).unwrap_or_default()
    );
}

#[test]
fn invalid_field() {
    let path = [FlattenNodePathSegment::Field("c".into())];
    let mut data = json!({"a": 1});

    let mut result = vec![];
    traverse_and_callback(&mut data, &path, &SchemaMetadata::default(), &mut |value| {
        result.push(value);
    });
    assert_eq!(
        serde_json::to_string_pretty(&result).unwrap_or_default(),
        "[]"
    );
}

#[test]
fn invalid_nested_field() {
    let path = [
        FlattenNodePathSegment::Field("a".into()),
        FlattenNodePathSegment::Field("c".into()),
    ];
    let mut data = json!({"a": {"b": 1}});

    let mut result = vec![];
    traverse_and_callback(&mut data, &path, &SchemaMetadata::default(), &mut |value| {
        result.push(value);
    });
    assert_eq!(
        serde_json::to_string_pretty(&result).unwrap_or_default(),
        "[]"
    );
}

#[test]
fn initial_data_is_array() {
    let path = [FlattenNodePathSegment::List];
    let mut data = json!([1, 2, 3]);

    let mut result = vec![];
    traverse_and_callback(&mut data, &path, &SchemaMetadata::default(), &mut |value| {
        result.push(value);
    });
    assert_eq!(
        serde_json::to_string_pretty(&result).unwrap_or_default(),
        serde_json::to_string_pretty(&json!([1, 2, 3])).unwrap_or_default()
    );
}

#[test]
fn initial_data_is_array_with_field_access() {
    let path = [
        FlattenNodePathSegment::List,
        FlattenNodePathSegment::Field("a".into()),
    ];
    let mut data = json!([{"a": 1}, {"a": 2}]);

    let mut result = vec![];
    traverse_and_callback(&mut data, &path, &SchemaMetadata::default(), &mut |value| {
        result.push(value);
    });
    assert_eq!(
        serde_json::to_string_pretty(&result).unwrap_or_default(),
        serde_json::to_string_pretty(&json!([1, 2])).unwrap_or_default()
    );
}

#[test]
fn cast_on_object_without_typename() {
    let path = [
        FlattenNodePathSegment::Cast("MyType".into()),
        FlattenNodePathSegment::Field("a".into()),
    ];
    let mut data = json!({"a": 1});

    let mut result = vec![];
    traverse_and_callback(&mut data, &path, &SchemaMetadata::default(), &mut |value| {
        result.push(value);
    });
    assert_eq!(
        serde_json::to_string_pretty(&result).unwrap_or_default(),
        serde_json::to_string_pretty(&json!([1])).unwrap_or_default()
    );
}

#[test]
fn no_match_on_cast() {
    let path = [
        FlattenNodePathSegment::Field("a".into()),
        FlattenNodePathSegment::List,
        FlattenNodePathSegment::Cast("TypeC".into()),
    ];
    let mut data = json!({"a": [
        {"__typename": "TypeA", "b": 1},
        {"__typename": "TypeB", "b": 2}
    ]});

    let mut result = vec![];
    traverse_and_callback(&mut data, &path, &SchemaMetadata::default(), &mut |value| {
        result.push(value);
    });
    assert_eq!(
        serde_json::to_string_pretty(&result).unwrap_or_default(),
        "[]"
    );
}
