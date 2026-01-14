use crate::static_graphql::schema::{
    Definition, DirectiveDefinition, Document, EnumType, EnumValue, Field, InputObjectType,
    InputValue, InterfaceType, ObjectType, ScalarType, SchemaDefinition, TypeDefinition, UnionType,
};

/// A trait for implenenting a visitor for GraphQL schema definition.
pub trait SchemaVisitor<T = ()> {
    fn visit_schema_document(&self, document: &Document, _visitor_context: &mut T) {
        self.enter_document(document, _visitor_context);

        for definition in &document.definitions {
            match definition {
                Definition::SchemaDefinition(schema_definition) => {
                    self.enter_schema_definition(schema_definition, _visitor_context);
                    self.leave_schema_definition(schema_definition, _visitor_context);
                }
                Definition::TypeDefinition(type_definition) => {
                    self.enter_type_definition(type_definition, _visitor_context);

                    match type_definition {
                        TypeDefinition::Object(object) => {
                            self.enter_object_type(object, _visitor_context);

                            for field in &object.fields {
                                self.enter_object_type_field(field, object, _visitor_context);
                                // TODO: More advanced setup for fields: arguments, lists, null/non-null, directives
                                self.leave_object_type_field(field, object, _visitor_context);
                            }

                            self.leave_object_type(object, _visitor_context);
                        }
                        TypeDefinition::Scalar(scalar) => {
                            self.enter_scalar_type(scalar, _visitor_context);
                            self.leave_scalar_type(scalar, _visitor_context);
                        }
                        TypeDefinition::Enum(enum_) => {
                            self.enter_enum_type(enum_, _visitor_context);

                            for value in &enum_.values {
                                self.enter_enum_value(value, enum_, _visitor_context);
                                self.leave_enum_value(value, enum_, _visitor_context);
                            }

                            self.leave_enum_type(enum_, _visitor_context);
                        }
                        TypeDefinition::Union(union) => {
                            self.enter_union_type(union, _visitor_context);
                            self.leave_union_type(union, _visitor_context);
                        }
                        TypeDefinition::InputObject(input_object) => {
                            self.enter_input_object_type(input_object, _visitor_context);

                            for field in &input_object.fields {
                                self.enter_input_object_type_field(
                                    field,
                                    input_object,
                                    _visitor_context,
                                );
                                self.leave_input_object_type_field(
                                    field,
                                    input_object,
                                    _visitor_context,
                                );
                            }

                            self.leave_input_object_type(input_object, _visitor_context);
                        }
                        TypeDefinition::Interface(interface) => {
                            self.enter_interface_type(interface, _visitor_context);

                            for field in &interface.fields {
                                self.enter_interface_type_field(field, interface, _visitor_context);
                                self.leave_interface_type_field(field, interface, _visitor_context);
                            }

                            self.leave_interface_type(interface, _visitor_context);
                        }
                    }

                    self.leave_type_definition(type_definition, _visitor_context);
                }
                Definition::DirectiveDefinition(directive_definition) => {
                    self.enter_directive_definition(directive_definition, _visitor_context);
                    self.leave_directive_definition(directive_definition, _visitor_context);
                }
                Definition::TypeExtension(_type_extension) => {
                    // TODO: implement this
                    panic!("TypeExtension not supported at the moment");
                }
            }
        }

        self.leave_document(document, _visitor_context);
    }

    fn enter_document(&self, _node: &Document, _visitor_context: &mut T) {}
    fn leave_document(&self, _node: &Document, _visitor_context: &mut T) {}

    fn enter_schema_definition(&self, _node: &SchemaDefinition, _visitor_context: &mut T) {}
    fn leave_schema_definition(&self, _node: &SchemaDefinition, _visitor_context: &mut T) {}

    fn enter_directive_definition(&self, _node: &DirectiveDefinition, _visitor_context: &mut T) {}
    fn leave_directive_definition(&self, _node: &DirectiveDefinition, _visitor_context: &mut T) {}

