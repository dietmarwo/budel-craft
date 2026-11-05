use crate::Result;
use crate::export;
use crate::layout::Snapshot;
use crate::model::{Case, read_limited};
use crate::runtime::{Budget, Checkpoint};
use crate::search::{Candidate, Config, Lane, Prepared, Shared, Stats, solve_lane};
use crate::validate::{Validation, validate, validate_with};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

const CHECKPOINT_VERSION: u32 = 1;

pub struct Request {
    pub case_path: PathBuf,
    pub output: PathBuf,
    pub run_dir: PathBuf,
    pub config: Config,
    pub seconds: f64,
    pub requested_proposals: Option<u64>,
    pub named_budget: Option<String>,
    pub resume: Option<Checkpoint<Config>>,
}

#[derive(Serialize)]
pub struct Outcome {
    pub case: String,
    pub case_sha256: String,
    pub config_sha256: String,
    pub config: Config,
    pub termination: &'static str,
    pub requested_seconds: f64,
    pub runtime_s: f64,
    pub cumulative_s: f64,
    pub preprocess_s: f64,
    pub overshoot_s: f64,
    pub requested_proposals: Option<u64>,
    pub proposals: u64,
    pub cumulative_proposals: u64,
    pub first_complete_s: Option<f64>,
    pub best: Validation,
    pub certified_answer_written: bool,
    pub candidate_validations: u64,
    pub superseded_candidates: u64,
    pub stats: Stats,
}

#[derive(Deserialize)]
struct RuntimeHeader {
    rules: RuntimeOnly,
}
#[derive(Deserialize)]
struct RuntimeOnly {
    #[serde(default)]
    runtime: crate::model::RuntimeRules,
}

pub fn choose_time(
    case: &Path,
    seconds: Option<f64>,
    name: Option<&str>,
) -> Result<(f64, Option<String>)> {
    if let Some(seconds) = seconds {
        if name.is_some() {
            return Err("use either --time-limit or --budget".into());
        }
        return Ok((seconds, None));
    }
    let header: RuntimeHeader = serde_json::from_slice(&read_limited(case, 64 * 1024 * 1024)?)?;
    let budgets = header.rules.runtime.budgets;
    let budget = name.unwrap_or("interactive");
    if let Some(&value) = budgets.get(budget) {
        return Ok((value, Some(budget.into())));
    }
    if name.is_none() {
        Ok((1.0, None))
    } else {
        Err(format!("case has no budget named {budget:?}").into())
    }
}

fn config_hash(config: &Config) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(config)?)))
}

pub fn load_checkpoint(path: &Path) -> Result<Checkpoint<Config>> {
    let cp: Checkpoint<Config> = serde_json::from_slice(&read_limited(path, 16 * 1024 * 1024)?)?;
    if cp.version != CHECKPOINT_VERSION
        || cp.solver_version != env!("CARGO_PKG_VERSION")
        || cp.config_sha256 != config_hash(&cp.config)?
        || !cp.total_seconds.is_finite()
        || cp.total_seconds < 0.0
    {
        return Err("checkpoint version, configuration hash or time is invalid".into());
    }
    Ok(cp)
}

fn recover(case: &Case, cp: &Checkpoint<Config>) -> Result<Snapshot> {
    if cp.case_sha256 != case.sha256 {
        return Err("checkpoint case SHA-256 differs from the input".into());
    }
    if cp
        .snapshot
        .poses
        .iter()
        .any(|&(i, _)| i >= case.items.len())
    {
        return Err("checkpoint contains an out-of-range item".into());
    }
    let mut snapshot = cp.snapshot.clone();
    let report = validate(case, &snapshot.answer(case));
    if !usable_progress(&report) {
        return Err("checkpoint contains invalid geometry or IDs".into());
    }
    snapshot.area = report.area;
    Ok(snapshot)
}

fn usable_progress(report: &Validation) -> bool {
    report.checked_all && report.violations.iter().all(|v| v.rule == "must-seat-all")
}

struct Recorder<'a> {
    case: &'a Case,
    request: &'a Request,
    budget: &'a Budget,
    config_sha256: String,
    prior_s: f64,
    prior_work: u64,
    attempts: u64,
    best: Snapshot,
    first_complete: Option<f64>,
    saved: bool,
    events: u32,
    trace: BufWriter<File>,
    /// Report for `best`, kept so the final record needs no second validation.
    report: Option<Validation>,
    validations: u64,
    superseded: u64,
}

