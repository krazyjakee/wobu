//! The condition and effect language.
//!
//! Read the shape of these enums as the promise they are: a [`Condition`] is
//! boolean operators over comparisons of declared variables against literals or
//! other declared variables, and nothing else. There is no `Call`, no `Script`,
//! no `Raw(String)`, and no escape hatch that takes an expression as text. An
//! [`Effect`] assigns, adds, or names a host command; it cannot compute.
//!
//! That closure is the whole portable contract. The exported package is read by
//! a Unity, Godot or Unreal adapter with no interpreter in it (#173–#176), it is
//! checked by a compiler in CI that has to terminate (#158), and it is analysed
//! for coverage by enumerating finite domains (#171). Every one of those stops
//! working the moment a single variant can hold arbitrary code, and the pressure
//! to add that variant will arrive as a small, reasonable-sounding request. It
//! is refused here, once, so it does not have to be refused in five adapters.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::state::{Name, Owner, StateSchema, Value, VarType, check_literal};

/// A comparison operator. Equality is defined on every type; ordering only on
/// bounded integers — see [`VarType::is_ordered`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl CompareOp {
    pub fn as_str(self) -> &'static str {
        match self {
            CompareOp::Eq => "eq",
            CompareOp::Ne => "ne",
            CompareOp::Lt => "lt",
            CompareOp::Le => "le",
            CompareOp::Gt => "gt",
            CompareOp::Ge => "ge",
        }
    }

    /// Whether this operator needs its operands to be ordered.
    pub fn needs_ordering(self) -> bool {
        !matches!(self, CompareOp::Eq | CompareOp::Ne)
    }
}

/// The right-hand side of a comparison, or the value of an assignment.
///
/// Explicitly tagged rather than untagged, and that costs a word of source at
/// every use site on purpose: `value: trust` would be ambiguous between the enum
/// member `trust` and the variable `trust`, and resolving it by looking at what
/// happens to be declared would mean declaring a new variable silently changed
/// the meaning of a branch written before it existed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Operand {
    Literal(Value),
    Var(Name),
}

/// One comparison of a declared variable against an operand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Comparison {
    pub var: Name,
    pub op: CompareOp,
    pub value: Operand,
}

/// A boolean expression over declared state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Condition {
    /// Always satisfied. The "always" row of the choice table in #151.
    Always,
    /// Never satisfied. Kept as a first-class value rather than expressed as an
    /// unsatisfiable comparison so an author can deliberately close a branch off
    /// and a reader can see that is what happened.
    Never,
    Not(Box<Condition>),
    /// Every child must hold. An empty `all` is satisfied, matching the usual
    /// reading of "all of nothing".
    All(Vec<Condition>),
    /// At least one child must hold. An empty `any` is not satisfied.
    Any(Vec<Condition>),
    Compare(Comparison),
}

impl Condition {
    /// Every variable this condition reads, in the order it reads them.
    ///
    /// The dependency edge #168 needs: when a variable's declaration changes,
    /// this is what says which branches have to be looked at again. Duplicates
    /// are kept because the caller may want to know how often a variable is
    /// mentioned, and de-duplicating is one `collect` away.
    pub fn variables(&self) -> Vec<&Name> {
        let mut out = Vec::new();
        self.collect_variables(&mut out);
        out
    }

    fn collect_variables<'a>(&'a self, out: &mut Vec<&'a Name>) {
        match self {
            Condition::Always | Condition::Never => {}
            Condition::Not(inner) => inner.collect_variables(out),
            Condition::All(items) | Condition::Any(items) => {
                for item in items {
                    item.collect_variables(out);
                }
            }
            Condition::Compare(cmp) => {
                out.push(&cmp.var);
                if let Operand::Var(name) = &cmp.value {
                    out.push(name);
                }
            }
        }
    }
}

/// Assign a value to a declared variable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Assignment {
    pub var: Name,
    pub value: Operand,
}