    fn enter_type_definition(&self, _node: &TypeDefinition, _visitor_context: &mut T) {}
    fn leave_type_definition(&self, _node: &TypeDefinition, _visitor_context: &mut T) {}

    fn enter_interface_type(&self, _node: &InterfaceType, _visitor_context: &mut T) {}
    fn leave_interface_type(&self, _node: &InterfaceType, _visitor_context: &mut T) {}

    fn enter_interface_type_field(
        &self,
        _node: &Field,
        _type_: &InterfaceType,
        _visitor_context: &mut T,
    ) {
    }
    fn leave_interface_type_field(
        &self,
        _node: &Field,
        _type_: &InterfaceType,
        _visitor_context: &mut T,
    ) {
    }

    fn enter_object_type(&self, _node: &ObjectType, _visitor_context: &mut T) {}
    fn leave_object_type(&self, _node: &ObjectType, _visitor_context: &mut T) {}

    fn enter_object_type_field(
        &self,
        _node: &Field,
        _type_: &ObjectType,
        _visitor_context: &mut T,
    ) {
    }
    fn leave_object_type_field(
        &self,
        _node: &Field,
        _type_: &ObjectType,
        _visitor_context: &mut T,
    ) {
    }

    fn enter_input_object_type(&self, _node: &InputObjectType, _visitor_context: &mut T) {}
    fn leave_input_object_type(&self, _node: &InputObjectType, _visitor_context: &mut T) {}

    fn enter_input_object_type_field(
        &self,
        _node: &InputValue,
        _input_type: &InputObjectType,
        _visitor_context: &mut T,
    ) {
    }
    fn leave_input_object_type_field(
        &self,
        _node: &InputValue,
        _input_type: &InputObjectType,
        _visitor_context: &mut T,
    ) {
    }

    fn enter_union_type(&self, _node: &UnionType, _visitor_context: &mut T) {}
    fn leave_union_type(&self, _node: &UnionType, _visitor_context: &mut T) {}

    fn enter_scalar_type(&self, _node: &ScalarType, _visitor_context: &mut T) {}
    fn leave_scalar_type(&self, _node: &ScalarType, _visitor_context: &mut T) {}

    fn enter_enum_type(&self, _node: &EnumType, _visitor_context: &mut T) {}
    fn leave_enum_type(&self, _node: &EnumType, _visitor_context: &mut T) {}

    fn enter_enum_value(&self, _node: &EnumValue, _enum: &EnumType, _visitor_context: &mut T) {}
    fn leave_enum_value(&self, _node: &EnumValue, _enum: &EnumType, _visitor_context: &mut T) {}
}

