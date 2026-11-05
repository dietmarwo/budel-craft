mod construct;
#[cfg(feature = "fcmaes")]
pub mod fcmaes;
mod improve;
mod pockets;
mod separate;

use crate::Result;
use crate::geometry::{
    Aabb, Point, Pose,
    index::{KernelStats, PoseCache, ShapeIndex, prepare},
};
use crate::layout::{Layout, Placed, Snapshot};
use crate::model::{Case, Poses};
use crate::runtime::Budget;
use rand::{Rng, SeedableRng, rngs::StdRng};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Algorithm {
    Baseline,
    Restart,
    Local,
    /// The 0.1.0 default: up to 512 from-scratch attempts, each with a short LNS burst.
    Multistart,
    /// Persistent search on the incumbent after a few construction attempts.
    #[default]
    Lns,
    Repair,
    FcmaesBlock,
}

impl Algorithm {
    /// Whether the algorithm keeps improving one working layout after its restarts.
    pub fn persistent(self) -> bool {
        matches!(self, Self::Lns | Self::Repair | Self::FcmaesBlock)
    }
}

impl std::str::FromStr for Algorithm {
    type Err = &'static str;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "baseline" => Ok(Self::Baseline),
            "restart" => Ok(Self::Restart),
            "local" => Ok(Self::Local),
            "multistart" => Ok(Self::Multistart),
            "lns" => Ok(Self::Lns),
            "repair" => Ok(Self::Repair),
            "fcmaes-block" if cfg!(feature = "fcmaes") => Ok(Self::FcmaesBlock),
            "fcmaes-block" => Err("fcmaes-block requires a build with --features fcmaes"),
            _ => Err(
                "algorithm must be baseline, restart, local, multistart, lns, repair or fcmaes-block",
            ),
        }
    }
}

/// Numeric search policy. Fields missing from an older policy file take their defaults.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Policy {
    pub stock_area: f64,
    pub box_area: f64,
    pub extent: f64,
    pub rect_waste: f64,
    pub rect_height: f64,
    pub contact_rate: f64,
    pub gap_trials: usize,
    pub rebuild_size: usize,
    /// Cost per blocker relative to blocked area when choosing an LNS insertion (optional stock).
    pub blocker_weight: f64,
    /// From-scratch construction attempts before the persistent phase.
    pub restarts: usize,
    /// Slide-to-contact sweeps per persistent round (0 disables sliding).
    pub slide_sweeps: usize,
    /// Late-acceptance history length in LNS moves (0 accepts improvements only).
    pub late_length: usize,
    /// Item moves allowed in one overlap-tolerant insertion attempt (0 disables it).
    pub separate_moves: usize,
    /// Feasible proposals compared by `find` before choosing (1 is first-fit).
    pub best_of: usize,
    /// Free-space pocket candidates tried per missing item and round (0 disables pockets).
    pub pocket_trials: usize,
    /// Persistent rounds without a new best before pocket search and separation run
    /// (0 runs them every round).
    pub rescue_after: usize,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            stock_area: 0.32,
            box_area: 0.58,
            extent: 0.10,
            rect_waste: 0.30,
            rect_height: 0.12,
            contact_rate: 0.28,
            gap_trials: 120,
            rebuild_size: 8,
            blocker_weight: 0.36,
            restarts: 5,
            slide_sweeps: 3,
            late_length: 0,
            separate_moves: 600,
            best_of: 1,
            pocket_trials: 16,
            rescue_after: 2,
        }
    }
}

impl Policy {
    pub fn validate(self) -> Result<()> {
        if [
            self.stock_area,
            self.box_area,
            self.extent,
            self.rect_waste,
            self.rect_height,
            self.contact_rate,
            self.blocker_weight,
        ]
        .iter()
        .any(|x| !x.is_finite() || !(0.0..=1.0).contains(x))
            || !(16..=1024).contains(&self.gap_trials)
            || !(2..=16).contains(&self.rebuild_size)
            || !(1..=64).contains(&self.restarts)
            || self.slide_sweeps > 8
            || self.late_length > 4096
            || self.separate_moves > 4000
            || !(1..=8).contains(&self.best_of)
            || self.pocket_trials > 128
            || self.rescue_after > 64
        {
            return Err(
                "policy out of range: weights in [0,1], gap_trials 16..=1024, \
                 rebuild_size 2..=16, restarts 1..=64, slide_sweeps <= 8, late_length <= 4096, \
                 separate_moves <= 4000, best_of 1..=8, pocket_trials <= 128, rescue_after <= 64"
                    .into(),
            );
        }
        Ok(())
    }
}

