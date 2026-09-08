//! Why a line is affected, in terms a person can act on.
//!
//! A fingerprint answers *whether*, and a fingerprint alone is a dead end: told
//! that a bark is out of date, a writer's next question is which edit did it,
//! and "the hash moved" is not an answer. So the comparison here is structural
//! rather than a hash equality, and every difference it finds keeps the address
//! it was found at.
//!
//! The shape of the answer is the one #168 asks for: **source field → context
//! or variant → line**. [`Explanation`] is exactly those three legs, and
//! [`Reason`] is what fills them in — the source address the change was seen
//! at, the context piece or query that carried it, and the line it reached.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{DependencySet, TargetRef};

/// One difference between what a line was written against and what is there
/// now.
///
/// Every variant carries the address it was found at rather than a summary,
/// because the value of this whole exercise is a writer being able to click
/// from "affected" to the field somebody changed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Reason {
    /// The value at a recorded address is different.
    FieldChanged {
        source: String,
    },
    /// An address that had no value now has one — a fact that was deleted has
    /// come back, or a participant has become a character again.
    FieldAdded {
        source: String,
    },
    /// A recorded address no longer resolves. Recorded rather than dropped,
    /// because a deleted fact is a change to every line that cited it.
    FieldRemoved {
        source: String,
    },
    /// The line now consults an address it did not before, and there is nothing
    /// there yet — a claim that cites a fact nobody has written.
    ///
    /// Its own reason rather than a [`Reason::FieldAdded`] with an empty value,
    /// because the two mean opposite things to a reader: one says a value
    /// arrived, this one says a hole did. Both exist so that `compare` finds a
    /// reason for *every* structural difference, which is what lets a caller
    /// treat "no reasons" and "same fingerprint" as the same answer.
    SourceWatched {
        source: String,
    },
    /// The line no longer consults an address that had nothing at it anyway.
    SourceUnwatched {
        source: String,
    },
    /// A record that no old request could have named has entered a candidate
    /// set. This is the case a purely id-based reverse index cannot see.
    MemberAdded {
        query: String,
        member: String,
    },
    /// A record has left a candidate set.
    MemberRemoved {
        query: String,
        member: String,
    },
    /// A record in a candidate set has been rewritten.
    MemberChanged {
        query: String,
        member: String,
    },
    /// The set was resolved against different parameters — a different speaker,
    /// a different participant list, a different reachable fact set.
    QueryParametersChanged {
        query: String,
    },
    QueryAdded {
        query: String,
    },
    QueryRemoved {
        query: String,
    },
    /// A version in the toolchain moved, so the same source would now be
    /// compiled, resolved or prompted differently.
    VersionChanged {
        component: String,
        before: u32,
        after: u32,
    },
    /// The provider, model or settings that produced this wording changed.
    ProducerChanged,
    /// The recorded set describes a different container for the same wording
    /// identity, or a different dependency format version.
    TargetMoved,
}

impl Reason {
    /// The first leg of the explanation: the source address that moved.
    pub fn source(&self) -> String {
        match self {
            Reason::FieldChanged { source }
            | Reason::FieldAdded { source }
            | Reason::FieldRemoved { source }
            | Reason::SourceWatched { source }
            | Reason::SourceUnwatched { source } => source.clone(),
            Reason::MemberAdded { member, .. }
            | Reason::MemberRemoved { member, .. }
            | Reason::MemberChanged { member, .. } => member.clone(),
            Reason::QueryParametersChanged { query }
            | Reason::QueryAdded { query }
            | Reason::QueryRemoved { query } => query.clone(),
            Reason::VersionChanged { component, .. } => component.clone(),
            Reason::ProducerChanged => "provider".into(),
            Reason::TargetMoved => "target".into(),
        }
    }

    /// The second leg: which part of the frozen context, or which query, or
    /// which toolchain input carried this source to the line.
    pub fn context(&self) -> String {
        match self {
            Reason::FieldChanged { .. }
            | Reason::FieldAdded { .. }
            | Reason::FieldRemoved { .. }
            | Reason::SourceWatched { .. }
            | Reason::SourceUnwatched { .. } => "context".into(),
            Reason::MemberAdded { query, .. }
            | Reason::MemberRemoved { query, .. }
            | Reason::MemberChanged { query, .. }
            | Reason::QueryParametersChanged { query }
            | Reason::QueryAdded { query }
            | Reason::QueryRemoved { query } => format!("query {query}"),
            Reason::VersionChanged { .. } => "toolchain".into(),
            Reason::ProducerChanged => "generation settings".into(),
            Reason::TargetMoved => "variant identity".into(),
        }
    }

    /// The sentence shown beside the badge. The backend's own words, so the
    /// pane does not have to invent a second vocabulary for the same fact.
    pub fn message(&self) -> String {
        match self {
            Reason::FieldChanged { source } => format!("{source} was edited."),
            Reason::FieldAdded { source } => format!("{source} now exists."),
            Reason::FieldRemoved { source } => format!("{source} was deleted."),
            Reason::SourceWatched { source } => {
                format!("This line now depends on {source}, which does not exist.")
            }
            Reason::SourceUnwatched { source } => {
                format!("This line no longer depends on {source}.")
            }
            Reason::MemberAdded { query, member } => {
                format!("{member} is new and matches {query}.")
            }
            Reason::MemberRemoved { query, member } => {
                format!("{member} no longer matches {query}.")
            }
            Reason::MemberChanged { query, member } => {
                format!("{member}, considered by {query}, was edited.")
            }
            Reason::QueryParametersChanged { query } => {
                format!("{query} is now resolved against different inputs.")
            }
            Reason::QueryAdded { query } => format!("{query} is now considered here."),
            Reason::QueryRemoved { query } => format!("{query} is no longer considered here."),
            Reason::VersionChanged { component, before, after } => {
                format!("The {component} version moved from {before} to {after}.")
            }
            Reason::ProducerChanged => "The provider, model or generation settings changed.".into(),
            Reason::TargetMoved => {
                "This wording identity is recorded against a different slot or format.".into()
            }
        }
    }
}

