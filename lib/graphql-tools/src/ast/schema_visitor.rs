use crate::static_graphql::schema::{
    Definition, DirectiveDefinition, Document, EnumType, EnumValue, Field, InputObjectType,
    InputValue, InterfaceType, ObjectType, ScalarType, SchemaDefinition, TypeDefinition, UnionType,
};

/// A trait for implenenting a visitor for GraphQL schema definition.
pub trait SchemaVisitor<T = ()> {
    fn visit_schema_document(
        &self,
        document: Document,
        _visitor_context: &mut T,
    ) -> Option<Document> {
        self.enter_document(document, _visitor_context)
            .and_then(|mut document| {
                document.definitions = document
                    .definitions
                    .into_iter()
                    .filter_map(|definition| {
                        match definition {
                            Definition::SchemaDefinition(schema_definition) => self
                                .enter_schema_definition(schema_definition, _visitor_context)
                                .and_then(|schema_definition| {
                                    self.leave_schema_definition(
                                        schema_definition,
                                        _visitor_context,
                                    )
                                })
                                .map(Definition::SchemaDefinition),
                            Definition::TypeDefinition(type_definition) => {
                                self.enter_type_definition(type_definition, _visitor_context)
                                    .and_then(|type_definition| {
                                        match type_definition {
                                            TypeDefinition::Object(object) => {
                                                self.enter_object_type(object, _visitor_context)
                                                    .and_then(|mut object| {
                                                        object.fields = object
                                                            .fields
                                                            .clone()
                                                            .into_iter()
                                                            .filter_map(|field| {
                                                                self.enter_object_type_field(
                                                                    field,
                                                                    &object,
                                                                    _visitor_context,
                                                                )
                                                                .and_then(|field| {
                                                                    // TODO: More advanced setup for fields: arguments, lists, null/non-null, directives
                                                                    self.leave_object_type_field(
                                                                        field,
                                                                        &object,
                                                                        _visitor_context,
                                                                    )
                                                                })
                                                            })
                                                            .collect();
                                                        self.leave_object_type(
                                                            object,
                                                            _visitor_context,
                                                        )
                                                    })
                                                    .map(TypeDefinition::Object)
                                            }
                                            TypeDefinition::Scalar(scalar) => self
                                                .enter_scalar_type(scalar, _visitor_context)
                                                .and_then(|scalar| {
                                                    self.leave_scalar_type(scalar, _visitor_context)
                                                })
                                                .map(TypeDefinition::Scalar),
                                            TypeDefinition::Enum(enum_) => self
                                                .enter_enum_type(enum_, _visitor_context)
                                                .and_then(|mut enum_| {
                                                    enum_.values = enum_
                                                        .values
                                                        .clone()
                                                        .into_iter()
                                                        .filter_map(|enum_value| {
                                                            self.enter_enum_value(
                                                                enum_value,
                                                                &enum_,
                                                                _visitor_context,
                                                            )
                                                            .and_then(|enum_value| {
                                                                self.leave_enum_value(
                                                                    enum_value,
                                                                    &enum_,
                                                                    _visitor_context,
                                                                )
                                                            })
                                                        })
                                                        .collect();
                                                    self.leave_enum_type(enum_, _visitor_context)
                                                })
                                                .map(TypeDefinition::Enum),
                                            TypeDefinition::Union(union) => self
                                                .enter_union_type(union, _visitor_context)
                                                .and_then(|union| {
                                                    self.leave_union_type(union, _visitor_context)
                                                })
                                                .map(TypeDefinition::Union),
                                            TypeDefinition::InputObject(input_object) => self
                                                .enter_input_object_type(
                                                    input_object,
                                                    _visitor_context,
                                                )
                                                .and_then(|mut input_object| {
                                                    input_object.fields = input_object
                                                        .fields
                                                        .clone()
                                                        .into_iter()
                                                        .filter_map(|input_value| {
                                                            self.enter_input_object_type_field(
                                                                input_value,
                                                                &input_object,
                                                                _visitor_context,
                                                            )
                                                            .and_then(|input_value| {
                                                                self.leave_input_object_type_field(
                                                                    input_value,
                                                                    &input_object,
                                                                    _visitor_context,
                                                                )
                                                            })
                                                        })
                                                        .collect();
                                                    self.leave_input_object_type(
                                                        input_object,
                                                        _visitor_context,
                                                    )
                                                })
                                                .map(TypeDefinition::InputObject),
                                            TypeDefinition::Interface(interface) => self
                                                .enter_interface_type(interface, _visitor_context)
                                                .and_then(|mut interface| {
                                                    interface.fields = interface
                                                        .fields
                                                        .clone()
                                                        .into_iter()
                                                        .filter_map(|field| {
                                                            self.enter_interface_type_field(
                                                                field,
                                                                &interface,
                                                                _visitor_context,
                                                            )
                                                            .and_then(|field| {
                                                                self.leave_interface_type_field(
                                                                    field,
                                                                    &interface,
                                                                    _visitor_context,
                                                                )
                                                            })
                                                        })
                                                        .collect();
                                                    self.leave_interface_type(
                                                        interface,
                                                        _visitor_context,
                                                    )
                                                })
                                                .map(TypeDefinition::Interface),
                                        }
                                    })
                                    .and_then(|type_definition| {
                                        self.leave_type_definition(
                                            type_definition,
                                            _visitor_context,
                                        )
                                    })
                                    .map(Definition::TypeDefinition)
                            }
                            Definition::DirectiveDefinition(directive_definition) => self
                                .enter_directive_definition(directive_definition, _visitor_context)
                                .and_then(|directive_definition| {
                                    self.leave_directive_definition(
                                        directive_definition,
                                        _visitor_context,
                                    )
                                })
                                .map(Definition::DirectiveDefinition),
                            Definition::TypeExtension(_type_extension) => {
                                // TODO: implement this
                                panic!("TypeExtension not supported at the moment");
                            }
                        }
                    })
                    .collect();
                self.leave_document(document, _visitor_context)
            })
    }