/// Switches for the persistent-search components, used for ablations.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct Features {
    pub slide: bool,
    pub pockets: bool,
    pub late_acceptance: bool,
    pub separate: bool,
    pub share: bool,
}

impl Default for Features {
    fn default() -> Self {
        Self {
            slide: true,
            pockets: true,
            late_acceptance: true,
            separate: true,
            share: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Config {
    pub algorithm: Algorithm,
    pub seed: u64,
    pub workers: usize,
    pub cache_bytes: usize,
    pub edge_tree: bool,
    pub pair_hints: bool,
    pub policy: Policy,
    #[serde(default)]
    pub features: Features,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            algorithm: Algorithm::default(),
            seed: 1,
            workers: 1,
            cache_bytes: 64 * 1024 * 1024,
            edge_tree: true,
            pair_hints: true,
            policy: Policy::default(),
            features: Features::default(),
        }
    }
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        if self.workers == 0 || self.workers > 16 || self.cache_bytes > 256 * 1024 * 1024 {
            return Err("workers must be 1..=16 and pose cache at most 256 MiB".into());
        }
        if self.algorithm == Algorithm::FcmaesBlock && !cfg!(feature = "fcmaes") {
            return Err("fcmaes-block requires a build with --features fcmaes".into());
        }
        self.policy.validate()
    }
}

#[derive(Clone, Copy)]
struct Hint {
    pose: Pose,
    bb: Aabb,
}

#[derive(Clone)]
struct Hints {
    rects: Vec<Hint>,
    samples: Vec<Hint>,
    compact: Hint,
}

pub struct Prepared {
    indices: Vec<Arc<ShapeIndex>>,
    hints: Vec<Hints>,
}

impl Prepared {
    pub fn new(case: &Case, budget: &Budget) -> Result<Self> {
        let indices = prepare(case, || budget.check_preprocessing())?;
        let mut hints = Vec::with_capacity(indices.len());
        for index in &indices {
            budget.check_preprocessing()?;
            hints.push(make_hints(index, case));
        }
        Ok(Self { indices, hints })
    }
}

