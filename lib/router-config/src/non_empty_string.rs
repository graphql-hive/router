use std::{
    ops::{Deref, DerefMut},
    str::FromStr,
};

pub struct NonEmptyStringValue {
    value: Option<String>,
}

impl FromStr for NonEmptyStringValue {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed_str = s.trim();
        if trimmed_str.is_empty() {
            Ok(NonEmptyStringValue { value: None })
        } else {
            Ok(NonEmptyStringValue {
                value: Some(trimmed_str.to_string()),
            })
        }
    }
}

impl DerefMut for NonEmptyStringValue {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

impl Deref for NonEmptyStringValue {
    type Target = Option<String>;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}
