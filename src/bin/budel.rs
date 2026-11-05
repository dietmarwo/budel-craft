use budel_craft::{
    Result, export,
    model::{Answer, Case},
    validate::validate,
};
use budel_craft::{
    execution::{self, Request},
    runtime::{install_interrupt_handler, limit_address_space},
    search::{Config, Features, Policy},
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

fn inspect(case: &Case) -> Value {
    let mut lengths: Vec<_> = case
        .items
        .iter()
        .map(|item| item.geometry.ring.len())
        .collect();
    lengths.sort_unstable();
    let unique: BTreeSet<_> = case.items.iter().map(|item| item.geometry.key).collect();
    let median = (!lengths.is_empty())
        .then(|| (lengths[(lengths.len() - 1) / 2] + lengths[lengths.len() / 2]) as f64 / 2.0);
    json!({
        "case": case.name, "sha256": case.sha256, "plane": case.plane,
        "usable": case.usable, "items": case.items.len(),
        "distinct_collision_bodies": unique.len(),
        "outer_vertices": lengths.iter().sum::<usize>(),
        "vertices_min": lengths.first(), "vertices_median": median, "vertices_max": lengths.last(),
        "nominal_gap": case.rules.min_channel, "strict_gap": case.gap,
        "judge_tolerance": case.rules.tolerance, "poses": case.rules.poses,
        "mirror_allowed": case.rules.mirror_allowed, "must_seat_all": case.rules.must_seat_all,
        "available_area": case.available_area, "coverage_ceiling": case.coverage(case.available_area),
        "budgets_s": case.rules.runtime.budgets
    })
}

#[derive(Default)]
struct Arguments {
    positional: Vec<String>,
    values: BTreeMap<String, String>,
    flags: BTreeSet<String>,
}

impl Arguments {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self> {
        let mut result = Self::default();
        let mut args = args.peekable();
        while let Some(arg) = args.next() {
            if let Some(flag) = arg.strip_prefix("--") {
                if [
                    "strict",
                    "help",
                    "no-cache",
                    "scalar",
                    "no-pairs",
                    "no-slide",
                    "no-pockets",
                    "strict-accept",
                    "no-separate",
                    "no-share",
                ]
                .contains(&flag)
                {
                    if !result.flags.insert(flag.into()) {
                        return Err(format!("duplicate flag {arg}").into());
                    }
                } else {
                    let value = args
                        .next()
                        .ok_or_else(|| format!("missing value for {arg}"))?;
                    if value.starts_with("--") {
                        return Err(format!("missing value for {arg}").into());
                    }
                    if result.values.insert(flag.into(), value).is_some() {
                        return Err(format!("duplicate option {arg}").into());
                    }
                }
            } else {
                result.positional.push(arg);
            }
        }
        Ok(result)
    }

    fn path(&self, key: &str) -> Option<PathBuf> {
        self.values.get(key).map(PathBuf::from)
    }

    fn value<T: std::str::FromStr>(&self, key: &str) -> Result<Option<T>> {
        self.values
            .get(key)
            .map(|s| {
                s.parse()
                    .map_err(|_| format!("invalid value for --{key}: {s:?}").into())
            })
            .transpose()
    }

    fn allow(&self, count: usize, values: &[&str], flags: &[&str]) -> Result<()> {
        if self.positional.len() != count {
            return Err(format!("expected {count} positional arguments").into());
        }
        for key in self.values.keys() {
            if !values.contains(&key.as_str()) {
                return Err(format!("unknown option --{key}").into());
            }
        }
        for key in &self.flags {
            if !flags.contains(&key.as_str()) {
                return Err(format!("unknown flag --{key}").into());
            }
        }
        Ok(())
    }
}

fn show(value: &impl serde::Serialize, path: Option<PathBuf>) -> Result<()> {
    if let Some(path) = path {
        export::json(&path, value)
    } else {
        println!("{}", serde_json::to_string_pretty(value)?);
        Ok(())
    }
}

fn run(started: Instant) -> Result<u8> {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "--help".into());
    if command == "--version" {
        println!("budel {}", env!("CARGO_PKG_VERSION"));
        return Ok(0);
    }
    if command == "--help" || command == "help" {
        println!(
            "budel inspect CASE [--json PATH]\nbudel validate CASE ANSWER [--strict] [--json PATH]\nbudel render CASE ANSWER --output PATH.svg\nbudel solve CASE --output ANSWER --run-dir DIR [--time-limit SECONDS | --budget NAME]\n  [--algorithm baseline|restart|local|multistart|lns|repair|fcmaes-block] [--seed N] [--workers N]\n  [--eval-limit N] [--policy JSON] [--cache-mib N | --no-cache] [--scalar] [--no-pairs]\n  [--no-slide] [--no-pockets] [--strict-accept] [--no-separate] [--no-share]\nbudel resume CHECKPOINT --additional-time SECONDS --output ANSWER --run-dir DIR [--case CASE]\nbudel tune CASE --output POLICY.json [--method fixed|random|de|bite] [--evaluations N]\n  [--per-trial PROPOSALS] [--seeds N] [--holdout-seeds N] [--seed N] [--time-limit S] [--json PATH]   (fcmaes builds)"
        );
        return Ok(0);
    }
    let args = Arguments::parse(args)?;
    match command.as_str() {
        "inspect" => {
            args.allow(1, &["json"], &[])?;
            let case = Case::load(Path::new(&args.positional[0]))?;
            show(&inspect(&case), args.path("json"))?;
            Ok(0)
        }
        "validate" => {
            args.allow(2, &["json"], &["strict"])?;
            let case = Case::load(Path::new(&args.positional[0]))?;
            let answer = Answer::load(Path::new(&args.positional[1]))?;
            let report = validate(&case, &answer);
            show(&report, args.path("json"))?;
            Ok(if report.strict_valid { 0 } else { 1 })
        }
        "render" => {
            args.allow(2, &["output"], &[])?;
            let case = Case::load(Path::new(&args.positional[0]))?;
            let answer = Answer::load(Path::new(&args.positional[1]))?;
            let path = args.path("output").ok_or("render requires --output")?;
            export::svg(&case, &answer, &path)?;
            Ok(0)
        }
        "solve" => {
            args.allow(
                1,
                &[
                    "output",
                    "run-dir",
                    "time-limit",
                    "budget",
                    "algorithm",
                    "seed",
                    "workers",
                    "eval-limit",
                    "policy",
                    "cache-mib",
                    "memory-limit-mib",
                ],
                &[
                    "scalar",
                    "no-cache",
                    "no-pairs",
                    "no-slide",
                    "no-pockets",
                    "strict-accept",
                    "no-separate",
                    "no-share",
                ],
            )?;
            let case_path = PathBuf::from(&args.positional[0]);
            let (seconds, named_budget) = execution::choose_time(
                &case_path,
                args.value("time-limit")?,
                args.values.get("budget").map(String::as_str),
            )?;
            let output = args.path("output").ok_or("solve requires --output")?;
            let run_dir = args.path("run-dir").ok_or("solve requires --run-dir")?;
            let cache_mib = args.value::<usize>("cache-mib")?.unwrap_or(64);
            if args.flags.contains("no-cache") && args.values.contains_key("cache-mib") {
                return Err("use either --cache-mib or --no-cache".into());
            }
            let policy = match args.path("policy") {
                Some(path) => serde_json::from_slice::<Policy>(&budel_craft::model::read_limited(
                    &path,
                    64 * 1024,
                )?)?,
                None => Policy::default(),
            };
            let config = Config {
                algorithm: args.value("algorithm")?.unwrap_or_default(),
                seed: args.value("seed")?.unwrap_or(1),
                workers: args.value("workers")?.unwrap_or(1),
                cache_bytes: if args.flags.contains("no-cache") {
                    0
                } else {
                    cache_mib
                        .checked_mul(1024 * 1024)
                        .ok_or("cache size overflow")?
                },
                edge_tree: !args.flags.contains("scalar"),
                pair_hints: !args.flags.contains("no-pairs"),
                policy,
                features: Features {
                    slide: !args.flags.contains("no-slide"),
                    pockets: !args.flags.contains("no-pockets"),
                    late_acceptance: !args.flags.contains("strict-accept"),
                    separate: !args.flags.contains("no-separate"),
                    share: !args.flags.contains("no-share"),
                },
            };
            install_interrupt_handler()?;
            if let Some(mib) = args.value("memory-limit-mib")? {
                limit_address_space(mib)?;
            }
            let outcome = execution::run(
                Request {
                    case_path,
                    output,
                    run_dir,
                    config,
                    seconds,
                    requested_proposals: args.value("eval-limit")?,
                    named_budget,
                    resume: None,
                },
                started,
            )?;
            show(&outcome, None)?;
            Ok(if outcome.certified_answer_written {
                0
            } else {
                1
            })
        }
        "resume" => {
            args.allow(
                1,
                &[
                    "additional-time",
                    "output",
                    "run-dir",
                    "case",
                    "eval-limit",
                    "memory-limit-mib",
                ],
                &[],
            )?;
            let checkpoint = execution::load_checkpoint(Path::new(&args.positional[0]))?;
            let case_path = args
                .path("case")
                .unwrap_or_else(|| checkpoint.case_path.clone());
            let output = args.path("output").ok_or("resume requires --output")?;
            let run_dir = args.path("run-dir").ok_or("resume requires --run-dir")?;
            let seconds = args
                .value("additional-time")?
                .ok_or("resume requires --additional-time")?;
            install_interrupt_handler()?;
            if let Some(mib) = args.value("memory-limit-mib")? {
                limit_address_space(mib)?;
            }
            let outcome = execution::run(
                Request {
                    case_path,
                    output,
                    run_dir,
                    config: checkpoint.config.clone(),
                    seconds,
                    requested_proposals: args.value("eval-limit")?,
                    named_budget: None,
                    resume: Some(checkpoint),
                },
                started,
            )?;
            show(&outcome, None)?;
            Ok(if outcome.certified_answer_written {
                0
            } else {
                1
            })
        }
        #[cfg(feature = "fcmaes")]
        "tune" => {
            use budel_craft::search::fcmaes::{Tune, tune};
            args.allow(
                1,
                &[
                    "method",
                    "seed",
                    "evaluations",
                    "per-trial",
                    "seeds",
                    "holdout-seeds",
                    "time-limit",
                    "output",
                    "json",
                ],
                &[],
            )?;
            let setup = Tune {
                method: args
                    .values
                    .get("method")
                    .cloned()
                    .unwrap_or_else(|| "de".into()),
                seed: args.value("seed")?.unwrap_or(1),
                evaluations: args.value("evaluations")?.unwrap_or(24),
                per_trial: args.value("per-trial")?.unwrap_or(4000),
                seconds: args.value("time-limit")?.unwrap_or(8.0),
                seeds: args.value("seeds")?.unwrap_or(5),
                holdout_seeds: args.value("holdout-seeds")?.unwrap_or(5),
            };
            let output = args
                .path("output")
                .ok_or("tune requires --output policy.json")?;
            install_interrupt_handler()?;
            let case = Case::load(Path::new(&args.positional[0]))?;
            let result = tune(&case, setup, started)?;
            if result.actual_evaluations == 0 {
                return Err("no completed policy evaluations".into());
            }
            export::json(&output, &result.selected_policy)?;
            show(&result, args.path("json"))?;
            Ok(0)
        }
        #[cfg(not(feature = "fcmaes"))]
        "tune" => Err("tune requires a build with --features fcmaes".into()),
        _ => Err(format!("unknown command {command:?}, see --help").into()),
    }
}

fn main() -> std::process::ExitCode {
    let started = Instant::now();
    match run(started) {
        Ok(code) => std::process::ExitCode::from(code),
        Err(err) => {
            eprintln!("budel: {err}");
            std::process::ExitCode::from(2)
        }
    }
}
