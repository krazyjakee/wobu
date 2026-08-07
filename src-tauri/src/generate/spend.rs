//! What an image costs, and the ceiling it is charged against.
//!
//! The ledger is files under `.wobu/spend` rather than process state, because
//! two Wobu windows on one project must not each believe the whole remaining
//! ceiling is theirs. A reservation is written before a job starts and either
//! committed at its recorded cost or released on drop, so a crash mid-generation
//! over-counts for one poll rather than losing the spend entirely.

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tauri::State;
use wobu_core::{Generation, Id, new_id};
use wobu_imagine::{Resolution, gemini};
use wobu_jobs::JobKind;
use wobu_store::Project;

use crate::error::{Code, CommandResult, WobuError};
use crate::state::{AppState, Jobs};

const PRICE_SOURCE: &str = "https://ai.google.dev/gemini-api/docs/pricing";

const PRICE_CHECKED_AT: &str = "2026-08-01";

pub(super) const SPEND_DIR: &str = ".wobu/spend";

pub(super) const SPEND_AGGREGATE: &str = "aggregate.json";

const SPEND_AGGREGATE_VERSION: u32 = 1;

const LOCK_ATTEMPTS: usize = 200;

#[derive(Debug, Clone, Copy)]
pub(super) struct Price {
    pub(super) per_image_usd_micros: u64,
    pub(super) conservative_fallback: bool,
}