impl Recorder<'_> {
    fn event(&mut self, stage: &str, count: usize, area: f64, discovered: f64) -> Result<()> {
        if self.events < 1024 {
            serde_json::to_writer(
                &mut self.trace,
                &json!({
                    "stage":stage,"discovered_s":discovered,"elapsed_s":self.budget.elapsed(),
                    "proposals":self.budget.work(),"seated":count,"area":area
                }),
            )?;
            writeln!(&mut self.trace)?;
            self.trace.flush()?;
            self.events += 1;
        }
        Ok(())
    }

    fn save(&mut self, report: &Validation) -> Result<()> {
        if report.strict_valid {
            let mut answer = self.best.answer(self.case);
            answer.runtime_s = Some(self.prior_s + self.budget.elapsed());
            answer.budget = self.request.named_budget.clone();
            answer.machine = Some(format!(
                "{} {} ({} search workers)",
                std::env::consts::OS,
                std::env::consts::ARCH,
                self.request.config.workers
            ));
            export::json(&self.request.output, &answer)?;
            self.saved = true;
        }
        let checkpoint = Checkpoint {
            version: CHECKPOINT_VERSION,
            solver_version: env!("CARGO_PKG_VERSION").into(),
            case_path: self.request.case_path.clone(),
            case_sha256: self.case.sha256.clone(),
            config: self.request.config.clone(),
            config_sha256: self.config_sha256.clone(),
            snapshot: self.best.clone(),
            total_seconds: self.prior_s + self.budget.elapsed(),
            total_proposals: self.prior_work.saturating_add(self.budget.work()),
            attempts: self.attempts,
        };
        export::json(&self.request.run_dir.join("checkpoint.json"), &checkpoint)
    }

    fn candidate(&mut self, candidate: Candidate) -> Result<()> {
        if !candidate.snapshot.better_than(&self.best, self.case) {
            return Ok(());
        }
        let answer = candidate.snapshot.answer(self.case);
        self.validations += 1;
        let report = validate_with(self.case, &answer, || !self.budget.deadline_reached());
        if !usable_progress(&report) {
            self.event(
                if report.checked_all {
                    "candidate-rejected"
                } else {
                    "certification-timeout"
                },
                report.seated,
                report.area,
                candidate.discovered_s,
            )?;
            return Ok(());
        }
        self.best = candidate.snapshot;
        self.best.area = report.area;
        self.event(
            candidate.stage,
            report.seated,
            report.area,
            candidate.discovered_s,
        )?;
        if report.complete && report.strict_valid {
            self.first_complete
                .get_or_insert(self.prior_s + self.budget.elapsed());
        }
        self.save(&report)?;
        if report.complete && report.strict_valid {
            self.budget.cancel();
        }
        self.report = Some(report);
        Ok(())
    }
}

/// Holds the best unrecorded candidate. Workers overwrite it without blocking, and the
/// recorder validates only the newest best, so validation never stalls the search.
struct Mailbox {
    slot: Mutex<Option<Candidate>>,
    offered: std::sync::atomic::AtomicU64,
}

impl Mailbox {
    fn offer(&self, candidate: Candidate, case: &Case) {
        self.offered
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut slot = self.slot.lock().expect("mailbox lock");
        if slot
            .as_ref()
            .is_none_or(|old| candidate.snapshot.better_than(&old.snapshot, case))
        {
            *slot = Some(candidate);
        }
    }

    fn take(&self) -> Option<Candidate> {
        self.slot.lock().expect("mailbox lock").take()
    }
}

fn portfolio(
    config: &Config,
    case: &Case,
    prepared: &Prepared,
    budget: &Budget,
    initial: &Snapshot,
    prior_attempts: u64,
    recorder: &mut Recorder<'_>,
) -> Result<Stats> {
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::time::Duration;
    let mut sum = Stats::default();
    let mut error = None;
    let mailbox = Mailbox {
        slot: Mutex::new(None),
        offered: std::sync::atomic::AtomicU64::new(0),
    };
    let shared = Shared::new(initial.clone());
    std::thread::scope(|scope| {
        let (tx, rx) = mpsc::channel::<Stats>();
        for i in 0..config.workers {
            let tx = tx.clone();
            let mailbox = &mailbox;
            let shared = &shared;
            let seed = config.seed.wrapping_add(
                prior_attempts
                    .wrapping_add(i as u64)
                    .wrapping_mul(0x9e3779b97f4a7c15),
            );
            scope.spawn(move || {
                let lane = Lane {
                    case,
                    prepared,
                    budget,
                    config,
                    initial,
                    seed,
                    shared: (config.workers > 1 && config.features.share).then_some(shared),
                };
                let stats = solve_lane(lane, |c| mailbox.offer(c, case));
                let _ = tx.send(stats);
            });
        }
        drop(tx);
        let mut last = budget.elapsed();
        let record = |recorder: &mut Recorder<'_>, error: &mut Option<_>| {
            if let Some(candidate) = mailbox.take()
                && error.is_none()
                && let Err(err) = recorder.candidate(candidate)
            {
                *error = Some(err);
                budget.cancel();
            }
        };
        loop {
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(stats) => sum.add(stats),
                Err(RecvTimeoutError::Disconnected) => {
                    record(recorder, &mut error);
                    break;
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            record(recorder, &mut error);
            if budget.elapsed() - last >= 5.0 {
                last = budget.elapsed();
                if error.is_none() {
                    let snapshot = &recorder.best;
                    let count = snapshot.poses.len();
                    let area = snapshot.area;
                    if let Err(err) = recorder.event("heartbeat", count, area, last) {
                        error = Some(err);
                        budget.cancel();
                    }
                }
            }
        }
    });
    let offered = mailbox.offered.load(std::sync::atomic::Ordering::Relaxed);
    recorder.superseded = offered.saturating_sub(recorder.validations);
    match error {
        Some(err) => Err(err),
        None => Ok(sum),
    }
}

