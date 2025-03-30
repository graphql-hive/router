pub use crate::federation_spec::inacessible::InaccessibleDirective;
pub use crate::federation_spec::join_field::JoinFieldDirective;
pub use crate::federation_spec::join_implements::JoinImplementsDirective;
pub use crate::federation_spec::join_type::JoinTypeDirective;

pub struct JoinEnumValueDirective {}

impl JoinEnumValueDirective {
    pub const NAME: &str = "join__enumValue";
}

pub struct JoinUnionMemberDirective {}

impl JoinUnionMemberDirective {
    pub const NAME: &str = "join__unionMember";
}

pub struct JoinGraphDirective {}

impl JoinGraphDirective {
    pub const NAME: &str = "join__graph";
}

pub struct TagDirective {}

impl TagDirective {
    pub const NAME: &str = "tag";
}

pub struct LinkDirective {}

impl LinkDirective {
    pub const NAME: &str = "link";
}
