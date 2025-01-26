mod edge;
mod graph;
mod join_field;
mod join_implements;
mod join_type;
mod move_validator;
mod node;
mod operation_advisor;
mod supergraph;

use std::{fs, path::Path};

use graphql_parser_hive_fork::parse_query;
use operation_advisor::OperationAdvisor;
use supergraph::SupergraphIR;

fn main() {
    let current_dir = Path::new(file!()).parent().unwrap().parent().unwrap();
    let supergraph_sdl = fs::read_to_string(current_dir.join("./fixture/dotan.supergraph.graphql"))
        .expect("Unable to read supergraph.graphql");

    let operation = fs::read_to_string(current_dir.join("./fixture/dotan.operation.graphql"))
        .expect("Unable to read dotan.operation.graphql");
    let parsed_operation = parse_query(&operation).unwrap().into_static();

    let parsed_schema = graphql_parser_hive_fork::parse_schema(&supergraph_sdl)
        .unwrap()
        .into_static();
    let supergraph_ir = SupergraphIR::new(&parsed_schema);
    let advisor = OperationAdvisor::new(supergraph_ir);
    let result = advisor.validate(parsed_operation);

    match result {
        Ok(result) => {
            println!("Result = {:?}", result);
        }
        Err(e) => eprintln!("Failed to validate op: {:?}", e),
    }
}
