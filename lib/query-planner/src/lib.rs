mod edge;
mod graph;
mod join_field;
mod join_implements;
mod join_type;
mod move_validator;
mod node;
pub mod operation_advisor;
pub mod supergraph;

pub fn parse_schema(sdl: &str) -> graphql_parser_hive_fork::schema::Document<'static, String> {
    graphql_parser_hive_fork::parse_schema(sdl)
        .unwrap()
        .into_static()
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use graphql_parser_hive_fork::parse_query;

    use crate::{operation_advisor::OperationAdvisor, supergraph::SupergraphIR};

    #[test]
    fn test_run() {
        let current_dir = Path::new(file!()).parent().unwrap().parent().unwrap();
        let supergraph_sdl =
            fs::read_to_string(current_dir.join("./fixture/dotan.supergraph.graphql"))
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
}
