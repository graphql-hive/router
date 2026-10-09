#[cfg(test)]
mod context_tests {
    use crate::query_planner::{
        ast::selection_set::FieldSelection,
        graph::{
            edge::{Edge, PlannerOverrideContext},
            Graph,
        },
        planner::walker::{
            path::OperationPath,
            pathfinder::{find_direct_paths, find_indirect_paths, NavigationTarget},
        },
        state::supergraph_state::SupergraphState,
        utils::{cancellation::CancellationToken, parsing::parse_schema},
    };
    const SUPERGRAPH: &str = r#"
        enum join__Graph {
            A @join__graph(name: "a", url: "http://a")
            B @join__graph(name: "b", url: "http://b")
        }
        type Query @join__type(graph: A) {
            users: [User]
            transaction: Transaction
            account: Account
            viewer: Viewer
            outer: Outer
            broken: Broken
        }
        interface Account @join__type(graph: A) @context(name: "a__account") {
            currency: String
            txs: [Transaction]
        }
        union Viewer @join__type(graph: A)
            @join__unionMember(graph: A, member: "User")
            @join__unionMember(graph: A, member: "Other")
            @context(name: "a__viewer") = User | Other
        type User implements Account
            @join__type(graph: A, key: "id")
            @join__implements(graph: A, interface: "Account")
            @context(name: "a__user") {
            id: ID!
            currency: String
            child: User
            wrapper: Wrapper
            broken: Broken
            txs: [Transaction]
        }
        type Other @join__type(graph: A) {
            currency: String
            txs: [Transaction]
        }
        type Wrapper @join__type(graph: A) { txs: [Transaction] }
        type Broken @join__type(graph: A) @context(name: "a__user") {
            currency: String @join__field(graph: A, external: true)
            txs: [Transaction]
        }
        type Outer @join__type(graph: A) @context(name: "a__outer") {
            code: String
            user: User
        }
        type Transaction @join__type(graph: A, key: "id") @join__type(graph: B, key: "id") {
            id: ID!
            base: Int @join__field(graph: A)
            amount: Int @join__field(graph: A, contextArguments: [
                {name: "code", type: "String", context: "a__user", selection: "{ currency }"}
            ])
            detail: User @join__field(graph: A, contextArguments: [
                {name: "code", type: "String", context: "a__user", selection: "{ currency }"}
            ])
            remote: Int @join__field(graph: B, contextArguments: [
                {name: "code", type: "String", context: "a__user", selection: "{ currency }"}
            ])
            fromInterface: Int @join__field(graph: A, contextArguments: [
                {name: "code", type: "String", context: "a__account", selection: "{ currency }"}
            ])
            fromUnion: Int @join__field(graph: A, contextArguments: [
                {name: "code", type: "String", context: "a__viewer", selection: "... on User { currency } ... on Other { currency }"}
            ])
            required: Int @join__field(graph: A, requires: "base", contextArguments: [
                {name: "code", type: "String", context: "a__user", selection: "{ currency }"}
            ])
            multiple: Int @join__field(graph: A, contextArguments: [
                {name: "code", type: "String", context: "a__user", selection: "{ currency }"},
                {name: "outer", type: "String!", context: "a__outer", selection: "{ code }"}
            ])
        }
    "#;

