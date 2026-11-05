//! Bounded in-process use of the pinned fcmaes-core ask/tell API.
use super::{Algorithm, Config, Lane, Policy, Prepared, solve_lane};
use crate::Result;
use crate::layout::Snapshot;
use crate::model::Case;
use crate::runtime::Budget;
use crate::validate::validate;
use fcmaes_core::de::{De, DeParams};
use fcmaes_core::fitness::Fitness;
use rand::{Rng, SeedableRng, rngs::StdRng};
use serde::Serialize;
use std::time::Instant;

pub(super) fn de_search(
    dim: usize,
    seed: u64,
    requested: u64,
    target: Option<f64>,
    mut objective: impl FnMut(&[f64]) -> Option<f64>,
) -> u64 {
    let population = requested.clamp(4, 12) as i32;
    let params = DeParams {
        popsize: population,
        max_evaluations: requested,
        seed,
        ..DeParams::default()
    };
    let fit = Fitness::new(dim, 1, vec![-1.; dim], vec![1.; dim]);
    let mut optimizer = De::new(fit, &[], &[], None, &params);
    let mut actual = 0;
    while actual < requested {
        let candidates = optimizer.ask();
        let mut scores = Vec::with_capacity(candidates.len());
        for x in &candidates {
            if actual == requested {
                return actual;
            }
            let Some(cost) = objective(x) else {
                return actual;
            };
            actual += 1;
            if target.is_some_and(|threshold| cost <= threshold) {
                return actual;
            }
            scores.push(cost);
        }
        if optimizer.tell(&scores) != 0 {
            break;
        }
    }
    actual
}

/// Number of tuned search-control dimensions. The six construction-order weights keep
/// their defaults so a small evaluation budget is spent on the persistent search.
pub const DIM: usize = 10;

/// Maps a point in [-1, 1]^DIM to a policy.
pub fn decode(v: &[f64]) -> Policy {
    let scale = |i: usize| (v[i].clamp(-1., 1.) + 1.) * 0.5;
    let int = |i: usize, lo: usize, hi: usize| lo + (scale(i) * (hi - lo) as f64).round() as usize;
    Policy {
        gap_trials: int(0, 24, 240),
        rebuild_size: int(1, 2, 16),
        blocker_weight: scale(2),
        restarts: int(3, 1, 12),
        slide_sweeps: int(4, 0, 6),
        late_length: [0, 16, 32, 64, 128, 256, 512, 1024][int(5, 0, 7)],
        separate_moves: int(6, 0, 800),
        best_of: int(7, 1, 4),
        pocket_trials: int(8, 0, 48),
        rescue_after: int(9, 0, 6),
        ..Policy::default()
    }
}

/// Inverse of [`decode`] for the default policy (approximate for discrete fields).
fn encode_default() -> Vec<f64> {
    let p = Policy::default();
    let unit = |x: f64| x * 2.0 - 1.0;
    let int = |x: usize, lo: usize, hi: usize| unit((x - lo) as f64 / (hi - lo) as f64);
    let late = [0, 16, 32, 64, 128, 256, 512, 1024]
        .iter()
        .position(|&l| l == p.late_length)
        .unwrap_or(4);
    vec![
        int(p.gap_trials, 24, 240),
        int(p.rebuild_size, 2, 16),
        unit(p.blocker_weight),
        int(p.restarts, 1, 12),
        int(p.slide_sweeps, 0, 6),
        int(late, 0, 7),
        int(p.separate_moves, 0, 800),
        int(p.best_of, 1, 4),
        int(p.pocket_trials, 0, 48),
        int(p.rescue_after, 0, 6),
    ]
}

pub struct Tune {
    pub method: String,
    pub seed: u64,
    pub evaluations: u64,
    pub per_trial: u64,
    pub seconds: f64,
    /// Training seeds per policy evaluation.
    pub seeds: usize,
    /// Held-out seeds for the selected and the default policy.
    pub holdout_seeds: usize,
}

