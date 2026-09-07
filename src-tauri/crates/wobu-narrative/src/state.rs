//! Declared state: the finite, typed domain that every condition and effect
//! ranges over.
//!
//! This is a *seam*, not the world model. The full picture of what is true, who
//! believes it, and how they came to — facts, beliefs with provenance, directed
//! relationships, events — is a larger thing that lives elsewhere (#155). What
//! a scene needs from it is much smaller: the set of variables it is allowed to
//! mention, what type each one is, and who is allowed to write it. That is all
//! this module defines, and keeping it that small is what lets the scene model
//! be type-checked without loading a project.
//!
//! Every domain here is finite and declared. That is a deliberate restriction
//! rather than a simplification: the exported contract has to be analysable by a
//! compiler that runs in CI (#158) and executable by an engine runtime that has
//! no interpreter in it (#159), and neither is possible over open-ended values.

use std::fmt;
use std::str::FromStr;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::expr::TypeError;

/// Bare words YAML reads as something other than a string.
///
/// A name from this list survives being written and comes back as `true` or
/// `null`, so the comparison an author wrote against it stops matching a value
/// that looks identical in the file. Rejected at declaration, where there is
/// still somebody to tell.
const RESERVED: &[&str] =
    &["true", "false", "yes", "no", "on", "off", "y", "n", "null", "nil", "none"];

/// A declared identifier: a state variable, an enum member, or a host command.
///
/// One type for all three because they obey the same rule and appear in the
/// same places — a picker offering declared variables (US-02) has to be able to
/// show them, and an engine adapter in another language has to be able to parse
/// them without a Unicode table.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Name(String);

impl Name {
    /// Validate and wrap. Rejects anything that is not `[a-z][a-z0-9_]*`, and
    /// anything on the YAML bare-word list.
    pub fn new(value: impl Into<String>) -> Result<Name> {
        let value = value.into();
        let mut chars = value.chars();
        let starts_well = matches!(chars.next(), Some(c) if c.is_ascii_lowercase());
        if !starts_well || !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return Err(Error::InvalidName(value));
        }
        if RESERVED.contains(&value.as_str()) {
            return Err(Error::ReservedName(value));
        }
        Ok(Name(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for Name {
    type Err = Error;

    fn from_str(s: &str) -> Result<Name> {
        Name::new(s)
    }
}

impl TryFrom<String> for Name {
    type Error = Error;

    fn try_from(value: String) -> Result<Name> {
        Name::new(value)
    }
}

impl From<Name> for String {
    fn from(name: Name) -> String {
        name.0
    }
}

/// One value of one declared variable.
///
/// Untagged so a source file says `default: 40` and `default: investigating`
/// rather than wrapping every literal in a type name. That is only safe because
/// [`Name`] refuses the bare words YAML would otherwise re-read as booleans, so
/// the three cases cannot overlap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Enum(Name),
}

impl Value {
    /// How this value is described in a type error, in the words an author
    /// would use rather than the Rust variant name.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Value::Bool(_) => "a boolean",
            Value::Int(_) => "an integer",
            Value::Enum(_) => "an enum member",
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(i) => write!(f, "{i}"),
            Value::Enum(name) => write!(f, "{name}"),
        }
    }
}

/// The type of a declared variable.
///
/// Three cases, all finite. There is no string, no float and no list, and that
/// is the point: a free-text variable cannot be enumerated, so the moment one
/// exists the bounded variant planning of #170 and the coverage analysis of
/// #171 both stop being able to say anything useful about the scenes that use
/// it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum VarType {
    Bool,
    /// An ordered set of declared members. Order is the declaration order and is
    /// kept, because it is what a picker shows; it carries no comparison
    /// meaning — enums are compared for equality only.
    Enum {
        members: Vec<Name>,
    },
    /// A closed integer interval, inclusive at both ends.
    Int {
        min: i64,
        max: i64,
    },
}

