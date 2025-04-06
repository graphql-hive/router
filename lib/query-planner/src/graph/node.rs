use std::fmt::Debug;

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SubgraphType {
    pub name: String,
    pub subgraph: String,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum Node {
    QueryRoot(String),
    MutationRoot(String),
    SubscriptionRoot(String),
    /// Represent an entity type or a scalar living in a specific subgraph
    SubgraphType(SubgraphType),
    /// Represent an sub-set (view) of an entity, for cases like `@provides` where only some
    /// fields on the type are really available.
    SubgraphTypeView {
        view_id: u64,
        node: SubgraphType,
        selection_set: String,
    },
}

impl Node {
    pub fn id(&self) -> String {
        match self {
            Node::QueryRoot(name) => format!("root({})", name),
            Node::MutationRoot(name) => format!("root({})", name),
            Node::SubscriptionRoot(name) => format!("root({})", name),
            Node::SubgraphType(st) => format!("{}/{}", st.name, st.subgraph),
            Node::SubgraphTypeView { node, view_id, .. } => {
                format!("({}/{}).view{view_id}", node.name, node.subgraph)
            }
        }
    }

    pub fn is_view_node(&self) -> bool {
        match self {
            Node::QueryRoot(_) => false,
            Node::MutationRoot(_) => false,
            Node::SubscriptionRoot(_) => false,
            Node::SubgraphType(_) => false,
            Node::SubgraphTypeView { .. } => true,
        }
    }

    pub fn subgraph_type(name: &str, subgraph: &str) -> Node {
        Node::SubgraphType(SubgraphType {
            name: name.to_string(),
            subgraph: subgraph.to_string(),
        })
    }

    pub fn create_node_for_definition(
        name: &str,
        subgraph: &str,
        view: Option<(u64, String)>,
    ) -> Node {
        match view {
            Some(view) => Node::SubgraphTypeView {
                view_id: view.0,
                node: SubgraphType {
                    name: name.to_string(),
                    subgraph: subgraph.to_string(),
                },
                selection_set: view.1,
            },
            None => Node::SubgraphType(SubgraphType {
                name: name.to_string(),
                subgraph: subgraph.to_string(),
            }),
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
        write!(f, "{}", self.id())
    }
}
