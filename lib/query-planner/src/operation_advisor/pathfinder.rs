use graphql_parser_hive_fork::query::{
    Field, FragmentSpread, InlineFragment, Selection, SelectionSet,
};
use graphql_tools::{
    ast::OperationDefinitionExtension, static_graphql::query::OperationDefinition,
};
use petgraph::{graph::NodeIndex, visit::EdgeRef};
use std::{
    collections::HashMap,
    fmt::{Display, Error},
};

use crate::satisfiability_graph::{edge::Edge, graph::GraphQLSatisfiabilityGraph, node::Node};

// Represents a resolved field with its path and the subgraph that will resolve it
use petgraph::graph::EdgeIndex;

/// Represents a complete path through the query graph for resolving a field
#[derive(Debug, Clone)]
pub struct StepsToField {
    pub path: Vec<String>,

    /// The subgraph that will resolve this field
    pub subgraph: String,

    /// Any fields required by @requires directive
    pub required_fields: Option<Vec<String>>,

    /// The complete sequence of nodes traversed in the satisfiability graph
    pub nodes: Vec<NodeIndex>,

    /// The edges connecting the nodes
    pub edges: Vec<EdgeIndex>,

    /// The number of times we changed subgraphs in this path
    pub subgraph_jumps: usize,

    /// The starting node of the path (typically a Query root)
    pub head: NodeIndex,

    /// The ending node of the path (the field's type)
    pub tail: NodeIndex,

    /// Entity references used in this path (entity name -> key expression)
    pub entity_references: HashMap<String, String>,
}

impl StepsToField {
    /// Create a new enhanced resolved field
    pub fn new(path: Vec<String>, subgraph: String) -> Self {
        Self {
            path,
            subgraph,
            required_fields: None,
            nodes: Vec::new(),
            edges: Vec::new(),
            subgraph_jumps: 0,
            head: NodeIndex::new(0), // Default value, should be set properly
            tail: NodeIndex::new(0), // Default value, should be set properly
            entity_references: HashMap::new(),
        }
    }

    /// Add a node to the path
    pub fn add_node(&mut self, node: NodeIndex) {
        self.nodes.push(node);

        // Update head/tail
        if self.nodes.len() == 1 {
            self.head = node;
        }
        self.tail = node;
    }

    /// Add an edge to the path
    pub fn add_edge(&mut self, edge: EdgeIndex) {
        self.edges.push(edge);
    }

    /// Add an entity reference
    pub fn add_entity_reference(&mut self, entity: String, key: String) {
        self.entity_references.insert(entity, key);
    }

    /// Record a subgraph jump
    pub fn record_subgraph_jump(&mut self) {
        self.subgraph_jumps += 1;
    }

    /// Set required fields
    pub fn with_required_fields(&mut self, fields: Option<Vec<String>>) {
        self.required_fields = fields;
    }
}

impl Display for StepsToField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "coordinate: {}, subgraph: {}",
            self.path.join("."),
            self.subgraph
        )
    }
}

pub struct Pathfinder<'a> {
    graph: &'a GraphQLSatisfiabilityGraph,
    indent: usize,
    // Track resolved fields and which subgraph will handle them
    pub resolved_fields: Vec<StepsToField>,
    // Track entity references we'll need to use
    pub entity_references: HashMap<String, Vec<String>>,
    // Current path in the traversal (for tracking field paths)
    current_path: Vec<String>,
    // Store fragment definitions for fragment spreads
    fragment_definitions: HashMap<String, SelectionSet<'static, String>>,
}

impl<'a> Pathfinder<'a> {
    pub fn new(graph: &'a GraphQLSatisfiabilityGraph) -> Self {
        Self {
            graph,
            indent: 0,
            resolved_fields: Vec::new(),
            entity_references: HashMap::new(),
            current_path: Vec::new(),
            fragment_definitions: HashMap::new(),
        }
    }

    // Enhanced logging with tree visualization
    fn log_entry(&self, msg: &str) {
        println!("{:indent$}--> {}", "", msg, indent = self.indent * 2);
    }

