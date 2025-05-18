use graphql_parser::{
    query::{Directive, Text, Value},
    schema::{
        Definition, DirectiveDefinition, Document, EnumType, EnumValue, Field, InputObjectType,
        InputValue, InterfaceType, ObjectType, ScalarType, SchemaDefinition, TypeDefinition,
        UnionType,
    },
};

#[derive(Clone, Debug)]
pub enum Transformed<T> {
    Keep,
    Replace(T),
}

#[derive(Clone, Debug)]
pub enum TransformedValue<T> {
    Keep,
    Replace(T),
}

impl<T> TransformedValue<T> {
    pub fn should_keep(&self) -> bool {
        match self {
            TransformedValue::Keep => true,
            TransformedValue::Replace(_) => false,
        }
    }

    pub fn replace_or_else<F>(self, f: F) -> T
    where
        F: FnOnce() -> T,
    {
        match self {
            TransformedValue::Keep => f(),
            TransformedValue::Replace(next_value) => next_value,
        }
    }
}

impl<T> From<TransformedValue<T>> for Transformed<T> {
    fn from(val: TransformedValue<T>) -> Self {
        match val {
            TransformedValue::Keep => Transformed::Keep,
            TransformedValue::Replace(replacement) => Transformed::Replace(replacement),
        }
    }
}

impl<T> Iterator for Transformed<T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        match std::mem::replace(self, Transformed::Keep) {
            Transformed::Keep => None,
            Transformed::Replace(val) => Some(val),
        }
    }
}

impl<T> Transformed<T> {
    pub fn map<U, F>(self, f: F) -> Transformed<U>
    where
        F: FnOnce(T) -> U,
    {
        match self {
            Transformed::Keep => Transformed::Keep,
            Transformed::Replace(t) => Transformed::Replace(f(t)),
        }
    }
}

