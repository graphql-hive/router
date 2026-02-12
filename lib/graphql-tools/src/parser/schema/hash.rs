use crate::{
    parser::{
        hash::hash_list_unordered,
        schema::{
            Definition, DirectiveLocation, EnumTypeExtension, InputObjectTypeExtension,
            InterfaceTypeExtension, ObjectTypeExtension, ScalarTypeExtension, TypeExtension,
            UnionTypeExtension,
        },
    },
    static_graphql::schema::{
        DirectiveDefinition, Document, EnumType, EnumValue, Field, InputObjectType, InputValue,
        InterfaceType, ObjectType, ScalarType, SchemaDefinition, TypeDefinition, UnionType,
    },
};
use std::hash::Hash;

impl Hash for Document {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "Document".hash(state);
        hash_list_unordered(self.definitions.iter()).hash(state);
    }
}

impl Hash for Definition<'static, String> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            Definition::SchemaDefinition(schema) => {
                "Definition::SchemaDefinition".hash(state);
                schema.hash(state);
            }
            Definition::TypeDefinition(type_def) => {
                "Definition::TypeDefinition".hash(state);
                type_def.hash(state);
            }
            Definition::TypeExtension(type_ext) => {
                "Definition::TypeExtension".hash(state);
                type_ext.hash(state);
            }
            Definition::DirectiveDefinition(directive_def) => {
                "Definition::DirectiveDefinition".hash(state);
                directive_def.hash(state);
            }
        }
    }
}

impl Hash for TypeDefinition {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            TypeDefinition::Scalar(scalar) => {
                "TypeDefinition::Scalar".hash(state);
                scalar.hash(state);
            }
            TypeDefinition::Object(object) => {
                "TypeDefinition::Object".hash(state);
                object.hash(state);
            }
            TypeDefinition::Interface(interface) => {
                "TypeDefinition::Interface".hash(state);
                interface.hash(state);
            }
            TypeDefinition::Union(union) => {
                "TypeDefinition::Union".hash(state);
                union.hash(state);
            }
            TypeDefinition::Enum(enum_) => {
                "TypeDefinition::Enum".hash(state);
                enum_.hash(state);
            }
            TypeDefinition::InputObject(input_object) => {
                "TypeDefinition::InputObject".hash(state);
                input_object.hash(state);
            }
        }
    }
}

impl Hash for TypeExtension<'static, String> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            TypeExtension::Scalar(scalar) => {
                "TypeExtension::Scalar".hash(state);
                scalar.hash(state);
            }
            TypeExtension::Object(object) => {
                "TypeExtension::Object".hash(state);
                object.hash(state);
            }
            TypeExtension::Interface(interface) => {
                "TypeExtension::Interface".hash(state);
                interface.hash(state);
            }
            TypeExtension::Union(union) => {
                "TypeExtension::Union".hash(state);
                union.hash(state);
            }
            TypeExtension::Enum(enum_) => {
                "TypeExtension::Enum".hash(state);
                enum_.hash(state);
            }
            TypeExtension::InputObject(input_object) => {
                "TypeExtension::InputObject".hash(state);
                input_object.hash(state);
            }
        }
    }
}

impl Hash for SchemaDefinition {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "SchemaDefinition".hash(state);
        self.query.hash(state);
        self.mutation.hash(state);
        self.subscription.hash(state);
        hash_list_unordered(self.directives.iter()).hash(state);
    }
}

impl Hash for ScalarType {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "ScalarType".hash(state);
        self.name.hash(state);
        self.description.hash(state);
        hash_list_unordered(self.directives.iter()).hash(state);
    }
}

impl Hash for ScalarTypeExtension<'static, String> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "ScalarTypeExtension".hash(state);
        self.name.hash(state);
        hash_list_unordered(self.directives.iter()).hash(state);
    }
}

impl Hash for ObjectType {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "ObjectType".hash(state);
        self.name.hash(state);
        self.description.hash(state);
        hash_list_unordered(self.implements_interfaces.iter()).hash(state);
        hash_list_unordered(self.directives.iter()).hash(state);
        hash_list_unordered(self.fields.iter()).hash(state);
    }
}

impl Hash for ObjectTypeExtension<'static, String> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "ObjectTypeExtension".hash(state);
        self.name.hash(state);
        hash_list_unordered(self.implements_interfaces.iter()).hash(state);
        hash_list_unordered(self.directives.iter()).hash(state);
        hash_list_unordered(self.fields.iter()).hash(state);
    }
}

impl Hash for Field {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "Field".hash(state);
        self.name.hash(state);
        self.description.hash(state);
        hash_list_unordered(self.arguments.iter()).hash(state);
        self.field_type.hash(state);
        hash_list_unordered(self.directives.iter()).hash(state);
    }
}

