use std::fmt;
use std::ops::Deref;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedDocumentId(String);

impl PersistedDocumentId {
    #[inline]
    pub fn new(id: String) -> Self {
        Self(id)
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    #[inline]
    pub fn from_option(raw: Option<&str>) -> Option<Self> {
        raw.and_then(|raw| raw.try_into().ok())
    }
}

impl TryFrom<&str> for PersistedDocumentId {
    type Error = ();

    fn try_from(raw: &str) -> Result<Self, Self::Error> {
        if raw.is_empty() {
            return Err(());
        }

        // Keep IDs exactly as provided (including algorithm prefixes like
        // "sha256:...") so extraction and storage use the same key.
        Ok(Self::new(raw.to_string()))
    }
}

impl Deref for PersistedDocumentId {
    type Target = str;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl AsRef<str> for PersistedDocumentId {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for PersistedDocumentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ClientIdentity<'a> {
    // Optional client name and version provided by request identification.
    // Not all persisted-document sources need these fields.
    pub name: Option<&'a str>,
    pub version: Option<&'a str>,
}

impl ClientIdentity<'_> {
    pub fn is_empty(&self) -> bool {
        self.name.is_none() && self.version.is_none()
    }
}
