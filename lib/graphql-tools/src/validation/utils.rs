use crate::parser::Pos;
use serde::ser::*;
use serde::{Serialize, Serializer};
use serde_with::{serde_as, SerializeAs};
use std::fmt::Debug;

#[derive(Debug, Default)]
pub struct ValidationErrorContext {
    pub errors: Vec<ValidationError>,
}

impl ValidationErrorContext {
    pub fn new() -> ValidationErrorContext {
        ValidationErrorContext { errors: vec![] }
    }

    pub fn report_error(&mut self, error: ValidationError) {
        self.errors.push(error);
    }
}

struct PositionDef;

impl SerializeAs<Pos> for PositionDef {
    fn serialize_as<S>(value: &Pos, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut s = serializer.serialize_map(Some(2))?;
        s.serialize_entry("line", &value.line)?;
        s.serialize_entry("column", &value.column)?;
        s.end()
    }
}

#[serde_as]
#[derive(Serialize, Debug, Clone)]
pub struct ValidationError {
    #[serde_as(as = "Vec<PositionDef>")]
    pub locations: Vec<Pos>,
    pub message: String,
    #[serde(skip_serializing)]
    pub error_code: &'static str,
}

#[test]
fn serialization_test() {
    let error = ValidationError {
        locations: vec![Pos { line: 1, column: 2 }],
        message: "test".to_string(),
        error_code: "test",
    };
    let serialized = serde_json::to_string(&error).unwrap();
    assert_eq!(
        serialized,
        r#"{"locations":[{"line":1,"column":2}],"message":"test"}"#
    );
}

#[test]
fn serialization_test_vec() {
    let error = ValidationError {
        locations: vec![Pos { line: 1, column: 2 }],
        message: "test".to_string(),
        error_code: "test",
    };
    let serialized = serde_json::to_string(&vec![error]).unwrap();
    assert_eq!(
        serialized,
        r#"[{"locations":[{"line":1,"column":2}],"message":"test"}]"#
    );
}
