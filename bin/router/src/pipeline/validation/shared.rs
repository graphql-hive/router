use graphql_tools::static_graphql::query::{
    Directive, Field, FragmentDefinition, FragmentSpread, InlineFragment, OperationDefinition,
    Selection, SelectionSet,
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
     *
     * We use this to traverse the selections in the validation rules.
     *
     * The return value is `Option<&SelectionSet>` because
     * only `FragmentSpread` does not have a selection set.
     */
    pub fn selection_set(&self) -> Option<&'a SelectionSet> {
        match self {
            CountableNode::Field(field) => Some(&field.selection_set),
            CountableNode::InlineFragment(inline_fragment) => Some(&inline_fragment.selection_set),
            CountableNode::OperationDefinition(node) => Some(node.selection_set()),
            CountableNode::FragmentDefinition(node) => Some(&node.selection_set),
            // Fragment spreads do not have selection sets
            // So we return None here
            CountableNode::FragmentSpread(_) => None,
        }
    }
    /**
     * This returns the directives slice of the relevant ast node
     *
     * We use this in `max_directives_rule.rs` to count the number of directives
     */
    pub fn get_directives(&self) -> Option<&'a [Directive]> {
        match self {
            CountableNode::Field(field) => Some(&field.directives),
            CountableNode::FragmentDefinition(fragment_def) => Some(&fragment_def.directives),
            CountableNode::InlineFragment(inline_fragment) => Some(&inline_fragment.directives),
            CountableNode::OperationDefinition(op_def) => Some(op_def.directives()),
            CountableNode::FragmentSpread(fragment_spread) => Some(&fragment_spread.directives),
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

/**
 * While visiting fragments, we need to keep track of
 * whether the fragment was already counted, or is being visited.
 * In case of recursive fragments, this avoids infinite loops.
 */
pub enum VisitedFragment {
    Counted(usize),
    Visiting,
}
