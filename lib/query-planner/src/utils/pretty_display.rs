use std::fmt::{Formatter as FmtFormatter, Result as FmtResult};

pub fn get_indent(depth: usize) -> String {
    "  ".repeat(depth)
}

pub trait PrettyDisplay {
    fn pretty_fmt(&self, f: &mut FmtFormatter<'_>, depth: usize) -> FmtResult;
}
