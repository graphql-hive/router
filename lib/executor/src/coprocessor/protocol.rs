use std::fmt;

use ntex::http::StatusCode;
use serde::de::{self, MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const COPROCESSOR_VERSION: u8 = 1;
const CONTINUE_KEY: &str = "continue";
const BREAK_KEY: &str = "break";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoprocessorControl {
    Continue,
    Break(StatusCode),
}

impl CoprocessorControl {
    pub fn break_status(&self) -> Option<StatusCode> {
        match self {
            CoprocessorControl::Break(status_code) => Some(*status_code),
            CoprocessorControl::Continue => None,
        }
    }
}

impl Serialize for CoprocessorControl {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            CoprocessorControl::Continue => serializer.serialize_str(CONTINUE_KEY),
            CoprocessorControl::Break(status_code) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry(BREAK_KEY, &status_code.as_u16())?;
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for CoprocessorControl {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(CoprocessorControlVisitor)
    }
}

struct CoprocessorControlVisitor;

impl CoprocessorControlVisitor {
    fn parse_continue<E>(&self, value: &str) -> Result<CoprocessorControl, E>
    where
        E: de::Error,
    {
        if value == CONTINUE_KEY {
            Ok(CoprocessorControl::Continue)
        } else {
            Err(E::invalid_value(
                de::Unexpected::Str(value),
                &"\"continue\"",
            ))
        }
    }
}

impl<'de> Visitor<'de> for CoprocessorControlVisitor {
    type Value = CoprocessorControl;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("\"continue\" or {\"break\": <status_code>}")
    }

    fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.parse_continue(value)
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.parse_continue(value)
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.parse_continue(&value)
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let Some(key) = map.next_key::<&str>()? else {
            return Err(de::Error::custom(
                "coprocessor control object cannot be empty",
            ));
        };

        if key != BREAK_KEY {
            return Err(de::Error::custom("unsupported coprocessor control field"));
        }

        let status_code = map.next_value::<u16>()?;
        let status_code = StatusCode::from_u16(status_code)
            .map_err(|error| de::Error::custom(error.to_string()))?;

        if map.next_key::<&str>()?.is_some() {
            return Err(de::Error::custom(
                "coprocessor control object must contain only one field",
            ));
        }

        Ok(CoprocessorControl::Break(status_code))
    }
}
