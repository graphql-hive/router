#[cfg(test)]
mod context_tests {
    use crate::query_planner::{
        graph::{edge::Edge, Graph},
        state::supergraph_state::SupergraphState,
        tests::testkit::read_supergraph,
        utils::parsing::parse_schema,
    };

    #[test]
    fn context_cost_only_charges_the_resolved_ancestor_selection() {
        use crate::query_planner::{
            ast::normalization::normalize_operation,
            planner::walker::walk_operation,
            utils::{cancellation::CancellationToken, parsing::parse_operation},
        };

        let sdl = r#"
            enum join__Graph {
                A @join__graph(name: "a", url: "")
                B @join__graph(name: "b", url: "")
            }
            type Query @join__type(graph: A) { user: User }
            type User @join__type(graph: A, key: "id") @join__type(graph: B, key: "id") @context(name: "a__ctx") {
                id: ID!
                currency: String @join__field(graph: A)
                tx: Transaction @join__field(graph: A)
            }
            type Other @join__type(graph: A) @context(name: "a__ctx") { currency: String }
            type Transaction @join__type(graph: A, key: "id") {
                id: ID!
                amount: Int @join__field(graph: A, contextArguments: [
                    {context: "a__ctx", name: "code", type: "String", selection: "{ currency }"}
                ])
            }
        "#;
        for (sdl, expected_cost) in [
            (sdl.to_string(), 1),
            (
                sdl.replace(
                    "currency: String @join__field(graph: A)",
                    "currency: String @join__field(graph: B)",
                ),
                1003,
            ),
            (
                sdl.replace(
                    "selection: \"{ currency }\"",
                    "selection: \"... on Other { currency }\"",
                ),
                0,
            ),
        ] {
            let supergraph = SupergraphState::new(&parse_schema(&sdl));
            let graph = Graph::graph_from_supergraph_state(&supergraph).unwrap();
            let query = normalize_operation(
                &supergraph,
                &parse_operation("{ user { tx { amount } } }"),
                None,
            )
            .unwrap();
            let override_context = Default::default();
            let cancellation = CancellationToken::new();
            let paths = walk_operation(
                &graph,
                &supergraph,
                &override_context,
                query.executable_operation(),
                &cancellation,
            )
            .unwrap();
            let path = &paths.root_field_groups[0][0][0];
            let segments = path.get_segments();
            let context = &segments.last().unwrap().context_requirements()[0];
            assert_eq!(context.cost, expected_cost);
            assert_eq!(path.cost, 1003 + expected_cost);
        }
    }

    #[test]
    fn context_metadata_is_added_without_charging_all_ancestor_types() {
        let supergraph = SupergraphState::new(&parse_schema(&read_supergraph(
            "fixture/tests/context-edge-cases.supergraph.graphql",
        )));
        let graph = Graph::graph_from_supergraph_state(&supergraph).unwrap();
        let node = graph
            .graph
            .node_indices()
            .find(|&index| graph.graph[index].display_name() == "Transaction/a")
            .unwrap();
        let edge = graph
            .edges_from(node)
            .find(|edge| edge.weight().display_name() == "amount")
            .unwrap();
        let Edge::FieldMove(field) = edge.weight() else {
            panic!("expected field move")
        };
        let argument = &field.context_arguments[0];
        assert_eq!(argument.name, "code");
        assert_eq!(argument.type_in_graph.to_string(), "String");
        assert_eq!(argument.selections.len(), 3);
        let interface_selection = argument
            .selections
            .iter()
            .find(|selection| selection.type_name == "Account")
            .unwrap();
        assert_eq!(
            interface_selection.selection_set.to_string(),
            "{...on User{currency}}"
        );
        assert!(argument
            .selections
            .iter()
            .any(|selection| selection.type_name == "User"));
        assert_eq!(edge.weight().cost(), 1);
    }
}

