//! Overlap-tolerant insertion: seat a missing item where it overlaps least, then
//! resolve the overlaps by moving colliding items, guided by penetration depth and
//! guided-local-search pair weights. The working layout contains overlapping
//! placements only inside `separate_insert`; it is either fully feasible again on
//! return or restored exactly. Nothing is published while overlaps exist.
use super::{Candidate, Engine, clip_point, random_point};
use crate::geometry::{
    Aabb, Point, Pose,
    index::{conflict, penetration},
};
use crate::layout::{Placed, Snapshot};
use crate::model::Poses;
use rand::Rng;
use rand::seq::SliceRandom;
use std::collections::HashMap;

type Weights = HashMap<(usize, usize), f64>;

fn key(a: usize, b: usize) -> (usize, usize) {
    if a < b { (a, b) } else { (b, a) }
}

/// GLS multiplier for pairs that still collide after a sweep.
const WEIGHT_GROWTH: f64 = 1.3;
/// Decay toward 1 for pairs that no longer collide.
const WEIGHT_DECAY: f64 = 0.95;
/// Sweeps without a 1% drop in raw overlap before giving up.
const MAX_STRIKES: usize = 6;

impl Engine<'_> {
    /// Weighted overlap of `p` (item `id`, not in the layout) with placed items.
    /// `None` means the work budget is exhausted. Evaluation stops once `bound` is reached.
    fn overlap_cost(
        &mut self,
        id: usize,
        p: &Placed,
        bound: f64,
        weights: &Weights,
    ) -> Option<f64> {
        if !self.budget.claim() {
            return None;
        }
        self.stats.kernel.candidates += 1;
        if !self.case.usable.contains(p.bounds) {
            return Some(f64::INFINITY);
        }
        let gap = self.case.gap + self.case.guard;
        let mut near = std::mem::take(&mut self.near);
        self.layout
            .nearby_into(p.bounds.expand(gap + self.case.guard), &mut near);
        let mut cost = 0.0;
        for &j in &near {
            if j == id {
                continue;
            }
            let other = self.layout.entries[j]
                .as_ref()
                .expect("grid entries have placements");
            let offset = other.pose.translation() - p.pose.translation();
            if conflict(
                &p.shape,
                &other.shape,
                offset,
                gap,
                self.cfg.edge_tree,
                &mut self.stats.kernel,
            ) {
                let w = weights.get(&key(id, j)).copied().unwrap_or(1.0);
                cost += w * penetration(&p.shape, &other.shape, offset, gap);
                if cost >= bound {
                    break;
                }
            }
        }
        self.near = near;
        Some(cost)
    }

    /// A placement of `id` in `pose` whose bounds are centred at `center`, clipped
    /// inside the usable rectangle.
    pub(super) fn centered(&mut self, id: usize, pose: Pose, center: Point) -> Option<Placed> {
        let shape = self.oriented(id, pose);
        let size = shape.bounds.size();
        let bin = self.case.usable.expand(-self.case.guard * 2.0);
        let max = bin.max - size;
        if max.0 < bin.min.0 || max.1 < bin.min.1 {
            return None;
        }
        let anchor = clip_point(center - size.scale(0.5), Aabb { min: bin.min, max });
        Some(Placed::at_lower_left(shape, pose, anchor))
    }

    /// Samples an orientation. Local samples usually keep the current orientation.
    pub(super) fn sample_pose(&mut self, id: usize, current: Pose, local: bool) -> Pose {
        let free = self.case.rules.poses == Poses::Free;
        let r: f64 = self.rng.r#gen();
        if local {
            if r < 0.7 {
                return current;
            }
            if free && r < 0.9 {
                let delta = self.rng.gen_range(-12.0..=12.0);
                return Pose {
                    rot: Pose::canonical_angle(current.rot + delta),
                    ..current
                };
            }
        }
        let n = self.hint(id).samples.len();
        let j = self.rng.gen_range(0..n);
        self.hint(id).samples[j].pose
    }

    /// Moves one placed item to the lowest weighted-overlap pose found by sampling and
    /// coordinate descent. Returns false when the work budget ran out.
    fn separate_move(&mut self, id: usize, weights: &Weights) -> bool {
        let Some(cur) = self.layout.remove(self.case, id) else {
            return true;
        };
        let Some(cur_cost) = self.overlap_cost(id, &cur, f64::INFINITY, weights) else {
            self.layout.insert(self.case, id, cur);
            return false;
        };
        let mut best = (cur_cost, cur.clone());
        let Point(w, h) = cur.bounds.size();
        let scale = w.max(h);
        let center = cur.bounds.center();
        let mut alive = true;
        for k in 0..40 {
            if best.0 == 0.0 {
                break;
            }
            let local = k < 32;
            let pose = self.sample_pose(id, cur.pose, local);
            let c = if local {
                let r = scale * [0.6, 0.3, 0.12, 0.05][k % 4];
                center + Point(self.rng.gen_range(-r..=r), self.rng.gen_range(-r..=r))
            } else {
                random_point(&mut self.rng, self.case.usable)
            };
            let Some(p) = self.centered(id, pose, c) else {
                continue;
            };
            match self.overlap_cost(id, &p, best.0, weights) {
                Some(cost) if cost < best.0 => best = (cost, p),
                Some(_) => {}
                None => {
                    alive = false;
                    break;
                }
            }
        }
        let mut step = scale * 0.08;
        let min_step = self.case.gap * 0.02;
        while alive && best.0 > 0.0 && step > min_step {
            let mut improved = false;
            for d in [Point(1., 0.), Point(-1., 0.), Point(0., 1.), Point(0., -1.)] {
                let t = best.1.pose.translation() + d.scale(step);
                let p = Placed::new(best.1.pose.with_translation(t), best.1.shape.clone());
                match self.overlap_cost(id, &p, best.0, weights) {
                    Some(cost) if cost < best.0 => {
                        best = (cost, p);
                        improved = true;
                    }
                    Some(_) => {}
                    None => {
                        alive = false;
                        break;
                    }
                }
            }
            if !improved {
                step *= 0.5;
            }
        }
        self.stats.separation_moves += 1;
        self.layout.insert(self.case, id, best.1);
        alive
    }

    /// Colliding pairs among `touched` items and everything near them, with raw overlap.
    fn collisions(&mut self, touched: &[usize]) -> (Vec<(usize, usize)>, f64) {
        let mut pairs = Vec::new();
        let mut hits = std::mem::take(&mut self.hits);
        for &i in touched {
            let Some(p) = self.layout.entries[i].clone() else {
                continue;
            };
            self.layout.colliders(
                self.case,
                &p,
                i,
                self.cfg.edge_tree,
                &mut self.stats.kernel,
                &mut hits,
            );
            pairs.extend(hits.iter().map(|&j| key(i, j)));
        }
        self.hits = hits;
        pairs.sort_unstable();
        pairs.dedup();
        let gap = self.case.gap + self.case.guard;
        let raw = pairs
            .iter()
            .map(|&(a, b)| {
                let pa = self.layout.entries[a].as_ref().expect("placed");
                let pb = self.layout.entries[b].as_ref().expect("placed");
                penetration(
                    &pa.shape,
                    &pb.shape,
                    pb.pose.translation() - pa.pose.translation(),
                    gap,
                )
            })
            .sum();
        (pairs, raw)
    }

    /// One overlap-tolerant insertion attempt for a missing item. Returns true when the
    /// layout gained the item and is feasible; otherwise the layout is restored exactly.
    pub(super) fn separate_insert(
        &mut self,
        best: &mut Snapshot,
        publish: &mut impl FnMut(Candidate),
    ) -> bool {
        let n = self.case.items.len();
        let missing: Vec<usize> = (0..n)
            .filter(|&i| self.layout.entries[i].is_none())
            .collect();
        if missing.is_empty() || !self.budget.can_search() {
            return false;
        }
        // Tournament: large items first, discounted by earlier failures.
        let score = |e: &Self, i: usize| {
            e.case.items[i].score_area / (1.0 + f64::from(e.separation_failures[i])).powi(2)
        };
        let mut target = missing[self.rng.gen_range(0..missing.len())];
        for _ in 0..3 {
            let other = missing[self.rng.gen_range(0..missing.len())];
            if score(self, other) > score(self, target) {
                target = other;
            }
        }
        self.stats.separation_trials += 1;
        let unweighted = Weights::new();
        let mut start: Option<(f64, Placed)> = None;
        for _ in 0..48 {
            let pose = self.sample_pose(target, Pose::default(), false);
            let c = random_point(&mut self.rng, self.case.usable);
            let Some(p) = self.centered(target, pose, c) else {
                continue;
            };
            let bound = start.as_ref().map_or(f64::INFINITY, |s| s.0);
            match self.overlap_cost(target, &p, bound, &unweighted) {
                Some(cost) if cost < bound => start = Some((cost, p)),
                Some(_) => {}
                None => return false,
            }
            if start.as_ref().is_some_and(|s| s.0 == 0.0) {
                break;
            }
        }
        let Some((_, p)) = start else {
            return false;
        };
        let old_area = self.layout.area;
        let mut originals: HashMap<usize, Placed> = HashMap::new();
        let mut touched = vec![target];
        self.layout.insert(self.case, target, p);
        let mut weights = Weights::new();
        let limit = self.cfg.policy.separate_moves;
        let mut moves = 0;
        let mut best_raw = f64::INFINITY;
        let mut strikes = 0;
        let success = loop {
            let (pairs, raw) = self.collisions(&touched);
            if pairs.is_empty() {
                break true;
            }
            if raw < best_raw * 0.99 {
                best_raw = raw;
                strikes = 0;
            } else {
                strikes += 1;
            }
            if strikes >= MAX_STRIKES || moves >= limit || !self.budget.can_search() {
                break false;
            }
            for w in weights.values_mut() {
                *w = (*w * WEIGHT_DECAY).max(1.0);
            }
            for &pair in &pairs {
                let w = weights.entry(pair).or_insert(1.0);
                *w = (*w * WEIGHT_GROWTH / WEIGHT_DECAY).min(1e4);
            }
            let mut members: Vec<usize> = pairs.iter().flat_map(|&(a, b)| [a, b]).collect();
            members.sort_unstable();
            members.dedup();
            members.shuffle(&mut self.rng);
            let mut alive = true;
            for i in members {
                if moves >= limit {
                    break;
                }
                if i != target && !originals.contains_key(&i) {
                    let original = self.layout.entries[i].clone().expect("placed member");
                    originals.insert(i, original);
                    touched.push(i);
                }
                moves += 1;
                if !self.separate_move(i, &weights) {
                    alive = false;
                    break;
                }
            }
            if !alive {
                break false;
            }
        };
        if success {
            self.stats.separations += 1;
            if let Some(late) = self.late.as_mut() {
                late.accept(old_area, self.layout.area);
            }
            self.emit(best, "separation", publish);
            return true;
        }
        self.separation_failures[target] = self.separation_failures[target].saturating_add(1);
        self.layout.remove(self.case, target);
        let mut restore: Vec<_> = originals.into_iter().collect();
        restore.sort_unstable_by_key(|(i, _)| *i);
        for (i, _) in &restore {
            self.layout.remove(self.case, *i);
        }
        for (i, p) in restore {
            self.layout.insert(self.case, i, p);
        }
        self.layout.area = old_area;
        false
    }
}

