//! `graphql_tools::validation::rules::ValuesOfCorrectType`, flat.

use graphql_tools::flat::{Span, Sym, Value, ValueId};
use graphql_tools::static_graphql::query::Type;
use graphql_tools::static_graphql::schema::TypeDefinition;

use crate::pipeline::validation::flat::visit::{
    display_value, error, BoxedVisitor, Ctx, Errors, FlatRule, Visitor,
};

const CODE: &str = "ValuesOfCorrectType";

pub struct ValuesOfCorrectType {
    pub options: graphql_tools::validation::rules::ValidationOptions,
}

impl FlatRule for ValuesOfCorrectType {
    fn visitor<'a>(&self) -> BoxedVisitor<'a> {
        Box::new(Rule {
            disable_int_value_range_validation: self.options.disable_int_value_range_validation,
        })
    }

    fn error_code(&self) -> &'static str {
        CODE
    }
}

struct Rule {
    disable_int_value_range_validation: bool,
}

/// What the tree hands to `validate_value`: an enum and a list come as their parts.
#[derive(Clone, Copy)]
enum Raw {
    Value(ValueId),
    Enum(Sym),
    List(Span),
    Object(Span),
}

impl Raw {
    fn value(self, ctx: &Ctx<'_>) -> Value {
        match self {
            Raw::Value(id) => *ctx.doc.value(id),
            Raw::Enum(name) => Value::Enum(name),
            Raw::List(items) => Value::List(items),
            Raw::Object(fields) => Value::Object(fields),
        }
    }

    fn display(self, ctx: &Ctx<'_>) -> String {
        match self {
            Raw::Value(id) => display_value(ctx.doc, id),
            Raw::Enum(name) => ctx.str(name).to_string(),
            Raw::List(items) => {
                let items: Vec<_> = ctx.doc.value_items[items.range()]
                    .iter()
                    .map(|&item| display_value(ctx.doc, item))
                    .collect();
                format!("[{}]", items.join(", "))
            }
            Raw::Object(fields) => {
                let fields: Vec<_> = ctx.doc.object_fields[fields.range()]
                    .iter()
                    .map(|&(name, value)| {
                        format!("{}: {}", ctx.str(name), display_value(ctx.doc, value))
                    })
                    .collect();
                format!("{{{}}}", fields.join(", "))
            }
        }
    }
}

fn is_custom_scalar(name: &str) -> bool {
    !matches!(name, "String" | "Int" | "Float" | "Boolean" | "ID")
}

impl Rule {
    fn validate_value(&self, ctx: &Ctx<'_>, errors: &mut Errors, raw: Raw) {
        let Some(input_type) = ctx.current_input_type_literal() else {
            return;
        };
        let named_type = input_type.inner_type();
        let Some(type_def) = ctx.type_by_name(named_type) else {
            return;
        };
        let value = raw.value(ctx);

        if !type_def.is_leaf_type() {
            errors.report_error(error(
                CODE,
                format!(
                    "Expected value of type \"{}\", found {}.",
                    named_type,
                    raw.display(ctx)
                ),
                vec![],
            ));
        }

        if let TypeDefinition::Scalar(scalar) = type_def {
            match (scalar.name.as_str(), value) {
                ("Int", Value::Int(n))
                    if !self.disable_int_value_range_validation && i32::try_from(n).is_err() =>
                {
                    errors.report_error(error(
                        CODE,
                        format!(
                            "Int cannot represent non 32-bit signed integer value: {}",
                            raw.display(ctx)
                        ),
                        vec![],
                    ));
                }
                ("Int", Value::Int(_))
                | ("ID", Value::Int(_))
                | ("ID", Value::String(_))
                | ("Float", Value::Int(_))
                | ("Float", Value::Float(_))
                | ("Boolean", Value::Boolean(_))
                | ("String", Value::String(_)) => return,
                (expected, _) => {
                    if is_custom_scalar(expected) {
                        return;
                    }
                    errors.report_error(error(
                        CODE,
                        format!(
                            "Expected value of type \"{}\", found {}.",
                            expected,
                            raw.display(ctx)
                        ),
                        vec![],
                    ));
                }
            }
        }

        if let TypeDefinition::Enum(enum_def) = type_def {
            match value {
                Value::Enum(name) => {
                    let name = ctx.str(name);
                    if !enum_def.values.iter().any(|v| v.name == name) {
                        errors.report_error(error(
                            CODE,
                            format!(
                                "Value \"{}\" does not exist in \"{}\" enum.",
                                name, enum_def.name
                            ),
                            vec![],
                        ));
                    }
                }
                _ => errors.report_error(error(
                    CODE,
                    format!(
                        "Enum \"{}\" cannot represent non-enum value: {}",
                        enum_def.name,
                        raw.display(ctx)
                    ),
                    vec![],
                )),
            }
        }
    }
}

impl<'a> Visitor<'a> for Rule {
    fn enter_null_value(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors) {
        if let Some(input_type) = ctx.current_input_type_literal() {
            if input_type.is_non_null() {
                errors.report_error(error(
                    CODE,
                    format!("Expected value of type \"{}\", found null", input_type),
                    vec![],
                ));
            }
        }
    }

    fn enter_object_value(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, fields: Span) {
        let Some(input_type @ TypeDefinition::InputObject(def)) = ctx.current_input_type() else {
            // an object literal outside an input object position is only valid for a custom
            // scalar, which `validate_value` allows. objects nested in it have no type, so
            // they're skipped
            if ctx.current_input_type_literal().is_some() {
                self.validate_value(ctx, errors, Raw::Object(fields));
            }
            return;
        };
        let object = &ctx.doc.object_fields[fields.range()];

        for field in &def.fields {
            if field.is_required() && !object.iter().any(|&(k, _)| ctx.str(k) == field.name) {
                errors.report_error(error(
                    CODE,
                    format!(
                        "Field \"{}.{}\" of required type \"{}\" was not provided.",
                        def.name, field.name, field.value_type
                    ),
                    vec![],
                ));
            }
        }

        for &(name, _) in object {
            let name = ctx.str(name);
            if !def.fields.iter().any(|f| f.name == name) {
                errors.report_error(error(
                    CODE,
                    format!(
                        "Field \"{}\" is not defined by type \"{}\".",
                        name, def.name
                    ),
                    vec![],
                ));
            }
        }

        if input_type.is_one_of() {
            match object {
                [(name, value)] if matches!(ctx.doc.value(*value), Value::Null) => errors
                    .report_error(error(
                        CODE,
                        format!(
                            "Field \"{}.{}\" must be non-null.",
                            def.name,
                            ctx.str(*name)
                        ),
                        vec![],
                    )),
                [_] => {}
                _ => errors.report_error(error(
                    CODE,
                    format!(
                        "OneOf Input Object \"{}\" must specify exactly one key.",
                        def.name
                    ),
                    vec![],
                )),
            }
        }
    }

    fn enter_list_value(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, items: Span) {
        if let Some(input_type) = ctx.current_input_type_literal() {
            let nullable = match input_type {
                Type::NonNullType(inner) => inner.as_ref(),
                t => t,
            };
            if !matches!(nullable, Type::ListType(_)) {
                self.validate_value(ctx, errors, Raw::List(items));
            }
        }
    }

    fn enter_enum_value(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, name: Sym) {
        self.validate_value(ctx, errors, Raw::Enum(name));
    }

    fn enter_scalar_value(&mut self, ctx: &mut Ctx<'a>, errors: &mut Errors, value: ValueId) {
        self.validate_value(ctx, errors, Raw::Value(value));
    }
}
