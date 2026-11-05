use crate::geometry::{Aabb, Point, reference_conflict};
use crate::model::{Answer, Case};
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Debug, Serialize)]
pub struct Violation {
    pub rule: &'static str,
    pub ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Validation {
    pub strict_valid: bool,
    pub complete: bool,
    pub seated: usize,
    pub available: usize,
    pub area: f64,
    pub coverage: f64,
    pub available_fraction: f64,
    pub certified_gap: Option<f64>,
    pub pair_checks: usize,
    pub checked_all: bool,
    pub violations: Vec<Violation>,
}

pub fn validate(case: &Case, answer: &Answer) -> Validation {
    validate_with(case, answer, || true)
}

pub fn validate_with(
    case: &Case,
    answer: &Answer,
    mut keep_going: impl FnMut() -> bool,
) -> Validation {
    let mut violations = Vec::new();
    let mut issue = |rule, ids| {
        if violations.len() < 100 {
            violations.push(Violation { rule, ids });
        }
    };
    if answer.case != case.name {
        issue("wrong-case", vec![]);
    }
    let mut seen = BTreeSet::new();
    let mut placed = Vec::<(usize, Vec<Point>, Aabb)>::new();
    let mut checked_all = true;
    for pl in &answer.placements {
        if !keep_going() {
            checked_all = false;
            break;
        }
        let Some(&idx) = case.ids.get(&pl.id) else {
            issue("unknown-shape", vec![pl.id.clone()]);
            continue;
        };
        if !seen.insert(idx) {
            issue("duplicate-shape", vec![pl.id.clone()]);
            continue;
        }
        if !case.allowed(pl.pose) {
            issue("forbidden-pose", vec![pl.id.clone()]);
            continue;
        }
        let ring = pl.pose.apply(&case.items[idx].geometry.ring);
        let bb = Aabb::of(&ring);
        if !bb.min.finite() || !bb.max.finite() || !case.usable.contains(bb) {
            issue("outside-plane", vec![pl.id.clone()]);
            continue;
        }
        placed.push((idx, ring, bb));
    }
    let complete = placed.len() == case.items.len();
    if case.rules.must_seat_all && !complete {
        issue("must-seat-all", vec![]);
    }
    let mut pair_checks = 0;
    let gap2 = case.gap * case.gap;
    'pairs: for (pos, (i, a, box_a)) in placed.iter().enumerate() {
        if !keep_going() {
            checked_all = false;
            break;
        }
        for (j, b, box_b) in &placed[..pos] {
            if box_a.gap2(*box_b) > gap2 {
                continue;
            }
            if !keep_going() {
                checked_all = false;
                break 'pairs;
            }
            pair_checks += 1;
            if reference_conflict(a, b, case.gap) {
                issue(
                    "contact-overlap-or-clearance",
                    vec![case.items[*i].id.clone(), case.items[*j].id.clone()],
                );
            }
        }
    }
    let area: f64 = placed
        .iter()
        .map(|(i, _, _)| case.items[*i].score_area)
        .sum();
    let strict_valid = checked_all && violations.is_empty();
    Validation {
        strict_valid,
        complete,
        seated: placed.len(),
        available: case.items.len(),
        area,
        coverage: case.coverage(area),
        available_fraction: if case.available_area > 0.0 {
            area / case.available_area
        } else {
            1.0
        },
        certified_gap: (strict_valid && placed.len() > 1).then_some(case.gap),
        pair_checks,
        checked_all,
        violations,
    }
}