#[derive(Serialize, Clone)]
pub struct Trial {
    pub seed: u64,
    pub seated: usize,
    pub area: f64,
    pub strict_valid: bool,
    pub proposals: u64,
    pub loss: f64,
}

#[derive(Serialize, Clone)]
pub struct Evaluation {
    pub policy: Policy,
    pub mean_loss: f64,
    pub trials: Vec<Trial>,
}

#[derive(Serialize)]
pub struct Tuning {
    pub method: String,
    pub case: String,
    pub case_sha256: String,
    pub requested_evaluations: u64,
    pub actual_evaluations: u64,
    pub requested_proposals_per_trial: u64,
    pub training_seeds: Vec<u64>,
    pub holdout_seeds: Vec<u64>,
    pub training_proposals: u64,
    pub history: Vec<Evaluation>,
    pub training: Option<Evaluation>,
    pub holdout: Option<Evaluation>,
    pub default_holdout: Option<Evaluation>,
    pub selected_policy: Policy,
    pub runtime_s: f64,
}

fn trial(
    case: &Case,
    prepared: &Prepared,
    policy: Policy,
    seed: u64,
    per_trial: u64,
    remaining: f64,
) -> Option<Trial> {
    if remaining < 0.01 {
        return None;
    }
    let budget = Budget::new(Instant::now(), remaining, 0.001, Some(per_trial)).ok()?;
    let config = Config {
        algorithm: Algorithm::Lns,
        seed,
        policy,
        ..Config::default()
    };
    let initial = Snapshot::default();
    let mut best = initial.clone();
    solve_lane(
        Lane {
            case,
            prepared,
            budget: &budget,
            config: &config,
            initial: &initial,
            seed,
            shared: None,
        },
        |c| {
            if c.snapshot.better_than(&best, case) {
                best = c.snapshot;
            }
        },
    );
    if budget.work() < per_trial && budget.deadline_reached() {
        // Cut short by the outer tuning deadline: not comparable with complete trials.
        return None;
    }
    let result = validate(case, &best.answer(case));
    let geometry_ok = result.violations.iter().all(|v| v.rule == "must-seat-all");
    let loss = if !geometry_ok {
        2.0
    } else if case.rules.must_seat_all {
        1.0 - result.seated as f64 / case.items.len().max(1) as f64
            - 0.001 * result.available_fraction
    } else {
        1.0 - result.available_fraction
    };
    Some(Trial {
        seed,
        seated: result.seated,
        area: result.area,
        strict_valid: result.strict_valid,
        proposals: budget.work(),
        loss,
    })
}

fn evaluate_seeds(
    case: &Case,
    prepared: &Prepared,
    policy: Policy,
    seeds: &[u64],
    per_trial: u64,
    deadline: &Budget,
    end_s: f64,
) -> Option<Evaluation> {
    let mut trials = Vec::with_capacity(seeds.len());
    for &seed in seeds {
        let remaining = end_s - deadline.elapsed();
        trials.push(trial(case, prepared, policy, seed, per_trial, remaining)?);
    }
    let mean_loss = trials.iter().map(|t| t.loss).sum::<f64>() / trials.len().max(1) as f64;
    Some(Evaluation {
        policy,
        mean_loss,
        trials,
    })
}

