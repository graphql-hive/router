use graphql_parser_hive_fork::schema::Directive;

#[derive(Debug, Default, Clone)]
pub struct InaccessibleDirective {}

impl InaccessibleDirective {
    pub const NAME: &str = "inaccessible";

    pub fn is(directive: &Directive<'_, String>) -> bool {
        directive.name == Self::NAME
    }
}

impl From<&Directive<'_, String>> for InaccessibleDirective {
    fn from(_directive: &Directive<'_, String>) -> Self {
        Self::default()
    }
}
