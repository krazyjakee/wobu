//! Which Tencent namespace we call, which regions exist, and the ticket that
//! stops a poll going to the wrong one.
//!
//! Three constants and one enum, and every one of them is a day somebody has
//! already lost. Split out of the adapter rather than declared next to the HTTP
//! because they are pure values with a rule attached, and a rule that can be
//! tested without a network is a rule that is actually checked.
//!
//! ## The namespace
//!
//! Tencent ships this product under three overlapping namespaces
//! (`docs/08-providers.md`), and the one with 3.1 on it is the one whose version
//! string looks oldest:
//!
//! | | Host | Service | Version | Has 3.1? |
//! | --- | --- | --- | --- | --- |
//! | **International** | `hunyuan.intl.tencentcloudapi.com` | `hunyuan` | `2023-09-01` | **yes** |
//! | International `ai3d` | `ai3d.intl.tencentcloudapi.com` | `ai3d` | `2025-05-13` | endpoint not live |
//! | Mainland China | `ai3d.tencentcloudapi.com` | `ai3d` | `2025-05-13` | yes |
//!
//! The counter-intuitive part is the reason these are constants with no setter:
//! reaching for the newer-looking `2025-05-13` is the obvious thing to do and it
//! is wrong, and a signed `QueryHunyuanTo3DProJob` against `ai3d.intl` answers
//! `ResourceUnavailable.InterfaceNotExist` — which reads like a broken action
//! rather than a namespace mistake.
//!
//! ## The regions
//!
//! [`Region`] has three variants because exactly three work. `ap-guangzhou`
//! appears throughout Tencent's own documentation examples, is the obvious
//! default to reach for, and returns `UnsupportedRegion` here. There is no
//! `Region::from_str` that accepts an arbitrary string for that reason: a region
//! is picked from [`Region::ALL`], and a string that names one is parsed rather
//! than trusted.
//!
//! ## The two 24-hour clocks
//!
//! A `JobId` is valid for 24 hours and so is every result URL. Both are handled
//! by making the thing that expires impossible to keep: [`JobTicket`] is not
//! serialisable and never leaves this crate's stack, and [`crate::GeneratedMesh`]
//! has no field for a URL. See the notes on each.

use std::time::Duration;

/// The host. Signed, and the first thing to change if every call starts
/// answering `ResourceUnavailable.InterfaceNotExist`.
pub const HOST: &str = "hunyuan.intl.tencentcloudapi.com";

/// The service, which is the middle segment of the TC3 credential scope. It has
/// to match the host's leading label — signing `ai3d` against a `hunyuan.` host
/// is an `AuthFailure.SignatureFailure`, which reads as a bad key.
pub const SERVICE: &str = "hunyuan";

/// **`2023-09-01`, and not `2025-05-13`.** The newer capability lives under the
/// older-looking version string; see the module note.
pub const VERSION: &str = "2023-09-01";

/// The action that starts a job.
pub const SUBMIT: &str = "SubmitHunyuanTo3DProJob";

/// The action that asks about one. Pro, matching [`SUBMIT`] — the Rapid pair is
/// a different product with no `Model` parameter and no 3.1.
pub const QUERY: &str = "QueryHunyuanTo3DProJob";

/// How long a `JobId` stays valid.
///
/// Not enforced by a timer — see [`JobTicket`] — but the number the poll deadline
/// is checked against, so that raising the deadline past it is a test failure
/// rather than a job that vanishes overnight.
pub const JOB_ID_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);

/// One of the three regions the international endpoint actually serves.
///
/// Sweeping twelve regions against `QueryHunyuanTo3DProJob` on 2026-07-31, these
/// three answered and every other returned `UnsupportedRegion` — including
/// `ap-guangzhou`, `ap-hongkong`, `ap-tokyo`, `ap-seoul`, `ap-bangkok`,
/// `ap-mumbai`, `ap-jakarta`, `na-ashburn`, `na-toronto` and `sa-saopaulo`.
///
/// An enum and not a string, because the failure a string produces is a
/// well-formed signed request that is refused for a reason the user cannot fix,
/// and because it makes the region a *value* that a [`JobTicket`] can carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Region {
    ApSingapore,
    NaSiliconValley,
    EuFrankfurt,
}

