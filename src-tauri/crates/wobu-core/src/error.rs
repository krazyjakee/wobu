use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("unknown node kind: {0}")]
    UnknownKind(String),

    #[error("`{kind}` is a singleton and already exists in this project")]
    DuplicateSingleton { kind: NodeKindName },

    #[error("`{kind}` nodes cannot nest inside one another")]
    KindDoesNotNest { kind: NodeKindName },

    #[error("a node cannot be its own parent")]
    SelfParent,

    #[error("moving `{child}` under `{parent}` would create a cycle")]
    ParentCycle { child: String, parent: String },

    #[error("a node can only nest inside another node of the same kind")]
    CrossKindParent,

    #[error("name cannot be empty")]
    EmptyName,

    #[error("`{0}` does not reduce to a usable filename")]
    UnslugifiableName(String),
}

/// Newtype so `Error` does not need to depend on the kind registry's Display impl
/// ordering. It is just the snake_case kind string.
pub type NodeKindName = &'static str;