/// Move a bounded integer by a fixed amount — the `support +10` of #151's
/// choice table.
///
/// Separate from [`Assignment`] rather than expressed as `set x = x + 10`,
/// because that spelling would need arithmetic in [`Operand`], and arithmetic in
/// operands is the first step towards the expression language this module exists
/// to not have. What happens when the result leaves the declared range is a
/// runtime rule and is defined with the runtime (#159); the source model's job
/// is only to say that the range exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Increment {
    pub var: Name,
    pub by: i64,
}

/// Hand something to the game to do — `start_combat`, `play_cutscene`.
///
/// The narrative says *that* a command was issued and with which arguments; it
/// has no opinion about what the host does with it, and cannot observe the
/// result except through a host-owned variable the host later writes. That
/// asymmetry is what keeps the exported package playable by an engine Wobu has
/// never seen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct HostCommand {
    pub name: Name,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<Operand>,
}

/// One consequence of a choice or an outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Effect {
    Set(Assignment),
    Add(Increment),
    Command(HostCommand),
}

impl Effect {
    /// The variable this effect writes, if it writes one. A host command writes
    /// nothing this model can see.
    pub fn writes(&self) -> Option<&Name> {
        match self {
            Effect::Set(a) => Some(&a.var),
            Effect::Add(i) => Some(&i.var),
            Effect::Command(_) => None,
        }
    }
}

/// A condition or effect that does not type-check against the declared state.
///
/// Separate from [`Error`](crate::Error) because these are the failures a writer
/// sees while working — they are attached to a field in a form and to a node on
/// the canvas — so they have to be cheap to clone, compare and carry inside a
/// [`Diagnostic`](crate::Diagnostic).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TypeError {
    #[error("state variable `{0}` is not declared")]
    UndeclaredVariable(Name),

    #[error("`{var}` is {declared}, and this compares it with {found}")]
    Mismatch { var: Name, declared: String, found: String },

    #[error("`{op}` needs values that can be ordered, and `{var}` is {declared}")]
    NotOrdered { var: Name, op: &'static str, declared: String },

    #[error("`{value}` is not a member of `{var}`; declared members are {members}")]
    NotAMember { var: Name, value: Name, members: String },

    #[error("{value} is outside the declared range {min}..={max} of `{var}`")]
    OutOfRange { var: Name, value: i64, min: i64, max: i64 },

    #[error("`{a}` and `{b}` declare different members, so comparing them has no defined answer")]
    IncomparableEnums { a: Name, b: Name },

    #[error("`{0}` is owned by the host game; narrative effects cannot assign it")]
    HostOwned(Name),

    #[error("`add` moves a bounded integer, and `{var}` is {declared}")]
    NotAnInteger { var: Name, declared: String },
}

impl StateSchema {
    /// Type-check a condition. Fails on the first problem, because a comparison
    /// over an undeclared variable makes everything the author might have meant
    /// by the rest of it unknowable.
    pub fn check_condition(&self, condition: &Condition) -> Result<(), TypeError> {
        match condition {
            Condition::Always | Condition::Never => Ok(()),
            Condition::Not(inner) => self.check_condition(inner),
            Condition::All(items) | Condition::Any(items) => {
                items.iter().try_for_each(|item| self.check_condition(item))
            }
            Condition::Compare(cmp) => self.check_comparison(cmp),
        }
    }

    fn check_comparison(&self, cmp: &Comparison) -> Result<(), TypeError> {
        let decl = self.declaration(&cmp.var)?;
        if cmp.op.needs_ordering() && !decl.ty.is_ordered() {
            return Err(TypeError::NotOrdered {
                var: cmp.var.clone(),
                op: cmp.op.as_str(),
                declared: decl.ty.describe(),
            });
        }
        self.check_operand(&cmp.var, &decl.ty, &cmp.value)
    }