impl Region {
    /// Every region the endpoint serves. This is the whole dropdown.
    pub const ALL: [Region; 3] = [Region::ApSingapore, Region::NaSiliconValley, Region::EuFrankfurt];

    pub fn as_str(self) -> &'static str {
        match self {
            Region::ApSingapore => "ap-singapore",
            Region::NaSiliconValley => "na-siliconvalley",
            Region::EuFrankfurt => "eu-frankfurt",
        }
    }

    /// The name a person sees in a picker.
    pub fn label(self) -> &'static str {
        match self {
            Region::ApSingapore => "Singapore (Asia-Pacific)",
            Region::NaSiliconValley => "Silicon Valley (North America)",
            Region::EuFrankfurt => "Frankfurt (Europe)",
        }
    }

    /// Read a region out of `project.json`.
    ///
    /// `None` for anything else, `ap-guangzhou` very much included — a project
    /// file written by hand, or copied from one of Tencent's own examples, must
    /// not produce a request that is signed, sent and refused.
    pub fn parse(name: &str) -> Option<Region> {
        Region::ALL.into_iter().find(|region| region.as_str() == name)
    }

    /// The nearest region to a machine at this UTC offset.
    ///
    /// `docs/08-providers.md` asks for the default to be "by rough geographic
    /// proximity", and the UTC offset is the only proxy for longitude available
    /// without asking the user or the network where they are. The three anchors
    /// are the offsets of the three sites, and the distance is measured around
    /// the clock rather than along the number line — otherwise New Zealand at
    /// +12 would be routed to Frankfurt.
    ///
    /// Rough on purpose. Latency across an ocean is a second on a job that takes
    /// minutes, so the cost of getting this wrong is small and the cost of asking
    /// the user before they have done anything is not.
    pub fn nearest_to_utc_offset(hours: i32) -> Region {
        const ANCHORS: [(Region, i32); 3] =
            [(Region::ApSingapore, 8), (Region::EuFrankfurt, 1), (Region::NaSiliconValley, -8)];
        ANCHORS
            .into_iter()
            .min_by_key(|(_, anchor)| {
                let apart = (hours - anchor).rem_euclid(24);
                apart.min(24 - apart)
            })
            .map(|(region, _)| region)
            // `min_by_key` over a non-empty array cannot be `None`; spelled out
            // rather than unwrapped so a future edit to `ANCHORS` that emptied it
            // still compiles into something sane.
            .unwrap_or(Region::ApSingapore)
    }
}

impl std::fmt::Display for Region {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The namespace plus a region: everything a signed call needs except the body.
///
/// A type rather than three arguments because it is the single place a region
/// enters a request, which is what makes "the poll targets the same region as the
/// submit" enforceable. `Copy`, so a [`JobTicket`] can own one without borrowing
/// the backend it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Endpoint {
    pub region: Region,
}

impl Endpoint {
    pub fn new(region: Region) -> Endpoint {
        Endpoint { region }
    }

    /// The call to sign, for an action and a body.
    ///
    /// The only constructor of [`sign::Call`](super::Call) in this crate. Host,
    /// service and version come from the constants above and cannot be passed in
    /// — a per-call override is how a submit and a poll end up in two namespaces.
    ///
    /// Takes `self` by value rather than by reference, which is what lets a
    /// [`JobTicket`] hand its own endpoint straight to it: a borrowing signature
    /// would make `ticket.endpoint().call(..)` a borrow of a temporary, and the
    /// obvious way around that is to reach for the backend's own region instead —
    /// which is the exact bug this type exists to prevent.
    pub fn call<'a>(self, action: &'a str, body: &'a str) -> super::Call<'a> {
        super::Call {
            host: HOST,
            service: SERVICE,
            action,
            version: VERSION,
            region: self.region.as_str(),
            body,
        }
    }
}