impl VarType {
    /// How this type is described in a type error.
    pub fn describe(&self) -> String {
        match self {
            VarType::Bool => "a boolean".to_string(),
            VarType::Enum { .. } => "an enum".to_string(),
            VarType::Int { min, max } => format!("an integer in {min}..={max}"),
        }
    }

    /// Whether `<`, `<=`, `>` and `>=` mean anything here.
    ///
    /// Only integers order. Booleans and enums are sets of names; asking whether
    /// `investigating > resolved` has no answer the author intended, and
    /// answering it by declaration order would silently make a reordering of the
    /// picker list change what a branch does.
    pub fn is_ordered(&self) -> bool {
        matches!(self, VarType::Int { .. })
    }
}

/// Who is allowed to write a variable.
///
/// #151 requires exactly one owner per variable, and this is the distinction
/// that check can be built on: a variable the host game supplies is an *input*,
/// and an effect that assigns one is a bug the compiler should refuse rather
/// than a race the player discovers. A finer-grained scheme — ownership by quest
/// or by system — belongs with the world model in #155; it can be added beside
/// this without changing what the runtime has to enforce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Owner {
    /// Narrative effects are the only writer.
    #[default]
    Narrative,
    /// The host game is the only writer. Conditions may read it; effects may not
    /// assign it.
    Host,
}

/// One declared state variable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct VariableDecl {
    pub name: Name,
    #[serde(rename = "type")]
    pub ty: VarType,
    /// The value before anything has assigned one. Required rather than
    /// defaulted: "what is `trust` at the start" has no answer this crate could
    /// invent that would not be wrong for some project, and a missing initial
    /// value is the classic source of a branch that behaves differently on the
    /// first playthrough than on a reload.
    pub default: Value,
    #[serde(default)]
    pub owner: Owner,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// The declared variables a scene is checked against.
///
/// Ordered, because the declaration order is what a variable picker offers and
/// a set that reshuffles between sessions is a picker that moves under the
/// cursor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StateSchema {
    variables: IndexMap<Name, VariableDecl>,
}

impl StateSchema {
    /// The schema with nothing declared. Every condition mentioning a variable
    /// fails against it, which is the correct answer for a scene opened without
    /// its project rather than a reason to skip checking.
    pub fn empty() -> StateSchema {
        StateSchema::default()
    }

    /// Build a schema, checking each declaration against itself.
    ///
    /// The self-consistency checks happen here rather than at first use because
    /// a variable whose default is outside its own range is wrong in every scene
    /// that mentions it, and reporting it once at the declaration is the only
    /// place the author can fix it.
    pub fn new(decls: impl IntoIterator<Item = VariableDecl>) -> Result<StateSchema> {
        let mut variables = IndexMap::new();
        for decl in decls {
            match &decl.ty {
                VarType::Enum { members } if members.is_empty() => {
                    return Err(Error::EmptyEnum(decl.name));
                }
                VarType::Int { min, max } if min > max => {
                    return Err(Error::EmptyRange { name: decl.name, min: *min, max: *max });
                }
                _ => {}
            }
            if let Err(source) = check_literal(&decl.name, &decl.ty, &decl.default) {
                return Err(Error::InvalidDefault { name: decl.name, source });
            }
            if let Some(previous) = variables.insert(decl.name.clone(), decl) {
                return Err(Error::DuplicateVariable(previous.name));
            }
        }
        Ok(StateSchema { variables })
    }

    pub fn get(&self, name: &Name) -> Option<&VariableDecl> {
        self.variables.get(name)
    }

    /// Declarations in declaration order.
    pub fn iter(&self) -> impl Iterator<Item = &VariableDecl> {
        self.variables.values()
    }

    pub fn len(&self) -> usize {
        self.variables.len()
    }

    pub fn is_empty(&self) -> bool {
        self.variables.is_empty()
    }