/// One row of the Why affected inspector: source field → context/variant →
/// line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Explanation {
    pub source: String,
    pub context: String,
    pub line: String,
    pub message: String,
}

/// Why a target is in the affected set at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AffectedKind {
    /// The line exists and what it was written against has moved.
    Changed,
    /// The line exists and nothing has ever been recorded for it — a new
    /// variant, or every variant after the local index was deleted. Reported
    /// rather than silently indexed so that a rebuild from canonical data and
    /// an incremental update are distinguishable to a caller that cares.
    Untracked,
    /// A recorded line is no longer in the project. Its receipts are retained;
    /// this says only that nothing in the current source answers to them.
    Absent,
}

/// One affected line and the complete reason it is affected.
///
/// `before` and `after` are the fingerprints either side of the change. `before`
/// is kept rather than discarded because it is the key every receipt,
/// attestation and production artifact for this line was recorded under, and
/// throwing it away is how a "retained" receipt becomes unfindable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Affected {
    pub target: TargetRef,
    pub kind: AffectedKind,
    pub before: Option<String>,
    pub after: Option<String>,
    pub reasons: Vec<Reason>,
}

impl Affected {
    /// The Why affected rows for this line, in the order the reasons were
    /// found — which is address order, so two runs over the same change read
    /// the same way.
    pub fn explanations(&self) -> Vec<Explanation> {
        let line = self.target.line();
        self.reasons
            .iter()
            .map(|reason| Explanation {
                source: reason.source(),
                context: reason.context(),
                line: line.clone(),
                message: reason.message(),
            })
            .collect()
    }
}

/// Every way `current` differs from `previous`, in a deterministic order.
///
/// Structural rather than a fingerprint comparison, and the two must agree: a
/// non-empty result here and an unchanged fingerprint would mean a difference
/// nothing keys on, and an equal result with a moved fingerprint would mean a
/// change nothing can explain. `tests/compare.rs` asserts the equivalence in
/// both directions.
pub fn compare(previous: &DependencySet, current: &DependencySet) -> Vec<Reason> {
    let mut reasons = Vec::new();

    if previous.target != current.target || previous.version != current.version {
        reasons.push(Reason::TargetMoved);
    }
    for ((component, before), (_, after)) in
        previous.versions.components().into_iter().zip(current.versions.components())
    {
        if before != after {
            reasons.push(Reason::VersionChanged {
                component: component.to_string(),
                before,
                after,
            });
        }
    }
    if previous.producer != current.producer {
        reasons.push(Reason::ProducerChanged);
    }

    // Four states per address, not two: an address can be unwatched, watched and
    // empty, or watched with a value, and the transitions between them mean
    // different things. Collapsing "watched and empty" into "unwatched" is the
    // mistake that makes deleting a fact invisible; collapsing it into "has a
    // value" is the one that makes every missing record look like an edit.
    let addresses: BTreeSet<_> = previous.fields.keys().chain(current.fields.keys()).collect();
    for address in addresses {
        let source = address.clone();
        let before = previous.fields.get(address);
        let after = current.fields.get(address);
        match (before.map(Option::as_ref), after.map(Option::as_ref)) {
            (Some(Some(a)), Some(Some(b))) if a != b => {
                reasons.push(Reason::FieldChanged { source })
            }
            (Some(None) | None, Some(Some(_))) => reasons.push(Reason::FieldAdded { source }),
            (Some(Some(_)), Some(None) | None) => reasons.push(Reason::FieldRemoved { source }),
            (None, Some(None)) => reasons.push(Reason::SourceWatched { source }),
            (Some(None), None) => reasons.push(Reason::SourceUnwatched { source }),
            _ => {}
        }
    }

    let names: BTreeSet<_> = previous.queries.keys().chain(current.queries.keys()).collect();
    for name in names {
        let query = name.clone();
        match (previous.queries.get(name), current.queries.get(name)) {
            (None, Some(_)) => reasons.push(Reason::QueryAdded { query }),
            (Some(_), None) => reasons.push(Reason::QueryRemoved { query }),
            (None, None) => {}
            (Some(before), Some(after)) => {
                if before.parameters != after.parameters {
                    reasons.push(Reason::QueryParametersChanged { query: query.clone() });
                }
                let members: BTreeSet<_> =
                    before.members.keys().chain(after.members.keys()).collect();
                for member in members {
                    let (query, member) = (query.clone(), member.clone());
                    match (before.members.get(&member), after.members.get(&member)) {
                        (Some(a), Some(b)) if a == b => {}
                        (Some(_), Some(_)) => reasons.push(Reason::MemberChanged { query, member }),
                        (None, Some(_)) => reasons.push(Reason::MemberAdded { query, member }),
                        (Some(_), None) => reasons.push(Reason::MemberRemoved { query, member }),
                        (None, None) => {}
                    }
                }
            }
        }
    }

    reasons
}