/// An accepted job, bound to the endpoint that accepted it.
///
/// **This type is why a poll cannot go to the wrong region.** The submit returns
/// one of these carrying the [`Endpoint`] it used, and the poll builds its call
/// from `ticket.endpoint` rather than from the backend's own field. There is no
/// constructor that takes a job id without a region, so a poll aimed elsewhere is
/// not something to remember not to write — it is something there is no way to
/// write. The failure it prevents is the nastiest kind: a job submitted to
/// Singapore and polled in Frankfurt is not an error, it is
/// `FailedOperation.JobNotFound`, which is indistinguishable from a job that
/// never existed.
///
/// **It is deliberately not `Serialize`.** A `JobId` is valid for 24 hours, so a
/// ticket written into `project.json` is a resumable-looking thing that is dead
/// by morning; and because a ticket cannot be stored, a result URL cannot be
/// reached again later, which is the other half of the same rule. Resuming a job
/// across a restart would be `wobu-jobs`' decision and would need the expiry
/// handled properly — it is not this crate's to make by accident.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobTicket {
    job_id: String,
    endpoint: Endpoint,
}

impl JobTicket {
    pub(crate) fn new(job_id: impl Into<String>, endpoint: Endpoint) -> JobTicket {
        JobTicket { job_id: job_id.into(), endpoint }
    }

    pub(crate) fn job_id(&self) -> &str {
        &self.job_id
    }

