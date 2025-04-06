use crate::federation_spec::directives::FederationDirective;
use crate::federation_spec::directives::InaccessibleDirective;
use crate::utils::schema_transformer::Transformed;

use crate::utils::schema_transformer::TransformedValue;

use crate::utils::schema_transformer::SchemaTransformer;

use graphql_parser_hive_fork::query::Text;
use graphql_parser_hive_fork::schema::*;

// directive @inaccessible on FIELD_DEFINITION | OBJECT | INTERFACE | UNION | ENUM | ENUM_VALUE | SCALAR | INPUT_OBJECT | INPUT_FIELD_DEFINITION | ARGUMENT_DEFINITION
pub(crate) struct PruneInaccessible;

impl PruneInaccessible {
    pub fn prune(schema: &Document<'static, String>) -> Document<'static, String> {
        let mut transformer = PruneInaccessible {};
        let result = transformer
            .transform_document(schema)
            .replace_or_else(|| schema.clone());

        result
    }

    pub(crate) fn has_inaccessible_directive<'a, T: Text<'a> + Clone>(
        directives: &Vec<Directive<'a, T>>,
    ) -> bool {
        directives
            .iter()
            .any(|d| d.name == InaccessibleDirective::directive_name().into())
    }
}

impl<'a, T: Text<'a> + Clone> SchemaTransformer<'a, T> for PruneInaccessible {
    fn transform_document(
        &mut self,
        document: &Document<'a, T>,
    ) -> TransformedValue<Document<'a, T>> {
        let new_doc = Document {
            definitions: document
                .definitions
                .iter()
                .filter(|def| match def {
                    Definition::SchemaDefinition(_) => true,
                    Definition::TypeDefinition(TypeDefinition::Object(obj)) => {
                        !Self::has_inaccessible_directive(&obj.directives)
                    }
                    Definition::TypeDefinition(TypeDefinition::Interface(interface)) => {
                        !Self::has_inaccessible_directive(&interface.directives)
                    }
                    Definition::TypeDefinition(TypeDefinition::Union(union)) => {
                        !Self::has_inaccessible_directive(&union.directives)
                    }
                    Definition::TypeDefinition(TypeDefinition::Scalar(scalar)) => {
                        !Self::has_inaccessible_directive(&scalar.directives)
                    }
                    Definition::TypeDefinition(TypeDefinition::Enum(enm)) => {
                        !Self::has_inaccessible_directive(&enm.directives)
                    }
                    Definition::TypeDefinition(TypeDefinition::InputObject(input)) => {
                        !Self::has_inaccessible_directive(&input.directives)
                    }
                    Definition::DirectiveDefinition(_) => true,
                    Definition::TypeExtension(_) => true,
                })
                .cloned()
                .collect(),
        };

        self.default_transform_document(&new_doc)
    }

    fn transform_input_values(
        &mut self,
        values: &Vec<InputValue<'a, T>>,
    ) -> TransformedValue<Vec<InputValue<'a, T>>> {
        TransformedValue::Replace(
            values
                .iter()
                .filter(|v| !Self::has_inaccessible_directive(&v.directives))
                .cloned()
                .collect(),
        )
    }
    fn transform_input_object_type(
        &mut self,
        input: &InputObjectType<'a, T>,
    ) -> Transformed<InputObjectType<'a, T>> {
        Transformed::Replace(InputObjectType {
            description: input.description.clone(),
            directives: input.directives.clone(),
            name: input.name.clone(),
            fields: input
                .fields
                .iter()
                .filter(|v| !Self::has_inaccessible_directive(&v.directives))
                .cloned()
                .collect(),
            position: input.position,
        })
    }

    fn transform_enum_type(&mut self, enum_type: &EnumType<'a, T>) -> Transformed<EnumType<'a, T>> {
        Transformed::Replace(EnumType {
            description: enum_type.description.clone(),
            directives: enum_type.directives.clone(),
            name: enum_type.name.clone(),
            values: enum_type
                .values
                .iter()
                .filter(|v| !Self::has_inaccessible_directive(&v.directives))
                .cloned()
                .collect(),
            position: enum_type.position,
        })
    }

    // FIELD_DEFINITION for both interface and object type
    fn transform_fields(
        &mut self,
        fields: &Vec<Field<'a, T>>,
    ) -> TransformedValue<Vec<Field<'a, T>>> {
        let new_fields = fields
            .iter()
            .filter(|v| !Self::has_inaccessible_directive(&v.directives))
            .cloned()
            .collect();

        self.default_transform_fields(&new_fields)
    }
}
