use graphql_tools::{
    ast::OperationDefinitionExtension,
    static_graphql::query::{
        Field, FragmentDefinition, FragmentSpread, InlineFragment, OperationDefinition, Selection,
        SelectionSet,
    },
};

/**
 * This enum represents a union of ast node types,
 * that are used for counting selections, or directives
 * in the ast node.
 *
 * It is used by `max_depth` rule to calculate the depth based
 * on selection,
 * and it is used by `max_directives` to count the number of
 * directives used in the operation string.
 *
 * In this module, we define `CountableNode.selection_set` only,
 * but `CountableNode.directives` one is implemented in
 * `max_directives_rule.rs` file.
 */
pub enum CountableNode<'a> {
    Field(&'a Field),
    FragmentSpread(&'a FragmentSpread),
    InlineFragment(&'a InlineFragment),
    OperationDefinition(&'a OperationDefinition),
    FragmentDefinition(&'a FragmentDefinition),
}

impl<'a> CountableNode<'a> {
    /**
     * This returns the selection set object of the relevant ast node
     */
    pub fn selection_set(&self) -> Option<&'a SelectionSet> {
        match self {
            CountableNode::Field(field) => Some(&field.selection_set),
            CountableNode::InlineFragment(inline_fragment) => Some(&inline_fragment.selection_set),
            CountableNode::OperationDefinition(node) => Some(node.selection_set()),
            CountableNode::FragmentDefinition(node) => Some(&node.selection_set),
            CountableNode::FragmentSpread(_) => None,
        }
    }
}

/**
 * The following `impl` definitions implements `From` trait
 * for the original AST Node types to get `CountableNode` for each.
 */
impl<'a> From<&'a Selection> for CountableNode<'a> {
    fn from(selection: &'a Selection) -> Self {
        match selection {
            Selection::Field(field) => CountableNode::Field(field),
            Selection::InlineFragment(inline_fragment) => {
                CountableNode::InlineFragment(inline_fragment)
            }
            Selection::FragmentSpread(fragment_spread) => {
                CountableNode::FragmentSpread(fragment_spread)
            }
        }
    }
}

impl<'a> From<&&'a FragmentDefinition> for CountableNode<'a> {
    fn from(fragment_definition: &&'a FragmentDefinition) -> Self {
        CountableNode::FragmentDefinition(fragment_definition)
    }
}

impl<'a> From<&'a OperationDefinition> for CountableNode<'a> {
    fn from(operation_definition: &'a OperationDefinition) -> Self {
        CountableNode::OperationDefinition(operation_definition)
    }
}
