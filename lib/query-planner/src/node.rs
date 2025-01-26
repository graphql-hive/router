use std::fmt::Debug;

pub enum Node {
    QueryRoot(String),
    MutationRoot(String),
    SubscriptionRoot(String),
    SubgraphType { name: String, subgraph: String },
}

impl Node {
    pub fn id(&self) -> String {
        match self {
            Node::QueryRoot(name) => name.to_string(),
            Node::MutationRoot(name) => name.to_string(),
            Node::SubscriptionRoot(name) => name.to_string(),
            Node::SubgraphType { name, subgraph } => format!("{}/{}", name, subgraph),
        }
    }

    pub fn id_from(type_name: &str, subgraph: Option<&str>) -> String {
        match subgraph {
            Some(subgraph) => format!("{}/{}", type_name, subgraph),
            None => type_name.to_string(),
        }
    }
}

impl Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Node::QueryRoot(name) => write!(f, "QueryRoot({})", name),
            Node::MutationRoot(name) => write!(f, "MutationRoot({})", name),
            Node::SubscriptionRoot(name) => write!(f, "SubscriptionRoot({})", name),
            Node::SubgraphType { name, subgraph } => {
                write!(f, "{}/{}", name, subgraph)
            }
        }
    }
}
