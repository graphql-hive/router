use graphql_parser::schema::Directive;

use super::directives::FederationDirective;

#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct InaccessibleDirective;

impl InaccessibleDirective {
    pub const NAME: &str = "inaccessible";
}

impl FederationDirective for InaccessibleDirective {
    fn directive_name() -> &'static str {
        Self::NAME
    }

    fn parse(_: &Directive<'_, String>) -> Self {
        Self
    }
}

impl Ord for InaccessibleDirective {
    fn cmp(&self, _other: &Self) -> std::cmp::Ordering {
        std::cmp::Ordering::Equal
    }
}

impl PartialOrd for InaccessibleDirective {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
