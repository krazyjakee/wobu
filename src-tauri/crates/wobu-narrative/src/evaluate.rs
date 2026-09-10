//! Shared checked state semantics for execution and bounded authoring analysis.
use crate::{CompareOp, Condition, Effect, Name, Operand, Owner, Value, VarType};
use std::collections::BTreeMap;
pub type State = BTreeMap<Name, Value>;
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvaluationError {
    #[error("missing state variable {0}")]
    MissingState(String),
    #[error("invalid runtime state: {0}")]
    InvalidState(String),
    #[error("arithmetic overflow or declared range exceeded for {0}")]
    Overflow(String),
}
pub fn operand(value: &Operand, state: &State) -> Result<Value, EvaluationError> {
    match value {
        Operand::Literal(value) => Ok(value.clone()),
        Operand::Var(name) => {
            state.get(name).cloned().ok_or_else(|| EvaluationError::MissingState(name.to_string()))
        }
    }
}
pub fn evaluate(condition: &Condition, state: &State) -> Result<bool, EvaluationError> {
    evaluate_recording(condition, state, &mut Vec::new(), &mut |_, _, _| {})
}
/// Callback order and short-circuit paths are part of the runtime trace contract.
pub fn evaluate_recording(
    condition: &Condition,
    state: &State,
    path: &mut Vec<usize>,
    record: &mut impl FnMut(&[usize], &Condition, bool),
) -> Result<bool, EvaluationError> {
    let mut child = |index, inner: &Condition| {
        path.push(index);
        let result = evaluate_recording(inner, state, path, record);
        path.pop();
        result
    };
    let result = match condition {
        Condition::Always => true,
        Condition::Never => false,
        Condition::Not(inner) => !child(0, inner)?,
        Condition::All(items) => {
            let mut passed = true;
            for (index, item) in items.iter().enumerate() {
                if !child(index, item)? {
                    passed = false;
                    break;
                }
            }
            passed
        }
        Condition::Any(items) => {
            let mut passed = false;
            for (index, item) in items.iter().enumerate() {
                if child(index, item)? {
                    passed = true;
                    break;
                }
            }
            passed
        }
        Condition::Compare(cmp) => {
            let left = state
                .get(&cmp.var)
                .ok_or_else(|| EvaluationError::MissingState(cmp.var.to_string()))?;
            let right = operand(&cmp.value, state)?;
            if left.kind_name() != right.kind_name() {
                return Err(EvaluationError::InvalidState(cmp.var.to_string()));
            }
            match cmp.op {
                CompareOp::Eq => left == &right,
                CompareOp::Ne => left != &right,
                op => {
                    let (Value::Int(a), Value::Int(b)) = (left, right) else {
                        return Err(EvaluationError::InvalidState(cmp.var.to_string()));
                    };
                    match op {
                        CompareOp::Lt => *a < b,
                        CompareOp::Le => *a <= b,
                        CompareOp::Gt => *a > b,
                        CompareOp::Ge => *a >= b,
                        _ => unreachable!(),
                    }
                }
            }
        }
    };
    record(path, condition, result);
    Ok(result)
}
pub fn accepts(ty: &VarType, value: &Value) -> bool {
    match (ty, value) {
        (VarType::Bool, Value::Bool(_)) => true,
        (VarType::Int { min, max }, Value::Int(value)) => (*min..=*max).contains(value),
        (VarType::Enum { members }, Value::Enum(value)) => members.contains(value),
        _ => false,
    }
}
/// Set/Add calculate against the current state; ordered callers commit only after validation.
pub fn assignment(effect: &Effect, state: &State) -> Result<(Name, Value), EvaluationError> {
    match effect {
        Effect::Set(value) => Ok((value.var.clone(), operand(&value.value, state)?)),
        Effect::Add(value) => {
            let before = state
                .get(&value.var)
                .ok_or_else(|| EvaluationError::MissingState(value.var.to_string()))?;
            let Value::Int(before) = before else {
                return Err(EvaluationError::InvalidState(value.var.to_string()));
            };
            let after = before
                .checked_add(value.by)
                .ok_or_else(|| EvaluationError::Overflow(value.var.to_string()))?;
            Ok((value.var.clone(), Value::Int(after)))
        }
        Effect::Command(_) => {
            Err(EvaluationError::InvalidState("host commands have no narrative assignment".into()))
        }
    }
}
pub fn validate_write(
    name: &Name,
    ty: &VarType,
    owner: Owner,
    value: &Value,
) -> Result<(), EvaluationError> {
    if owner != Owner::Narrative {
        return Err(EvaluationError::InvalidState(format!("{name} is host-owned")));
    }
    if !accepts(ty, value) {
        return Err(if matches!(value, Value::Int(_)) {
            EvaluationError::Overflow(name.to_string())
        } else {
            EvaluationError::InvalidState(name.to_string())
        });
    }
    Ok(())
}
