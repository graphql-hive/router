#[cfg(test)]
mod star_stuff {
    use crate::{
        graph::{edge::Edge, node::Node, Graph},
        parse_schema,
        state::supergraph_state::SupergraphState,
    };
    use petgraph::visit::{EdgeRef, NodeRef};
    use std::path::PathBuf;

    fn init_test(supergraph_sdl: &str) -> Graph {
        let schema = parse_schema(supergraph_sdl);
        let metadata = SupergraphState::new(&schema);

        Graph::new_from_supergraph(&metadata)
    }

    #[derive(Debug)]
    struct FoundEdges<'a> {
        pub edges: Vec<(&'a Edge, String)>,
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
            let edge = self.edges.iter().find(|(e, _)| e.id() == key);

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

        pub fn edge_field(&self, key: &str) -> Option<&(&Edge, String)> {
            let mut r = self.edges.iter().filter(|(e, _target)| e.id() == key);
            assert_eq!(
                r.clone().count(),
                1,
                "expected to find exactly one edge field named '{}', found {}, available fields: {:?}",
                key,
                r.clone().count(),
                self.edges.iter().map(|(e, _)| e.id()).collect::<Vec<_>>()
            );

            r.nth(0)
        }

        pub fn edge(&self, key: &str, other_side: &str) -> Option<&(&Edge, String)> {
            self.edges
                .iter()
                .find(|(e, target)| e.id() == key && target == other_side)
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
        let node_res = graph.node_to_index.get(node_id);

        assert!(
            node_res.is_none(),
            "found node {} that should not exists",
            node_id
        );
    }

    fn find_node<'a>(graph: &'a Graph, node_id: &str) -> (FoundEdges<'a>, FoundEdges<'a>) {
        let node_res = graph.node_to_index.get(node_id);

        assert!(node_res.is_some(), "failed to find node {}", node_id);

        let node_index = node_res.unwrap();

        let incoming_edges = FoundEdges {
            edges: graph
                .edges_to(*node_index)
                .map(|v| (v.weight(), graph.graph[v.source().id()].id()))
                .collect(),
        };
        let outgoing_edges = FoundEdges {
            edges: graph
                .edges_from(*node_index)
                .map(|v| (v.weight(), graph.graph[v.target().id()].id()))
                .collect(),
        };

        (incoming_edges, outgoing_edges)
    }