fn make_hints(index: &ShapeIndex, case: &Case) -> Hints {
    let sample_count = if case.rules.poses == Poses::Eight {
        8
    } else {
        24
    };
    let mut samples = Vec::new();
    for mirror in [false, true] {
        if mirror && !case.rules.mirror_allowed {
            continue;
        }
        for i in 0..sample_count {
            let rot = i as f64 * 360.0 / sample_count as f64;
            samples.push(Hint {
                pose: Pose {
                    rot,
                    mirror,
                    ..Pose::default()
                },
                bb: index.bounds(rot, mirror),
            });
        }
    }
    let mut rects = Vec::new();
    if case.rules.poses == Poses::Eight {
        rects.extend(samples.iter().copied().take(2));
    } else {
        let mut angles: Vec<_> = (0..32).map(|i| i as f64 * 90.0 / 32.0).collect();
        for i in 0..index.hull.len() {
            let p = index.hull[(i + 1) % index.hull.len()] - index.hull[i];
            angles.push((-p.1.atan2(p.0).to_degrees()).rem_euclid(90.0));
        }
        let mut pool: Vec<_> = angles
            .into_iter()
            .map(|rot| Hint {
                pose: Pose {
                    rot,
                    ..Pose::default()
                },
                bb: index.bounds(rot, false),
            })
            .collect();
        pool.sort_by(|a, b| {
            a.bb.area()
                .total_cmp(&b.bb.area())
                .then(a.pose.rot.total_cmp(&b.pose.rot))
        });
        // Keep several different aspect ratios, plus their quarter turns.
        let mut chosen: Vec<Hint> = Vec::new();
        for hint in pool.iter().copied() {
            let Point(w, h) = hint.bb.size();
            if chosen.iter().all(|other| {
                let Point(ow, oh) = other.bb.size();
                (w / ow - 1.0).abs().max((h / oh - 1.0).abs()) > 0.025
            }) {
                chosen.push(hint);
            }
            if chosen.len() == 7 {
                break;
            }
        }
        for axis in 0..2 {
            let extent = |h: &Hint| {
                let Point(w, v) = h.bb.size();
                if axis == 0 { w } else { v }
            };
            if let Some(hint) = pool.iter().min_by(|a, b| extent(a).total_cmp(&extent(b))) {
                chosen.push(*hint);
            }
        }
        for hint in chosen {
            rects.push(hint);
            let rot = hint.pose.rot + 90.0;
            rects.push(Hint {
                pose: Pose {
                    rot,
                    ..Pose::default()
                },
                bb: index.bounds(rot, false),
            });
        }
    }
    let compact = *rects
        .iter()
        .min_by(|a, b| a.bb.area().total_cmp(&b.bb.area()))
        .unwrap();
    samples.extend(rects.iter().copied());
    Hints {
        rects,
        samples,
        compact,
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Stats {
    pub kernel: KernelStats,
    pub attempts: u64,
    pub rectangle_insertions: u64,
    pub gap_insertions: u64,
    pub move_trials: u64,
    pub moves: u64,
    pub rebuild_trials: u64,
    pub rebuilds: u64,
    pub repair_trials: u64,
    pub repairs: u64,
    pub fcmaes_evaluations: u64,
    pub slides: u64,
    pub pocket_trials: u64,
    pub pocket_insertions: u64,
    pub late_accepts: u64,
    pub separation_trials: u64,
    pub separations: u64,
    pub separation_moves: u64,
    pub migrations: u64,
    pub persistent_rounds: u64,
    pub incumbent_resets: u64,
}

impl Stats {
    pub fn add(&mut self, other: Self) {
        self.kernel.add(other.kernel);
        self.attempts += other.attempts;
        self.rectangle_insertions += other.rectangle_insertions;
        self.gap_insertions += other.gap_insertions;
        self.move_trials += other.move_trials;
        self.moves += other.moves;
        self.rebuild_trials += other.rebuild_trials;
        self.rebuilds += other.rebuilds;
        self.repair_trials += other.repair_trials;
        self.repairs += other.repairs;
        self.fcmaes_evaluations += other.fcmaes_evaluations;
        self.slides += other.slides;
        self.pocket_trials += other.pocket_trials;
        self.pocket_insertions += other.pocket_insertions;
        self.late_accepts += other.late_accepts;
        self.separation_trials += other.separation_trials;
        self.separations += other.separations;
        self.separation_moves += other.separation_moves;
        self.migrations += other.migrations;
        self.persistent_rounds += other.persistent_rounds;
        self.incumbent_resets += other.incumbent_resets;
    }
}

pub struct Candidate {
    pub snapshot: Snapshot,
    pub stage: &'static str,
    pub discovered_s: f64,
}

/// Best certified-shape snapshot shared by the workers of one run (island exchange).
/// Snapshots entering it come from feasible working layouts; the recorder still
/// validates every published candidate independently.
pub struct Shared {
    best: Mutex<Snapshot>,
}

impl Shared {
    pub fn new(initial: Snapshot) -> Self {
        Self {
            best: Mutex::new(initial),
        }
    }

    fn offer(&self, snapshot: &Snapshot, case: &Case) {
        let mut best = self.best.lock().expect("shared incumbent lock");
        if snapshot.better_than(&best, case) {
            *best = snapshot.clone();
        }
    }

    fn better(&self, than: &Snapshot, case: &Case) -> Option<Snapshot> {
        let best = self.best.lock().expect("shared incumbent lock");
        best.better_than(than, case).then(|| best.clone())
    }
}

/// Late-acceptance hill climbing on the working layout's area (maximization).
struct Late {
    history: Vec<f64>,
    position: usize,
}

impl Late {
    fn new(length: usize, area: f64) -> Self {
        Self {
            history: vec![area; length],
            position: 0,
        }
    }

    /// Decides one move and records the resulting current value.
    fn accept(&mut self, current: f64, candidate: f64) -> bool {
        if self.history.is_empty() {
            return candidate > current + 1e-8;
        }
        let v = self.position % self.history.len();
        let accept = candidate > current + 1e-8 || candidate >= self.history[v] - 1e-9;
        self.history[v] = if accept { candidate } else { current };
        self.position = self.position.wrapping_add(1);
        accept
    }
}

struct Engine<'a> {
    case: &'a Case,
    prepared: &'a Prepared,
    budget: &'a Budget,
    cfg: &'a Config,
    layout: Layout,
    cache: PoseCache,
    rng: StdRng,
    stats: Stats,
    shared: Option<&'a Shared>,
    late: Option<Late>,
    separation_failures: Vec<u32>,
    near: Vec<usize>,
    hits: Vec<usize>,
}

impl<'a> Engine<'a> {
    fn new(
        case: &'a Case,
        prepared: &'a Prepared,
        budget: &'a Budget,
        cfg: &'a Config,
        seed: u64,
    ) -> Self {
        Self {
            case,
            prepared,
            budget,
            cfg,
            layout: Layout::new(case),
            cache: PoseCache::new(cfg.cache_bytes / cfg.workers),
            rng: StdRng::seed_from_u64(seed),
            stats: Stats::default(),
            shared: None,
            late: None,
            separation_failures: vec![0; case.items.len()],
            near: Vec::new(),
            hits: Vec::new(),
        }
    }

    fn index(&self, i: usize) -> &Arc<ShapeIndex> {
        &self.prepared.indices[self.case.items[i].geometry.key]
    }

    fn hint(&self, i: usize) -> &Hints {
        &self.prepared.hints[self.case.items[i].geometry.key]
    }

    fn oriented(&mut self, i: usize, pose: Pose) -> Arc<crate::geometry::index::Oriented> {
        self.cache.get(
            &self.prepared.indices[self.case.items[i].geometry.key],
            pose.rot,
            pose.mirror,
            &mut self.stats.kernel,
        )
    }

    fn at(&mut self, i: usize, pose: Pose, p: Point) -> Placed {
        Placed::at_lower_left(self.oriented(i, pose), pose, p)
    }

    fn fits(&mut self, p: &Placed) -> bool {
        self.budget.claim()
            && self
                .layout
                .fits(self.case, p, self.cfg.edge_tree, &mut self.stats.kernel)
    }

    fn rebuild_from(&mut self, start: &Snapshot) {
        self.layout = Layout::new(self.case);
        for &(i, pose) in &start.poses {
            let shape = self.oriented(i, pose);
            self.layout.insert(self.case, i, Placed::new(pose, shape));
        }
    }

    fn emit(&self, best: &mut Snapshot, stage: &'static str, publish: &mut impl FnMut(Candidate)) {
        let next = self.layout.snapshot(self.case);
        if next.better_than(best, self.case) {
            *best = next.clone();
            if let Some(shared) = self.shared {
                shared.offer(&next, self.case);
            }
            publish(Candidate {
                snapshot: next,
                stage,
                discovered_s: self.budget.elapsed(),
            });
        }
    }

    /// Persistent phase: keep improving one working layout. Its area may drop under
    /// late acceptance; the lane best is kept separately and restored after stagnation.
    fn persistent(&mut self, best: &mut Snapshot, publish: &mut impl FnMut(Candidate)) {
        let n = self.case.items.len();
        let features = self.cfg.features;
        let policy = self.cfg.policy;
        self.rebuild_from(best);
        let late =
            features.late_acceptance && !self.case.rules.must_seat_all && policy.late_length > 0;
        self.late = late.then(|| Late::new(policy.late_length, self.layout.area));
        let mut round = 0usize;
        let mut stale = 0usize;
        while self.budget.can_search() && self.layout.count < n {
            self.stats.persistent_rounds += 1;
            let before = best.area;
            if features.slide && policy.slide_sweeps > 0 {
                self.slide_compact(policy.slide_sweeps);
            }
            let rescue = stale >= policy.rescue_after;
            if features.pockets && policy.pocket_trials > 0 && rescue {
                self.pocket_fill(best, publish);
            }
            self.fill_gaps(policy.gap_trials, best, publish);
            self.improve(best, publish, round);
            if features.separate && policy.separate_moves > 0 && rescue && self.layout.count < n {
                self.separate_insert(best, publish);
            }
            self.emit(best, "persistent", publish);
            if features.share
                && let Some(other) = self.shared.and_then(|s| s.better(best, self.case))
            {
                *best = other;
                self.rebuild_from(best);
                self.stats.migrations += 1;
                if let Some(late) = self.late.as_mut() {
                    *late = Late::new(policy.late_length, self.layout.area);
                }
            }
            stale = if best.area > before + 1e-9 {
                0
            } else {
                stale + 1
            };
            if stale >= 6 && best.better_than(&self.layout.snapshot(self.case), self.case) {
                self.rebuild_from(best);
                self.stats.incumbent_resets += 1;
                if let Some(late) = self.late.as_mut() {
                    *late = Late::new(policy.late_length, self.layout.area);
                }
                stale = 0;
            }
            round += 1;
        }
    }
}

pub struct Lane<'a> {
    pub case: &'a Case,
    pub prepared: &'a Prepared,
    pub budget: &'a Budget,
    pub config: &'a Config,
    pub initial: &'a Snapshot,
    pub seed: u64,
    pub shared: Option<&'a Shared>,
}

