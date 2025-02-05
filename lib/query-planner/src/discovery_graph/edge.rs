use std::fmt::Debug;

pub enum DiscoveryEdge {
    Typename,
    Field { name: String },
    InterfaceReference { interface_name: String },
}

impl Debug for DiscoveryEdge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiscoveryEdge::Typename => write!(f, "__typename"),
            DiscoveryEdge::Field { name } => write!(f, "{}", name),
            DiscoveryEdge::InterfaceReference { interface_name } => {
                write!(f, "🔮 {}", interface_name)
            }
        }
    }
}