impl Hash for InputValue {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "InputValue".hash(state);
        self.name.hash(state);
        self.description.hash(state);
        self.value_type.hash(state);
        self.default_value.hash(state);
        hash_list_unordered(self.directives.iter()).hash(state);
    }
}

impl Hash for InterfaceType {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "InterfaceType".hash(state);
        self.name.hash(state);
        self.description.hash(state);
        hash_list_unordered(self.implements_interfaces.iter()).hash(state);
        hash_list_unordered(self.directives.iter()).hash(state);
        hash_list_unordered(self.fields.iter()).hash(state);
    }
}

impl Hash for InterfaceTypeExtension<'static, String> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "InterfaceTypeExtension".hash(state);
        self.name.hash(state);
        hash_list_unordered(self.implements_interfaces.iter()).hash(state);
        hash_list_unordered(self.directives.iter()).hash(state);
        hash_list_unordered(self.fields.iter()).hash(state);
    }
}

impl Hash for UnionType {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "UnionType".hash(state);
        self.name.hash(state);
        self.description.hash(state);
        hash_list_unordered(self.directives.iter()).hash(state);
        hash_list_unordered(self.types.iter()).hash(state);
    }
}

impl Hash for UnionTypeExtension<'static, String> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "UnionTypeExtension".hash(state);
        self.name.hash(state);
        hash_list_unordered(self.directives.iter()).hash(state);
        hash_list_unordered(self.types.iter()).hash(state);
    }
}

impl Hash for EnumType {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "EnumType".hash(state);
        self.name.hash(state);
        self.description.hash(state);
        hash_list_unordered(self.directives.iter()).hash(state);
        hash_list_unordered(self.values.iter()).hash(state);
    }
}

impl Hash for EnumValue {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "EnumValue".hash(state);
        self.name.hash(state);
        self.description.hash(state);
        hash_list_unordered(self.directives.iter()).hash(state);
    }
}

impl Hash for EnumTypeExtension<'static, String> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "EnumTypeExtension".hash(state);
        self.name.hash(state);
        hash_list_unordered(self.directives.iter()).hash(state);
        hash_list_unordered(self.values.iter()).hash(state);
    }
}

impl Hash for InputObjectType {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "InputObjectType".hash(state);
        self.name.hash(state);
        self.description.hash(state);
        hash_list_unordered(self.directives.iter()).hash(state);
        hash_list_unordered(self.fields.iter()).hash(state);
    }
}

impl Hash for InputObjectTypeExtension<'static, String> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "InputObjectTypeExtension".hash(state);
        self.name.hash(state);
        hash_list_unordered(self.directives.iter()).hash(state);
        hash_list_unordered(self.fields.iter()).hash(state);
    }
}

impl Hash for DirectiveLocation {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "DirectiveLocation".hash(state);
        self.as_str().hash(state);
    }
}

impl Hash for DirectiveDefinition {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "DirectiveDefinition".hash(state);
        self.name.hash(state);
        self.description.hash(state);
        hash_list_unordered(self.arguments.iter()).hash(state);
        self.repeatable.hash(state);
        hash_list_unordered(self.locations.iter()).hash(state);
    }
}

#[cfg(test)]
mod tests {
    use std::hash::{DefaultHasher, Hash, Hasher};

    use crate::parser::parse_schema;

