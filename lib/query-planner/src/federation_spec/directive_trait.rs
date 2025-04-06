use graphql_parser_hive_fork::schema::Directive;

pub trait FederationDirective<'a> {
    fn directive_name() -> &'a str;
    fn is(directive: &Directive<'_, String>) -> bool {
        Self::directive_name() == directive.name
    }
    fn parse(directive: &Directive<'_, String>) -> Self
    where
        Self: Sized;
}
