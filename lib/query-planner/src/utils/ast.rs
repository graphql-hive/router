use graphql_tools::parser::query::{Directive, Text};

/// Checks if two directives are equal without comparing their positions
pub fn equal_directives<'a, T: Text<'a> + PartialEq>(
    a: &Directive<'a, T>,
    b: &Directive<'a, T>,
) -> bool {
    if a.name != b.name {
        return false;
    }

    if a.arguments != b.arguments {
        return false;
    }

    true
}

/// Checks if two arrays of directives are equal without comparing their positions
pub fn equal_directives_arr<'a, T: Text<'a> + PartialEq>(
    a: &[Directive<'a, T>],
    b: &[Directive<'a, T>],
) -> bool {
    if a.len() != b.len() {
        return false;
    }

    for (a, b) in a.iter().zip(b) {
        if !equal_directives(a, b) {
            return false;
        }
    }

    true
}