    /// Type-check an effect, including who is allowed to write the variable.
    pub fn check_effect(&self, effect: &Effect) -> Result<(), TypeError> {
        match effect {
            Effect::Set(assignment) => {
                let decl = self.writable(&assignment.var)?;
                self.check_operand(&assignment.var, &decl.ty, &assignment.value)
            }
            Effect::Add(increment) => {
                let decl = self.writable(&increment.var)?;
                if !matches!(decl.ty, VarType::Int { .. }) {
                    return Err(TypeError::NotAnInteger {
                        var: increment.var.clone(),
                        declared: decl.ty.describe(),
                    });
                }
                Ok(())
            }
            // A command's parameters are the host's contract, not this model's,
            // so the only thing checkable here is that any variable it reads
            // exists. Claiming to check argument types would be a promise the
            // engine adapter is the only thing able to keep.
            Effect::Command(command) => command.args.iter().try_for_each(|arg| match arg {
                Operand::Literal(_) => Ok(()),
                Operand::Var(name) => self.declaration(name).map(|_| ()),
            }),
        }
    }

    fn writable(&self, name: &Name) -> Result<&crate::state::VariableDecl, TypeError> {
        let decl = self.declaration(name)?;
        if decl.owner == Owner::Host {
            return Err(TypeError::HostOwned(name.clone()));
        }
        Ok(decl)
    }

