//! Compatibility wrapper; storage owns every source/editorial write guard.
use crate::error::CommandResult;
use wobu_narrative::Scene;
pub(in crate::commands) fn validate_approval(
    previous: Option<&Scene>,
    next: &Scene,
    _restoring: bool,
) -> CommandResult<()> {
    Ok(wobu_store::project::narrative_review::validate_manual(previous, next)?)
}
