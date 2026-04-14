use crate::federation_spec::directives::FederationDirective;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CostDirective {
    pub weight: u64,
}

impl CostDirective {
    pub const NAME: &str = "cost";
}

impl FederationDirective for CostDirective {
    fn directive_name() -> &'static str {
        Self::NAME
    }

    fn parse(directive: &graphql_tools::parser::schema::Directive<'_, String>) -> Self
    where
        Self: Sized,
    {
        let mut weight = None; // default value

        for (arg_name, arg_value) in &directive.arguments {
            if arg_name == "weight" {
                if let graphql_tools::parser::schema::Value::Int(int_value) = arg_value {
                    weight = int_value.as_i64();
                }
            }
        }

        if let Some(weight) = weight {
            Self {
                weight: weight as u64,
            }
        } else {
            panic!(
                "'cost' directive is missing required 'weight' argument or it is not an integer"
            );
        }
    }
}

impl Ord for CostDirective {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.weight.cmp(&other.weight)
    }
}

impl PartialOrd for CostDirective {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ListSizeDirective {
    pub assumed_size: Option<usize>,
    pub slicing_arguments: Option<Vec<String>>,
    pub sized_fields: Option<Vec<Vec<String>>>,
    pub require_one_slicing_argument: bool,
}

impl ListSizeDirective {
    pub const NAME: &str = "listSize";
}

impl FederationDirective for ListSizeDirective {
    fn directive_name() -> &'static str {
        Self::NAME
    }

    fn parse(directive: &graphql_tools::parser::schema::Directive<'_, String>) -> Self
    where
        Self: Sized,
    {
        let mut assumed_size = None;
        let mut slicing_arguments = None;
        let mut sized_fields = None;
        let mut require_one_slicing_argument = true;

        for (arg_name, arg_value) in &directive.arguments {
            match arg_name.as_str() {
                "assumedSize" => {
                    if let graphql_tools::parser::schema::Value::Int(int_value) = arg_value {
                        assumed_size = int_value.as_i64().map(|v| v as usize);
                    }
                }
                "slicingArguments" => {
                    if let graphql_tools::parser::schema::Value::List(list_value) = arg_value {
                        slicing_arguments = Some(
                            list_value
                                .iter()
                                .filter_map(|item| {
                                    if let graphql_tools::parser::schema::Value::String(str_value) =
                                        item
                                    {
                                        Some(str_value.clone())
                                    } else {
                                        None
                                    }
                                })
                                .collect(),
                        );
                    }
                }
                "sizedFields" => {
                    if let graphql_tools::parser::schema::Value::List(list_value) = arg_value {
                        sized_fields = Some(
                            list_value
                                .iter()
                                .filter_map(|item| {
                                    if let graphql_tools::parser::schema::Value::String(path) = item
                                    {
                                        let mut current = String::new();
                                        let mut parsed_path = Vec::new();

                                        for ch in path.chars() {
                                            if ch.is_ascii_alphanumeric() || ch == '_' {
                                                current.push(ch);
                                            } else if !current.is_empty() {
                                                parsed_path.push(std::mem::take(&mut current));
                                            }
                                        }

                                        if !current.is_empty() {
                                            parsed_path.push(current);
                                        }
                                        Some(parsed_path)
                                    } else {
                                        None
                                    }
                                })
                                .collect(),
                        );
                    }
                }
                "requireOneSlicingArgument" => {
                    if let graphql_tools::parser::schema::Value::Boolean(bool_value) = arg_value {
                        require_one_slicing_argument = *bool_value;
                    }
                }
                _ => {}
            }
        }

        Self {
            assumed_size,
            slicing_arguments,
            sized_fields,
            require_one_slicing_argument,
        }
    }
}

impl Ord for ListSizeDirective {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Compare all arguments nestedly
        self.assumed_size
            .cmp(&other.assumed_size)
            .then(self.slicing_arguments.cmp(&other.slicing_arguments))
            .then(self.sized_fields.cmp(&other.sized_fields))
            .then(
                self.require_one_slicing_argument
                    .cmp(&other.require_one_slicing_argument),
            )
    }
}

impl PartialOrd for ListSizeDirective {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