    #[test]
    fn hashes_independent_from_position() {
        let schema_a = parse_schema(
            r#"
            type Query {
                field: String
            }
        "#,
        )
        .unwrap()
        .into_static();
        let schema_b = parse_schema(
            r#"
                type Query {
            field: String
            }
        "#,
        )
        .unwrap()
        .into_static();
        let hash_a = {
            let mut hasher = DefaultHasher::new();
            schema_a.hash(&mut hasher);
            hasher.finish()
        };
        let hash_b = {
            let mut hasher = DefaultHasher::new();
            schema_b.hash(&mut hasher);
            hasher.finish()
        };
        assert_eq!(hash_a, hash_b);
    }
    #[test]
    fn hashes_independent_from_definition_order() {
        let schema_a = parse_schema(
            r#"
            type Query {
                field: String
            }
            type Mutation {
                field: String
            }
        "#,
        )
        .unwrap()
        .into_static();
        let schema_b = parse_schema(
            r#"
            type Mutation {
                field: String
            }
            type Query {
                field: String
            }
        "#,
        )
        .unwrap()
        .into_static();
        let hash_a = {
            let mut hasher = DefaultHasher::new();
            schema_a.hash(&mut hasher);
            hasher.finish()
        };
        let hash_b = {
            let mut hasher = DefaultHasher::new();
            schema_b.hash(&mut hasher);
            hasher.finish()
        };
        assert_eq!(hash_a, hash_b);
    }
    #[test]
    fn hashes_independent_from_argument_order() {
        let schema_a = parse_schema(
            r#"
            directive @test(arg1: String, arg2: String) on FIELD
        "#,
        )
        .unwrap()
        .into_static();
        let schema_b = parse_schema(
            r#"
            directive @test(arg2: String, arg1: String) on FIELD
        "#,
        )
        .unwrap()
        .into_static();
        let hash_a = {
            let mut hasher = DefaultHasher::new();
            schema_a.hash(&mut hasher);
            hasher.finish()
        };
        let hash_b = {
            let mut hasher = DefaultHasher::new();
            schema_b.hash(&mut hasher);
            hasher.finish()
        };
        assert_eq!(hash_a, hash_b);
    }
    #[test]
    fn hashes_independent_from_directive_order() {
        let schema_a = parse_schema(
            r#"
            type Query {
                field: String @dir1 @dir2
    }        "#,
        )
        .unwrap()
        .into_static();
        let schema_b = parse_schema(
            r#"
            type Query {
                field: String @dir2 @dir1
            }
        "#,
        )
        .unwrap()
        .into_static();
        let hash_a = {
            let mut hasher = DefaultHasher::new();
            schema_a.hash(&mut hasher);
            hasher.finish()
        };
        let hash_b = {
            let mut hasher = DefaultHasher::new();
            schema_b.hash(&mut hasher);
            hasher.finish()
        };
        assert_eq!(hash_a, hash_b);
    }
    #[test]
    fn hashes_independent_from_interface_order() {
        let schema_a = parse_schema(
            r#"
            interface A {
                field: String
    }
            interface B {
                field: String
            }
            type Query implements A & B {
                field: String
            }
        "#,
        )
        .unwrap()
        .into_static();
        let schema_b = parse_schema(
            r#"
            interface B {
                field: String
            }
            interface A {
                field: String
            }
            type Query implements B & A {
                field: String
            }
        "#,
        )
        .unwrap()
        .into_static();
        let hash_a = {
            let mut hasher = DefaultHasher::new();
            schema_a.hash(&mut hasher);
            hasher.finish()
        };
        let hash_b = {
            let mut hasher = DefaultHasher::new();
            schema_b.hash(&mut hasher);
            hasher.finish()
        };
        assert_eq!(hash_a, hash_b);
    }
    #[test]
    fn hashes_independent_from_enum_value_order() {
        let schema_a = parse_schema(
            r#"
            enum MyEnum {
                A
                B
    }
        "#,
        )
        .unwrap()
        .into_static();
        let schema_b = parse_schema(
            r#"
            enum MyEnum {
                B
                A
            }
        "#,
        )
        .unwrap()
        .into_static();
        let hash_a = {
            let mut hasher = DefaultHasher::new();
            schema_a.hash(&mut hasher);
            hasher.finish()
        };
        let hash_b = {
            let mut hasher = DefaultHasher::new();
            schema_b.hash(&mut hasher);
            hasher.finish()
        };
        assert_eq!(hash_a, hash_b);
    }
    #[test]
    fn hashes_independent_from_union_member_order() {
        let schema_a = parse_schema(
            r#"
            union MyUnion = A | B
        "#,
        )
        .unwrap()
        .into_static();
        let schema_b = parse_schema(
            r#"
            union MyUnion = B | A
        "#,
        )
        .unwrap()
        .into_static();
        let hash_a = {
            let mut hasher = DefaultHasher::new();
            schema_a.hash(&mut hasher);
            hasher.finish()
        };
        let hash_b = {
            let mut hasher = DefaultHasher::new();
            schema_b.hash(&mut hasher);
            hasher.finish()
        };
        assert_eq!(hash_a, hash_b);
    }
    #[test]
    fn hashes_independent_from_schema_definition_operation_order() {
        let schema_a = parse_schema(
            r#"
            schema {
                query: Query
                mutation: Mutation
                subscription: Subscription
    }
        "#,
        )
        .unwrap()
        .into_static();
        let schema_b = parse_schema(
            r#"
            schema {
                subscription: Subscription
                mutation: Mutation
                query: Query
            }
        "#,
        )
        .unwrap()
        .into_static();
        let hash_a = {
            let mut hasher = DefaultHasher::new();
            schema_a.hash(&mut hasher);
            hasher.finish()
        };
        let hash_b = {
            let mut hasher = DefaultHasher::new();
            schema_b.hash(&mut hasher);
            hasher.finish()
        };
        assert_eq!(hash_a, hash_b);
    }
    #[test]
    fn hashes_independent_from_field_order() {
        let schema_a = parse_schema(
            r#"
            type Query {
                field1: String
                field2: String
            }
        "#,
        )
        .unwrap()
        .into_static();
        let schema_b = parse_schema(
            r#"
            type Query {
                field2: String
                field1: String
            }
        "#,
        )
        .unwrap()
        .into_static();
        let hash_a = {
            let mut hasher = DefaultHasher::new();
            schema_a.hash(&mut hasher);
            hasher.finish()
        };
        let hash_b = {
            let mut hasher = DefaultHasher::new();
            schema_b.hash(&mut hasher);
            hasher.finish()
        };
        assert_eq!(hash_a, hash_b);
    }
    #[test]
    fn hashes_independent_from_input_value_order() {
        let schema_a = parse_schema(
            r#"
            type Query {
                field(input: InputType): String
            }
            input InputType {
                arg1: String
                arg2: String
            }
        "#,
        )
        .unwrap()
        .into_static();
        let schema_b = parse_schema(
            r#"
            type Query {
                field(input: InputType): String
            }
            input InputType {
                arg2: String
                arg1: String
            }
        "#,
        )
        .unwrap()
        .into_static();
        let hash_a = {
            let mut hasher = DefaultHasher::new();
            schema_a.hash(&mut hasher);
            hasher.finish()
        };
        let hash_b = {
            let mut hasher = DefaultHasher::new();
            schema_b.hash(&mut hasher);
            hasher.finish()
        };
        assert_eq!(hash_a, hash_b);
    }
    #[test]
    fn hashes_independent_from_directive_definition_location_order() {
        let schema_a = parse_schema(
            r#"
            directive @test on FIELD | QUERY
        "#,
        )
        .unwrap()
        .into_static();
        let schema_b = parse_schema(
            r#"
            directive @test on QUERY | FIELD
        "#,
        )
        .unwrap()
        .into_static();
        let hash_a = {
            let mut hasher = DefaultHasher::new();
            schema_a.hash(&mut hasher);
            hasher.finish()
        };
        let hash_b = {
            let mut hasher = DefaultHasher::new();
            schema_b.hash(&mut hasher);
            hasher.finish()
        };
        assert_eq!(hash_a, hash_b);
    }
    #[test]
    fn hashes_different_for_different_schemas() {
        let schema_a = parse_schema(
            r#"
            type Query {
                field: String
            }
        "#,
        )
        .unwrap()
        .into_static();
        let schema_b = parse_schema(
            r#"
            type Query {
                field: Int
            }
        "#,
        )
        .unwrap()
        .into_static();
        let hash_a = {
            let mut hasher = DefaultHasher::new();
            schema_a.hash(&mut hasher);
            hasher.finish()
        };
        let hash_b = {
            let mut hasher = DefaultHasher::new();
            schema_b.hash(&mut hasher);
            hasher.finish()
        };
        assert_ne!(hash_a, hash_b);
    }