pub fn solve_lane(lane: Lane<'_>, mut publish: impl FnMut(Candidate)) -> Stats {
    let Lane {
        case,
        prepared,
        budget,
        config: cfg,
        initial,
        seed,
        shared,
    } = lane;
    let mut engine = Engine::new(case, prepared, budget, cfg, seed);
    engine.shared = shared;
    let mut best = initial.clone();
    let persistent = cfg.algorithm.persistent();
    let slide = persistent && cfg.features.slide && cfg.policy.slide_sweeps > 0;
    let mut attempt = 0;
    // A resumed run first improves its certified arrangement, then runs the usual
    // restarts (which cannot lose it) and continues the persistent phase from the best.
    if !initial.poses.is_empty() && budget.can_search() {
        engine.rebuild_from(initial);
        engine.fill_gaps(cfg.policy.gap_trials, &mut best, &mut publish);
        engine.improve(&mut best, &mut publish, 0);
    }
    let restarts = if persistent { cfg.policy.restarts } else { 512 };
    while budget.can_search() && best.poses.len() < case.items.len() {
        engine.stats.attempts += 1;
        engine.layout = Layout::new(case);
        engine.construct(attempt, &mut best, &mut publish);
        if slide {
            engine.slide_compact(2);
        }
        engine.fill_gaps(cfg.policy.gap_trials, &mut best, &mut publish);
        engine.improve(&mut best, &mut publish, attempt);
        engine.emit(&mut best, "attempt-end", &mut publish);
        attempt += 1;
        if cfg.algorithm == Algorithm::Baseline || attempt >= restarts {
            break;
        }
    }
    if persistent && budget.can_search() && best.poses.len() < case.items.len() {
        engine.persistent(&mut best, &mut publish);
    }
    engine.stats
}