    /// The endpoint this job was submitted to, and therefore the only one it can
    /// be asked about.
    pub(crate) fn endpoint(&self) -> Endpoint {
        self.endpoint
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_namespace_is_the_international_hunyuan_one_and_not_either_ai3d() {
        // The single most likely way to lose a day on this provider. `ai3d.intl`
        // answers a signed `QueryHunyuanTo3DProJob` with
        // `ResourceUnavailable.InterfaceNotExist`, which reads like a broken
        // action rather than a wrong host; the mainland `ai3d` host works and is
        // not reachable from a non-mainland account.
        assert_eq!(HOST, "hunyuan.intl.tencentcloudapi.com");
        assert!(!HOST.starts_with("ai3d"), "ai3d.intl is not live and mainland is not ours");
        assert!(HOST.contains(".intl."), "the mainland host is a different account estate");
    }

    #[test]
    fn the_version_is_the_older_looking_one_because_that_is_where_3_1_lives() {
        // The counter-intuitive part, pinned. `2025-05-13` looks newer, is the
        // `ai3d` namespace's version, and does not exist on this host — so the
        // obvious edit to "update" this constant breaks every call.
        assert_eq!(VERSION, "2023-09-01");
        assert_ne!(VERSION, "2025-05-13");
    }

    #[test]
    fn the_service_matches_the_hosts_leading_label_because_the_signature_covers_both() {
        // The credential scope names the service and the canonical request covers
        // the host. Signing `ai3d` against a `hunyuan.` host is an
        // `AuthFailure.SignatureFailure`, which sends the user to regenerate an
        // account-wide master credential that was never the problem.
        assert_eq!(SERVICE, "hunyuan");
        assert_eq!(HOST.split('.').next(), Some(SERVICE));
    }

    #[test]
    fn exactly_three_regions_are_offered_and_ap_guangzhou_is_not_one_of_them() {
        // `ap-guangzhou` appears throughout Tencent's own documentation examples
        // and is the obvious default to reach for. It returns `UnsupportedRegion`
        // on this endpoint. So does every other region in the general list.
        let offered: Vec<&str> = Region::ALL.iter().map(|region| region.as_str()).collect();
        assert_eq!(offered, ["ap-singapore", "na-siliconvalley", "eu-frankfurt"]);

        for rejected in [
            "ap-guangzhou",
            "ap-hongkong",
            "ap-tokyo",
            "ap-seoul",
            "ap-bangkok",
            "ap-mumbai",
            "ap-jakarta",
            "na-ashburn",
            "na-toronto",
            "sa-saopaulo",
        ] {
            assert_eq!(Region::parse(rejected), None, "{rejected} returns UnsupportedRegion");
        }
        assert_eq!(Region::parse("ap-singapore"), Some(Region::ApSingapore));
    }

    #[test]
    fn the_default_region_follows_the_machines_clock_around_the_world_rather_than_along_a_line() {
        // Proximity measured on a 24-hour circle, because a machine at +12 is
        // three hours from Singapore and eleven from Frankfurt — and a subtraction
        // that ignored the wrap would send New Zealand to Germany.
        assert_eq!(Region::nearest_to_utc_offset(8), Region::ApSingapore);
        assert_eq!(Region::nearest_to_utc_offset(12), Region::ApSingapore);
        assert_eq!(Region::nearest_to_utc_offset(1), Region::EuFrankfurt);
        assert_eq!(Region::nearest_to_utc_offset(0), Region::EuFrankfurt, "UK");
        assert_eq!(Region::nearest_to_utc_offset(-8), Region::NaSiliconValley);
        assert_eq!(Region::nearest_to_utc_offset(-5), Region::NaSiliconValley, "US east coast");
        assert_eq!(Region::nearest_to_utc_offset(-11), Region::NaSiliconValley);

        // And every offset lands somewhere, which is what stops a machine with an
        // exotic clock from having no default at all.
        for hours in -12..=14 {
            assert!(Region::ALL.contains(&Region::nearest_to_utc_offset(hours)), "{hours}");
        }
    }

    #[test]
    fn a_ticket_can_only_be_asked_about_at_the_endpoint_that_issued_it() {
        // The correctness bug this type exists to make unwritable: a job
        // submitted to Singapore and polled in Frankfurt is not an error, it is
        // `FailedOperation.JobNotFound` — the same answer as a job that never
        // existed, on a call that was billed.
        let submitted = Endpoint::new(Region::ApSingapore);
        let ticket = JobTicket::new("1338-abc", submitted);

        // A backend that has since been pointed elsewhere changes nothing: the
        // poll reads the region off the ticket, and there is no constructor that
        // takes a job id without one.
        let moved = Endpoint::new(Region::EuFrankfurt);
        assert_ne!(ticket.endpoint(), moved);
        assert_eq!(ticket.endpoint().call(QUERY, "{}").region, "ap-singapore");
        assert_eq!(ticket.job_id(), "1338-abc");
    }

    #[test]
    fn every_signed_call_carries_the_one_namespace_whichever_action_it_is() {
        // Host, service and version are not parameters of `call`, so a submit and
        // a poll cannot end up in two namespaces. That combination is what
        // produces a job id from one product being queried against another.
        let endpoint = Endpoint::new(Region::NaSiliconValley);
        for action in [SUBMIT, QUERY] {
            let call = endpoint.call(action, r#"{"Model":"3.1"}"#);
            assert_eq!(call.host, HOST);
            assert_eq!(call.service, SERVICE);
            assert_eq!(call.version, VERSION);
            assert_eq!(call.region, "na-siliconvalley");
            assert_eq!(call.action, action);
        }
    }

    #[test]
    fn both_actions_are_the_pro_pair_because_only_pro_takes_a_model() {
        // `Model: "3.1"` is a parameter on the Pro submit and the Rapid action has
        // no `Model` field at all. Mixing the two gives a job id one action can
        // see and the other cannot.
        assert!(SUBMIT.contains("Pro"));
        assert!(QUERY.contains("Pro"));
        assert_eq!(SUBMIT.replace("Submit", "Query"), QUERY);
    }
}