    fn enter_document(&self, node: Document, _visitor_context: &mut T) -> Option<Document> {
        Some(node)
    }
    fn leave_document(&self, node: Document, _visitor_context: &mut T) -> Option<Document> {
        Some(node)
    }

    fn enter_schema_definition(
        &self,
        node: SchemaDefinition,
        _visitor_context: &mut T,
    ) -> Option<SchemaDefinition> {
        Some(node)
    }
    fn leave_schema_definition(
        &self,
        node: SchemaDefinition,
        _visitor_context: &mut T,
    ) -> Option<SchemaDefinition> {
        Some(node)
    }

    fn enter_directive_definition(
        &self,
        node: DirectiveDefinition,
        _visitor_context: &mut T,
    ) -> Option<DirectiveDefinition> {
        Some(node)
    }
    fn leave_directive_definition(
        &self,
        node: DirectiveDefinition,
        _visitor_context: &mut T,
    ) -> Option<DirectiveDefinition> {
        Some(node)
    }

    fn enter_type_definition(
        &self,
        node: TypeDefinition,
        _visitor_context: &mut T,
    ) -> Option<TypeDefinition> {
        Some(node)
    }
    fn leave_type_definition(
        &self,
        node: TypeDefinition,
        _visitor_context: &mut T,
    ) -> Option<TypeDefinition> {
        Some(node)
    }

    fn enter_interface_type(
        &self,
        node: InterfaceType,
        _visitor_context: &mut T,
    ) -> Option<InterfaceType> {
        Some(node)
    }
    fn leave_interface_type(
        &self,
        node: InterfaceType,
        _visitor_context: &mut T,
    ) -> Option<InterfaceType> {
        Some(node)
    }

    fn enter_interface_type_field(
        &self,
        node: Field,
        _type_: &InterfaceType,
        _visitor_context: &mut T,
    ) -> Option<Field> {
        Some(node)
    }
    fn leave_interface_type_field(
        &self,
        node: Field,
        _type_: &InterfaceType,
        _visitor_context: &mut T,
    ) -> Option<Field> {
        Some(node)
    }

    fn enter_object_type(&self, node: ObjectType, _visitor_context: &mut T) -> Option<ObjectType> {
        Some(node)
    }
    fn leave_object_type(&self, node: ObjectType, _visitor_context: &mut T) -> Option<ObjectType> {
        Some(node)
    }

    fn enter_object_type_field(
        &self,
        node: Field,
        _type_: &ObjectType,
        _visitor_context: &mut T,
    ) -> Option<Field> {
        Some(node)
    }
    fn leave_object_type_field(
        &self,
        node: Field,
        _type_: &ObjectType,
        _visitor_context: &mut T,
    ) -> Option<Field> {
        Some(node)
    }

    fn enter_input_object_type(
        &self,
        node: InputObjectType,
        _visitor_context: &mut T,
    ) -> Option<InputObjectType> {
        Some(node)
    }
    fn leave_input_object_type(
        &self,
        node: InputObjectType,
        _visitor_context: &mut T,
    ) -> Option<InputObjectType> {
        Some(node)
    }

