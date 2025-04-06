use graphql_parser_hive_fork::schema::Directive;

use super::directives::FederationDirective;

#[derive(Debug, Default, Clone)]
pub struct InaccessibleDirective;

impl InaccessibleDirective {
    pub const NAME: &str = "inaccessible";
}

impl<'a> FederationDirective<'a> for InaccessibleDirective {
    fn directive_name() -> &'a str {
        Self::NAME
    }

    fn parse(_: &Directive<'_, String>) -> Self {
        Self::default()
    }
}
