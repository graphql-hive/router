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
        for (arg_name, arg_value) in &directive.arguments {
            if arg_name == "weight" {
                if let graphql_tools::parser::schema::Value::Int(int_value) = arg_value {
                    if let Some(weight_value) = int_value.as_i64() {
                        return Self {
                            weight: u64::try_from(weight_value).expect(
                                "'cost' directive 'weight' argument must be a non-negative integer",
                            ),
                        };
                    }
                }
            }
        }

        panic!(
            "'cost' directive is missing required 'weight' argument that must be a non-negative integer"
        );
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
    pub slicing_arguments: Option<Vec<Vec<String>>>,
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
                        assumed_size = int_value.as_i64().map(|v| {
                            usize::try_from(v).expect(
                                "'listSize' directive 'assumedSize' argument must be a non-negative integer",
                            )
                        });
                    }
                }
                "slicingArguments" => {
                    if let graphql_tools::parser::schema::Value::List(list_value) = arg_value {
                        let parsed_val: Vec<Vec<String>> = list_value
                            .iter()
                            .filter_map(|item| {
                                if let graphql_tools::parser::schema::Value::String(str_value) =
                                    item
                                {
                                    Some(
                                        str_value
                                            .split('.')
                                            .map(|str| str.to_string())
                                            .collect::<Vec<_>>(),
                                    )
                                } else {
                                    None
                                }
                            })
                            .collect();
                        if !parsed_val.is_empty() {
                            slicing_arguments = Some(parsed_val);
                        }
                    }
                }
                "sizedFields" => {
                    if let graphql_tools::parser::schema::Value::List(list_value) = arg_value {
                        let parsed_val: Vec<Vec<String>> = list_value
                            .iter()
                            .filter_map(|item| {
                                if let graphql_tools::parser::schema::Value::String(path) = item {
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
                                    if parsed_path.is_empty() {
                                        None
                                    } else {
                                        Some(parsed_path)
                                    }
                                } else {
                                    None
                                }
                            })
                            .collect();
                        if !parsed_val.is_empty() {
                            sized_fields = Some(parsed_val);
                        }
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
