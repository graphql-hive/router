pub use crate::federation_spec::directive_trait::FederationDirective;
pub use crate::federation_spec::inacessible::InaccessibleDirective;
pub use crate::federation_spec::join_enum_value::JoinEnumValueDirective;
pub use crate::federation_spec::join_field::JoinFieldDirective;
pub use crate::federation_spec::join_graph::JoinGraphDirective;
pub use crate::federation_spec::join_implements::JoinImplementsDirective;
pub use crate::federation_spec::join_type::JoinTypeDirective;
pub use crate::federation_spec::join_union::JoinUnionMemberDirective;

pub struct TagDirective {}

impl TagDirective {
    pub const NAME: &str = "tag";
}

pub struct LinkDirective {}

impl LinkDirective {
    pub const NAME: &str = "link";
}
