use std::{
    cell::RefCell,
    env, fmt, fs, io,
    path::{Path, PathBuf},
};

use schemars::{json_schema, JsonSchema};
use serde::{
    de::{self, Visitor},
    Deserialize, Deserializer, Serialize,
};

#[derive(Debug, Clone, Serialize)]
pub struct FilePath {
    #[serde(flatten)]
    pub relative: String,
    #[serde(skip)]
    pub absolute: String,
}

// This is a workaround/solution to pass some kind of "context" to the deserialization process.
thread_local!(static CONTEXT_START_PATH: RefCell<Option<PathBuf>> = const { RefCell::new(None) });

pub fn with_start_path<F, T>(start_path: &Path, f: F) -> T
where
    F: FnOnce() -> T,
{
    CONTEXT_START_PATH.with(|ctx| {
        *ctx.borrow_mut() = Some(start_path.to_path_buf());
    });

    let result = f();

    CONTEXT_START_PATH.with(|ctx| {
        *ctx.borrow_mut() = None;
    });

    result
}

impl JsonSchema for FilePath {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "FilePath".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        json_schema!({
            "type": "string",
            "format": "path"
        })
    }

    fn inline_schema() -> bool {
        true
    }
}

struct FilePathVisitor;

impl<'de> Visitor<'de> for FilePathVisitor {
    type Value = FilePath;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a string representing a relative file path")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        CONTEXT_START_PATH.with(|ctx| {
            if let Some(start_path) = ctx.borrow().as_ref() {
                match FilePath::resolve_relative(start_path, v, true) {
                    Ok(file_path) => Ok(file_path),
                    Err(err) => Err(E::custom(format!("Failed to canonicalize path: {}", err))),
                }
            } else {
                Err(E::custom(
                    "FilePath deserialization context (start_path) is not set",
                ))
            }
        })
    }
}

impl<'de> Deserialize<'de> for FilePath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(FilePathVisitor)
    }
}

impl FilePath {
    pub fn new_from_relative(relative_path: &str) -> io::Result<FilePath> {
        Self::resolve_relative(&env::current_dir()?, relative_path, false)
    }

    fn resolve_relative<RootPath: AsRef<Path>>(
        base_path: &RootPath,
        relative_path: &str,
        canonicalize: bool,
    ) -> io::Result<FilePath> {
        let absolute_path = base_path.as_ref().join(relative_path);
        let canonical_path = if canonicalize {
            fs::canonicalize(absolute_path)?
        } else {
            absolute_path
        };

        Ok(FilePath {
            relative: relative_path.to_string(),
            absolute: canonical_path.to_string_lossy().to_string(),
        })
    }
}
