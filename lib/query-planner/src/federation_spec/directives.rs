use graphql_parser_hive_fork::schema::Directive;

pub use crate::federation_spec::join_field::JoinFieldDirective;
pub use crate::federation_spec::join_implements::JoinImplementsDirective;
pub use crate::federation_spec::join_type::JoinTypeDirective;

pub struct JoinEnumValueDirective {}

impl JoinEnumValueDirective {
    pub const NAME: &str = "join__enumValue";

    pub fn is(directive: &Directive<'_, String>) -> bool {
        directive.name == Self::NAME
    }
}

pub struct JoinUnionMemberDirective {}

impl JoinUnionMemberDirective {
    pub const NAME: &str = "join__unionMember";

    pub fn is(directive: &Directive<'_, String>) -> bool {
        directive.name == Self::NAME
    }
}

pub struct JoinGraphDirective {}

impl JoinGraphDirective {
    pub const NAME: &str = "join__graph";

    pub fn is(directive: &Directive<'_, String>) -> bool {
        directive.name == Self::NAME
    }
}

pub struct TagDirective {}

impl TagDirective {
    pub const NAME: &str = "tag";

    pub fn is(directive: &Directive<'_, String>) -> bool {
        directive.name == Self::NAME
    }
}

pub struct LinkDirective {}

impl LinkDirective {
    pub const NAME: &str = "link";

    pub fn is(directive: &Directive<'_, String>) -> bool {
        directive.name == Self::NAME
    }
}

pub struct InaccessibleDirective {}

impl InaccessibleDirective {
    pub const NAME: &str = "inaccessible";

    pub fn is(directive: &Directive<'_, String>) -> bool {
        directive.name == Self::NAME
    }
}