    fn log_exit(&self, msg: &str) {
        println!("{:indent$}<-- {}", "", msg, indent = self.indent * 2);
    }

    fn log_info(&self, msg: &str) {
        println!("{:indent$}    {}", "", msg, indent = self.indent * 2);
    }

    // Main traversal entry point
    pub fn traverse(mut self, operation: &OperationDefinition) -> Vec<StepsToField> {
        self.log_entry("Starting traversal from Query root");
        self.process_root_selection_set(operation.selection_set());
        self.log_exit("Completed Query root traversal");

        self.resolved_fields
    }

    // Special method to handle the root selection set
    fn process_root_selection_set(&mut self, selection_set: &SelectionSet<'static, String>) {
        for selection in &selection_set.items {
            if let Selection::Field(field) = selection {
                self.process_root_field(field);
            }
        }
    }

    // Process a root field
    // Process a root field
    fn process_root_field(&mut self, field: &Field<'static, String>) {
        self.log_entry(&format!("Root Field '{}'", field.name));

        // Add the field to the current path
        self.current_path.push(field.name.clone());

        // For each subgraph that can handle this root field
        let edges = self.graph.lookup.graph.edges(self.graph.lookup.query_root);

        for edge_ref in edges {
            if !matches!(edge_ref.weight(), Edge::Root) {
                continue;
            }

            let edge_idx = edge_ref.id();
            let target_idx = edge_ref.target();
            let target_node = &self.graph.lookup.graph[target_idx];

            // Check if this subgraph has the field
            if let Node::SubgraphType {
                subgraph,
                name: _query_type,
            } = target_node
            {
                // Check if this subgraph Query type has this field
                let mut field_found = false;
                let subgraph_edges = self.graph.lookup.graph.edges(target_idx);

                for subgraph_edge_ref in subgraph_edges {
                    if let Edge::Field {
                        name, join_field, ..
                    } = subgraph_edge_ref.weight()
                    {
                        if name == &field.name {
                            // We found the field in this subgraph
                            field_found = true;
                            let subgraph_edge_idx = subgraph_edge_ref.id();
                            let field_target_idx = subgraph_edge_ref.target();

                            // Create an enhanced field recording the full path
                            let mut enhanced_field =
                                StepsToField::new(self.current_path.clone(), subgraph.clone());

                            // Record the path through the graph
                            enhanced_field.add_node(self.graph.lookup.query_root); // Start at Query root
                            enhanced_field.add_edge(edge_idx); // Root edge to subgraph Query
                            enhanced_field.add_node(target_idx); // Subgraph Query node
                            enhanced_field.add_edge(subgraph_edge_idx); // Field edge
                            enhanced_field.add_node(field_target_idx); // Field target node

                            // Capture required fields if present
                            enhanced_field.with_required_fields(
                                join_field
                                    .as_ref()
                                    .and_then(|jf| jf.requires.clone().map(|req| vec![req])),
                            );

                            // Add the enhanced field to our result
                            self.resolved_fields.push(enhanced_field);

                            // Process the selection set for this field in this subgraph
                            if !field.selection_set.items.is_empty() {
                                self.process_selection_set(
                                    field_target_idx,
                                    &field.selection_set,
                                    None,
                                );
                            }

                            break;
                        }
                    }
                }

                if !field_found {
                    self.log_info(&format!(
                        "Field '{}' not found in subgraph '{}'",
                        field.name, subgraph
                    ));
                }
            }
        }

        // Pop the field from the path
        self.current_path.pop();

        self.log_exit(&format!("Root Field '{}'", field.name));
    }

    // Process a selection set
    fn process_selection_set(
        &mut self,
        node_idx: NodeIndex,
        selection_set: &SelectionSet<'static, String>,
        parent_type_condition: Option<&str>,
    ) {
        for selection in &selection_set.items {
            match selection {
                Selection::Field(field) => {
                    let _ = self.process_field(node_idx, field, parent_type_condition);
                }
                Selection::FragmentSpread(fragment_spread) => {
                    let _ = self.process_fragment_spread(node_idx, fragment_spread);
                }
                Selection::InlineFragment(inline_fragment) => {
                    let _ = self.process_inline_fragment(
                        node_idx,
                        inline_fragment,
                        parent_type_condition,
                    );
                }
            }
        }
    }