pub(super) fn apply_pricing_metadata(params: &mut Map<String, Value>, price: Option<Price>) {
    if let Some(price) = price {
        params.insert("pricingCheckedAt".into(), json!(PRICE_CHECKED_AT));
        params.insert("pricingSource".into(), json!(PRICE_SOURCE));
        params.insert("pricingIndicative".into(), json!(true));
        params.insert("pricingConservativeFallback".into(), json!(price.conservative_fallback));
    } else {
        params.remove("pricingCheckedAt");
        params.remove("pricingSource");
        params.remove("pricingIndicative");
        params.remove("pricingConservativeFallback");
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CostEstimate {
    pub(super) currency: &'static str,
    pub(super) per_image_usd_micros: u64,
    pub(super) batch_usd_micros: u64,
    pub(super) images: usize,
    pub(super) varies_by_cell: bool,
    pub(super) indicative: bool,
    pub(super) conservative_fallback: bool,
    pub(super) checked_at: &'static str,
    pub(super) source_url: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpendStatus {
    pub(super) ceiling_usd_micros: Option<u64>,
    pub(super) spent_usd_micros: u64,
    pub(super) reserved_usd_micros: u64,
    pub(super) remaining_usd_micros: Option<u64>,
    pub(super) pending_reservations: usize,
    pub(super) oldest_reservation_at: Option<String>,
    pub(super) ledger_locked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReservationFile {
    pub(super) id: Id,
    pub(super) remaining_usd_micros: u64,
    pub(super) created_at: String,
}

/// Disposable display cache for the immutable receipt ledger.
///
/// Admission never trusts this file. It reconstructs the same values from the
/// canonical receipts while holding [`SpendLock`], then refreshes this cache as
/// a side effect. Losing or corrupting it therefore costs one reconstruction,
/// not either money or history.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SpendAggregate {
    pub(super) version: u32,
    pub(super) spent_usd_micros: u64,
    pub(super) receipts: usize,
}

/// Reconstructed, never trusted from a mutable counter.
#[tauri::command]
pub fn spend_status(state: State<'_, AppState>) -> CommandResult<SpendStatus> {
    let root = state.with(|project| Ok(project.root().to_path_buf()))?;
    spend_status_for_report(&root)
}

/// Change the shared hard ceiling. `null` disables paid generation rather than
/// turning the guard off; local ComfyUI remains unaffected.
#[tauri::command]
pub fn spend_ceiling_set(
    state: State<'_, AppState>,
    ceiling_usd_micros: Option<u64>,
) -> CommandResult<SpendStatus> {
    let root = state.with(|project| {
        if project.is_read_only() {
            return Err(WobuError::new(
                Code::ReadOnly,
                "This project is read-only, so its spend ceiling cannot be changed.",
            ));
        }
        let root = project.root().to_path_buf();
        let _guard = SpendLock::acquire(&root)?;
        project.set_spend_ceiling(ceiling_usd_micros)?;
        Ok(root)
    })?;
    spend_status_for(&root)
}

/// Archive reservations after a crash. This is deliberately explicit and
/// refuses while this process has paid work queued or running. Another machine
/// cannot be interrogated reliably, so the UI requires the user to confirm all
/// other Wobu instances using the project have stopped paid work.
#[tauri::command]
pub fn spend_recovery_reset(
    state: State<'_, AppState>,
    jobs: State<'_, Jobs>,
    confirm_no_paid_jobs: bool,
) -> CommandResult<SpendStatus> {
    if !confirm_no_paid_jobs {
        return Err(WobuError::new(
            Code::Invalid,
            "Confirm that no paid generations are running before recovering spend reservations.",
        ));
    }
    if jobs
        .snapshot()
        .jobs
        .iter()
        .any(|job| job.kind == JobKind::Generate && !job.state.is_terminal())
    {
        return Err(WobuError::new(
            Code::Invalid,
            "A generation is still queued or running in this Wobu window.",
        ));
    }
    let root = state.with(|project| {
        if project.is_read_only() {
            return Err(WobuError::new(
                Code::ReadOnly,
                "This project is read-only, so spend recovery cannot be changed.",
            ));
        }
        Ok(project.root().to_path_buf())
    })?;
    let ledger = root.join(SPEND_DIR);
    if ledger.exists() {
        let archive = root.join(".wobu").join(format!(
            "spend-recovery-{}-{}",
            Utc::now().format("%Y%m%dT%H%M%SZ"),
            new_id()
        ));
        std::fs::rename(&ledger, &archive).map_err(|error| {
            spend_io("The pending spend ledger could not be archived.", &ledger, error)
        })?;
    }
    spend_status_for(&root)
}

pub(super) fn image_price(provider: &str, model: &str, resolution: Resolution) -> Option<Price> {
    if provider != gemini::ID {
        return None;
    }
    let longest = resolution.width.max(resolution.height);
    let (per_image_usd_micros, conservative_fallback) = match model {
        "gemini-3.1-flash-image" if longest <= 512 => (45_000, false),
        "gemini-3.1-flash-image" if longest <= 1_024 => (67_000, false),
        "gemini-3.1-flash-image" if longest <= 2_048 => (101_000, false),
        "gemini-3.1-flash-image" => (151_000, false),
        "gemini-3.1-flash-lite-image" => (33_600, false),
        "gemini-3-pro-image" if longest <= 2_048 => (134_000, false),
        "gemini-3-pro-image" => (240_000, false),
        "gemini-2.5-flash-image" => (39_000, false),
        // A newly selected paid Gemini model must not silently bypass the
        // ceiling before its price is added. Use the highest current known
        // synchronous image price and say that the estimate is conservative.
        _ => (240_000, true),
    };
    Some(Price { per_image_usd_micros, conservative_fallback })
}

#[cfg(test)]
pub(super) fn cost_estimate(
    provider: &str,
    model: &str,
    resolution: Resolution,
    images: usize,
) -> Option<CostEstimate> {
    let price = image_price(provider, model, resolution)?;
    Some(CostEstimate {
        currency: "USD",
        per_image_usd_micros: price.per_image_usd_micros,
        batch_usd_micros: price.per_image_usd_micros.saturating_mul(images as u64),
        images,
        varies_by_cell: false,
        indicative: true,
        conservative_fallback: price.conservative_fallback,
        checked_at: PRICE_CHECKED_AT,
        source_url: PRICE_SOURCE,
    })
}

pub(super) fn cost_estimate_prices(prices: Vec<Price>, images: usize) -> Option<CostEstimate> {
    let first = *prices.first()?;
    let batch_usd_micros =
        prices.iter().fold(0_u64, |total, price| total.saturating_add(price.per_image_usd_micros));
    let varies_by_cell =
        prices.iter().any(|price| price.per_image_usd_micros != first.per_image_usd_micros);
    Some(CostEstimate {
        currency: "USD",
        per_image_usd_micros: first.per_image_usd_micros,
        batch_usd_micros,
        images,
        varies_by_cell,
        indicative: true,
        conservative_fallback: prices.iter().any(|price| price.conservative_fallback),
        checked_at: PRICE_CHECKED_AT,
        source_url: PRICE_SOURCE,
    })
}

pub(super) fn receipt_cost(generation: &Generation) -> u64 {
    if generation.backend != gemini::ID {
        return 0;
    }
    if let Some(cost) = generation.params.get("estimatedCostUsdMicros").and_then(Value::as_u64) {
        return cost;
    }
    let width = generation
        .params
        .get("width")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(1_024);
    let height = generation
        .params
        .get("height")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(1_024);
    image_price(&generation.backend, &generation.model, Resolution::new(width, height))
        .map_or(240_000, |price| price.per_image_usd_micros)
}

pub(super) struct SpendLock {
    pub(super) path: PathBuf,
}

impl SpendLock {
    fn acquire(root: &Path) -> CommandResult<SpendLock> {
        let dir = root.join(SPEND_DIR);
        std::fs::create_dir_all(dir.join("reservations"))
            .map_err(|error| spend_io("The spend ledger could not be prepared.", &dir, error))?;
        let path = dir.join("lock");
        for _ in 0..LOCK_ATTEMPTS {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    writeln!(file, "{}", Utc::now().to_rfc3339()).map_err(|error| {
                        spend_io("The spend ledger lock could not be written.", &path, error)
                    })?;
                    return Ok(SpendLock { path });
                }
                // Never steal by age. A legitimate critical section can be
                // arbitrarily slow on a network share; age is not ownership.
                // Crash recovery is an explicit, user-confirmed archive path.
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => {
                    return Err(spend_io("The spend ledger could not be locked.", &path, error));
                }
            }
        }
        Err(WobuError::new(Code::Io, "The shared spend ledger is busy. Try Generate again."))
    }
}

impl Drop for SpendLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[derive(Debug)]
pub(super) struct SpendReservation {
    pub(super) root: PathBuf,
    pub(super) path: PathBuf,
    pub(super) file: ReservationFile,
    pub(super) release_on_drop: bool,
}

impl SpendReservation {
    pub(super) fn create(root: &Path, amount_usd_micros: u64) -> CommandResult<SpendReservation> {
        let _guard = SpendLock::acquire(root)?;
        let status = reconstruct_spend_status_locked(root)?;
        let ceiling = status.ceiling_usd_micros.ok_or_else(|| {
            WobuError::new(
                Code::SpendCeilingExceeded,
                "Paid generation is disabled for this project. Set a spend ceiling first.",
            )
        })?;
        let committed = status
            .spent_usd_micros
            .checked_add(status.reserved_usd_micros)
            .and_then(|used| used.checked_add(amount_usd_micros))
            .ok_or_else(|| {
                WobuError::new(Code::Invalid, "The project spend total is too large.")
            })?;
        if committed > ceiling {
            return Err(WobuError::new(
                Code::SpendCeilingExceeded,
                "This batch would cross the project's shared spend ceiling.",
            )
            .with_detail(format!(
                "spent={} reserved={} batch={} ceiling={} USD micros",
                status.spent_usd_micros, status.reserved_usd_micros, amount_usd_micros, ceiling,
            )));
        }
        let id = new_id();
        let path = root.join(SPEND_DIR).join("reservations").join(format!("{id}.json"));
        let file = ReservationFile {
            id,
            remaining_usd_micros: amount_usd_micros,
            created_at: Utc::now().to_rfc3339(),
        };
        write_reservation_new(&path, &file)?;
        Ok(SpendReservation { root: root.to_path_buf(), path, file, release_on_drop: true })
    }

    pub(super) fn commit(&mut self, amount_usd_micros: u64) -> CommandResult<()> {
        if amount_usd_micros > self.file.remaining_usd_micros {
            return Err(WobuError::new(
                Code::Internal,
                "A generation cost exceeded its spend reservation.",
            ));
        }
        let _guard = SpendLock::acquire(&self.root)?;
        // The receipt was persisted before this call. Refresh from canonical
        // bytes rather than adding `amount_usd_micros` to a mutable counter: a
        // second process may have reconstructed the aggregate in the narrow
        // interval between that receipt landing and this lock being acquired.
        // Re-reading makes that interleaving exact instead of double-counting.
        reconstruct_spend_aggregate_with(&self.root, || {
            Project::spend_ledger(&self.root).map_err(WobuError::from)
        })?;
        let remaining = self.file.remaining_usd_micros - amount_usd_micros;
        if remaining > 0 {
            // Reservations are write-once. Publishing the replacement before
            // removing the old file means a crash can only over-reserve, never
            // open a window where concurrent work can overspend.
            let id = new_id();
            let path = self.root.join(SPEND_DIR).join("reservations").join(format!("{id}.json"));
            let replacement = ReservationFile {
                id,
                remaining_usd_micros: remaining,
                created_at: self.file.created_at.clone(),
            };
            write_reservation_new(&path, &replacement)?;
            std::fs::remove_file(&self.path).map_err(|error| {
                spend_io("The previous spend reservation could not be retired.", &self.path, error)
            })?;
            self.path = path;
            self.file = replacement;
        } else {
            std::fs::remove_file(&self.path).map_err(|error| {
                spend_io("The completed spend reservation could not be retired.", &self.path, error)
            })?;
            self.file.remaining_usd_micros = 0;
        }
        Ok(())
    }
}

impl Drop for SpendReservation {
    fn drop(&mut self) {
        if !self.release_on_drop {
            return;
        }
        if let Ok(_guard) = SpendLock::acquire(&self.root) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

pub(super) fn spend_status_for(root: &Path) -> CommandResult<SpendStatus> {
    let _guard = SpendLock::acquire(root)?;
    reconstruct_spend_status_locked(root)
}

pub(super) fn spend_status_for_report(root: &Path) -> CommandResult<SpendStatus> {
    match SpendLock::acquire(root) {
        Ok(_guard) => read_cached_spend_status_locked(root),
        Err(_error) if root.join(SPEND_DIR).join("lock").exists() => {
            // Display-only fallback. Admission never uses this snapshot: it
            // still requires the exclusive lock. This lets the Inspector
            // explain and recover a crash-orphaned lock instead of replacing
            // the whole cost report with an opaque busy error.
            let mut status = read_cached_spend_status_locked(root)?;
            status.ledger_locked = true;
            Ok(status)
        }
        Err(error) => Err(error),
    }
}

/// Admission's view: canonical receipts are opened and validated every time.
/// The aggregate is refreshed only after that succeeds, so no mutable cache can
/// authorise spend or hide a malformed receipt.
fn reconstruct_spend_status_locked(root: &Path) -> CommandResult<SpendStatus> {
    let (ceiling_usd_micros, aggregate) = reconstruct_spend_aggregate_with(root, || {
        Project::spend_ledger(root).map_err(WobuError::from)
    })?;
    status_with_reservations(root, ceiling_usd_micros, aggregate.spent_usd_micros)
}

/// Display's view: one small aggregate plus the changing reservation set.
/// Cache loss is recoverable and pays for one strict reconstruction; unchanged
/// five-second polls never walk a month shard or open a receipt.
fn read_cached_spend_status_locked(root: &Path) -> CommandResult<SpendStatus> {
    read_cached_spend_status_locked_with(root, || {
        Project::spend_ledger(root).map_err(WobuError::from)
    })
}

pub(super) fn read_cached_spend_status_locked_with(
    root: &Path,
    reconstruct: impl FnOnce() -> CommandResult<(Option<u64>, Vec<Generation>)>,
) -> CommandResult<SpendStatus> {
    let (ceiling_usd_micros, aggregate) = match read_spend_aggregate(root) {
        Some(aggregate) => (Project::spend_ceiling(root)?, aggregate),
        None => reconstruct_spend_aggregate_with(root, reconstruct)?,
    };
    status_with_reservations(root, ceiling_usd_micros, aggregate.spent_usd_micros)
}

fn reconstruct_spend_aggregate_with(
    root: &Path,
    read_ledger: impl FnOnce() -> CommandResult<(Option<u64>, Vec<Generation>)>,
) -> CommandResult<(Option<u64>, SpendAggregate)> {
    let (ceiling_usd_micros, receipts) = read_ledger()?;
    let spent_usd_micros = receipts.iter().try_fold(0_u64, |total, generation| {
        total
            .checked_add(receipt_cost(generation))
            .ok_or_else(|| WobuError::new(Code::Invalid, "The project spend total is too large."))
    })?;
    let aggregate = SpendAggregate {
        version: SPEND_AGGREGATE_VERSION,
        spent_usd_micros,
        receipts: receipts.len(),
    };
    // Disposable optimisation only. A read-only or temporarily unavailable
    // cache must never turn a successfully reconstructed canonical ledger into
    // a failed admission; the next call can reconstruct it again.
    let _ = write_spend_aggregate(root, &aggregate);
    Ok((ceiling_usd_micros, aggregate))
}

fn read_spend_aggregate(root: &Path) -> Option<SpendAggregate> {
    let path = root.join(SPEND_DIR).join(SPEND_AGGREGATE);
    let aggregate: SpendAggregate = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    (aggregate.version == SPEND_AGGREGATE_VERSION).then_some(aggregate)
}

fn write_spend_aggregate(root: &Path, aggregate: &SpendAggregate) -> CommandResult<()> {
    let path = root.join(SPEND_DIR).join(SPEND_AGGREGATE);
    let mut file =
        OpenOptions::new().write(true).create(true).truncate(true).open(&path).map_err(
            |error| spend_io("The spend display cache could not be written.", &path, error),
        )?;
    serde_json::to_writer(&mut file, aggregate).map_err(|error| {
        WobuError::new(Code::Internal, "The spend display cache could not be encoded.")
            .with_detail(error.to_string())
    })?;
    file.flush()
        .map_err(|error| spend_io("The spend display cache could not be written.", &path, error))?;
    file.sync_all()
        .map_err(|error| spend_io("The spend display cache could not be secured.", &path, error))
}

fn status_with_reservations(
    root: &Path,
    ceiling_usd_micros: Option<u64>,
    spent_usd_micros: u64,
) -> CommandResult<SpendStatus> {
    let reservations = root.join(SPEND_DIR).join("reservations");
    let mut reserved_usd_micros = 0_u64;
    let mut pending_reservations = 0_usize;
    let mut oldest_reservation_at: Option<String> = None;
    for entry in std::fs::read_dir(&reservations).map_err(|error| {
        spend_io("The spend reservations could not be read.", &reservations, error)
    })? {
        let entry = entry.map_err(|error| {
            spend_io("A spend reservation could not be read.", &reservations, error)
        })?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let bytes = std::fs::read(&path)
            .map_err(|error| spend_io("A spend reservation could not be read.", &path, error))?;
        let reservation: ReservationFile = serde_json::from_slice(&bytes).map_err(|error| {
            WobuError::new(
                Code::Malformed,
                "A spend reservation is malformed; paid generation is stopped safely.",
            )
            .with_detail(format!("{}: {error}", path.display()))
        })?;
        reserved_usd_micros =
            reserved_usd_micros.checked_add(reservation.remaining_usd_micros).ok_or_else(|| {
                WobuError::new(Code::Invalid, "The reserved spend total is too large.")
            })?;
        pending_reservations += 1;
        if oldest_reservation_at
            .as_ref()
            .is_none_or(|oldest| reservation.created_at.as_str() < oldest.as_str())
        {
            oldest_reservation_at = Some(reservation.created_at);
        }
    }
    let used = spent_usd_micros.saturating_add(reserved_usd_micros);
    Ok(SpendStatus {
        ceiling_usd_micros,
        spent_usd_micros,
        reserved_usd_micros,
        remaining_usd_micros: ceiling_usd_micros.map(|ceiling| ceiling.saturating_sub(used)),
        pending_reservations,
        oldest_reservation_at,
        ledger_locked: false,
    })
}

fn write_reservation_new(path: &Path, reservation: &ReservationFile) -> CommandResult<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| spend_io("A spend reservation could not be created.", path, error))?;
    serde_json::to_writer_pretty(&mut file, reservation).map_err(|error| {
        WobuError::new(Code::Internal, "A spend reservation could not be encoded.")
            .with_detail(error.to_string())
    })?;
    file.flush()
        .map_err(|error| spend_io("A spend reservation could not be written.", path, error))?;
    file.sync_all()
        .map_err(|error| spend_io("A spend reservation could not be secured.", path, error))
}

fn spend_io(message: &str, path: &Path, error: std::io::Error) -> WobuError {
    WobuError::new(Code::Io, message).with_detail(format!("{}: {error}", path.display()))
}