#[cfg(test)]
mod tests {
    use crate::model::Case;
    use crate::runtime::Budget;
    use crate::search::{Config, Engine, Prepared};
    use crate::validate::validate;
    use std::path::Path;
    use std::time::Instant;

    #[test]
    fn separation_leaves_a_feasible_or_identical_layout() {
        let case = Case::load(Path::new("cases/medium.json")).expect("case");
        let budget = Budget::new(Instant::now(), 60.0, 0.0, Some(400_000)).expect("budget");
        let prepared = Prepared::new(&case, &budget).expect("prepared");
        let cfg = Config::default();
        let mut engine = Engine::new(&case, &prepared, &budget, &cfg, 7);
        let mut best = crate::layout::Snapshot::default();
        let mut sink = |_c: crate::search::Candidate| {};
        engine.construct(0, &mut best, &mut sink);
        engine.fill_gaps(60, &mut best, &mut sink);
        let mut outcomes = [0usize; 2];
        for _ in 0..6 {
            let before = engine.layout.snapshot(&case);
            let ok = engine.separate_insert(&mut best, &mut sink);
            let after = engine.layout.snapshot(&case);
            let report = validate(&case, &after.answer(&case));
            assert!(report.strict_valid, "separator left an infeasible layout");
            if ok {
                assert_eq!(after.poses.len(), before.poses.len() + 1);
            } else {
                assert_eq!(
                    after.poses, before.poses,
                    "failed separation must restore poses"
                );
            }
            outcomes[usize::from(ok)] += 1;
        }
        assert_eq!(outcomes.iter().sum::<usize>(), 6);
    }
}
