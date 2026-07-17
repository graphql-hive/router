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

impl<'doc> OperationVisitor<'doc, ValidationErrorContext> for ValidationVisitors<'doc> {
    fn enter_document(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        document: &'doc Document,
    ) {
        for visitor in &mut self.visitors {
            visitor.enter_document(context, errors, document);
        }
    }

    fn leave_document(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        document: &Document,
    ) {
        for visitor in &mut self.visitors {
            visitor.leave_document(context, errors, document);
        }
    }

    fn enter_operation_definition(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        operation: &'doc OperationDefinition,
    ) {
        for visitor in &mut self.visitors {
            visitor.enter_operation_definition(context, errors, operation);
        }
    }

    fn leave_operation_definition(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        operation: &OperationDefinition,
    ) {
        for visitor in &mut self.visitors {
            visitor.leave_operation_definition(context, errors, operation);
        }
    }

    fn enter_fragment_definition(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        fragment: &'doc FragmentDefinition,
    ) {
        for visitor in &mut self.visitors {
            visitor.enter_fragment_definition(context, errors, fragment);
        }
    }

    fn leave_fragment_definition(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        fragment: &FragmentDefinition,
    ) {
        for visitor in &mut self.visitors {
            visitor.leave_fragment_definition(context, errors, fragment);
        }
    }

    fn enter_variable_definition(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        variable: &'doc VariableDefinition,
    ) {
        for visitor in &mut self.visitors {
            visitor.enter_variable_definition(context, errors, variable);
        }
    }

    fn leave_variable_definition(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        variable: &VariableDefinition,
    ) {
        for visitor in &mut self.visitors {
            visitor.leave_variable_definition(context, errors, variable);
        }
    }

    fn enter_directive(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        directive: &Directive,
    ) {
        for visitor in &mut self.visitors {
            visitor.enter_directive(context, errors, directive);
        }
    }

    fn leave_directive(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        directive: &Directive,
    ) {
        for visitor in &mut self.visitors {
            visitor.leave_directive(context, errors, directive);
        }
    }

    fn enter_argument(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        argument: &'doc (String, Value),
    ) {
        for visitor in &mut self.visitors {
            visitor.enter_argument(context, errors, argument);
        }
    }

    fn leave_argument(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        argument: &(String, Value),
    ) {
        for visitor in &mut self.visitors {
            visitor.leave_argument(context, errors, argument);
        }
    }

    fn enter_selection_set(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        selection_set: &'doc SelectionSet,
    ) {
        for visitor in &mut self.visitors {
            visitor.enter_selection_set(context, errors, selection_set);
        }
    }

    fn leave_selection_set(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        selection_set: &SelectionSet,
    ) {
        for visitor in &mut self.visitors {
            visitor.leave_selection_set(context, errors, selection_set);
        }
    }

    fn enter_field(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        field: &Field,
    ) {
        for visitor in &mut self.visitors {
            visitor.enter_field(context, errors, field);
        }
    }

    fn leave_field(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        field: &Field,
    ) {
        for visitor in &mut self.visitors {
            visitor.leave_field(context, errors, field);
        }
    }

    fn enter_fragment_spread(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        fragment_spread: &'doc FragmentSpread,
    ) {
        for visitor in &mut self.visitors {
            visitor.enter_fragment_spread(context, errors, fragment_spread);
        }
    }

    fn leave_fragment_spread(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        fragment_spread: &FragmentSpread,
    ) {
        for visitor in &mut self.visitors {
            visitor.leave_fragment_spread(context, errors, fragment_spread);
        }
    }

    fn enter_inline_fragment(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        inline_fragment: &InlineFragment,
    ) {
        for visitor in &mut self.visitors {
            visitor.enter_inline_fragment(context, errors, inline_fragment);
        }
    }

    fn leave_inline_fragment(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        inline_fragment: &InlineFragment,
    ) {
        for visitor in &mut self.visitors {
            visitor.leave_inline_fragment(context, errors, inline_fragment);
        }
    }

    fn enter_null_value(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        value: (),
    ) {
        for visitor in &mut self.visitors {
            visitor.enter_null_value(context, errors, value);
        }
    }

    fn leave_null_value(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        value: (),
    ) {
        for visitor in &mut self.visitors {
            visitor.leave_null_value(context, errors, value);
        }
    }

    fn enter_scalar_value(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        value: &Value,
    ) {
        for visitor in &mut self.visitors {
            visitor.enter_scalar_value(context, errors, value);
        }
    }

    fn leave_scalar_value(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        value: &Value,
    ) {
        for visitor in &mut self.visitors {
            visitor.leave_scalar_value(context, errors, value);
        }
    }

    fn enter_enum_value(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        value: &String,
    ) {
        for visitor in &mut self.visitors {
            visitor.enter_enum_value(context, errors, value);
        }
    }

    fn leave_enum_value(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        value: &String,
    ) {
        for visitor in &mut self.visitors {
            visitor.leave_enum_value(context, errors, value);
        }
    }

    fn enter_variable_value(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        variable_name: &'doc str,
    ) {
        for visitor in &mut self.visitors {
            visitor.enter_variable_value(context, errors, variable_name);
        }
    }

    fn leave_variable_value(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        variable_name: &String,
    ) {
        for visitor in &mut self.visitors {
            visitor.leave_variable_value(context, errors, variable_name);
        }
    }

    fn enter_list_value(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        values: &Vec<Value>,
    ) {
        for visitor in &mut self.visitors {
            visitor.enter_list_value(context, errors, values);
        }
    }

    fn leave_list_value(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        values: &Vec<Value>,
    ) {
        for visitor in &mut self.visitors {
            visitor.leave_list_value(context, errors, values);
        }
    }

    fn enter_object_value(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        fields: &[(String, Value)],
    ) {
        for visitor in &mut self.visitors {
            visitor.enter_object_value(context, errors, fields);
        }
    }

    fn leave_object_value(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        fields: &[(String, Value)],
    ) {
        for visitor in &mut self.visitors {
            visitor.leave_object_value(context, errors, fields);
        }
    }

    fn enter_object_field(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        field: &(String, Value),
    ) {
        for visitor in &mut self.visitors {
            visitor.enter_object_field(context, errors, field);
        }
    }

    fn leave_object_field(
        &mut self,
        context: &mut OperationVisitorContext<'doc>,
        errors: &mut ValidationErrorContext,
        field: &(String, Value),
    ) {
        for visitor in &mut self.visitors {
            visitor.leave_object_field(context, errors, field);
        }
    }
}

pub trait ValidationRule: Send + Sync {
    fn visitor<'doc>(&self) -> ValidationVisitor<'doc>;
    fn error_code(&self) -> &'static str;
}
