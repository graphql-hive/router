use graphql_parser_hive_fork::schema::{Definition, TypeDefinition};

pub mod definitions;
pub mod directives;

pub(crate) mod join_field;
pub(crate) mod join_implements;
pub(crate) mod join_type;

pub struct FederationSpec;

impl FederationSpec {
    pub fn is_core_definition(def: &Definition<'static, String>) -> bool {
        match def {
            Definition::SchemaDefinition(_) => false,
            Definition::TypeDefinition(type_definition) => match type_definition {
                TypeDefinition::Enum(enum_type) => {
                    enum_type.name == definitions::LinkPurposeEnum::NAME
                        || enum_type.name == definitions::JoinGraphEnum::NAME
                }
                TypeDefinition::Scalar(scalar_definition) => {
                    scalar_definition.name == definitions::JoinFieldSetScalar::NAME
                        || scalar_definition.name == definitions::LinkImportScalar::NAME
                }
                _ => false,
            },
            Definition::TypeExtension(_) => todo!(),
            Definition::DirectiveDefinition(_) => false,
        }
    }
}
