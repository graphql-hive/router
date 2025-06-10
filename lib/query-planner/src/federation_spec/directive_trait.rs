use graphql_parser::schema::Directive;

pub trait FederationDirective: Ord + PartialOrd {
    fn directive_name() -> &'static str;
    fn is(directive: &Directive<'_, String>) -> bool {
        Self::directive_name() == directive.name
    }
    fn parse(directive: &Directive<'_, String>) -> Self
    where
        Self: Sized;
}
