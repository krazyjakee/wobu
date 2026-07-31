use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

/// What can go wrong between asking a provider for a description and having one
/// we are willing to write down.
///
/// Every variant here is a fault in the model's output rather than in the
/// request, which is why they are all retryable: the alternative is putting
/// half-formed or mistyped data into somebody's world, and a node is canon —
/// once written, it feeds every prompt compiled from that entity.
#[derive(Debug, Error)]
pub enum Error {
    #[error("the response was not JSON: {0}")]
    NotJson(String),

    #[error("the response was {found}, not a JSON object")]
    NotAnObject { found: JsonTypeName },

    #[error("`{section}` is missing from the response")]
    MissingSection { section: SectionName },

    #[error("`{section}` should be {expected} but the response had {found}")]
    WrongSectionType { section: SectionName, expected: &'static str, found: JsonTypeName },

    #[error("`{section}` came back empty")]
    EmptySection { section: SectionName },

    #[error("`{section}` contains `{value}`, which is not a #rrggbb colour")]
    NotAHexColor { section: SectionName, value: String },
}

impl Error {
    /// Whether asking the same provider the same question again is worth doing.
    ///
    /// Matched exhaustively rather than returning a blanket `true` so that the
    /// first non-retryable failure added here — a rejected API key, a quota —
    /// has to state which side of the line it falls on.
    pub fn is_retryable(&self) -> bool {
        match self {
            Error::NotJson(_)
            | Error::NotAnObject { .. }
            | Error::MissingSection { .. }
            | Error::WrongSectionType { .. }
            | Error::EmptySection { .. }
            | Error::NotAHexColor { .. } => true,
        }
    }
}

/// Section keys are `&'static str` from the kind registry, never user input, so
/// they cost nothing to carry in an error. Mirrors `NodeKindName` in
/// `wobu-core`'s error module.
pub type SectionName = &'static str;

/// A JSON type named for a human: "a string", "an array".
pub type JsonTypeName = &'static str;

/// How a value is described in an error message. Deliberately reads as a noun
/// phrase so the `#[error]` strings above form sentences.
pub(crate) fn json_type_name(value: &serde_json::Value) -> JsonTypeName {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "a boolean",
        serde_json::Value::Number(_) => "a number",
        serde_json::Value::String(_) => "a string",
        serde_json::Value::Array(_) => "an array",
        serde_json::Value::Object(_) => "an object",
    }
}
