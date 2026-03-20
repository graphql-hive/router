use serde::de;

pub trait MapAccessSerdeExt<'de>: de::MapAccess<'de> {
    #[inline]
    /// Deserializes an optional field value from the current map entry into `slot`.
    /// Returns a duplicate-field error when the same field appears more than once.
    fn deserialize_once_into_option<T>(
        &mut self,
        slot: &mut Option<T>,
        field_name: &'static str,
    ) -> Result<(), Self::Error>
    where
        T: serde::Deserialize<'de>,
    {
        if slot.is_some() {
            return Err(de::Error::duplicate_field(field_name));
        }

        *slot = self.next_value::<Option<T>>()?;
        Ok(())
    }
}

impl<'de, A> MapAccessSerdeExt<'de> for A where A: de::MapAccess<'de> {}