    fn walk<'graph>(
        graph: &'graph Graph,
        supergraph: &'graph SupergraphState,
        overrides: &'graph PlannerOverrideContext,
        cancellation: &'graph CancellationToken,
        steps: &[&str],
    ) -> Option<OperationPath<'graph>> {
        let entrypoint = graph.edges_from(graph.query_root).next().unwrap();
        let mut paths = vec![OperationPath::new_entrypoint(&entrypoint)];
        for step in steps {
            let mut field = FieldSelection::new_typename();
            if let Some((alias, name)) = step.split_once(':') {
                field.name = name.to_string();
                field.alias = Some(alias.to_string());
            } else {
                field.name = step.to_string();
            }
            let target = if let Some(type_name) = step.strip_prefix("... on ") {
                NavigationTarget::ConcreteType(type_name, None)
            } else {
                NavigationTarget::Field {
                    field: &field,
                    target_subgraph_ids: None,
                }
            };
            let mut next_paths = vec![];
            for path in &paths {
                let mut direct =
                    find_direct_paths(graph, supergraph, overrides, path, &target, cancellation)
                        .unwrap();
                if direct.is_empty() {
                    direct = find_indirect_paths(
                        graph,
                        supergraph,
                        overrides,
                        path,
                        &target,
                        &Default::default(),
                        cancellation,
                    )
                    .unwrap();
                }
                next_paths.extend(direct);
            }
            paths = next_paths;
        }
        paths.into_iter().next()
    }

    #[test]
    fn context_metadata_and_cost_are_added_to_field_edges() {
        let supergraph = SupergraphState::new(&parse_schema(SUPERGRAPH));
        let graph = Graph::graph_from_supergraph_state(&supergraph).unwrap();
        let node = graph.node_display_name_to_index["Transaction/a"];
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
        assert_eq!(argument.context, "a__user");
        assert_eq!(argument.selections.len(), 2);
        assert!(argument
            .selections
            .iter()
            .any(|selection| selection.type_name == "User"));
        assert_eq!(
            edge.weight().cost(),
            1 + argument
                .selections
                .iter()
                .map(|selection| selection.selection_set.cost())
                .sum::<u64>()
        );
    }

    #[test]
    fn context_uses_nearest_ancestor_and_counts_only_data_levels() {
        let supergraph = SupergraphState::new(&parse_schema(SUPERGRAPH));
        let graph = Graph::graph_from_supergraph_state(&supergraph).unwrap();
        let overrides = PlannerOverrideContext::default();
        let cancellation = CancellationToken::new();
        for (steps, levels, back, subgraph) in [
            (vec!["users", "txs", "amount"], 1, 1, "a"),
            (vec!["users", "child", "txs", "amount"], 1, 1, "a"),
            (vec!["users", "wrapper", "txs", "amount"], 2, 2, "a"),
            (vec!["users", "txs", "remote"], 1, 2, "b"),
        ] {
            let path = walk(&graph, &supergraph, &overrides, &cancellation, &steps).unwrap();
            assert_eq!(graph.node(path.tail()).unwrap().graph_id(), Some(subgraph));
            let segment = path.last_segment.as_ref().unwrap();
            let requirement = &segment.context_requirements[0];
            assert_eq!(requirement.argument, 0);
            assert_eq!(requirement.matched_type, "User");
            assert_eq!(requirement.levels, levels);
            assert_eq!(requirement.segments_back, back);
            let ancestor_segment = &path.get_segments()[path.get_segments().len() - 1 - back];
            assert_eq!(
                requirement.tree.node_index,
                graph.get_edge_head(&ancestor_segment.edge_index).unwrap()
            );
            assert_eq!(requirement.tree.children.len(), 1);
            assert_eq!(
                graph
                    .edge(requirement.tree.children[0].edge_from_parent.unwrap())
                    .unwrap()
                    .display_name(),
                "currency"
            );
        }
    }

    #[test]
    fn context_matches_interfaces_and_union_members() {
        let supergraph = SupergraphState::new(&parse_schema(SUPERGRAPH));
        let graph = Graph::graph_from_supergraph_state(&supergraph).unwrap();
        let overrides = PlannerOverrideContext::default();
        let cancellation = CancellationToken::new();
        for (steps, matched_type) in [
            (vec!["users", "txs", "fromInterface"], "Account"),
            (
                vec!["account", "... on User", "txs", "fromInterface"],
                "Account",
            ),
            (vec!["viewer", "... on User", "txs", "fromUnion"], "Viewer"),
            (vec!["viewer", "... on Other", "txs", "fromUnion"], "Viewer"),
        ] {
            let path = walk(&graph, &supergraph, &overrides, &cancellation, &steps).unwrap();
            let requirement = &path.last_segment.as_ref().unwrap().context_requirements[0];
            assert_eq!(requirement.matched_type, matched_type);
            assert_eq!(requirement.levels, 1);
            assert!(!requirement.tree.children.is_empty());
        }
    }

    #[test]
    fn context_and_requires_are_kept_separate_and_multiple_arguments_are_resolved() {
        let supergraph = SupergraphState::new(&parse_schema(SUPERGRAPH));
        let graph = Graph::graph_from_supergraph_state(&supergraph).unwrap();
        let overrides = PlannerOverrideContext::default();
        let cancellation = CancellationToken::new();
        let path = walk(
            &graph,
            &supergraph,
            &overrides,
            &cancellation,
            &["users", "txs", "required"],
        )
        .unwrap();
        let segment = path.last_segment.as_ref().unwrap();
        assert_eq!(segment.context_requirements.len(), 1);
        let tree = segment.requirement_tree.as_ref().unwrap();
        assert_eq!(
            graph.node(tree.node_index).unwrap().name_str(),
            "Transaction"
        );
        assert_eq!(
            graph
                .edge(tree.children[0].edge_from_parent.unwrap())
                .unwrap()
                .display_name(),
            "base"
        );
        let path = walk(
            &graph,
            &supergraph,
            &overrides,
            &cancellation,
            &["outer", "user", "txs", "multiple"],
        )
        .unwrap();
        let requirements = &path.last_segment.as_ref().unwrap().context_requirements;
        assert_eq!(requirements.len(), 2);
        assert_eq!(
            (
                requirements[0].argument,
                requirements[0].levels,
                requirements[0].matched_type.as_str()
            ),
            (0, 1, "User")
        );
        assert_eq!(
            (
                requirements[1].argument,
                requirements[1].levels,
                requirements[1].matched_type.as_str()
            ),
            (1, 2, "Outer")
        );
    }

    #[test]
    fn contextual_fetches_wait_for_ancestors_and_keep_their_variables() {
        use crate::query_planner::{
            ast::{operation::OperationDefinition, selection_set::SelectionSet},
            planner::{
                add_variables_to_fetch_steps,
                fetch::fetch_graph::build_fetch_graph_from_query_tree,
                plan_nodes::FetchNodePathSegment, tree::query_tree::QueryTree, QueryPlannerOptions,
            },
            utils::parsing::parse_operation,
        };
        use graphql_tools::parser::query::Definition;

        let supergraph = SupergraphState::new(&parse_schema(SUPERGRAPH));
        let graph = Graph::graph_from_supergraph_state(&supergraph).unwrap();
        let overrides = PlannerOverrideContext::default();
        let cancellation = CancellationToken::new();
        for (steps, levels, arguments) in [
            (vec!["users", "txs", "amount"], vec![1], vec!["code"]),
            (
                vec!["users", "wrapper", "txs", "amount"],
                vec![2],
                vec!["code"],
            ),
            (
                vec!["users", "child", "txs", "amount"],
                vec![1],
                vec!["code"],
            ),
            (vec!["users", "txs", "remote"], vec![1], vec!["code"]),
            (
                vec!["users", "txs", "detail", "currency"],
                vec![1],
                vec!["code"],
            ),
            (
                vec!["users", "txs", "detail", "txs", "amount"],
                vec![1],
                vec!["code"],
            ),
            (vec!["users", "txs", "required"], vec![1], vec!["code"]),
            (vec!["users", "txs", "fromInterface"], vec![1], vec!["code"]),
            (
                vec!["viewer", "... on User", "txs", "fromUnion"],
                vec![1],
                vec!["code"],
            ),
            (
                vec!["outer", "user", "txs", "multiple"],
                vec![1, 2],
                vec!["code", "outer"],
            ),
        ] {
            let path = walk(&graph, &supergraph, &overrides, &cancellation, &steps).unwrap();
            let tree = QueryTree::from_path(&graph, &path, None).unwrap();
            let tree = QueryTree::merge_trees(vec![tree.clone(), tree]);
            let Definition::Operation(parsed) =
                parse_operation("query($__context_0: String) { users { txs { amount } } }")
                    .definitions
                    .remove(0)
            else {
                panic!("expected operation")
            };
            let operation: OperationDefinition = parsed.into();
            let operation = OperationDefinition {
                selection_set: SelectionSet::default(),
                ..operation
            };
            let mut fetch_graph = build_fetch_graph_from_query_tree(
                &graph,
                &supergraph,
                &overrides,
                tree,
                &operation,
                &QueryPlannerOptions::default(),
                &cancellation,
            )
            .unwrap();
            add_variables_to_fetch_steps(&mut fetch_graph, &operation.variable_definitions)
                .unwrap();
            let contextual = fetch_graph
                .all_nodes()
                .filter(|(_, step)| step.context_rewrites.is_some())
                .collect::<Vec<_>>();
            let expected_count = if steps.contains(&"detail") && steps.last() == Some(&"amount") {
                2
            } else {
                1
            };
            assert_eq!(contextual.len(), expected_count, "{steps:?}");
            if expected_count == 2 {
                assert!(fetch_graph.is_descendant_of(contextual[1].0, contextual[0].0));
                assert_eq!(
                    contextual[1].1.context_variable_definitions[0].name,
                    "__context_2"
                );
            }
            let (index, step) = contextual[0];
            assert!(step.is_entity_call());
            assert_eq!(step.context_variable_definitions.len(), arguments.len());
            assert!(!step.context_producers.is_empty());
            for (argument_index, (argument, levels)) in arguments.iter().zip(levels).enumerate() {
                let variable = &step.context_variable_definitions[argument_index];
                assert_eq!(variable.name, format!("__context_{}", argument_index + 1));
                assert!(step
                    .variable_usages
                    .as_ref()
                    .unwrap()
                    .contains(&variable.name));
                assert!(step
                    .variable_definitions
                    .as_ref()
                    .unwrap()
                    .iter()
                    .any(|definition| definition.name == variable.name));
                assert!(step
                    .output
                    .to_non_root_selection_set()
                    .to_string()
                    .contains(&format!("{argument}: ${}", variable.name)));
                let rewrite = step
                    .context_rewrites
                    .as_ref()
                    .unwrap()
                    .iter()
                    .find(|rewrite| rewrite.rename_key_to == variable.name)
                    .unwrap();
                assert_eq!(
                    rewrite
                        .path
                        .iter()
                        .take_while(|segment| matches!(segment, FetchNodePathSegment::Parent))
                        .count(),
                    levels
                );
            }
            for producer_id in &step.context_producers {
                let (producer_index, producer) = fetch_graph
                    .all_nodes()
                    .find(|(_, producer)| producer.id == *producer_id)
                    .unwrap();
                assert_ne!(producer_index, index);
                assert!(fetch_graph.is_descendant_of(index, producer_index));
                assert!(producer
                    .output
                    .to_non_root_selection_set()
                    .to_string()
                    .contains("__typename"));
            }
            if steps.last() == Some(&"required") {
                assert!(step
                    .input
                    .to_non_root_selection_set()
                    .to_string()
                    .contains("base"));
            }
        }
    }

    #[test]
    fn context_rewrites_read_internal_keys_under_client_aliases() {
        use crate::query_planner::{
            ast::operation::OperationDefinition,
            planner::{
                fetch::fetch_graph::build_fetch_graph_from_query_tree,
                plan_nodes::FetchNodePathSegment, tree::query_tree::QueryTree, QueryPlannerOptions,
            },
            utils::parsing::parse_operation,
        };
        use graphql_tools::parser::query::Definition;

        let supergraph = SupergraphState::new(&parse_schema(SUPERGRAPH));
        let graph = Graph::graph_from_supergraph_state(&supergraph).unwrap();
        let overrides = PlannerOverrideContext::default();
        let cancellation = CancellationToken::new();
        let path = walk(
            &graph,
            &supergraph,
            &overrides,
            &cancellation,
            &["me:users", "txs", "amount"],
        )
        .unwrap();
        let tree = QueryTree::from_path(&graph, &path, None).unwrap();
        let Definition::Operation(parsed) =
            parse_operation("{ me: users { currency: id txs { amount } } }")
                .definitions
                .remove(0)
        else {
            panic!("expected operation")
        };
        let operation: OperationDefinition = parsed.into();
        let fetch_graph = build_fetch_graph_from_query_tree(
            &graph,
            &supergraph,
            &overrides,
            tree,
            &operation,
            &QueryPlannerOptions::default(),
            &cancellation,
        )
        .unwrap();
        let (_, step) = fetch_graph
            .all_nodes()
            .find(|(_, step)| step.context_rewrites.is_some())
            .unwrap();
        assert_eq!(step.response_path.to_string(), "me.@.txs.@");
        assert_eq!(
            step.context_rewrites.as_ref().unwrap()[0].path,
            vec![
                FetchNodePathSegment::Parent,
                FetchNodePathSegment::typename_equals_from_type("User".to_string()),
                FetchNodePathSegment::Key("_internal_qp_alias_0".to_string()),
            ]
        );
        assert!(fetch_graph.all_nodes().any(|(_, step)| step
            .output
            .to_non_root_selection_set()
            .to_string()
            .contains("_internal_qp_alias_0: currency")));
    }

    #[test]
    fn context_is_unsatisfiable_without_an_ancestor_or_a_resolvable_selection() {
        let supergraph = SupergraphState::new(&parse_schema(SUPERGRAPH));
        let graph = Graph::graph_from_supergraph_state(&supergraph).unwrap();
        let overrides = PlannerOverrideContext::default();
        let cancellation = CancellationToken::new();
        for steps in [
            vec!["transaction", "amount"],
            vec!["broken", "txs", "amount"],
            vec!["users", "broken", "txs", "amount"],
        ] {
            assert!(walk(&graph, &supergraph, &overrides, &cancellation, &steps).is_none());
        }
    }

    #[test]
    fn context_requires_a_locally_resolvable_entity_key() {
        let overrides = PlannerOverrideContext::default();
        let cancellation = CancellationToken::new();
        for schema in [
            SUPERGRAPH.replace(
                "type Transaction @join__type(graph: A, key: \"id\") @join__type(graph: B, key: \"id\")",
                "type Transaction @join__type(graph: A) @join__type(graph: B)",
            ),
            SUPERGRAPH.replace(
                "type Transaction @join__type(graph: A, key: \"id\") @join__type(graph: B, key: \"id\")",
                "type Transaction @join__type(graph: A, key: \"id\", resolvable: false) @join__type(graph: B, key: \"id\", resolvable: false)",
            ),
            SUPERGRAPH.replace("id: ID!\n            base:", "id: ID! @join__field(graph: A, external: true) @join__field(graph: B, external: true)\n            base:"),
        ] {
            let supergraph = SupergraphState::new(&parse_schema(&schema));
            let graph = Graph::graph_from_supergraph_state(&supergraph).unwrap();
            assert!(walk(&graph, &supergraph, &overrides, &cancellation, &["users", "txs", "amount"]).is_none());
        }
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

    /// Name of the node the only `edge_name` edge of `node_id` leads to.
    fn follow(graph: &Graph, node_id: &str, edge_name: &str) -> String {
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
        assert_eq!(incoming.edges.len(), 14);
        assert_eq!(outgoing.edges.len(), 18);

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
            Edge::FieldMove(fm) => assert!(fm.is_list, "expected 'items' field move to be a list"),
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
}
