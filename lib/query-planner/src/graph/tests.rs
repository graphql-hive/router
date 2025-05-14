#[cfg(test)]
mod graph_tests {
    use crate::{
        graph::{
            edge::{Edge, EdgeReference},
            node::Node,
            Graph,
        },
        parse_schema,
        state::supergraph_state::SupergraphState,
    };
    use petgraph::{
        graph::NodeIndex,
        visit::{EdgeRef, NodeRef},
    };
    use std::path::PathBuf;

    fn init_test(supergraph_sdl: &str) -> Graph {
        let schema = parse_schema(supergraph_sdl);
        let metadata = SupergraphState::new(&schema);

        Graph::graph_from_supergraph_state(&metadata).expect("failed to create graph")
    }

    #[derive(Debug)]
    struct FoundEdges<'a> {
        pub edges: Vec<(EdgeReference<'a>, NodeIndex)>,
        pub graph: &'a Graph,
    }

    impl FoundEdges<'_> {
        pub fn assert_key_edge(&self, key: &str, other_side: &str) -> &Self {
            let edge = self.edge(key, other_side);

            assert!(
                edge.is_some(),
                "🔑 Key edge {} <-> {} not found",
                key,
                other_side
            );

            self
        }

        pub fn no_field_edge(&self, key: &str) -> &Self {
            let edge = self
                .edges
                .iter()
                .find(|(edge_ref, _)| edge_ref.weight().display_name() == key);

            assert!(edge.is_none(), "Field edge {} found", key);

            self
        }

        pub fn assert_field_edge_does_not_exist(&self, key: &str, other_side: &str) -> &Self {
            let edge = self.edge(key, other_side);

            assert!(
                edge.is_none(),
                "Field edge {} <-> {} found",
                key,
                other_side
            );

            self
        }

        pub fn edge_field(&self, key: &str) -> Option<&(EdgeReference, NodeIndex)> {
            let mut r = self
                .edges
                .iter()
                .filter(|(edge_ref, _to)| edge_ref.weight().display_name() == key);
            assert_eq!(
                r.clone().count(),
                1,
                "expected to find exactly one edge field named '{}', found {}, available fields: {:?}",
                key,
                r.clone().count(),
                self.edges.iter().map(|(e, _)| e.weight().display_name()).collect::<Vec<_>>()
            );

            r.nth(0)
        }

        pub fn edges_field(&self, key: &str) -> Vec<&(EdgeReference, NodeIndex)> {
            self.edges
                .iter()
                .filter(|(edge_ref, _to)| match edge_ref.weight() {
                    Edge::FieldMove(fm) => fm.name == key,
                    _ => false,
                })
                .collect()
        }

        pub fn edge(&self, key: &str, other_side: &str) -> Option<&(EdgeReference, NodeIndex)> {
            self.edges.iter().find(|(edge_ref, node_id)| {
                let edge = edge_ref.weight();
                let node = self.graph.node(*node_id).unwrap();
                let formatted_node = format!("{}", node);

                if node.is_using_provides() {
                    return edge.display_name() == key
                        && formatted_node.contains(&format!("{}/", other_side));
                }

                edge.display_name() == key && formatted_node == other_side
            })
        }

        pub fn assert_field_edge(&self, key: &str, other_side: &str) -> &Self {
            let edge = self.edge(key, other_side);

            assert!(
                edge.is_some(),
                "Field edge {} <-> {} not found",
                key,
                other_side
            );

            self
        }