pub fn tune(case: &Case, setup: Tune, started: Instant) -> Result<Tuning> {
    if !["fixed", "random", "de", "bite"].contains(&setup.method.as_str())
        || setup.evaluations == 0
        || setup.evaluations > 256
        || !(1..=2_000_000).contains(&setup.per_trial)
        || !(1..=32).contains(&setup.seeds)
        || !(1..=32).contains(&setup.holdout_seeds)
    {
        return Err("tune expects fixed/random/de/bite, 1..=256 evaluations, \
             1..=2000000 proposals per trial and 1..=32 training/held-out seeds"
            .into());
    }
    let budget = Budget::new(started, setup.seconds, 0.02, None)?;
    let prepared = Prepared::new(case, &budget)?;
    let training_seeds: Vec<u64> = (0..setup.seeds as u64)
        .map(|i| setup.seed.wrapping_add(i))
        .collect();
    let holdout_seeds: Vec<u64> = (0..setup.holdout_seeds as u64)
        .map(|i| setup.seed.wrapping_add(1_000_003 + i))
        .collect();
    let mut best = Policy::default();
    let mut training = None::<Evaluation>;
    let mut history = Vec::new();
    let mut proposals = 0_u64;
    let mut actual = 0_u64;
    let train_end = setup.seconds * 0.6;
    let mut evaluate = |policy: Policy| {
        if !budget.can_search() || policy.validate().is_err() {
            return None;
        }
        let result = evaluate_seeds(
            case,
            &prepared,
            policy,
            &training_seeds,
            setup.per_trial,
            &budget,
            train_end,
        )?;
        let loss = result.mean_loss;
        actual += 1;
        proposals = proposals.saturating_add(result.trials.iter().map(|t| t.proposals).sum());
        if training.as_ref().is_none_or(|old| loss < old.mean_loss) {
            training = Some(result.clone());
            best = policy;
        }
        history.push(result);
        Some(loss)
    };
    match setup.method.as_str() {
        "fixed" => {
            evaluate(Policy::default());
        }
        "random" => {
            let mut rng = StdRng::seed_from_u64(setup.seed ^ 0x52bf1f37);
            // The default policy is the first candidate of every search method.
            if evaluate(Policy::default()).is_some() {
                for _ in 1..setup.evaluations {
                    let x: [f64; DIM] = std::array::from_fn(|_| rng.gen_range(-1.0..=1.0));
                    if evaluate(decode(&x)).is_none() {
                        break;
                    }
                }
            }
        }
        "bite" => {
            use fcmaes_core::biteopt::{BiteOpt, BiteParams};
            if evaluate(Policy::default()).is_some() {
                let params = BiteParams {
                    popsize: 8,
                    max_evaluations: setup.evaluations.saturating_sub(1),
                    seed: setup.seed,
                    ..BiteParams::default()
                };
                let guess = encode_default();
                let mut optimizer = BiteOpt::new(&[-1.0; DIM], &[1.0; DIM], Some(&guess), &params);
                let mut done = 1;
                'outer: while done < setup.evaluations {
                    let batch = optimizer.ask(1);
                    if batch.is_empty() {
                        break;
                    }
                    let mut costs = Vec::with_capacity(batch.len());
                    for x in &batch {
                        let Some(loss) = evaluate(decode(x)) else {
                            break 'outer;
                        };
                        costs.push(loss);
                        done += 1;
                    }
                    if optimizer.tell(&costs) != 0 {
                        break;
                    }
                }
            }
        }
        _ => {
            if evaluate(Policy::default()).is_some() {
                de_search(DIM, setup.seed, setup.evaluations - 1, None, |x| {
                    evaluate(decode(x))
                });
            }
        }
    }
    let holdout_end = setup.seconds - 0.05;
    let holdout = evaluate_seeds(
        case,
        &prepared,
        best,
        &holdout_seeds,
        setup.per_trial,
        &budget,
        holdout_end,
    );
    let default_holdout = if best == Policy::default() {
        holdout.clone()
    } else {
        evaluate_seeds(
            case,
            &prepared,
            Policy::default(),
            &holdout_seeds,
            setup.per_trial,
            &budget,
            holdout_end,
        )
    };
    Ok(Tuning {
        method: setup.method,
        case: case.name.clone(),
        case_sha256: case.sha256.clone(),
        requested_evaluations: setup.evaluations,
        actual_evaluations: actual,
        requested_proposals_per_trial: setup.per_trial,
        training_seeds,
        holdout_seeds,
        training_proposals: proposals,
        history,
        training,
        holdout,
        default_holdout,
        selected_policy: best,
        runtime_s: budget.elapsed(),
    })
}
