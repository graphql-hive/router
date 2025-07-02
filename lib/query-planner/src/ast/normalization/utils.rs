use std::{collections::HashSet, hash::Hash};

use graphql_parser::query as query_ast;

pub fn extract_type_condition<'a, 'd, T: query_ast::Text<'d>>(
    type_condition: &'a query_ast::TypeCondition<'d, T>,
) -> String {
    match type_condition {
        query_ast::TypeCondition::On(v) => v.as_ref().to_string(),
    }
}

pub fn vec_to_hashset<T>(values: &[T]) -> HashSet<T>
where
    T: Hash + std::cmp::Eq + Clone,
{
    let mut hset: HashSet<T> = HashSet::new();

    for value in values {
        hset.insert(value.clone());
    }

    hset
}
