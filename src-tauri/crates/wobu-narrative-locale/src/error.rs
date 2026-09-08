use thiserror::Error as ThisError;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq, ThisError)]
pub enum Error {
    #[error(
        "`{value}` is not a usable locale: expected a language tag such as `en`, `pt-BR` or \
         `zh-Hant-HK`"
    )]
    Locale { value: String },

    /// The interchange file could not be split into rows and columns at all.
    /// Carries a 1-based line number because the only useful thing to tell
    /// somebody holding a broken spreadsheet is where to look.
    #[error("line {line}: {message}")]
    Interchange { line: usize, message: String },

    #[error(
        "locale pack is schema version {found}; this build reads version {supported}. \
         Open the project with a newer Wobu rather than saving over it."
    )]
    UnsupportedSchemaVersion { found: u32, supported: u32 },

    #[error("`{0}` is not a plural category: expected zero, one, two, few, many or other")]
    PluralCategory(String),

    #[error("{0}")]
    Malformed(String),
}

impl Error {
    pub(crate) fn interchange(line: usize, message: impl Into<String>) -> Error {
        Error::Interchange { line, message: message.into() }
    }
}