    // Process a non-root field
    fn process_field(
        &mut self,
        node_idx: NodeIndex,
        field: &Field<'static, String>,
        parent_type_condition: Option<&str>,
    ) -> Result<(), Error> {
        self.log_entry(&format!("Field '{}'", field.name));

        // Push this field to the current path
        self.current_path.push(field.name.clone());

        // Create an initially empty enhanced field
        let mut steps = StepsToField::new(self.current_path.clone(), "unknown".to_string());

        // Start the path with the current node
        steps.add_node(node_idx);

        let node = &self.graph.lookup.graph[node_idx];
        let edges = self.graph.lookup.graph.edges(node_idx);

        let mut handled = false;
        let mut target_for_selection_set = None;

        for edge_ref in edges {
            let edge = edge_ref.weight();
            let edge_idx = edge_ref.id();
            let target_idx = edge_ref.target();
            let target_node = &self.graph.lookup.graph[target_idx];

            match (node, edge) {
                (
                    Node::SubgraphType {
                        name: type_name,
                        subgraph: source_subgraph,
                    },
                    Edge::Field {
                        name, join_field, ..
                    },
                ) => {
                    if name == &field.name {
                        self.log_info(&format!(
                            "Found field '{}' in type '{}' of subgraph '{}'",
                            name, type_name, source_subgraph
                        ));

                        // Update our enhanced field
                        steps.subgraph = source_subgraph.clone();
                        steps.add_edge(edge_idx);
                        steps.add_node(target_idx);
                        steps.with_required_fields(
                            join_field
                                .as_ref()
                                .and_then(|jf| jf.requires.clone().map(|req| vec![req])),
                        );

                        // Store the target for traversing the selection set later
                        if !field.selection_set.items.is_empty() {
                            target_for_selection_set = Some(target_idx);
                        }

                        handled = true;
                    }
                }

                // Handle entity references (service boundary crossing)
                (_, Edge::EntityReference(key)) => {
                    match (node, target_node) {
                        (
                            Node::SubgraphType {
                                name: source_type,
                                subgraph: source_subgraph,
                            },
                            Node::SubgraphType {
                                name: target_type,
                                subgraph: target_subgraph,
                            },
                        ) => {
                            // We're crossing subgraph boundary
                            if source_subgraph != target_subgraph && source_type == target_type {
                                self.log_info(&format!(
                                    "Crossing boundary from '{}' to '{}' using key '{}'",
                                    source_subgraph, target_subgraph, key
                                ));

                                // Update our enhanced field
                                steps.subgraph = target_subgraph.clone();
                                steps.add_edge(edge_idx);
                                steps.add_node(target_idx);
                                steps.record_subgraph_jump();
                                steps.add_entity_reference(source_type.clone(), key.clone());

                                // Record the entity reference that's needed
                                let key_entry = self
                                    .entity_references
                                    .entry(source_type.clone())
                                    .or_insert_with(Vec::new);
                                if !key_entry.contains(key) {
                                    key_entry.push(key.clone());
                                }

                                // Store this target for traversing the selection set
                                if !field.selection_set.items.is_empty() {
                                    target_for_selection_set = Some(target_idx);
                                }

                                handled = true;
                            }
                        }
                        _ => {}
                    }
                }

                // Handle interface implementations
                (_, Edge::InterfaceImplementation(interface_name)) => {
                    self.log_info(&format!(
                        "Following interface implementation of '{}'",
                        interface_name
                    ));

                    // Update our enhanced field
                    steps.add_edge(edge_idx);
                    steps.add_node(target_idx);

                    // For interface implementations, we might need type conditions
                    if parent_type_condition.is_none()
                        || (parent_type_condition.is_some()
                            && interface_name == parent_type_condition.unwrap())
                    {
                        if !field.selection_set.items.is_empty() {
                            if let Node::SubgraphType { subgraph, .. } = target_node {
                                target_for_selection_set = Some(target_idx);

                                if steps.subgraph == "unknown" {
                                    steps.subgraph = subgraph.clone();
                                }
                            }
                        }
                        handled = true;
                    }
                }
                _ => {}
            }
        }

        if !handled {
            self.log_info(&format!("No handler found for field '{}'", field.name));
        } else {
            // Add the enhanced field to our collection
            self.resolved_fields.push(steps);
        }

        // Now process the selection set if we have one and we found a target
        if let Some(target_idx) = target_for_selection_set {
            // Process each field in the selection set
            for selection in &field.selection_set.items {
                match selection {
                    Selection::Field(nested_field) => {
                        let _ = self.process_field(target_idx, nested_field, parent_type_condition);
                    }
                    Selection::FragmentSpread(fragment_spread) => {
                        let _ = self.process_fragment_spread(target_idx, fragment_spread);
                    }
                    Selection::InlineFragment(inline_fragment) => {
                        let _ = self.process_inline_fragment(
                            target_idx,
                            inline_fragment,
                            parent_type_condition,
                        );
                    }
                }
            }
        }

        // Pop the current field from path when done
        self.current_path.pop();

        self.log_exit(&format!("Field '{}'", field.name));
        Ok(())
    }