    #[test]
    fn hashes_different_for_empty_and_same_two() {
        let schema_a = parse_schema(
            r#"
            directive @tag repeatable on OBJECT
            type Query @tag @tag {
                f: String
            }
        "#,
        )
        .unwrap()
        .into_static();
        let schema_b = parse_schema(
            r#"
            directive @tag repeatable on OBJECT
            type Query {
                f: String
            }
        "#,
        )
        .unwrap()
        .into_static();
        let hash_a = {
            let mut hasher = DefaultHasher::new();
            schema_a.hash(&mut hasher);
            hasher.finish()
        };
        let hash_b = {
            let mut hasher = DefaultHasher::new();
            schema_b.hash(&mut hasher);
            hasher.finish()
        };
        assert_ne!(hash_a, hash_b);
    }

    #[test]
    fn hashes_differently_when_a_xor_a() {
        let schema_a = parse_schema(
            r#"
            directive @a repeatable on OBJECT
            directive @b repeatable on OBJECT
            directive @c repeatable on OBJECT
            directive @d repeatable on OBJECT

            type Query @a @a @b @c {
                f: String
            }
        "#,
        )
        .unwrap()
        .into_static();
        let schema_b = parse_schema(
            r#"
            directive @a repeatable on OBJECT
            directive @b repeatable on OBJECT
            directive @c repeatable on OBJECT
            directive @d repeatable on OBJECT

            type Query @b @c @d @d {
                f: String
            }
        "#,
        )
        .unwrap()
        .into_static();
        let hash_a = {
            let mut hasher = DefaultHasher::new();
            schema_a.hash(&mut hasher);
            hasher.finish()
        };
        let hash_b = {
            let mut hasher = DefaultHasher::new();
            schema_b.hash(&mut hasher);
            hasher.finish()
        };
        assert_ne!(hash_a, hash_b);
    }
}
