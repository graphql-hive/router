use std::time::Duration;

use hive_router_config::traffic_shaping::DurationOrExpression;
use vrl::{compiler::Program as VrlProgram, prelude::Function};

use crate::utils::expression::compile_expression;

pub enum DurationOrProgram {
    Duration(Duration),
    Program(Box<VrlProgram>),
}

pub fn compile_duration_expression(
    duration_or_expr: &DurationOrExpression,
    fns: Option<&[Box<dyn Function>]>,
) -> Result<DurationOrProgram, String> {
    match duration_or_expr {
        DurationOrExpression::Duration(dur) => Ok(DurationOrProgram::Duration(*dur)),
        DurationOrExpression::Expression { expression } => {
            let program = compile_expression(expression, fns)?;
            Ok(DurationOrProgram::Program(Box::new(program)))
        }
    }
}
