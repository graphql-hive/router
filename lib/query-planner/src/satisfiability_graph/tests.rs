#[cfg(test)]
mod satisfiability_graph {
    use std::{path::PathBuf, process::id};

    use petgraph::{
        graph::{EdgeIndex, NodeIndex},
        visit::EdgeRef,
    };

    use crate::{
        parse_schema,
        satisfiability_graph::{
            edge::Edge,
            graph::{GraphQLSatisfiabilityGraph, GraphQLSatisfiabilityGraphError},
            node::Node,
        },
        supergraph_metadata::SupergraphMetadata,
    };

    fn init_test(
        supergraph_sdl: &str,
    ) -> Result<GraphQLSatisfiabilityGraph, GraphQLSatisfiabilityGraphError> {
        let schema = parse_schema(supergraph_sdl);
        let metadata = SupergraphMetadata::new(&schema);

        GraphQLSatisfiabilityGraph::new_from_supergraph(&metadata)
    }

    struct FindResult<'a> {
        pub from: (NodeIndex, &'a Node),
        pub to: (NodeIndex, &'a Node),
        pub edges: Vec<(EdgeIndex, &'a Edge)>,
    }

    impl FindResult<'_> {
        pub fn assert_field_node_exists_once(&self, field_name: &str) -> &Self {
            assert!(
                self.edges.iter().any(|v| v.1.id() == field_name),
                "Field edge '{}' not found between {} and {}",
                field_name,
                self.from.1.id(),
                self.to.1.id()
            );

            self
        }

        pub fn assert_entity_reference_node_exists_once(&self, selection_set: &str) -> &Self {
            assert!(
                self.edges.iter().any(|v| match v.1 {
                    Edge::EntityReference(key_edge) => key_edge == selection_set,
                    _ => false,
                }),
                "Entity reference edge not found between {} and {}",
                self.from.1.id(),
                self.to.1.id()
            );

            self
        }

        pub fn assert_interface_impl_node_exists_once(&self) -> &Self {
            assert!(
                self.edges.iter().any(|v| match v.1 {
                    // TODO: Replace _ with actual interface name?
                    Edge::InterfaceImplementation(_) => true,
                    _ => false,
                }),
                "Interface implementation edge not found between {} and {}",
                self.from.1.id(),
                self.to.1.id()
            );

            self
        }
    }

    fn validate_incoming_edges_count<'a>(
        graph: &'a GraphQLSatisfiabilityGraph,
        node: (&str, &str),
        expectec_count: usize,
    ) {
        let (node_index, _) = graph.find_definition_node(node.0, node.1).unwrap();
        let edges = graph.edges_to(node_index);
        let count = edges.clone().count();

        println!(
            "node: {}/{}, incoming edges: {:?}",
            node.0,
            node.1,
            edges.map(|v| v.weight().id()).collect::<Vec<_>>()
        );

        assert_eq!(count, expectec_count);
    }