        pub fn assert_interface_edge(&self, key: &str, other_side: &str) -> &Self {
            let edge = self.edge(key, other_side);

            assert!(
                edge.is_some(),
                "Interface edge {} <-> {} not found",
                key,
                other_side
            );

            self
        }
    }

    fn find_node_doesnt_exists(graph: &Graph, node_id: &str) {
        let node_res = graph.node_display_name_to_index.get(node_id);

        assert!(
            node_res.is_none(),
            "found node {} that should not exists",
            node_id
        );
    }

    fn find_node<'a>(graph: &'a Graph, node_id: &str) -> (FoundEdges<'a>, FoundEdges<'a>) {
        let node_res = graph.node_display_name_to_index.get(node_id);

        assert!(node_res.is_some(), "failed to find node {}", node_id);

        let node_index = node_res.unwrap();

        let incoming_edges = FoundEdges {
            edges: graph
                .edges_to(*node_index)
                .map(|edge_ref| (edge_ref, edge_ref.source().id()))
                .collect(),
            graph,
        };
        let outgoing_edges = FoundEdges {
            edges: graph
                .edges_from(*node_index)
                .map(|edge_ref| (edge_ref, edge_ref.target().id()))
                .collect(),
            graph,
        };

        (incoming_edges, outgoing_edges)
    }

    #[test]
    fn nested_provides() -> Result<(), Box<dyn std::error::Error>> {
        let supergraph_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixture/tests/nested-provides.supergraph.graphql");
        let graph = init_test(
            &std::fs::read_to_string(supergraph_path).expect("Unable to read input file"),
        );

        let (_, outgoing) = find_node(&graph, "Query/category");
        let field_edges = outgoing.edges_field("products");
        // one for provided, other one for regular
        assert_eq!(field_edges.len(), 2);

        // Provided ("viewed") field edge
        let (_, to) = field_edges
            .iter()
            .find(|(edge_ref, _to)| format!("{:?}", edge_ref.weight()) == "products @provides")
            .unwrap();

        let node = graph.node(*to)?;
        assert!(node.is_using_provides());
        assert_eq!(node.display_name(), "Product/category/1");

        let (_, viewed_outgoing) = find_node(&graph, &node.display_name());

        let (_, to) = viewed_outgoing
            .edge_field("categories")
            .expect("failed to find edge for field categories");
        let node1 = graph.node(*to)?;
        assert!(node1.is_using_provides());
        assert_eq!(node1.display_name(), "Category/category/1");

        // Regular field edge
        let (_, to_index) = field_edges
            .iter()
            .find(|(edge_ref, _to)| format!("{:?}", edge_ref.weight()) == "products")
            .unwrap();
        let node = graph.node(*to_index)?;
        assert_eq!(node.display_name(), "Product/category");
        assert!(!node.is_using_provides());

        find_node(&graph, "Product/category")
            .1
            .assert_field_edge("id", "ID/category");

        Ok(())
    }

    #[test]
    fn star_stuff() -> Result<(), Box<dyn std::error::Error>> {
        let supergraph_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixture/supergraph.graphql");
        let graph = init_test(
            &std::fs::read_to_string(supergraph_path).expect("Unable to read input file"),
        );

        // Field ownership: make sure fields defined where they belong
        find_node(&graph, "Product/reviews")
            .1
            .assert_field_edge("reviews", "Review/reviews");
        find_node(&graph, "Product/products")
            .1
            .no_field_edge("reviews");

        // external: true
        // Product.dimensions: ProductDimension @join__field(graph: inventory, external: true) @join__field(graph: products)
        let (_, outgoing) = find_node(&graph, "Product/products");
        outgoing
            .assert_field_edge("dimensions", "ProductDimension/products")
            .assert_field_edge_does_not_exist("dimensions", "ProductDimension/reviews")
            .assert_field_edge_does_not_exist("dimensions", "ProductDimension/users")
            .assert_field_edge_does_not_exist("dimensions", "ProductDimension/inventory");

        // User.totalProductsCreated: @shareable
        // Should be defined only in the relevant subgraphs.
        // Should not have nodes for types in other subgraphs.
        find_node(&graph, "User/products")
            .1
            .assert_field_edge("totalProductsCreated", "Int/products");
        find_node(&graph, "User/users")
            .1
            .assert_field_edge("totalProductsCreated", "Int/users");
        find_node_doesnt_exists(&graph, "User/reviews");
        find_node_doesnt_exists(&graph, "User/inventory");
        find_node_doesnt_exists(&graph, "User/PANDAS");

        // basic override
        // reviewsScore: Float! @join__field(graph: reviews, override: "products")
        find_node(&graph, "Product/products")
            .1
            .no_field_edge("reviewsScore");
        find_node(&graph, "Product/reviews")
            .1
            .assert_field_edge("reviewsScore", "Float/reviews");

        // Interface
        let (incoming, outgoing) = find_node(&graph, "ProductItf/products");

        incoming
            .assert_field_edge("product", "Query/products")
            .assert_field_edge("allProducts", "Query/products");
        outgoing
            .assert_field_edge("id", "ID/products")
            .assert_field_edge("variation", "ProductVariation/products")
            .assert_field_edge("dimensions", "ProductDimension/products")
            .assert_field_edge("hidden", "String/products")
            .assert_field_edge("name", "String/products")
            .assert_field_edge("oldField", "String/products")
            .assert_field_edge("package", "String/products")
            .assert_field_edge("sku", "String/products")
            .assert_field_edge("createdBy", "User/products")
            .no_field_edge("reviews");
        assert_eq!(incoming.edges.len(), 2);
        assert_eq!(outgoing.edges.len(), 10);

        // requires preserves selection set in the graph
        let outgoing = find_node(&graph, "Product/inventory").1;
        outgoing
            .assert_field_edge("delivery", "DeliveryEstimates/inventory")
            .edge("delivery", "DeliveryEstimates/inventory")
            .expect("cant find edge");

        Ok(())
    }

    // Sorry for the bad impl here, I wanted to make sure some nodes and edges are not breaking or duplicated.
    #[test]
    fn star_stuff_snapshot() {
        let supergraph_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixture/supergraph.graphql");
        let graph = init_test(
            &std::fs::read_to_string(supergraph_path).expect("Unable to read input file"),
        );

        // Validate root nodes
        assert_eq!(graph.root_query_node(), &Node::QueryRoot("Query".into()));
        assert_eq!(graph.root_mutation_node(), None);
        assert_eq!(graph.root_subscription_node(), None);

        let (incoming, outgoing) = find_node(&graph, "Product/products");
        assert_eq!(incoming.edges.len(), 11);
        assert_eq!(outgoing.edges.len(), 14);

        incoming
            .assert_key_edge("id", "Product/inventory")
            .assert_key_edge("sku package", "Product/inventory")
            .assert_key_edge("sku variation { id }", "Product/inventory")
            .assert_key_edge("id", "Product/products")
            .assert_key_edge("sku package", "Product/products")
            .assert_key_edge("sku variation { id }", "Product/products")
            .assert_key_edge("id", "Product/reviews")
            .assert_key_edge("sku package", "Product/reviews")
            .assert_key_edge("sku variation { id }", "Product/reviews")
            .assert_interface_edge("Product", "ProductItf/products")
            .assert_interface_edge("Product", "SkuItf/products");
        outgoing
            .assert_key_edge("id", "Product/inventory")
            .assert_key_edge("id", "Product/products")
            .assert_key_edge("id", "Product/reviews")
            .assert_key_edge("sku package", "Product/products")
            .assert_key_edge("sku variation { id }", "Product/products")
            .assert_field_edge("id", "ID/products")
            .assert_field_edge("variation", "ProductVariation/products")
            .assert_field_edge("dimensions", "ProductDimension/products")
            .assert_field_edge("hidden", "String/products")
            .assert_field_edge("name", "String/products")
            .assert_field_edge("oldField", "String/products")
            .assert_field_edge("package", "String/products")
            .assert_field_edge("sku", "String/products")
            .assert_field_edge("createdBy", "User/products");
    }

    #[test]
    fn multiple_provides() -> Result<(), Box<dyn std::error::Error>> {
        let supergraph_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixture/supergraph2.graphql");
        let graph = init_test(
            &std::fs::read_to_string(supergraph_path).expect("Unable to read input file"),
        );

        let (_, outgoing) = find_node(&graph, "Group/foo");
        // Multiple provides should create multiple edges, one for each "view"
        let (_, to) = outgoing
            .edges_field("users")
            .iter()
            .find(|(edge_ref, _to)| format!("{:?}", edge_ref.weight()) == "users @provides")
            .expect("failed to find edge for field users");
        let node1 = graph.node(*to)?;
        assert!(node1.is_using_provides());

        // Verify that each provided path points only to the relevant, provided fields
        let (viewed_incoming, viewed_outgoing) = find_node(&graph, &node1.display_name());
        viewed_outgoing.assert_field_edge("id", "String/foo");
        assert_eq!(viewed_incoming.edges.len(), 1);
        assert_eq!(viewed_outgoing.edges.len(), 1);

        let (_, to) = outgoing
            .edges_field("user")
            .iter()
            .find(|(edge_ref, _to)| format!("{:?}", edge_ref.weight()) == "user @provides")
            .expect("failed to find edge for field user");
        let node2 = graph.node(*to)?;
        assert!(node2.is_using_provides());

        // Verify that each provided path points only to the relevant, provided fields
        let (viewed_incoming, viewed_outgoing) = find_node(&graph, &node2.display_name());
        viewed_outgoing.assert_field_edge("name", "String/foo");
        assert_eq!(viewed_incoming.edges.len(), 1);
        assert_eq!(viewed_outgoing.edges.len(), 2);

        let (nested_provides_id, nested_provides_node) = viewed_outgoing
            .edge_field("profile")
            .map(|(_, node_index)| (*node_index, graph.node(*node_index).unwrap()))
            .expect("failed to located viewed node from profile field");

        assert!(nested_provides_node.is_using_provides());
        assert!(nested_provides_node
            .display_name()
            .starts_with("Profile/foo/"));

        let mut nested_edges = graph.edges_from(nested_provides_id);
        assert_eq!(nested_edges.clone().count(), 1);
        assert_eq!(
            nested_edges.next().unwrap().weight().display_name(),
            String::from("age")
        );

        // Two different views should be different nodes
        assert_ne!(node1, node2);

        Ok(())
    }
}