pub fn run(mut request: Request, started: Instant) -> Result<Outcome> {
    request.config.validate()?;
    request.case_path = request.case_path.canonicalize()?;
    if request.output.canonicalize().ok().as_ref() == Some(&request.case_path) {
        return Err("output path would overwrite the case".into());
    }
    let budget = Budget::new(
        started,
        request.seconds,
        0.05 + 0.09 * request.seconds.sqrt(),
        request.requested_proposals,
    )?;
    let case = Case::load_with(&request.case_path, || budget.check_preprocessing())?;
    let prepared = Prepared::new(&case, &budget)?;
    let initial = match &request.resume {
        Some(cp) => recover(&case, cp)?,
        None => Snapshot::default(),
    };
    let preprocess_s = budget.elapsed();
    std::fs::create_dir_all(&request.run_dir)?;
    let prior_s = request.resume.as_ref().map_or(0.0, |cp| cp.total_seconds);
    let prior_work = request.resume.as_ref().map_or(0, |cp| cp.total_proposals);
    let prior_attempts = request.resume.as_ref().map_or(0, |cp| cp.attempts);
    let mut recorder = Recorder {
        case: &case,
        request: &request,
        budget: &budget,
        config_sha256: config_hash(&request.config)?,
        prior_s,
        prior_work,
        attempts: prior_attempts,
        best: initial.clone(),
        first_complete: None,
        saved: false,
        events: 0,
        trace: BufWriter::new(File::create(request.run_dir.join("progress.jsonl"))?),
        report: None,
        validations: 0,
        superseded: 0,
    };
    let initial_report =
        validate_with(&case, &initial.answer(&case), || !budget.deadline_reached());
    recorder.save(&initial_report)?;
    recorder.report = Some(initial_report);
    let stats = portfolio(
        &request.config,
        &case,
        &prepared,
        &budget,
        &initial,
        prior_attempts,
        &mut recorder,
    )?;
    recorder.attempts = prior_attempts
        .saturating_add(stats.attempts)
        .saturating_add(1);
    // The best snapshot was validated when it was accepted; reuse that report.
    let report = recorder
        .report
        .take()
        .expect("initial or accepted report is always present");
    recorder.save(&report)?;
    if !report.strict_valid {
        export::json(
            &request.run_dir.join("partial.answer.json"),
            &recorder.best.answer(&case),
        )?;
    }
    export::json(&request.run_dir.join("strict.json"), &report)?;
    let termination = if report.complete && report.strict_valid {
        "available-area-complete"
    } else {
        budget.termination()
    };
    let runtime_s = budget.elapsed();
    let outcome = Outcome {
        case: case.name.clone(),
        case_sha256: case.sha256.clone(),
        config_sha256: recorder.config_sha256.clone(),
        config: request.config.clone(),
        termination,
        requested_seconds: request.seconds,
        runtime_s,
        cumulative_s: prior_s + runtime_s,
        preprocess_s,
        overshoot_s: (runtime_s - request.seconds).max(0.0),
        requested_proposals: request.requested_proposals,
        proposals: budget.work(),
        cumulative_proposals: prior_work.saturating_add(budget.work()),
        first_complete_s: recorder.first_complete,
        best: report,
        certified_answer_written: recorder.saved,
        candidate_validations: recorder.validations,
        superseded_candidates: recorder.superseded,
        stats,
    };
    export::json(&request.run_dir.join("packing_metrics.json"), &outcome)?;
    Ok(outcome)
}