    fn validate_outgoing_edges_count<'a>(
        graph: &'a GraphQLSatisfiabilityGraph,
        node: (&str, &str),
        expectec_count: usize,
    ) {
        let (node_index, _) = graph.find_definition_node(node.0, node.1).unwrap();
        let edges = graph.edges_from(node_index);
        let count = edges.clone().count();

        println!(
            "node: {}/{}, outgoing edges: {:?}",
            node.0,
            node.1,
            edges.map(|v| v.weight().id()).collect::<Vec<_>>()
        );

        assert_eq!(count, expectec_count);
    }

    fn validate_connection<'a>(
        graph: &'a GraphQLSatisfiabilityGraph,
        from: (&str, &str),
        to: (&str, &str),
    ) -> FindResult<'a> {
        let (from_node_index, _from_node) = graph
            .find_definition_node(from.0, from.1)
            .unwrap_or_else(|| {
                panic!(
                    "validate_connection: failde to locate 'from' node: {}/{}",
                    from.0, from.1
                )
            });
        let (to_node_index, _to_node) =
            graph.find_definition_node(to.0, to.1).unwrap_or_else(|| {
                panic!(
                    "validate_connection: failde to locate 'to' node: {}/{}",
                    to.0, to.1
                )
            });
        let edges = graph.edges_from(from_node_index);

        FindResult {
            from: (from_node_index, _from_node),
            to: (to_node_index, _to_node),
            edges: edges
                .filter(|edge| edge.target() == to_node_index)
                .map(|edge_ref| (edge_ref.id(), edge_ref.weight()))
                .collect(),
        }
    }

    #[test]
    fn star_stuff() {
        let supergraph_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixture/supergraph.graphql");
        let graph = init_test(
            &std::fs::read_to_string(supergraph_path).expect("Unable to read input file"),
        )
        .expect("failed to build graph");

        // Validate root nodes
        assert_eq!(graph.root_query_node(), &Node::QueryRoot("Query".into()));
        assert_eq!(graph.root_mutation_node(), None);
        assert_eq!(graph.root_subscription_node(), None);

        // validate the count expected count of incoming/outgoing edges
        validate_incoming_edges_count(&graph, ("ProductItf", "INVENTORY"), 2);
        validate_outgoing_edges_count(&graph, ("ProductItf", "INVENTORY"), 2);

        validate_incoming_edges_count(&graph, ("ProductItf", "PRODUCTS"), 2);
        // validate_outgoing_edges_count(&graph, ("String", "PRODUCTS"), 7);

        validate_connection(
            &graph,
            ("Product", "INVENTORY"),
            ("ProductDimension", "INVENTORY"),
        )
        .assert_field_node_exists_once("dimensions");

        validate_connection(
            &graph,
            ("Product", "INVENTORY"),
            ("DeliveryEstimates", "INVENTORY"),
        )
        .assert_field_node_exists_once("delivery");

        validate_connection(&graph, ("Product", "INVENTORY"), ("Product", "REVIEWS"))
            .assert_entity_reference_node_exists_once("id");

        validate_connection(&graph, ("Product", "INVENTORY"), ("Product", "PRODUCTS"))
            .assert_entity_reference_node_exists_once("sku variation { id }")
            .assert_entity_reference_node_exists_once("sku package")
            .assert_entity_reference_node_exists_once("id");

        // ProductDimension/inventory connections
        validate_connection(
            &graph,
            ("ProductDimension", "INVENTORY"),
            ("String", "INVENTORY"),
        )
        .assert_field_node_exists_once("size");

        validate_connection(
            &graph,
            ("ProductDimension", "INVENTORY"),
            ("Float", "INVENTORY"),
        )
        .assert_field_node_exists_once("weight");

        // DeliveryEstimates/inventory connections
        validate_connection(
            &graph,
            ("DeliveryEstimates", "INVENTORY"),
            ("String", "INVENTORY"),
        )
        .assert_field_node_exists_once("estimatedDelivery")
        .assert_field_node_exists_once("fastestDelivery");

        // ProductItf/inventory connections
        validate_connection(
            &graph,
            ("ProductItf", "INVENTORY"),
            ("Product", "INVENTORY"),
        )
        .assert_interface_impl_node_exists_once();

        // validate_connection(&graph, ("ProductItf", "INVENTORY"), ("ID", "INVENTORY"))
        //     .assert_field_node_exists_once("id");

        // Query/pandas connections

        validate_connection(&graph, ("Query", "PANDAS"), ("Panda", "PANDAS"))
            .assert_field_node_exists_once("allPandas")
            .assert_field_node_exists_once("panda");

        // Panda/pandas connections
        validate_connection(&graph, ("Panda", "PANDAS"), ("ID", "PANDAS"))
            .assert_field_node_exists_once("name");

        validate_connection(&graph, ("Panda", "PANDAS"), ("String", "PANDAS"))
            .assert_field_node_exists_once("favoriteFood");

        // Query/products connections

        validate_connection(&graph, ("Query", "PRODUCTS"), ("ProductItf", "PRODUCTS"))
            .assert_field_node_exists_once("allProducts")
            .assert_field_node_exists_once("product");

        validate_connection(&graph, ("Query", "PRODUCTS"), ("ProductItf", "REVIEWS"))
            .assert_field_node_exists_once("product")
            .assert_field_node_exists_once("allProducts");

        validate_connection(&graph, ("Query", "PRODUCTS"), ("ProductItf", "INVENTORY"))
            .assert_field_node_exists_once("product")
            .assert_field_node_exists_once("allProducts");

        // ProductItf/products connections
        validate_connection(&graph, ("ProductItf", "PRODUCTS"), ("Product", "PRODUCTS"))
            .assert_interface_impl_node_exists_once();

        validate_connection(&graph, ("ProductItf", "PRODUCTS"), ("ID", "PRODUCTS"))
            .assert_field_node_exists_once("id");

        validate_connection(&graph, ("ProductItf", "PRODUCTS"), ("String", "PRODUCTS"))
            .assert_field_node_exists_once("sku")
            .assert_field_node_exists_once("package")
            .assert_field_node_exists_once("hidden")
            .assert_field_node_exists_once("oldField");

        // Note: There's a typo in the graph "Stpring/products", assuming it should be "String/products"
        validate_connection(&graph, ("ProductItf", "PRODUCTS"), ("String", "PRODUCTS"))
            .assert_field_node_exists_once("name");

        // Product/products connections
        validate_connection(&graph, ("Product", "PRODUCTS"), ("ID", "PRODUCTS"))
            .assert_field_node_exists_once("id");

        validate_connection(
            &graph,
            ("Product", "PRODUCTS"),
            ("ProductDimension", "PRODUCTS"),
        )
        .assert_field_node_exists_once("dimensions");

        validate_connection(&graph, ("Product", "PRODUCTS"), ("String", "PRODUCTS"))
            .assert_field_node_exists_once("sku")
            .assert_field_node_exists_once("package")
            .assert_field_node_exists_once("name")
            .assert_field_node_exists_once("hidden")
            .assert_field_node_exists_once("oldField");

        validate_connection(
            &graph,
            ("Product", "PRODUCTS"),
            ("ProductVariation", "PRODUCTS"),
        )
        .assert_field_node_exists_once("variation");

        validate_connection(&graph, ("Product", "PRODUCTS"), ("User", "PRODUCTS"))
            .assert_field_node_exists_once("createdBy");

        validate_connection(&graph, ("Product", "PRODUCTS"), ("Float", "PRODUCTS"))
            .assert_field_node_exists_once("reviewsScore");

        validate_connection(&graph, ("Product", "PRODUCTS"), ("Product", "REVIEWS"))
            .assert_entity_reference_node_exists_once("id");

        validate_connection(&graph, ("Product", "PRODUCTS"), ("Product", "INVENTORY"))
            .assert_entity_reference_node_exists_once("id");

        // ProductDimension/products connections
        validate_connection(
            &graph,
            ("ProductDimension", "PRODUCTS"),
            ("String", "PRODUCTS"),
        )
        .assert_field_node_exists_once("size");

        validate_connection(
            &graph,
            ("ProductDimension", "PRODUCTS"),
            ("Float", "PRODUCTS"),
        )
        .assert_field_node_exists_once("weight");

        // ProductVariation/products connections
        validate_connection(&graph, ("ProductVariation", "PRODUCTS"), ("ID", "PRODUCTS"))
            .assert_field_node_exists_once("id");

        validate_connection(
            &graph,
            ("ProductVariation", "PRODUCTS"),
            ("String", "PRODUCTS"),
        )
        .assert_field_node_exists_once("name");

        // User/products connections
        validate_connection(&graph, ("User", "PRODUCTS"), ("ID", "PRODUCTS"))
            .assert_field_node_exists_once("email");

        validate_connection(&graph, ("User", "PRODUCTS"), ("Int", "PRODUCTS"))
            .assert_field_node_exists_once("totalProductsCreated");

        validate_connection(&graph, ("User", "PRODUCTS"), ("User", "USERS"))
            .assert_entity_reference_node_exists_once("email");

        // SkuItf/products connections
        validate_connection(&graph, ("SkuItf", "PRODUCTS"), ("Product", "PRODUCTS"))
            .assert_interface_impl_node_exists_once();

        validate_connection(&graph, ("SkuItf", "PRODUCTS"), ("String", "PRODUCTS"))
            .assert_field_node_exists_once("sku");

        // Query/reviews connections
        validate_connection(&graph, ("Query", "REVIEWS"), ("Review", "REVIEWS"))
            .assert_field_node_exists_once("review");

        // Review/reviews connections
        validate_connection(&graph, ("Review", "REVIEWS"), ("Int", "REVIEWS"))
            .assert_field_node_exists_once("id");

        validate_connection(&graph, ("Review", "REVIEWS"), ("String", "REVIEWS"))
            .assert_field_node_exists_once("body");

        // Product/reviews connections
        validate_connection(&graph, ("Product", "REVIEWS"), ("ID", "REVIEWS"))
            .assert_field_node_exists_once("id");

        validate_connection(&graph, ("Product", "REVIEWS"), ("Float", "REVIEWS"))
            .assert_field_node_exists_once("reviewsScore");

        validate_connection(&graph, ("Product", "REVIEWS"), ("Int", "REVIEWS"))
            .assert_field_node_exists_once("reviewsCount");

        validate_connection(&graph, ("Product", "REVIEWS"), ("Review", "REVIEWS"))
            .assert_field_node_exists_once("reviews");

        validate_connection(&graph, ("Product", "REVIEWS"), ("Product", "PRODUCTS"))
            .assert_entity_reference_node_exists_once("sku variation { id }")
            .assert_entity_reference_node_exists_once("sku package")
            .assert_entity_reference_node_exists_once("id");

        validate_connection(&graph, ("Product", "REVIEWS"), ("Product", "INVENTORY"))
            .assert_entity_reference_node_exists_once("id");

        // ProductItf/reviews connections
        validate_connection(&graph, ("ProductItf", "REVIEWS"), ("Product", "REVIEWS"))
            .assert_interface_impl_node_exists_once();

        validate_connection(&graph, ("ProductItf", "REVIEWS"), ("ID", "REVIEWS"))
            .assert_field_node_exists_once("id");

        validate_connection(&graph, ("ProductItf", "REVIEWS"), ("Int", "REVIEWS"))
            .assert_field_node_exists_once("reviewsCount");

        validate_connection(&graph, ("ProductItf", "REVIEWS"), ("Float", "REVIEWS"))
            .assert_field_node_exists_once("reviewsScore");

        // User/users connections
        validate_connection(&graph, ("User", "USERS"), ("ID", "USERS"))
            .assert_field_node_exists_once("email");

        validate_connection(&graph, ("User", "USERS"), ("Int", "USERS"))
            .assert_field_node_exists_once("totalProductsCreated");

        validate_connection(&graph, ("User", "USERS"), ("String", "USERS"))
            .assert_field_node_exists_once("name");

        validate_connection(&graph, ("User", "USERS"), ("User", "PRODUCTS"))
            .assert_entity_reference_node_exists_once("email");
    }
}
