use std::fmt::Debug;

pub enum Step {
    FieldStep { name: String },
}

impl Debug for Step {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Step::FieldStep { name } => f.write_str(name),
        }
    }
}

impl Step {
    pub fn parse_field_step(input: &str) -> Vec<Step> {
        input
            .trim()
            .split(".")
            .map(|n| Step::FieldStep {
                name: n.to_string(),
            })
            .collect()
    }

    pub fn field_name(&self) -> &str {
        match self {
            Step::FieldStep { name } => name,
        }
    }
}
