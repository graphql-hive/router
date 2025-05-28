use std::collections::HashMap;

use graphql_parser::Pos;

pub fn get_specified_scalars(
) -> HashMap<String, graphql_parser::schema::ScalarType<'static, String>> {
    let int_type = graphql_parser::schema::ScalarType {
        name: "Int".to_string(),
        description: Some("The `Int` scalar type represents non-fractional signed whole numeric values. Int can represent values between -(2^31) and 2^31 - 1.".to_string()),
        directives: vec![],
        position: Pos::default(),
    };

    let float_type =graphql_parser::schema::ScalarType {
        name: "Float".to_string(),
        description: Some("The `Float` scalar type represents signed double-precision fractional values as specified by [IEEE 754](https://en.wikipedia.org/wiki/IEEE_floating_point).".to_string()),
        directives: vec![],
        position: Pos::default(),
    };
    let string_type =graphql_parser::schema::ScalarType {
        name: "String".to_string(),
        description: Some("The `String` scalar type represents textual data, represented as UTF-8 character sequences. The String type is most often used by GraphQL to represent free-form human-readable text.".to_string()),
        directives: vec![],
        position: Pos::default(),
    };
    let boolean_type = graphql_parser::schema::ScalarType {
        name: "Boolean".to_string(),
        description: Some("The `Boolean` scalar type represents `true` or `false`.".to_string()),
        directives: vec![],
        position: Pos::default(),
    };
    let id_type =graphql_parser::schema::ScalarType {
        name: "ID".to_string(),
        description: Some("The `ID` scalar type represents a unique identifier, often used to refetch an object or as key for a cache. The ID type appears in a JSON response as a String; however, it is not intended to be human-readable.".to_string()),
        directives: vec![],
        position: Pos::default(),
    };
    HashMap::from([
        ("Int".to_string(), int_type),
        ("Float".to_string(), float_type),
        ("String".to_string(), string_type),
        ("Boolean".to_string(), boolean_type),
        ("ID".to_string(), id_type),
    ])
}

pub fn get_specified_directives(
) -> HashMap<String, graphql_parser::schema::DirectiveDefinition<'static, String>> {
    let include_directive = graphql_parser::schema::DirectiveDefinition {
        name: "include".to_string(),
        description: Some("Directs the executor to include this field or fragment only when the `if` argument is true.".to_string()),
        locations: vec![graphql_parser::schema::DirectiveLocation::Field, graphql_parser::schema::DirectiveLocation::FragmentSpread, graphql_parser::schema::DirectiveLocation::InlineFragment],
        arguments: vec![
            graphql_parser::schema::InputValue {
            name: "if".to_string(),
            description: Some("Included when true.".to_string()),
            value_type: graphql_parser::schema::Type::NamedType("Boolean".to_string()),
            default_value: None,
            position: Pos::default(),
            directives: vec![],
        }],
        position: Pos::default(),
        repeatable: false,
    };
    let skip_directive = graphql_parser::schema::DirectiveDefinition {
        name: "skip".to_string(),
        description: Some(
            "Directs the executor to skip this field or fragment when the `if` argument is true."
                .to_string(),
        ),
        locations: vec![
            graphql_parser::schema::DirectiveLocation::Field,
            graphql_parser::schema::DirectiveLocation::FragmentSpread,
            graphql_parser::schema::DirectiveLocation::InlineFragment,
        ],
        arguments: vec![graphql_parser::schema::InputValue {
            name: "if".to_string(),
            description: Some("Skipped when true.".to_string()),
            value_type: graphql_parser::schema::Type::NamedType("Boolean".to_string()),
            default_value: None,
            position: Pos::default(),
            directives: vec![],
        }],
        position: Pos::default(),
        repeatable: false,
    };
    let default_deprecation_reason = "No longer supported";
    let deprecated_directive = graphql_parser::schema::DirectiveDefinition {
        name: "deprecated".to_string(),
        description: Some("Marks an element of a GraphQL schema as no longer supported.".to_string()),
        locations: vec![graphql_parser::schema::DirectiveLocation::FieldDefinition, graphql_parser::schema::DirectiveLocation::EnumValue],
        arguments: vec![
            graphql_parser::schema::InputValue {
                name: "reason".to_string(),
                description: Some("Explains why this element was deprecated, usually also including a suggestion for how to access supported similar data.".to_string()),
                value_type: graphql_parser::schema::Type::NamedType("String".to_string()),
                default_value: Some(
                    graphql_parser::schema::Value::String(default_deprecation_reason.to_string())
                ),
                position: Pos::default(),
                directives: vec![],
            }],
        position: Pos::default(),
        repeatable: false,
    };
    let specified_by_directive = graphql_parser::schema::DirectiveDefinition {
        name: "specifiedBy".to_string(),
        description: Some("Exposes a URL that specifies the behavior of this scalar.".to_string()),
        locations: vec![graphql_parser::schema::DirectiveLocation::Scalar],
        arguments: vec![graphql_parser::schema::InputValue {
            name: "url".to_string(),
            description: Some("The URL that specifies the behavior of this scalar.".to_string()),
            value_type: graphql_parser::schema::Type::NamedType("String".to_string()),
            default_value: None,
            position: Pos::default(),
            directives: vec![],
        }],
        position: Pos::default(),
        repeatable: false,
    };
    let one_of_directive = graphql_parser::schema::DirectiveDefinition {
        name: "oneOf".to_string(),
        description: Some(
            "Indicates exactly one field must be supplied and this field must not be `null`."
                .to_string(),
        ),
        locations: vec![graphql_parser::schema::DirectiveLocation::FieldDefinition],
        arguments: vec![],
        position: Pos::default(),
        repeatable: false,
    };
    HashMap::from([
        ("include".to_string(), include_directive),
        ("skip".to_string(), skip_directive),
        ("deprecated".to_string(), deprecated_directive),
        ("specifiedBy".to_string(), specified_by_directive),
        ("oneOf".to_string(), one_of_directive),
    ])
}