    #[test]
    fn star_stuff() {
        let supergraph_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixture/supergraph.graphql");
        let graph = init_test(
            &std::fs::read_to_string(supergraph_path).expect("Unable to read input file"),
        );

        // Field ownership: make sure fields defined where they belong
        find_node(&graph, "Product/REVIEWS")
            .1
            .assert_field_edge("reviews", "Review/REVIEWS");
        find_node(&graph, "Product/PRODUCTS")
            .1
            .no_field_edge("reviews");

        // external: true
        // Product.dimensions: ProductDimension @join__field(graph: INVENTORY, external: true) @join__field(graph: PRODUCTS)
        let (_, outgoing) = find_node(&graph, "Product/PRODUCTS");
        outgoing
            .assert_field_edge("dimensions", "ProductDimension/PRODUCTS")
            .assert_field_edge_does_not_exist("dimensions", "ProductDimension/REVIEWS")
            .assert_field_edge_does_not_exist("dimensions", "ProductDimension/USERS")
            .assert_field_edge_does_not_exist("dimensions", "ProductDimension/INVENTORY");

        // User.totalProductsCreated: @shareable
        // Should be defined only in the relevant subgraphs.
        // Should not have nodes for types in other subgraphs.
        find_node(&graph, "User/PRODUCTS")
            .1
            .assert_field_edge("totalProductsCreated", "Int/PRODUCTS");
        find_node(&graph, "User/USERS")
            .1
            .assert_field_edge("totalProductsCreated", "Int/USERS");
        find_node_doesnt_exists(&graph, "User/REVIEWS");
        find_node_doesnt_exists(&graph, "User/INVENTORY");
        find_node_doesnt_exists(&graph, "User/PANDAS");

        // basic override
        // reviewsScore: Float! @join__field(graph: REVIEWS, override: "products")
        find_node(&graph, "Product/PRODUCTS")
            .1
            .no_field_edge("reviewsScore");
        find_node(&graph, "Product/REVIEWS")
            .1
            .assert_field_edge("reviewsScore", "Float/REVIEWS");

        // Interface
        let (incoming, outgoing) = find_node(&graph, "ProductItf/PRODUCTS");

        incoming
            .assert_field_edge("product", "Query/PRODUCTS")
            .assert_field_edge("allProducts", "Query/PRODUCTS");
        outgoing
            .assert_field_edge("id", "ID/PRODUCTS")
            .assert_field_edge("variation", "ProductVariation/PRODUCTS")
            .assert_field_edge("dimensions", "ProductDimension/PRODUCTS")
            .assert_field_edge("hidden", "String/PRODUCTS")
            .assert_field_edge("name", "String/PRODUCTS")
            .assert_field_edge("oldField", "String/PRODUCTS")
            .assert_field_edge("package", "String/PRODUCTS")
            .assert_field_edge("sku", "String/PRODUCTS")
            .assert_field_edge("createdBy", "User/PRODUCTS")
            .no_field_edge("reviews");
        assert_eq!(incoming.edges.len(), 2);
        assert_eq!(outgoing.edges.len(), 10);

        // requires preserves selection set in the graph
        let outgoing = find_node(&graph, "Product/INVENTORY").1;
        let (edge, _) = outgoing
            .assert_field_edge("delivery", "DeliveryEstimates/INVENTORY")
            .edge("delivery", "DeliveryEstimates/INVENTORY")
            .expect("cant find edge");
        assert_eq!(edge.requires(), Some("dimensions{size weight}"));
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

        let (incoming, outgoing) = find_node(&graph, "Product/PRODUCTS");
        assert_eq!(incoming.edges.len(), 11);
        assert_eq!(outgoing.edges.len(), 14);

        incoming
            .assert_key_edge("id", "Product/INVENTORY")
            .assert_key_edge("sku package", "Product/INVENTORY")
            .assert_key_edge("sku variation { id }", "Product/INVENTORY")
            .assert_key_edge("id", "Product/PRODUCTS")
            .assert_key_edge("sku package", "Product/PRODUCTS")
            .assert_key_edge("sku variation { id }", "Product/PRODUCTS")
            .assert_key_edge("id", "Product/REVIEWS")
            .assert_key_edge("sku package", "Product/REVIEWS")
            .assert_key_edge("sku variation { id }", "Product/REVIEWS")
            .assert_interface_edge("Product", "ProductItf/PRODUCTS")
            .assert_interface_edge("Product", "SkuItf/PRODUCTS");
        outgoing
            .assert_key_edge("id", "Product/INVENTORY")
            .assert_key_edge("id", "Product/PRODUCTS")
            .assert_key_edge("id", "Product/REVIEWS")
            .assert_key_edge("sku package", "Product/PRODUCTS")
            .assert_key_edge("sku variation { id }", "Product/PRODUCTS")
            .assert_field_edge("id", "ID/PRODUCTS")
            .assert_field_edge("variation", "ProductVariation/PRODUCTS")
            .assert_field_edge("dimensions", "ProductDimension/PRODUCTS")
            .assert_field_edge("hidden", "String/PRODUCTS")
            .assert_field_edge("name", "String/PRODUCTS")
            .assert_field_edge("oldField", "String/PRODUCTS")
            .assert_field_edge("package", "String/PRODUCTS")
            .assert_field_edge("sku", "String/PRODUCTS")
            .assert_field_edge("createdBy", "User/PRODUCTS");
    }

    #[test]
    fn multiple_provides() {
        let supergraph_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixture/supergraph2.graphql");
        let graph = init_test(
            &std::fs::read_to_string(supergraph_path).expect("Unable to read input file"),
        );

        let (_, outgoing) = find_node(&graph, "Group/FOO");
        // Multiple provides should create multiple edges, one for each "view"
        let (_, to) = outgoing
            .edge_field("users")
            .expect("failed to find edge for field users");
        let node1 = graph.node(*graph.node_to_index.get(to).unwrap());
        assert!(node1.is_view_node());

        // Verify that each provided path points only to the relevant, provided fields
        let (viewed_incoming, viewed_outgoing) = find_node(&graph, &node1.id());
        viewed_outgoing.assert_field_edge("id", "String/FOO");
        assert_eq!(viewed_incoming.edges.len(), 1);
        assert_eq!(viewed_outgoing.edges.len(), 1);

        let (_, to) = outgoing
            .edge_field("user")
            .expect("failed to find edge for field user");
        let node2 = graph.node(*graph.node_to_index.get(to).unwrap());
        assert!(node2.is_view_node());

        // Verify that each provided path points only to the relevant, provided fields
        let (viewed_incoming, viewed_outgoing) = find_node(&graph, &node2.id());
        viewed_outgoing.assert_field_edge("name", "String/FOO");
        assert_eq!(viewed_incoming.edges.len(), 1);
        assert_eq!(viewed_outgoing.edges.len(), 2);

        let nested_provides_id = viewed_outgoing
            .edge_field("profile")
            .map(|(_, key)| *graph.node_to_index.get(key).unwrap())
            .expect("failed to located viewed node from profile field");

        let nested_provides_node = graph.node(nested_provides_id);
        assert!(nested_provides_node.is_view_node());
        assert!(nested_provides_node.id().starts_with("(Profile/FOO)"));

        let mut nested_edges = graph.edges_from(nested_provides_id);
        assert_eq!(nested_edges.clone().count(), 1);
        assert_eq!(
            nested_edges.next().unwrap().weight().id(),
            String::from("age")
        );

        // Two different views should be different nodes
        assert_ne!(node1, node2);
    }
}
