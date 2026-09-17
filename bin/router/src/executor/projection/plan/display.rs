use super::*;
use crate::query_planner::utils::pretty_display::{get_indent, PrettyDisplay};
use std::fmt::{Display, Formatter as FmtFormatter, Result as FmtResult};

impl Display for ProjectionPlan {
    fn fmt(&self, f: &mut FmtFormatter<'_>) -> FmtResult {
        self.pretty_fmt(f, 0)
    }
}

impl PrettyDisplay for ProjectionPlan {
    fn pretty_fmt(&self, f: &mut FmtFormatter<'_>, depth: usize) -> FmtResult {
        for field in self.root_fields() {
            self.pretty_fmt_field(field, f, depth)?;
        }
        Ok(())
    }
}

impl ProjectionPlan {
    fn pretty_fmt_field(
        &self,
        field: &FieldRecord,
        f: &mut FmtFormatter<'_>,
        depth: usize,
    ) -> FmtResult {
        let indent = get_indent(depth);
        let field_name = self.field_name(field);
        let response_key = self.response_key(field);

        if response_key == field_name {
            writeln!(f, "{indent}{response_key}: {{")?;
        } else {
            writeln!(f, "{indent}{response_key} (alias for {field_name}) {{")?;
        }

        if let Some(guard) = field.parent_guard {
            write!(f, "{indent}  type guard: ")?;
            self.fmt_guard(guard, f)?;
            writeln!(f)?;
        }

        if let Some(condition) = field.condition {
            write!(f, "{indent}  conditions: ")?;
            self.fmt_condition(condition, f)?;
            writeln!(f)?;
        }

        if field.has_children() {
            writeln!(f, "{indent}  selections:")?;
            for nested in self.fields(field.children) {
                self.pretty_fmt_field(nested, f, depth + 2)?;
            }
        } else if field.is_null_value() {
            writeln!(f, "{indent}  value: Null")?;
        }

        writeln!(f, "{indent}}}")
    }

    fn fmt_guard(&self, id: TypeGuardId, f: &mut FmtFormatter<'_>) -> FmtResult {
        match self.guard(id) {
            Guard::Exact(symbol) => write!(f, "Exact({})", self.text(symbol)),
            Guard::Set(set) => {
                write!(f, "OneOf(")?;
                self.fmt_set(set, f)?;
                write!(f, ")")
            }
        }
    }

    fn fmt_condition(&self, id: ConditionId, f: &mut FmtFormatter<'_>) -> FmtResult {
        match self.condition(id) {
            Condition::Include(variable) => write!(f, "Include(if: ${})", self.text(variable)),
            Condition::Skip(variable) => write!(f, "Skip(if: ${})", self.text(variable)),
            Condition::ParentType(guard) => {
                write!(f, "ParentType(")?;
                self.fmt_guard(guard, f)?;
                write!(f, ")")
            }
            Condition::FieldType(guard) => {
                write!(f, "FieldType(")?;
                self.fmt_guard(guard, f)?;
                write!(f, ")")
            }
            Condition::EnumValues(set) => {
                write!(f, "EnumValues(")?;
                self.fmt_set(set, f)?;
                write!(f, ")")
            }
            Condition::And(left, right) => {
                write!(f, "(")?;
                self.fmt_condition(left, f)?;
                write!(f, " AND ")?;
                self.fmt_condition(right, f)?;
                write!(f, ")")
            }
            Condition::Or(left, right) => {
                write!(f, "(")?;
                self.fmt_condition(left, f)?;
                write!(f, " OR ")?;
                self.fmt_condition(right, f)?;
                write!(f, ")")
            }
        }
    }

    fn fmt_set(&self, id: SetId, f: &mut FmtFormatter<'_>) -> FmtResult {
        let range = self.tables.sets[id.0 as usize];
        for (index, member) in self.tables.set_members[range.start as usize..range.end()]
            .iter()
            .enumerate()
        {
            if index > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}", self.text(*member))?;
        }
        Ok(())
    }
}
