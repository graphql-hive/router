use std::fmt::Debug;

pub enum DiscoveryNode {
    ObjectType(String),
    InterfaceType(String),
    UnionType(String),
    ScalarType(String),
    EnumType(String),
}

impl DiscoveryNode {
    pub fn id(&self) -> String {
        match self {
            DiscoveryNode::ObjectType(name) => Self::id_from(name),
            DiscoveryNode::InterfaceType(name) => Self::id_from(name),
            DiscoveryNode::UnionType(name) => Self::id_from(name),
            DiscoveryNode::ScalarType(name) => Self::id_from(name),
            DiscoveryNode::EnumType(name) => Self::id_from(name),
        }
    }

    pub fn id_from(type_name: &str) -> String {
        type_name.to_string()
    }
}

impl Debug for DiscoveryNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiscoveryNode::ObjectType(name) => write!(f, "Type({})", name),
            DiscoveryNode::InterfaceType(name) => write!(f, "Interface({})", name),
            DiscoveryNode::UnionType(name) => write!(f, "Union({})", name),
            DiscoveryNode::ScalarType(name) => write!(f, "Scalar({})", name),
            DiscoveryNode::EnumType(name) => write!(f, "Enum({})", name),
        }
    }
}