#[test]
fn visit_schema() {
    use crate::parser::schema::parse_schema;
    let schema_ast = parse_schema(
        r#"
    scalar Date

    type Query {
      user(id: ID!): User!
      users(filter: UsersFilter): [User!]!
      now: Date
    }

    input UsersFilter {
      name: String
    }

    type User implements Node {
      id: ID!
      name: String!
      role: Role!
    }

    interface Node {
      id: ID!
    }

    type Test {
      foo: String!
    }

    enum Role {
      USER
      ADMIN
    }

    union TestUnion = Test | User

    "#,
    )
    .expect("Failed to parse schema");

    struct TestVisitorCollected {
        collected_object_type: Vec<String>,
        collected_scalar_type: Vec<String>,
        collected_union_type: Vec<String>,
        collected_input_type: Vec<String>,
        collected_enum_type: Vec<String>,
        collected_enum_value: Vec<String>,
        collected_interface_type: Vec<String>,
        collected_object_type_field: Vec<String>,
        collected_interface_type_field: Vec<String>,
        collected_input_type_fields: Vec<String>,
    }

    struct TestVisitor;

    impl TestVisitor {
        fn collect_visited_info(&self, document: &Document) -> TestVisitorCollected {
            let mut collected = TestVisitorCollected {
                collected_object_type: Vec::new(),
                collected_interface_type: Vec::new(),
                collected_object_type_field: Vec::new(),
                collected_interface_type_field: Vec::new(),
                collected_scalar_type: Vec::new(),
                collected_union_type: Vec::new(),
                collected_enum_type: Vec::new(),
                collected_enum_value: Vec::new(),
                collected_input_type: Vec::new(),
                collected_input_type_fields: Vec::new(),
            };
            self.visit_schema_document(document, &mut collected);

            collected
        }
    }

    impl SchemaVisitor<TestVisitorCollected> for TestVisitor {
        fn enter_object_type(
            &self,
            _node: &ObjectType,
            _visitor_context: &mut TestVisitorCollected,
        ) {
            _visitor_context
                .collected_object_type
                .push(_node.name.clone());
        }

        fn enter_object_type_field(
            &self,
            _node: &Field,
            _type_: &ObjectType,
            _visitor_context: &mut TestVisitorCollected,
        ) {
            let field_id = format!("{}.{}", _type_.name.as_str(), _node.name.as_str());
            _visitor_context.collected_object_type_field.push(field_id);
        }

        fn enter_interface_type(
            &self,
            _node: &InterfaceType,
            _visitor_context: &mut TestVisitorCollected,
        ) {
            _visitor_context
                .collected_interface_type
                .push(_node.name.clone());
        }

        fn enter_interface_type_field(
            &self,
            _node: &Field,
            _type_: &InterfaceType,
            _visitor_context: &mut TestVisitorCollected,
        ) {
            _visitor_context
                .collected_interface_type_field
                .push(_node.name.clone());
        }

        fn enter_scalar_type(
            &self,
            _node: &ScalarType,
            _visitor_context: &mut TestVisitorCollected,
        ) {
            _visitor_context
                .collected_scalar_type
                .push(_node.name.clone());
        }

        fn enter_union_type(&self, _node: &UnionType, _visitor_context: &mut TestVisitorCollected) {
            _visitor_context
                .collected_union_type
                .push(_node.name.clone());
        }

        fn enter_enum_type(&self, _node: &EnumType, _visitor_context: &mut TestVisitorCollected) {
            _visitor_context
                .collected_enum_type
                .push(_node.name.clone());
        }

        fn enter_enum_value(
            &self,
            _node: &EnumValue,
            _enum: &EnumType,
            _visitor_context: &mut TestVisitorCollected,
        ) {
            let enum_value_id = format!("{}.{}", _enum.name.as_str(), _node.name.as_str());
            _visitor_context.collected_enum_value.push(enum_value_id);
        }

        fn enter_input_object_type(
            &self,
            _node: &InputObjectType,
            _visitor_context: &mut TestVisitorCollected,
        ) {
            _visitor_context
                .collected_input_type
                .push(_node.name.clone());
        }

        fn enter_input_object_type_field(
            &self,
            _node: &InputValue,
            _input_type: &InputObjectType,
            _visitor_context: &mut TestVisitorCollected,
        ) {
            let field_id = format!("{}.{}", _input_type.name.as_str(), _node.name.as_str());
            _visitor_context.collected_input_type_fields.push(field_id);
        }
    }

    let visitor = TestVisitor {};
    let collected = visitor.collect_visited_info(&schema_ast);

    assert_eq!(
        collected.collected_object_type,
        vec!["Query", "User", "Test"]
    );
    assert_eq!(
        collected.collected_object_type_field,
        vec![
            "Query.user",
            "Query.users",
            "Query.now",
            "User.id",
            "User.name",
            "User.role",
            "Test.foo"
        ]
    );
    assert_eq!(collected.collected_interface_type, vec!["Node"]);
    assert_eq!(collected.collected_union_type, vec!["TestUnion"]);
    assert_eq!(collected.collected_scalar_type, vec!["Date"]);
    assert_eq!(collected.collected_enum_type, vec!["Role"]);
    assert_eq!(
        collected.collected_enum_value,
        vec!["Role.USER", "Role.ADMIN"]
    );
    assert_eq!(collected.collected_input_type, vec!["UsersFilter"]);
    assert_eq!(
        collected.collected_input_type_fields,
        vec!["UsersFilter.name"]
    );
}