    // Process a fragment spread
    fn process_fragment_spread(
        &mut self,
        node_idx: NodeIndex,
        fragment: &FragmentSpread<'static, String>,
    ) -> Result<(), Error> {
        self.log_entry(&format!("Fragment spread '{}'", fragment.fragment_name));

        // First, check if we have the fragment definition without mutating self
        let fragment_selection_set = self
            .fragment_definitions
            .get(&fragment.fragment_name)
            .cloned(); // Clone it so we don't keep the borrow active

        // Get the type condition from the fragment name (simplified approach for this example)
        let type_condition = Some(fragment.fragment_name.as_str());

        // Now handle the traversal if we found the fragment
        if let Some(selection_set) = fragment_selection_set {
            // Process the fragment's selection set
            self.process_selection_set(node_idx, &selection_set, type_condition);
        } else {
            self.log_info(&format!(
                "Fragment definition '{}' not found",
                fragment.fragment_name
            ));
        }

        self.log_exit(&format!("Fragment spread '{}'", fragment.fragment_name));
        Ok(())
    }

    // Process an inline fragment
    fn process_inline_fragment(
        &mut self,
        node_idx: NodeIndex,
        fragment: &InlineFragment<'static, String>,
        parent_type_condition: Option<&str>,
    ) -> Result<(), Error> {
        let condition_desc = if let Some(type_condition) = &fragment.type_condition {
            format!("with condition on type '{}'", type_condition)
        } else {
            "without type condition".to_string()
        };

        self.log_entry(&format!("Inline fragment {}", condition_desc));

        // Get the type condition if present
        let type_condition = fragment.type_condition.as_ref().map(|tc| match tc {
            graphql_parser_hive_fork::query::TypeCondition::On(name) => name.as_str(),
        });

        // Find appropriate edge for this type condition
        if let Some(type_name) = type_condition {
            let edges = self.graph.lookup.graph.edges(node_idx);
            let mut found_edge = false;

            for edge_ref in edges {
                if let Edge::InterfaceImplementation(impl_name) = edge_ref.weight() {
                    if impl_name == type_name {
                        // Follow this implementation edge
                        self.process_selection_set(
                            edge_ref.target(),
                            &fragment.selection_set,
                            type_condition,
                        );
                        found_edge = true;
                        break;
                    }
                }
            }

            if !found_edge {
                // No specific type edge found, try to continue traversal based on subgraph compatibility
                self.process_selection_set(node_idx, &fragment.selection_set, type_condition);
            }
        } else {
            // No type condition, just continue with current node
            self.process_selection_set(node_idx, &fragment.selection_set, parent_type_condition);
        }

        self.log_exit(&format!("Inline fragment {}", condition_desc));
        Ok(())
    }
}