pub trait SchemaTransformer<'a, T: Text<'a> + Clone> {
    fn transform_document(
        &mut self,
        document: &Document<'a, T>,
    ) -> TransformedValue<Document<'a, T>> {
        self.default_transform_document(document)
    }

    fn default_transform_document(
        &mut self,
        document: &Document<'a, T>,
    ) -> TransformedValue<Document<'a, T>> {
        let mut next_document = Document {
            definitions: Vec::new(),
        };
        let mut has_changes = false;

        for definition in document.definitions.clone() {
            match self.transform_definition(&definition) {
                Transformed::Keep => next_document.definitions.push(definition),
                Transformed::Replace(replacement) => {
                    has_changes = true;
                    next_document.definitions.push(replacement)
                }
            }
        }
        if has_changes {
            TransformedValue::Replace(next_document)
        } else {
            TransformedValue::Keep
        }
    }

    fn transform_definition(
        &mut self,
        definition: &Definition<'a, T>,
    ) -> Transformed<Definition<'a, T>> {
        match definition {
            Definition::SchemaDefinition(schema) => self
                .transform_schema_definition(schema)
                .map(Definition::SchemaDefinition),
            Definition::TypeDefinition(type_def) => self
                .transform_type_definition(type_def)
                .map(Definition::TypeDefinition),
            Definition::TypeExtension(_) => todo!("TypeExtension not implemented"),
            Definition::DirectiveDefinition(dir_def) => self
                .transform_directive_definition(dir_def)
                .map(Definition::DirectiveDefinition),
        }
    }

    fn transform_directive_definition(
        &mut self,
        directive: &DirectiveDefinition<'a, T>,
    ) -> Transformed<DirectiveDefinition<'a, T>> {
        let arguments = self.transform_input_values(&directive.arguments);

        if arguments.should_keep() {
            return Transformed::Keep;
        }

        Transformed::Replace(DirectiveDefinition {
            position: directive.position,
            description: directive.description.clone(),
            name: directive.name.clone(),
            arguments: arguments.replace_or_else(|| directive.arguments.clone()),
            repeatable: directive.repeatable,
            locations: directive.locations.clone(),
        })
    }

    fn transform_schema_definition(
        &mut self,
        schema: &SchemaDefinition<'a, T>,
    ) -> Transformed<SchemaDefinition<'a, T>> {
        let directives = self.transform_directives(&schema.directives);

        if directives.should_keep() {
            Transformed::Keep
        } else {
            Transformed::Replace(SchemaDefinition {
                position: schema.position,
                directives: directives.replace_or_else(|| schema.directives.clone()),
                query: schema.query.clone(),
                mutation: schema.mutation.clone(),
                subscription: schema.subscription.clone(),
            })
        }
    }

    fn transform_type_definition(
        &mut self,
        type_def: &TypeDefinition<'a, T>,
    ) -> Transformed<TypeDefinition<'a, T>> {
        match type_def {
            TypeDefinition::Scalar(scalar) => self
                .transform_scalar_type(scalar)
                .map(TypeDefinition::Scalar),
            TypeDefinition::Object(obj) => {
                self.transform_object_type(obj).map(TypeDefinition::Object)
            }
            TypeDefinition::Interface(interface) => self
                .transform_interface_type(interface)
                .map(TypeDefinition::Interface),
            TypeDefinition::Union(union) => {
                self.transform_union_type(union).map(TypeDefinition::Union)
            }
            TypeDefinition::Enum(enum_type) => self
                .transform_enum_type(enum_type)
                .map(TypeDefinition::Enum),
            TypeDefinition::InputObject(input) => self
                .transform_input_object_type(input)
                .map(TypeDefinition::InputObject),
        }
    }

    fn transform_enum_value(
        &mut self,
        enum_value: &EnumValue<'a, T>,
    ) -> Transformed<EnumValue<'a, T>> {
        let directives = self.transform_directives(&enum_value.directives);

        if directives.should_keep() {
            return Transformed::Keep;
        }

        Transformed::Replace(EnumValue {
            position: enum_value.position,
            description: enum_value.description.clone(),
            name: enum_value.name.clone(),
            directives: directives.replace_or_else(|| enum_value.directives.clone()),
        })
    }

    fn transform_enum_type(&mut self, enum_type: &EnumType<'a, T>) -> Transformed<EnumType<'a, T>> {
        let directives = self.transform_directives(&enum_type.directives);
        let values = self.transform_list(&enum_type.values, Self::transform_enum_value);

        if directives.should_keep() && values.should_keep() {
            return Transformed::Keep;
        }

        Transformed::Replace(EnumType {
            position: enum_type.position,
            description: enum_type.description.clone(),
            name: enum_type.name.clone(),
            directives: directives.replace_or_else(|| enum_type.directives.clone()),
            values: values.replace_or_else(|| enum_type.values.clone()),
        })
    }

    fn transform_input_object_type(
        &mut self,
        input: &InputObjectType<'a, T>,
    ) -> Transformed<InputObjectType<'a, T>> {
        let directives = self.transform_directives(&input.directives);
        let fields = self.transform_input_values(&input.fields);

        if directives.should_keep() && fields.should_keep() {
            return Transformed::Keep;
        }

        Transformed::Replace(InputObjectType {
            position: input.position,
            description: input.description.clone(),
            name: input.name.clone(),
            directives: directives.replace_or_else(|| input.directives.clone()),
            fields: fields.replace_or_else(|| input.fields.clone()),
        })
    }

    fn transform_interface_type(
        &mut self,
        interface: &InterfaceType<'a, T>,
    ) -> Transformed<InterfaceType<'a, T>> {
        let directives = self.transform_directives(&interface.directives);
        let fields = self.transform_fields(&interface.fields);

        if directives.should_keep() && fields.should_keep() {
            return Transformed::Keep;
        }

        Transformed::Replace(InterfaceType {
            position: interface.position,
            description: interface.description.clone(),
            name: interface.name.clone(),
            implements_interfaces: interface.implements_interfaces.clone(),
            directives: directives.replace_or_else(|| interface.directives.clone()),
            fields: fields.replace_or_else(|| interface.fields.clone()),
        })
    }

    fn transform_union_type(&mut self, union: &UnionType<'a, T>) -> Transformed<UnionType<'a, T>> {
        let directives = self.transform_directives(&union.directives);

        if directives.should_keep() {
            return Transformed::Keep;
        }

        Transformed::Replace(UnionType {
            position: union.position,
            description: union.description.clone(),
            name: union.name.clone(),
            directives: directives.replace_or_else(|| union.directives.clone()),
            types: union.types.clone(),
        })
    }

    fn transform_scalar_type(
        &mut self,
        scalar: &ScalarType<'a, T>,
    ) -> Transformed<ScalarType<'a, T>> {
        let directives = self.transform_directives(&scalar.directives);

        if directives.should_keep() {
            Transformed::Keep
        } else {
            Transformed::Replace(ScalarType {
                position: scalar.position,
                description: scalar.description.clone(),
                name: scalar.name.clone(),
                directives: directives.replace_or_else(|| scalar.directives.clone()),
            })
        }
    }

    fn transform_object_type(&mut self, obj: &ObjectType<'a, T>) -> Transformed<ObjectType<'a, T>> {
        let directives = self.transform_directives(&obj.directives);
        let fields = self.transform_fields(&obj.fields);

        if directives.should_keep() && fields.should_keep() {
            Transformed::Keep
        } else {
            Transformed::Replace(ObjectType {
                position: obj.position,
                description: obj.description.clone(),
                name: obj.name.clone(),
                implements_interfaces: obj.implements_interfaces.clone(),
                directives: directives.replace_or_else(|| obj.directives.clone()),
                fields: fields.replace_or_else(|| obj.fields.clone()),
            })
        }
    }

    fn transform_fields(
        &mut self,
        fields: &Vec<Field<'a, T>>,
    ) -> TransformedValue<Vec<Field<'a, T>>> {
        self.default_transform_fields(fields)
    }

    fn default_transform_fields(
        &mut self,
        fields: &Vec<Field<'a, T>>,
    ) -> TransformedValue<Vec<Field<'a, T>>> {
        self.transform_list(fields, Self::transform_field)
    }

    fn transform_field(&mut self, field: &Field<'a, T>) -> Transformed<Field<'a, T>> {
        let directives = self.transform_directives(&field.directives);
        let arguments = self.transform_input_values(&field.arguments);

        if directives.should_keep() && arguments.should_keep() {
            Transformed::Keep
        } else {
            Transformed::Replace(Field {
                position: field.position,
                description: field.description.clone(),
                name: field.name.clone(),
                arguments: arguments.replace_or_else(|| field.arguments.clone()),
                field_type: field.field_type.clone(),
                directives: directives.replace_or_else(|| field.directives.clone()),
            })
        }
    }

    fn transform_input_values(
        &mut self,
        values: &Vec<InputValue<'a, T>>,
    ) -> TransformedValue<Vec<InputValue<'a, T>>> {
        self.transform_list(values, Self::transform_input_value)
    }

    fn transform_input_value(
        &mut self,
        value: &InputValue<'a, T>,
    ) -> Transformed<InputValue<'a, T>> {
        let directives = self.transform_directives(&value.directives);
        let default_value = value
            .default_value
            .as_ref()
            .map(|v| self.transform_value(v))
            .unwrap_or(TransformedValue::Keep);

        if directives.should_keep() && default_value.should_keep() {
            Transformed::Keep
        } else {
            Transformed::Replace(InputValue {
                position: value.position,
                description: value.description.clone(),
                name: value.name.clone(),
                value_type: value.value_type.clone(),
                default_value: value
                    .default_value
                    .as_ref()
                    .map(|v| default_value.replace_or_else(|| v.clone())),
                directives: directives.replace_or_else(|| value.directives.clone()),
            })
        }
    }

    // Helper method for transforming lists
    fn transform_list<I, F, R>(&mut self, list: &[I], f: F) -> TransformedValue<Vec<I>>
    where
        I: Clone,
        F: Fn(&mut Self, &I) -> R,
        R: Into<Transformed<I>>,
    {
        let mut result = Vec::new();
        let mut has_changes = false;

        for (index, prev_item) in list.iter().enumerate() {
            match f(self, prev_item).into() {
                Transformed::Keep => {
                    if has_changes {
                        result.push(prev_item.clone());
                    }
                }
                Transformed::Replace(next_item) => {
                    if !has_changes {
                        result.reserve(list.len());
                        result.extend(list.iter().take(index).cloned());
                    }
                    result.push(next_item);
                    has_changes = true;
                }
            }
        }

        if has_changes {
            TransformedValue::Replace(result)
        } else {
            TransformedValue::Keep
        }
    }

    fn transform_value(&mut self, _value: &Value<'a, T>) -> TransformedValue<Value<'a, T>> {
        TransformedValue::Keep
    }

    fn transform_directives(
        &mut self,
        directives: &Vec<Directive<'a, T>>,
    ) -> TransformedValue<Vec<Directive<'a, T>>> {
        self.transform_list(directives, Self::transform_directive)
    }

    fn transform_directive(
        &mut self,
        _directive: &Directive<'a, T>,
    ) -> Transformed<Directive<'a, T>> {
        Transformed::Keep
    }
}