#[cfg(test)]
mod graph_tests {
    use crate::query_planner::{
        graph::{
            edge::{Edge, EdgeReference},
            node::Node,
            Graph,
        },
        state::supergraph_state::SupergraphState,
        utils::parsing::parse_schema,
    };
    use ahash::{HashSet, HashSetExt};
    use petgraph::{
        graph::NodeIndex,
        visit::{EdgeRef, NodeRef},
    };
    use std::path::PathBuf;

    fn init_test(supergraph_sdl: &str) -> Graph<'static> {
        let schema = parse_schema(supergraph_sdl);
        let metadata: &'static _ = Box::leak(Box::new(SupergraphState::new(&schema)));

        Graph::graph_from_supergraph_state(metadata).expect("failed to create graph")
    }

    #[derive(Debug)]
    struct FoundEdges<'a> {
        pub edges: Vec<(EdgeReference<'a>, NodeIndex)>,
        pub graph: &'a Graph<'a>,
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

        pub fn edge_field<'a>(&'a self, key: &str) -> Option<&'a (EdgeReference<'a>, NodeIndex)> {
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

        pub fn edges_field<'a>(&'a self, key: &str) -> Vec<&'a (EdgeReference<'a>, NodeIndex)> {
            self.edges
                .iter()
                .filter(|(edge_ref, _to)| match edge_ref.weight() {
                    Edge::FieldMove(fm) => fm.name == key,
                    _ => false,
                })
                .collect()
        }

        pub fn edge<'a>(
            &'a self,
            key: &str,
            other_side: &str,
        ) -> Option<&'a (EdgeReference<'a>, NodeIndex)> {
            self.edges.iter().find(|(edge_ref, node_id)| {
                let edge = edge_ref.weight();
                let node = self.graph.node(*node_id).unwrap();
                let formatted_node = format!("{}", node);

                // A copy is named after its plain node, plus what's provided on it.
                if node.is_using_provides() {
                    return edge.display_name() == key
                        && formatted_node.starts_with(&format!("{}{{", other_side));
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

    fn find_node_index(graph: &Graph<'_>, node_id: &str) -> Option<NodeIndex> {
        graph
            .graph
            .node_indices()
            .find(|&index| graph.graph[index].display_name() == node_id)
    }

    fn find_node_doesnt_exists(graph: &Graph<'_>, node_id: &str) {
        let node_res = find_node_index(graph, node_id);

        assert!(
            node_res.is_none(),
            "found node {} that should not exists",
            node_id
        );
    }

    fn find_node<'a>(graph: &'a Graph<'a>, node_id: &str) -> (FoundEdges<'a>, FoundEdges<'a>) {
        let node_res = find_node_index(graph, node_id);

        assert!(node_res.is_some(), "failed to find node {}", node_id);

        let node_index = node_res.unwrap();

        let incoming_edges = FoundEdges {
            edges: graph
                .edges_to(node_index)
                .map(|edge_ref| (edge_ref, edge_ref.source().id()))
                .collect(),
            graph,
        };
        let outgoing_edges = FoundEdges {
            edges: graph
                .edges_from(node_index)
                .map(|edge_ref| (edge_ref, edge_ref.target().id()))
                .collect(),
            graph,
        };

        (incoming_edges, outgoing_edges)
    }

    /// Name of the node the only `edge_name` edge of `node_id` leads to.
    fn follow(graph: &Graph<'_>, node_id: &str, edge_name: &str) -> String {
        let (_, outgoing) = find_node(graph, node_id);
        let (_, to) = outgoing
            .edge_field(edge_name)
            .unwrap_or_else(|| panic!("no {} edge on {}", edge_name, node_id));
        graph.node(*to).unwrap().display_name()
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
        // The field edge points at the copy. There is no second, plain `products` edge.
        assert_eq!(field_edges.len(), 1);

        // Provided ("viewed") field edge
        let (_, to) = field_edges
            .iter()
            .find(|(edge_ref, _to)| format!("{:?}", edge_ref.weight()) == "products @provides")
            .unwrap();

        let node = graph.node(*to)?;
        assert!(node.is_using_provides());
        assert_eq!(
            node.display_name(),
            "Product/category{categories{subCategories}}"
        );

        let (_, viewed_outgoing) = find_node(&graph, &node.display_name());

        let (_, to) = viewed_outgoing
            .edge_field("categories")
            .expect("failed to find edge for field categories");
        let node1 = graph.node(*to)?;
        assert!(node1.is_using_provides());
        assert_eq!(node1.display_name(), "Category/category{subCategories}");

        // The plain node still exists (other edges lead there), and the copy has its fields.
        find_node(&graph, "Product/category")
            .1
            .assert_field_edge("id", "ID/category");
        find_node(&graph, &follow(&graph, "Query/category", "products"))
            .1
            .assert_field_edge("id", "ID/category");

        Ok(())
    }

    // A copy is keyed by its plain node and what's provided on it.
    #[test]
    fn provides_copies_are_keyed_by_what_they_provide() -> Result<(), Box<dyn std::error::Error>> {
        let supergraph_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixture/tests/provides-copies.supergraph.graphql");
        let graph = init_test(
            &std::fs::read_to_string(supergraph_path).expect("Unable to read input file"),
        );

        // `User.orders` is handled before `ZOrder.product`, its copy still gets `name`. The
        // plain `ZOrder` and its copy share the one `Product` copy.
        let orders = follow(&graph, "User/orders", "orders");
        assert_eq!(orders, "ZOrder/orders{sku}");
        assert_eq!(follow(&graph, &orders, "product"), "Product/orders{name}");
        assert_eq!(
            follow(&graph, "ZOrder/orders", "product"),
            "Product/orders{name}"
        );

        // `User.related: User` provides `name` again, so it comes back to the copy it's on.
        let related = follow(&graph, "User/social", "related");
        assert_eq!(related, "User/social{name}");
        assert_eq!(follow(&graph, &related, "related"), related);

        Ok(())
    }

    // Returning the same type isn't enough to keep what's provided. `related` provides `name`
    // again, `other` doesn't.
    #[test]
    fn provides_copy_keeps_fields_only_where_they_are_provided(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let supergraph_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixture/tests/requires-nested-same-type.supergraph.graphql");
        let graph = init_test(
            &std::fs::read_to_string(supergraph_path).expect("Unable to read input file"),
        );

        let related = follow(&graph, "User/social", "related");
        assert_eq!(related, "User/social{name}");
        assert_eq!(follow(&graph, &related, "related"), related);
        assert_eq!(follow(&graph, &related, "other"), "User/social");

        Ok(())
    }

    // A union field has an edge per member. Every one of them goes to its copy, and keeps the
    // field's @override label and @provides.
    #[test]
    fn provides_on_union_field_redirects_every_member_edge(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let supergraph_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixture/tests/provides-union-progressive-override.supergraph.graphql");
        let graph = init_test(
            &std::fs::read_to_string(supergraph_path).expect("Unable to read input file"),
        );

        let (_, outgoing) = find_node(&graph, "User/b");
        let mut targets = vec![];
        for (edge_ref, to) in outgoing.edges_field("media") {
            let Edge::FieldMove(field_move) = edge_ref.weight() else {
                unreachable!()
            };
            assert!(field_move.override_label.is_some());
            assert!(field_move.join_field.as_ref().unwrap().provides.is_some());
            targets.push(graph.node(*to)?.display_name());
        }
        targets.sort();
        assert_eq!(
            targets,
            vec![
                "Media/b for User.media:Book{...on Book{title}}",
                "Media/b for User.media:Movie{...on Movie{title}}",
            ]
        );

        // a's edges are the overridden side.
        let (_, outgoing) = find_node(&graph, "User/a");
        for (edge_ref, _) in outgoing.edges_field("media") {
            let Edge::FieldMove(field_move) = edge_ref.weight() else {
                unreachable!()
            };
            assert!(matches!(&field_move.overridden_by, Some((b, Some(_))) if b == "b"));
        }

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
            .assert_field_edge("__typename", "String/products")
            .no_field_edge("reviews");
        assert_eq!(incoming.edges.len(), 3);
        assert_eq!(outgoing.edges.len(), 12);

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
        // One 🤳 edge, even though Product has three @join__type(graph: PRODUCTS).
        assert_eq!(incoming.edges.len(), 12);
        assert_eq!(outgoing.edges.len(), 16);

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
            .assert_field_edge("createdBy", "User/products")
            .assert_field_edge("__typename", "String/products");
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

        // A copy has every edge of the plain node, plus the provided fields.
        let plain_edges = find_node(&graph, "User/foo").1.edges.len();
        let (viewed_incoming, viewed_outgoing) = find_node(&graph, &node1.display_name());
        viewed_outgoing.assert_field_edge("id", "String/foo");
        assert_eq!(viewed_incoming.edges.len(), 2); // +1 for Selfie
        assert_eq!(viewed_outgoing.edges.len(), plain_edges + 1); // +1 for id

        let (_, to) = outgoing
            .edges_field("user")
            .iter()
            .find(|(edge_ref, _to)| format!("{:?}", edge_ref.weight()) == "user @provides")
            .expect("failed to find edge for field user");
        let node2 = graph.node(*to)?;
        assert!(node2.is_using_provides());

        let (viewed_incoming, viewed_outgoing) = find_node(&graph, &node2.display_name());
        viewed_outgoing.assert_field_edge("name", "String/foo");
        assert_eq!(viewed_incoming.edges.len(), 2); // +1 for Selfie
        assert_eq!(viewed_outgoing.edges.len(), plain_edges + 2); // +1 for name, +1 for profile

        // `Profile.age` is a local field in `foo`, so `profile { age }` adds nothing below
        // `profile`, and it leads to the plain node.
        let (_, profile) = viewed_outgoing
            .edge_field("profile")
            .expect("failed to locate the provided profile field");
        let profile = graph.node(*profile)?;
        assert!(!profile.is_using_provides());
        find_node(&graph, &profile.display_name())
            .1
            .assert_field_edge("age", "Int/foo");

        // Two different views should be different nodes
        assert_ne!(node1, node2);

        Ok(())
    }

    // https://github.com/graphql-hive/router/issues/1455
    // `__typename` is a meta-field, not part of the fields set in the schema, so a `@provides`
    // fieldset containing it must not fail when constructing the graph.
    #[test]
    fn provides_fieldset_with_typename() {
        let supergraph_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixture/tests/provides-typename.supergraph.graphql");
        let schema = parse_schema(
            &std::fs::read_to_string(supergraph_path).expect("Unable to read input file"),
        );
        let metadata = SupergraphState::new(&schema);
        let graph = Graph::graph_from_supergraph_state(&metadata);

        assert!(
            graph.is_ok(),
            "expected __typename inside a @provides fieldset to plan successfully, got: {:?}",
            graph.err()
        );
    }

    // __typename at two nested levels of the same @provides fieldset.
    #[test]
    fn provides_fieldset_with_nested_typename() -> Result<(), Box<dyn std::error::Error>> {
        let supergraph_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixture/tests/provides-typename-nested.supergraph.graphql");
        let graph = init_test(
            &std::fs::read_to_string(supergraph_path).expect("Unable to read input file"),
        );

        let holder = follow(&graph, "Query/provider", "holder");
        let item = follow(&graph, &holder, "item");
        find_node(&graph, &item)
            .1
            .assert_field_edge("__typename", "String/provider")
            .assert_field_edge("nested", "Nested/provider");

        find_node(&graph, &follow(&graph, &item, "nested"))
            .1
            .assert_field_edge("__typename", "String/provider")
            .assert_field_edge("label", "String/provider");

        Ok(())
    }

    // __typename inside a @provides fieldset nested under a list-returning field.
    #[test]
    fn provides_fieldset_with_typename_on_list() -> Result<(), Box<dyn std::error::Error>> {
        let supergraph_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixture/tests/provides-typename-list.supergraph.graphql");
        let graph = init_test(
            &std::fs::read_to_string(supergraph_path).expect("Unable to read input file"),
        );

        let (_, outgoing) = find_node(&graph, &follow(&graph, "Query/provider", "holder"));
        let (items_edge, to) = outgoing
            .edge_field("items")
            .expect("failed to find edge for field items");
        match items_edge.weight() {
            Edge::FieldMove(fm) => {
                assert_eq!(fm.list_depth, 1, "expected 'items' field move to be a list")
            }
            other => panic!("expected a field move edge, got {:?}", other),
        }

        let node = graph.node(*to)?;
        assert!(node.is_using_provides());

        find_node(&graph, &node.display_name())
            .1
            .assert_field_edge("__typename", "String/provider")
            .assert_field_edge("label", "String/provider");

        Ok(())
    }

    // __typename both at the interface level and inside an inline fragment
    // branch of the same @provides fieldset.
    #[test]
    fn provides_fieldset_with_typename_on_interface() -> Result<(), Box<dyn std::error::Error>> {
        let supergraph_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixture/tests/provides-typename-interface.supergraph.graphql");
        let graph = init_test(
            &std::fs::read_to_string(supergraph_path).expect("Unable to read input file"),
        );

        let book = follow(&graph, "Query/a", "book");
        let animals = follow(&graph, &book, "animals");
        find_node(&graph, &animals)
            .1
            .assert_field_edge("__typename", "String/a")
            .assert_interface_edge("Dog", "Dog/a");

        find_node(&graph, &follow(&graph, &animals, "Dog"))
            .1
            .assert_field_edge("__typename", "String/a")
            .assert_field_edge("name", "String/a");

        Ok(())
    }

    // __typename inside a @provides fieldset for a field that also carries @requires.
    #[test]
    fn provides_fieldset_with_typename_and_requires() -> Result<(), Box<dyn std::error::Error>> {
        let supergraph_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixture/tests/provides-requires-typename.supergraph.graphql");
        let graph = init_test(
            &std::fs::read_to_string(supergraph_path).expect("Unable to read input file"),
        );

        let (_, outgoing) = find_node(&graph, "Review/reviews");
        let (_, to) = outgoing
            .edges_field("author")
            .into_iter()
            .find(|(edge_ref, _to)| {
                format!("{:?}", edge_ref.weight()) == "author @requires(secret) @provides"
            })
            .expect("failed to find provides edge for field author");
        let node = graph.node(*to)?;
        assert!(node.is_using_provides());

        find_node(&graph, &node.display_name())
            .1
            .assert_field_edge("username", "String/reviews")
            .assert_field_edge("__typename", "String/reviews");

        Ok(())
    }

    #[test]
    fn repeated_join_type_adds_each_edge_once() {
        let supergraph_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixture/tests/keys-mashup.supergraph.graphql");
        let graph = init_test(
            &std::fs::read_to_string(supergraph_path).expect("Unable to read input file"),
        );

        for node_id in ["A/a", "A/b"] {
            let (_, outgoing) = find_node(&graph, node_id);
            let mut seen = HashSet::new();
            for (edge_ref, to) in &outgoing.edges {
                let edge = format!("{:?}", edge_ref.weight());
                assert!(
                    seen.insert((edge.clone(), *to)),
                    "edge {} on {} added twice",
                    edge,
                    node_id
                );
            }
            assert_eq!(outgoing.edges_field("id").len(), 1);
            assert_eq!(outgoing.edges_field("__typename").len(), 1);
        }
    }
}
