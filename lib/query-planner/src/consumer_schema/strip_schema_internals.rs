use graphql_parser_hive_fork::query::Text;
use graphql_parser_hive_fork::schema::*;

use crate::utils::schema_transformer::{SchemaTransformer, TransformedValue};

// directive @inaccessible on FIELD_DEFINITION | OBJECT | INTERFACE | UNION | ENUM | ENUM_VALUE | SCALAR | INPUT_OBJECT | INPUT_FIELD_DEFINITION | ARGUMENT_DEFINITION
pub(crate) struct StripSchemaInternals;

static DIRECTIVES_TO_STRIP: [&str; 9] = [
    "join__type",
    "join__enumValue",
    "join__field",
    "join__implements",
    "tag",
    "inaccessible",
    "link",
    "join__unionMember",
    "join__graph",
];

static DEFINITIONS_TO_STRIP: [&str; 4] = [
    "join__Graph",
    "link__Purpose",
    "link__Import",
    "join__FieldSet",
];

impl StripSchemaInternals {
    pub fn strip_schema_internals(schema: &Document<'static, String>) -> Document<'static, String> {
        let mut transformer = StripSchemaInternals {};
        let result = transformer
            .transform_document(&schema)
            .replace_or_else(|| schema.clone());

        result
    }

    pub(crate) fn filter_directives<'a, T: Text<'a> + Clone>(
        directives: &Vec<Directive<'a, T>>,
    ) -> Vec<Directive<'a, T>> {
        directives
            .iter()
            .filter(|d| !DIRECTIVES_TO_STRIP.contains(&d.name.as_ref()))
            .cloned()
            .collect()
    }
}

impl<'a, T: Text<'a> + Clone> SchemaTransformer<'a, T> for StripSchemaInternals {
    fn transform_directives(
        &mut self,
        directives: &Vec<Directive<'a, T>>,
    ) -> TransformedValue<Vec<Directive<'a, T>>> {
        let new_directives = StripSchemaInternals::filter_directives(&directives);

        TransformedValue::Replace(new_directives)
    }

    fn transform_document(
        &mut self,
        document: &Document<'a, T>,
    ) -> TransformedValue<Document<'a, T>> {
        let new_doc = Document {
            definitions: document
                .definitions
                .iter()
                .filter(|def| match def {
                    Definition::DirectiveDefinition(directive) => {
                        !DIRECTIVES_TO_STRIP.contains(&directive.name.as_ref())
                    }
                    Definition::TypeDefinition(TypeDefinition::Object(obj)) => {
                        !DEFINITIONS_TO_STRIP.contains(&obj.name.as_ref())
                    }
                    Definition::TypeDefinition(TypeDefinition::Interface(interface)) => {
                        !DEFINITIONS_TO_STRIP.contains(&interface.name.as_ref())
                    }
                    Definition::TypeDefinition(TypeDefinition::Union(union)) => {
                        !DEFINITIONS_TO_STRIP.contains(&union.name.as_ref())
                    }
                    Definition::TypeDefinition(TypeDefinition::Enum(enm)) => {
                        !DEFINITIONS_TO_STRIP.contains(&enm.name.as_ref())
                    }
                    Definition::TypeDefinition(TypeDefinition::Scalar(scalar)) => {
                        !DEFINITIONS_TO_STRIP.contains(&scalar.name.as_ref())
                    }
                    Definition::TypeDefinition(TypeDefinition::InputObject(input)) => {
                        !DEFINITIONS_TO_STRIP.contains(&input.name.as_ref())
                    }
                    Definition::TypeExtension(_) => todo!("TypeExtension not implemented"),
                    Definition::SchemaDefinition(_) => true,
                })
                .cloned()
                .collect(),
        };

        self.default_transform_document(&new_doc)
    }
}