    pub(crate) fn declaration(&self, name: &Name) -> std::result::Result<&VariableDecl, TypeError> {
        self.variables.get(name).ok_or_else(|| TypeError::UndeclaredVariable(name.clone()))
    }
}

/// Whether `value` is a legal value of `ty`.
///
/// Shared by the schema's own default check and by every condition and effect
/// check, so a literal that is rejected in a comparison is rejected identically
/// as a default. Two implementations of this rule would drift, and the symptom
/// would be a project that compiles but whose first assignment is out of range.
pub(crate) fn check_literal(
    var: &Name,
    ty: &VarType,
    value: &Value,
) -> std::result::Result<(), TypeError> {
    match (ty, value) {
        (VarType::Bool, Value::Bool(_)) => Ok(()),
        (VarType::Int { min, max }, Value::Int(n)) => {
            if n < min || n > max {
                return Err(TypeError::OutOfRange {
                    var: var.clone(),
                    value: *n,
                    min: *min,
                    max: *max,
                });
            }
            Ok(())
        }
        (VarType::Enum { members }, Value::Enum(member)) => {
            if members.contains(member) {
                return Ok(());
            }
            Err(TypeError::NotAMember {
                var: var.clone(),
                value: member.clone(),
                members: members.iter().map(Name::as_str).collect::<Vec<_>>().join(", "),
            })
        }
        (ty, value) => Err(TypeError::Mismatch {
            var: var.clone(),
            declared: ty.describe(),
            found: value.kind_name().to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(s: &str) -> Name {
        Name::new(s).unwrap()
    }

    fn trust() -> VariableDecl {
        VariableDecl {
            name: name("trust"),
            ty: VarType::Int { min: -100, max: 100 },
            default: Value::Int(0),
            owner: Owner::Narrative,
            description: String::new(),
        }
    }

    #[test]
    fn names_are_lowercase_identifiers() {
        name("beacon_quest");
        assert!(matches!(Name::new("Beacon Quest"), Err(Error::InvalidName(_))));
        assert!(matches!(Name::new("2fast"), Err(Error::InvalidName(_))));
        assert!(matches!(Name::new(""), Err(Error::InvalidName(_))));
    }

    #[test]
    fn yaml_bare_words_are_refused_as_names() {
        // `off` would round-trip through YAML as `false` and stop matching the
        // comparison written against it.
        assert!(matches!(Name::new("off"), Err(Error::ReservedName(_))));
        assert!(matches!(Name::new("null"), Err(Error::ReservedName(_))));
    }

    #[test]
    fn a_duplicate_declaration_is_refused() {
        let err = StateSchema::new([trust(), trust()]).unwrap_err();
        assert!(matches!(err, Error::DuplicateVariable(_)), "{err}");
    }

    #[test]
    fn a_default_outside_its_own_range_is_refused() {
        let mut decl = trust();
        decl.default = Value::Int(500);
        let err = StateSchema::new([decl]).unwrap_err();
        assert!(matches!(err, Error::InvalidDefault { .. }), "{err}");
    }

    #[test]
    fn an_enum_with_no_members_is_refused() {
        let decl = VariableDecl {
            name: name("beacon_quest"),
            ty: VarType::Enum { members: vec![] },
            default: Value::Enum(name("investigating")),
            owner: Owner::Narrative,
            description: String::new(),
        };
        assert!(matches!(StateSchema::new([decl]), Err(Error::EmptyEnum(_))));
    }

    #[test]
    fn declaration_order_is_the_order_a_picker_sees() {
        let mut second = trust();
        second.name = name("support");
        let schema = StateSchema::new([trust(), second]).unwrap();
        let names: Vec<_> = schema.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["trust", "support"]);
    }

    #[test]
    fn only_integers_order() {
        assert!(VarType::Int { min: 0, max: 1 }.is_ordered());
        assert!(!VarType::Bool.is_ordered());
        assert!(!VarType::Enum { members: vec![name("a")] }.is_ordered());
    }
}