    fn enter_input_object_type_field(
        &self,
        node: InputValue,
        _input_type: &InputObjectType,
        _visitor_context: &mut T,
    ) -> Option<InputValue> {
        Some(node)
    }
    fn leave_input_object_type_field(
        &self,
        node: InputValue,
        _input_type: &InputObjectType,
        _visitor_context: &mut T,
    ) -> Option<InputValue> {
        Some(node)
    }

    fn enter_union_type(&self, node: UnionType, _visitor_context: &mut T) -> Option<UnionType> {
        Some(node)
    }
    fn leave_union_type(&self, node: UnionType, _visitor_context: &mut T) -> Option<UnionType> {
        Some(node)
    }

    fn enter_scalar_type(&self, node: ScalarType, _visitor_context: &mut T) -> Option<ScalarType> {
        Some(node)
    }
    fn leave_scalar_type(&self, node: ScalarType, _visitor_context: &mut T) -> Option<ScalarType> {
        Some(node)
    }

    fn enter_enum_type(&self, node: EnumType, _visitor_context: &mut T) -> Option<EnumType> {
        Some(node)
    }
    fn leave_enum_type(&self, node: EnumType, _visitor_context: &mut T) -> Option<EnumType> {
        Some(node)
    }

    fn enter_enum_value(
        &self,
        node: EnumValue,
        _enum: &EnumType,
        _visitor_context: &mut T,
    ) -> Option<EnumValue> {
        Some(node)
    }
    fn leave_enum_value(
        &self,
        node: EnumValue,
        _enum: &EnumType,
        _visitor_context: &mut T,
    ) -> Option<EnumValue> {
        Some(node)
    }
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
        fn collect_visited_info(&self, document: Document) -> TestVisitorCollected {
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
            node: ObjectType,
            _visitor_context: &mut TestVisitorCollected,
        ) -> Option<ObjectType> {
            _visitor_context
                .collected_object_type
                .push(node.name.clone());
            Some(node)
        }

        fn enter_object_type_field(
            &self,
            node: Field,
            _type_: &ObjectType,
            _visitor_context: &mut TestVisitorCollected,
        ) -> Option<Field> {
            let field_id = format!("{}.{}", _type_.name.as_str(), node.name.as_str());
            _visitor_context.collected_object_type_field.push(field_id);
            Some(node)
        }

        fn enter_interface_type(
            &self,
            node: InterfaceType,
            _visitor_context: &mut TestVisitorCollected,
        ) -> Option<InterfaceType> {
            _visitor_context
                .collected_interface_type
                .push(node.name.clone());
            Some(node)
        }

        fn enter_interface_type_field(
            &self,
            node: Field,
            _type_: &InterfaceType,
            _visitor_context: &mut TestVisitorCollected,
        ) -> Option<Field> {
            _visitor_context
                .collected_interface_type_field
                .push(node.name.clone());
            Some(node)
        }

        fn enter_scalar_type(
            &self,
            node: ScalarType,
            _visitor_context: &mut TestVisitorCollected,
        ) -> Option<ScalarType> {
            _visitor_context
                .collected_scalar_type
                .push(node.name.clone());
            Some(node)
        }

        fn enter_union_type(
            &self,
            node: UnionType,
            _visitor_context: &mut TestVisitorCollected,
        ) -> Option<UnionType> {
            _visitor_context
                .collected_union_type
                .push(node.name.clone());
            Some(node)
        }

        fn enter_enum_type(
            &self,
            node: EnumType,
            _visitor_context: &mut TestVisitorCollected,
        ) -> Option<EnumType> {
            _visitor_context.collected_enum_type.push(node.name.clone());
            Some(node)
        }

        fn enter_enum_value(
            &self,
            node: EnumValue,
            _enum: &EnumType,
            _visitor_context: &mut TestVisitorCollected,
        ) -> Option<EnumValue> {
            let enum_value_id = format!("{}.{}", _enum.name.as_str(), node.name.as_str());
            _visitor_context.collected_enum_value.push(enum_value_id);
            Some(node)
        }

        fn enter_input_object_type(
            &self,
            node: InputObjectType,
            _visitor_context: &mut TestVisitorCollected,
        ) -> Option<InputObjectType> {
            _visitor_context
                .collected_input_type
                .push(node.name.clone());
            Some(node)
        }

        fn enter_input_object_type_field(
            &self,
            node: InputValue,
            _input_type: &InputObjectType,
            _visitor_context: &mut TestVisitorCollected,
        ) -> Option<InputValue> {
            let field_id = format!("{}.{}", _input_type.name.as_str(), node.name.as_str());
            _visitor_context.collected_input_type_fields.push(field_id);
            Some(node)
        }
    }

    let visitor = TestVisitor {};
    let collected = visitor.collect_visited_info(schema_ast);

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
