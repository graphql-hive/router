use crate::{
    ast::{OperationVisitor, OperationVisitorContext},
    static_graphql::query::*,
    validation::utils::ValidationErrorContext,
};

pub type ValidationVisitor<'doc> = Box<dyn OperationVisitor<'doc, ValidationErrorContext> + 'doc>;

pub struct ValidationVisitors<'doc> {
    visitors: Vec<ValidationVisitor<'doc>>,
}

impl<'doc> ValidationVisitors<'doc> {
    pub fn new(visitors: impl IntoIterator<Item = ValidationVisitor<'doc>>) -> Self {
        Self {
            visitors: visitors.into_iter().collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.visitors.is_empty()
    }
}

macro_rules! forward_to_visitors {
    ($method:ident($argument:ident: $argument_type:ty)) => {
        fn $method(
            &mut self,
            context: &mut OperationVisitorContext<'doc>,
            errors: &mut ValidationErrorContext,
            $argument: $argument_type,
        ) {
            for visitor in &mut self.visitors {
                visitor.$method(context, errors, $argument);
            }
        }
    };
}

impl<'doc> OperationVisitor<'doc, ValidationErrorContext> for ValidationVisitors<'doc> {
    forward_to_visitors!(enter_document(document: &'doc Document));
    forward_to_visitors!(leave_document(document: &Document));

    forward_to_visitors!(
        enter_operation_definition(operation: &'doc OperationDefinition)
    );
    forward_to_visitors!(
        leave_operation_definition(operation: &OperationDefinition)
    );

    forward_to_visitors!(
        enter_fragment_definition(fragment: &'doc FragmentDefinition)
    );
    forward_to_visitors!(
        leave_fragment_definition(fragment: &FragmentDefinition)
    );

    forward_to_visitors!(
        enter_variable_definition(variable: &'doc VariableDefinition)
    );
    forward_to_visitors!(
        leave_variable_definition(variable: &VariableDefinition)
    );

    forward_to_visitors!(enter_directive(directive: &Directive));
    forward_to_visitors!(leave_directive(directive: &Directive));

    forward_to_visitors!(enter_argument(argument: &'doc (String, Value)));
    forward_to_visitors!(leave_argument(argument: &(String, Value)));

    forward_to_visitors!(
        enter_selection_set(selection_set: &'doc SelectionSet)
    );
    forward_to_visitors!(leave_selection_set(selection_set: &SelectionSet));

    forward_to_visitors!(enter_field(field: &Field));
    forward_to_visitors!(leave_field(field: &Field));

    forward_to_visitors!(
        enter_fragment_spread(fragment_spread: &'doc FragmentSpread)
    );
    forward_to_visitors!(
        leave_fragment_spread(fragment_spread: &FragmentSpread)
    );

    forward_to_visitors!(
        enter_inline_fragment(inline_fragment: &InlineFragment)
    );
    forward_to_visitors!(
        leave_inline_fragment(inline_fragment: &InlineFragment)
    );

    // Unit is Copy, so the regular forwarding macro handles it.
    forward_to_visitors!(enter_null_value(value: ()));
    forward_to_visitors!(leave_null_value(value: ()));

    forward_to_visitors!(enter_scalar_value(value: &Value));
    forward_to_visitors!(leave_scalar_value(value: &Value));

    forward_to_visitors!(enter_enum_value(value: &String));
    forward_to_visitors!(leave_enum_value(value: &String));

    forward_to_visitors!(
        enter_variable_value(variable_name: &'doc str)
    );
    forward_to_visitors!(
        leave_variable_value(variable_name: &String)
    );

    forward_to_visitors!(enter_list_value(values: &Vec<Value>));
    forward_to_visitors!(leave_list_value(values: &Vec<Value>));

    forward_to_visitors!(
        enter_object_value(fields: &[(String, Value)])
    );
    forward_to_visitors!(
        leave_object_value(fields: &[(String, Value)])
    );

    forward_to_visitors!(
        enter_object_field(field: &(String, Value))
    );
    forward_to_visitors!(
        leave_object_field(field: &(String, Value))
    );
}

pub trait ValidationRule: Send + Sync {
    /// Creates a fresh visitor for this validation run.
    ///
    /// Visitors hold per-document state, while rules are shared by validation plans.
    /// Rules that require a custom document walk can use `validate` instead.
    fn visitor<'doc>(&self) -> ValidationVisitor<'doc>;
    fn error_code(&self) -> &'static str;
}