    fn check_operand(&self, var: &Name, ty: &VarType, operand: &Operand) -> Result<(), TypeError> {
        match operand {
            Operand::Literal(value) => check_literal(var, ty, value),
            Operand::Var(other) => {
                let other_decl = self.declaration(other)?;
                match (ty, &other_decl.ty) {
                    (VarType::Bool, VarType::Bool) => Ok(()),
                    (VarType::Int { .. }, VarType::Int { .. }) => Ok(()),
                    // Two enums are comparable only if they are the same enum.
                    // Comparing `beacon_quest` with `weather` can never be true,
                    // and letting it through would hide the mistyped variable
                    // name that is nearly always the real cause.
                    (VarType::Enum { members: a }, VarType::Enum { members: b }) => {
                        if a == b {
                            Ok(())
                        } else {
                            Err(TypeError::IncomparableEnums { a: var.clone(), b: other.clone() })
                        }
                    }
                    (declared, found) => Err(TypeError::Mismatch {
                        var: var.clone(),
                        declared: declared.describe(),
                        found: found.describe(),
                    }),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::VariableDecl;

    fn name(s: &str) -> Name {
        Name::new(s).unwrap()
    }

    fn decl(n: &str, ty: VarType, default: Value, owner: Owner) -> VariableDecl {
        VariableDecl { name: name(n), ty, default, owner, description: String::new() }
    }

    fn schema() -> StateSchema {
        StateSchema::new([
            decl("trust", VarType::Int { min: -100, max: 100 }, Value::Int(0), Owner::Narrative),
            decl("has_logbook", VarType::Bool, Value::Bool(false), Owner::Narrative),
            decl(
                "beacon_quest",
                VarType::Enum { members: vec![name("investigating"), name("resolved")] },
                Value::Enum(name("investigating")),
                Owner::Narrative,
            ),
            decl(
                "difficulty",
                VarType::Enum { members: vec![name("easy"), name("hard")] },
                Value::Enum(name("easy")),
                Owner::Host,
            ),
        ])
        .unwrap()
    }

    fn cmp(var: &str, op: CompareOp, value: Operand) -> Condition {
        Condition::Compare(Comparison { var: name(var), op, value })
    }

    #[test]
    fn a_well_typed_condition_passes() {
        let condition = Condition::All(vec![
            cmp("trust", CompareOp::Ge, Operand::Literal(Value::Int(40))),
            cmp("has_logbook", CompareOp::Eq, Operand::Literal(Value::Bool(true))),
            cmp(
                "beacon_quest",
                CompareOp::Eq,
                Operand::Literal(Value::Enum(name("investigating"))),
            ),
        ]);
        schema().check_condition(&condition).unwrap();
    }

    #[test]
    fn an_undeclared_variable_is_a_type_error() {
        let err = schema()
            .check_condition(&cmp("morale", CompareOp::Eq, Operand::Literal(Value::Int(1))))
            .unwrap_err();
        assert_eq!(err, TypeError::UndeclaredVariable(name("morale")));
    }

    #[test]
    fn ordering_a_boolean_is_refused() {
        let err = schema()
            .check_condition(&cmp(
                "has_logbook",
                CompareOp::Gt,
                Operand::Literal(Value::Bool(false)),
            ))
            .unwrap_err();
        assert!(matches!(err, TypeError::NotOrdered { .. }), "{err}");
    }

    #[test]
    fn an_enum_member_must_be_declared() {
        let err = schema()
            .check_condition(&cmp(
                "beacon_quest",
                CompareOp::Eq,
                Operand::Literal(Value::Enum(name("abandoned"))),
            ))
            .unwrap_err();
        assert!(matches!(err, TypeError::NotAMember { .. }), "{err}");
        // The message has to say what *is* allowed, or the author is left
        // guessing at a spelling.
        assert!(err.to_string().contains("investigating"), "{err}");
    }

    #[test]
    fn an_integer_literal_must_be_inside_the_declared_range() {
        let err = schema()
            .check_condition(&cmp("trust", CompareOp::Le, Operand::Literal(Value::Int(9_000))))
            .unwrap_err();
        assert!(matches!(err, TypeError::OutOfRange { .. }), "{err}");
    }

    #[test]
    fn comparing_across_types_is_refused() {
        let err = schema()
            .check_condition(&cmp("trust", CompareOp::Eq, Operand::Literal(Value::Bool(true))))
            .unwrap_err();
        assert!(matches!(err, TypeError::Mismatch { .. }), "{err}");
    }

    #[test]
    fn two_different_enums_cannot_be_compared() {
        let err = schema()
            .check_condition(&cmp("beacon_quest", CompareOp::Eq, Operand::Var(name("difficulty"))))
            .unwrap_err();
        assert!(matches!(err, TypeError::IncomparableEnums { .. }), "{err}");
    }

    #[test]
    fn an_effect_cannot_write_a_host_owned_variable() {
        let effect = Effect::Set(Assignment {
            var: name("difficulty"),
            value: Operand::Literal(Value::Enum(name("hard"))),
        });
        assert_eq!(
            schema().check_effect(&effect).unwrap_err(),
            TypeError::HostOwned(name("difficulty"))
        );
    }

    #[test]
    fn a_condition_may_still_read_a_host_owned_variable() {
        // Reading is how the host supplies an input; only writing is refused.
        schema()
            .check_condition(&cmp(
                "difficulty",
                CompareOp::Eq,
                Operand::Literal(Value::Enum(name("hard"))),
            ))
            .unwrap();
    }

    #[test]
    fn add_needs_a_bounded_integer() {
        let err = schema()
            .check_effect(&Effect::Add(Increment { var: name("has_logbook"), by: 1 }))
            .unwrap_err();
        assert!(matches!(err, TypeError::NotAnInteger { .. }), "{err}");
        schema().check_effect(&Effect::Add(Increment { var: name("trust"), by: -10 })).unwrap();
    }

    #[test]
    fn a_host_command_argument_must_still_name_a_declared_variable() {
        let effect = Effect::Command(HostCommand {
            name: name("start_combat"),
            args: vec![Operand::Var(name("morale"))],
        });
        assert!(matches!(
            schema().check_effect(&effect).unwrap_err(),
            TypeError::UndeclaredVariable(_)
        ));
    }

    #[test]
    fn a_condition_reports_the_variables_it_reads() {
        let condition = Condition::Any(vec![
            Condition::Not(Box::new(cmp(
                "has_logbook",
                CompareOp::Eq,
                Operand::Literal(Value::Bool(true)),
            ))),
            cmp("trust", CompareOp::Ge, Operand::Var(name("morale"))),
        ]);
        let read: Vec<_> = condition.variables().iter().map(|n| n.as_str()).collect();
        assert_eq!(read, vec!["has_logbook", "trust", "morale"]);
    }
}