fn clip_point(p: Point, bb: Aabb) -> Point {
    Point(p.0.clamp(bb.min.0, bb.max.0), p.1.clamp(bb.min.1, bb.max.1))
}

fn random_point(rng: &mut StdRng, bb: Aabb) -> Point {
    Point(
        rng.gen_range(bb.min.0..=bb.max.0),
        rng.gen_range(bb.min.1..=bb.max.1),
    )
}

#[cfg(test)]
mod tests {
    use super::{Config, Engine, Features, Late, Policy, Prepared};
    use crate::model::Case;
    use crate::runtime::Budget;
    use crate::validate::validate;
    use std::path::Path;
    use std::time::Instant;

    #[test]
    fn late_acceptance_uses_the_history_not_only_the_current_value() {
        let mut late = Late::new(3, 100.0);
        assert!(!late.accept(100.0, 99.0), "worse than current and history");
        assert!(late.accept(100.0, 101.0), "improvement");
        // Worse than the current 101 but not worse than the value recorded 3 moves ago.
        assert!(late.accept(101.0, 100.5));
        let mut strict = Late::new(0, 100.0);
        assert!(!strict.accept(100.0, 100.0));
        assert!(strict.accept(100.0, 100.1));
    }

    #[test]
    fn older_policy_and_config_files_take_new_defaults() {
        let old = r#"{"stock_area":0.32,"box_area":0.58,"extent":0.1,"rect_waste":0.3,
            "rect_height":0.12,"contact_rate":0.28,"gap_trials":120,"rebuild_size":8}"#;
        let policy: Policy = serde_json::from_str(old).expect("0.1.0 policy");
        assert_eq!(policy, Policy::default());
        policy.validate().expect("valid");
        let config = format!(
            r#"{{"algorithm":"lns","seed":3,"workers":1,"cache_bytes":1024,
            "edge_tree":true,"pair_hints":true,"policy":{old}}}"#
        );
        let config: Config = serde_json::from_str(&config).expect("0.1.0 config");
        assert_eq!(config.features, Features::default());
    }

    #[test]
    fn slide_compaction_keeps_the_layout_valid_and_its_area() {
        let case = Case::load(Path::new("cases/medium.json")).expect("case");
        let budget = Budget::new(Instant::now(), 60.0, 0.0, Some(300_000)).expect("budget");
        let prepared = Prepared::new(&case, &budget).expect("prepared");
        let cfg = Config::default();
        let mut engine = Engine::new(&case, &prepared, &budget, &cfg, 3);
        let mut best = crate::layout::Snapshot::default();
        let mut sink = |_c: super::Candidate| {};
        engine.construct(1, &mut best, &mut sink);
        let (count, area) = (engine.layout.count, engine.layout.area);
        let moved = engine.slide_compact(2);
        assert!(
            moved > 0,
            "a bounding-box construction leaves room to slide"
        );
        assert_eq!(engine.layout.count, count);
        assert!((engine.layout.area - area).abs() < 1e-6);
        let snapshot = engine.layout.snapshot(&case);
        let report = validate(&case, &snapshot.answer(&case));
        assert!(report.strict_valid);
    }
}
