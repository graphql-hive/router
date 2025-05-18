use crate::{
    parse_operation,
    planner::walker::walk_operation,
    tests::testkit::{init_logger, paths_to_trees, read_supergraph},
    utils::operation_utils::get_operation_to_execute,
};
use std::error::Error;

#[test]
fn one() -> Result<(), Box<dyn Error>> {
    Ok(())
}

#[test]
fn one_with_one_local() -> Result<(), Box<dyn Error>> {
    Ok(())
}

#[test]
fn two_fields_with_the_same_requirements() -> Result<(), Box<dyn Error>> {
    Ok(())
}

#[test]
fn one_more() -> Result<(), Box<dyn Error>> {
    Ok(())
}

#[test]
fn another_two_fields_with_the_same_requirements() -> Result<(), Box<dyn Error>> {
    Ok(())
}

#[test]
fn two_fields() -> Result<(), Box<dyn Error>> {
    Ok(())
}

#[test]
fn two_fields_same_requirement_different_order() -> Result<(), Box<dyn Error>> {
    Ok(())
}

#[test]
fn many() -> Result<(), Box<dyn Error>> {
    Ok(())
}
