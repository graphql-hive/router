use std::{fmt::Display, str::FromStr};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy)]
pub struct Percentage {
    value: f64,
}

impl Percentage {
    pub fn from_f64(value: f64) -> Result<Self, String> {
        if !(0.0..=1.0).contains(&value) {
            return Err(format!(
                "Percentage value must be between 0 and 1, got: {}",
                value
            ));
        }
        Ok(Percentage { value })
    }
    pub fn as_f64(&self) -> f64 {
        self.value
    }
}

impl FromStr for Percentage {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s_trimmed = s.trim();
        if let Some(number_part) = s_trimmed.strip_suffix('%') {
            let value: f64 = number_part.parse().map_err(|err| {
                format!(
                    "Failed to parse percentage value '{}': {}",
                    number_part, err
                )
            })?;
            Ok(Percentage::from_f64(value / 100.0)?)
        } else {
            Err(format!(
                "Percentage value must end with '%', got: '{}'",
                s_trimmed
            ))
        }
    }
}

impl Display for Percentage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}%", self.value * 100.0)
    }
}

// Deserializer from `n%` string to `Percentage` struct
impl<'de> Deserialize<'de> for Percentage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Percentage::from_str(&s).map_err(serde::de::Error::custom)
    }
}

// Serializer from `Percentage` struct to `n%` string
impl Serialize for Percentage {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}
