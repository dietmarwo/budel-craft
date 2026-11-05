use super::{Algorithm, Candidate, Engine, clip_point};
use crate::geometry::{Aabb, Point, Pose, index::conflict};
use crate::layout::{Placed, Snapshot};
use crate::model::Poses;
use rand::Rng;

impl Engine<'_> {
    pub(super) fn improve(
        &mut self,
        best: &mut Snapshot,
        publish: &mut impl FnMut(Candidate),
        attempt: usize,
    ) {
        if matches!(self.cfg.algorithm, Algorithm::Baseline | Algorithm::Restart) {
            return;
        }
        for sweep in 0..2 {
            if self.layout.count == self.case.items.len() || !self.budget.can_search() {
                return;
            }
            self.compact(attempt + sweep);
            self.fill_gaps(self.cfg.policy.gap_trials / 2, best, publish);
        }
        if self.cfg.algorithm == Algorithm::Local {
            return;
        }
        let missing = self.stock_order(attempt, true);
        for step in 0..missing.len().clamp(12, 64) {
            if self.layout.count == self.case.items.len()
                || !self.budget.can_search()
                || missing.is_empty()
            {
                break;
            }
            let i = missing
                .get(step)
                .copied()
                .unwrap_or_else(|| missing[self.rng.gen_range(0..missing.len())]);
            if self.layout.entries[i].is_some() {
                continue;
            }
            self.stats.rebuild_trials += 1;
            if self.rebuild_near(i, step) {
                self.stats.rebuilds += 1;
                self.emit(best, "neighborhood", publish);
            }
        }
    }

    /// Slides a placement that is not in the layout toward the lower-left by step
    /// halving on the strict `fits` predicate. Every intermediate pose is feasible.
    pub(super) fn slide_one(&mut self, mut cur: Placed, dirs: [Point; 2]) -> Placed {
        let min_step = (self.case.gap * 0.015).max(self.case.guard * 64.0);
        for _ in 0..3 {
            let mut any = false;
            for d in dirs {
                let Point(w, h) = cur.bounds.size();
                let mut step = w.max(h) * 0.5;
                while step > min_step {
                    let t = cur.pose.translation() + d.scale(step);
                    let p = Placed::new(cur.pose.with_translation(t), cur.shape.clone());
                    self.stats.move_trials += 1;
                    if self.fits(&p) {
                        cur = p;
                        any = true;
                    } else {
                        step *= 0.5;
                    }
                }
            }
            if !any {
                break;
            }
        }
        cur
    }

    /// Slides every placed item toward the lower-left corner until contact. Area is
    /// unchanged; scattered free space merges toward the opposite corner.
    pub(super) fn slide_compact(&mut self, sweeps: usize) -> usize {
        let mut total = 0;
        for sweep in 0..sweeps {
            let dirs = if sweep % 2 == 0 {
                [Point(0., -1.), Point(-1., 0.)]
            } else {
                [Point(-1., 0.), Point(0., -1.)]
            };
            let mut ids: Vec<_> = self
                .layout
                .present()
                .map(|(i, p)| (i, p.bounds.min.1 + 0.35 * p.bounds.min.0))
                .collect();
            ids.sort_unstable_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
            let mut moved = 0;
            for (id, _) in ids {
                if !self.budget.can_search() {
                    return total + moved;
                }
                let old = self.layout.remove(self.case, id).expect("present item");
                let start = old.pose.translation();
                let cur = self.slide_one(old, dirs);
                if (cur.pose.translation() - start).norm2() > 1e-4 {
                    moved += 1;
                    self.stats.slides += 1;
                }
                self.layout.insert(self.case, id, cur);
            }
            total += moved;
            if moved == 0 {
                break;
            }
        }
        total
    }

    fn compact(&mut self, sweep: usize) {
        let mut ids: Vec<_> = self
            .layout
            .present()
            .map(|(i, p)| (i, p.bounds.min.1))
            .collect();
        ids.sort_unstable_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        let cap = ids.len().min(128);
        let offset = if ids.len() > cap {
            self.rng.gen_range(0..ids.len())
        } else {
            0
        };
        for k in 0..cap {
            if !self.budget.can_search() {
                break;
            }
            let id = ids[(k + offset) % ids.len()].0;
            let old = self.layout.remove(self.case, id).expect("present item");
            let mut current = old.clone();
            let extent = old.bounds.size().0.min(old.bounds.size().1);
            let pitch = (extent * 0.42).max(self.case.gap);
            for fraction in [1.0, 0.35, 0.11] {
                for direction in [Point(0., -1.), Point(-1., 0.), Point(-1., -1.)] {
                    let t = current.pose.translation() + direction.scale(pitch * fraction);
                    let p = Placed::new(current.pose.with_translation(t), current.shape.clone());
                    self.stats.move_trials += 1;
                    if self.fits(&p) {
                        current = p;
                    }
                }
            }
            let weights = if sweep.is_multiple_of(2) {
                Point(0.45, 1.)
            } else {
                Point(1., 0.58)
            };
            for k in 0..3 {
                if !self.budget.can_search() {
                    break;
                }
                let mut pose = current.pose;
                if k == 2 && self.case.rules.mirror_allowed {
                    pose.mirror = !pose.mirror;
                } else {
                    let angle = if self.case.rules.poses == Poses::Eight {
                        45.
                    } else {
                        8. + 7. * (sweep % 4) as f64
                    };
                    pose.rot =
                        Pose::canonical_angle(pose.rot + if k == 0 { angle } else { -angle });
                }
                let shape = self.oriented(id, pose);
                let t = current.bounds.center() - shape.bounds.center();
                let p = Placed::new(pose.with_translation(t), shape);
                self.stats.move_trials += 1;
                if p.bounds.max.dot(weights) < current.bounds.max.dot(weights) && self.fits(&p) {
                    current = p;
                }
            }
            if current.pose != old.pose {
                self.stats.moves += 1;
            }
            self.layout.insert(self.case, id, current);
        }
    }

    fn rebuild_near(&mut self, target: usize, step: usize) -> bool {
        let cap = self.cfg.policy.rebuild_size;
        let target_area = self.case.items[target].score_area;
        let focus = self
            .layout
            .present()
            .nth(step % self.layout.count.max(1))
            .map(|(_, p)| p.bounds.center());
        let mut chosen: Option<(f64, Placed, Vec<usize>)> = None;
        for k in 0..48 {
            if !self.budget.can_search() {
                break;
            }
            let Some(p) =
                self.proposal(target, k, if step.is_multiple_of(3) { focus } else { None })
            else {
                continue;
            };
            if !self.budget.claim() {
                break;
            }
            let Some(blockers) = self.layout.blockers(
                self.case,
                &p,
                self.cfg.edge_tree,
                cap,
                &mut self.stats.kernel,
            ) else {
                continue;
            };
            if blockers.is_empty() {
                self.layout.insert(self.case, target, p);
                return true;
            }
            let area: f64 = blockers
                .iter()
                .map(|&i| self.case.items[i].score_area)
                .sum();
            let weight = if self.case.rules.must_seat_all {
                1.0
            } else {
                self.cfg.policy.blocker_weight
            };
            let cost = weight * blockers.len() as f64 + area / target_area;
            if chosen.as_ref().is_none_or(|old| cost < old.0) {
                chosen = Some((cost, p, blockers));
            }
        }
        let Some((_, p, mut blockers)) = chosen else {
            return false;
        };
        if step % 4 == 3 && blockers.len() < cap {
            let mut nearby = self.layout.nearby(p.bounds.expand(self.case.gap * 2.0));
            nearby.sort_unstable();
            if let Some(i) = nearby.into_iter().find(|i| !blockers.contains(i)) {
                blockers.push(i);
            }
        }
        blockers.sort_unstable_by(|&a, &b| {
            self.case.items[b]
                .score_area
                .total_cmp(&self.case.items[a].score_area)
                .then(a.cmp(&b))
        });
        let old_count = self.layout.count;
        let old_area = self.layout.area;
        let removed: Vec<_> = blockers
            .into_iter()
            .map(|i| {
                (
                    i,
                    self.layout.remove(self.case, i).expect("blocker present"),
                )
            })
            .collect();

        if matches!(
            self.cfg.algorithm,
            Algorithm::Repair | Algorithm::FcmaesBlock
        ) && removed.len() < 8
            && self.budget.can_search()
        {
            let mut block = vec![(target, p.clone())];
            block.extend(removed.iter().cloned());
            self.stats.repair_trials += 1;
            if let Some(repaired) = self.repair_block(&block) {
                for (i, q) in repaired {
                    self.layout.insert(self.case, i, q);
                }
                self.stats.repairs += 1;
                return true;
            }
        }

        let mut added = Vec::with_capacity(removed.len() + 1);
        if self.fits(&p) {
            self.layout.insert(self.case, target, p);
            added.push(target);
            for (id, old) in &removed {
                let new = if self.fits(old) {
                    Some(old.clone())
                } else {
                    self.find(
                        *id,
                        (self.cfg.policy.gap_trials / 2).max(24),
                        Some(old.bounds.center()),
                    )
                };
                if let Some(p) = new {
                    self.layout.insert(self.case, *id, p);
                    added.push(*id);
                }
            }
        }
        let area_gain = self.layout.area > old_area + 1e-8;
        let target_seated = added.first() == Some(&target);
        let improve = if self.case.rules.must_seat_all {
            self.layout.count > old_count || (self.layout.count == old_count && area_gain)
        } else if let Some(late) = self.late.as_mut() {
            // Late acceptance only judges moves that actually seated the target.
            let accept = (area_gain || target_seated) && late.accept(old_area, self.layout.area);
            if accept && !area_gain {
                self.stats.late_accepts += 1;
            }
            accept
        } else {
            area_gain
        };
        if improve {
            return true;
        }
        for id in added {
            self.layout.remove(self.case, id);
        }
        for (id, p) in removed {
            self.layout.insert(self.case, id, p);
        }
        self.layout.area = old_area;
        false
    }

    fn block_cost(&mut self, block: &[(usize, Placed)]) -> Option<f64> {
        if !self.budget.claim() {
            return None;
        }
        let mut penalty = 0.0;
        let gap = self.case.gap + self.case.guard;
        let bounds = self.case.usable;
        for (k, (_, p)) in block.iter().enumerate() {
            if !bounds.contains(p.bounds) {
                let lo = bounds.min - p.bounds.min;
                let hi = p.bounds.max - bounds.max;
                penalty += 10. + lo.0.max(0.) + lo.1.max(0.) + hi.0.max(0.) + hi.1.max(0.);
            } else if let Some(blockers) =
                self.layout
                    .blockers(self.case, p, self.cfg.edge_tree, 24, &mut self.stats.kernel)
            {
                for i in blockers {
                    penalty += conflict_weight(
                        p.bounds,
                        self.layout.entries[i].as_ref().unwrap().bounds,
                        gap,
                    );
                }
            } else {
                penalty += 100.;
            }
            for (_, q) in &block[..k] {
                if conflict(
                    &p.shape,
                    &q.shape,
                    q.pose.translation() - p.pose.translation(),
                    gap,
                    self.cfg.edge_tree,
                    &mut self.stats.kernel,
                ) {
                    penalty += conflict_weight(p.bounds, q.bounds, gap);
                }
            }
        }
        Some(penalty)
    }

    fn repair_block(&mut self, base: &[(usize, Placed)]) -> Option<Vec<(usize, Placed)>> {
        #[cfg(feature = "fcmaes")]
        if self.cfg.algorithm == Algorithm::FcmaesBlock {
            return self.de_block(base);
        }
        let mut block = base.to_vec();
        let mut cost = self.block_cost(&block)?;
        for k in 0..72 {
            if cost == 0. {
                return Some(block);
            }
            if !self.budget.can_search() {
                break;
            }
            let j = k % block.len();
            let (id, old) = block[j].clone();
            let Point(w, h) = old.bounds.size();
            let step = w.min(h).max(self.case.gap) * [0.40, 0.18, 0.07][k / 24];
            let mut pose = old.pose;
            if k % 5 == 0 {
                let delta = if self.case.rules.poses == Poses::Free {
                    self.rng.gen_range(-18.0..=18.0)
                } else {
                    45.
                };
                pose.rot = Pose::canonical_angle(pose.rot + delta);
            }
            let center = old.bounds.center()
                + Point(
                    self.rng.gen_range(-step..=step),
                    self.rng.gen_range(-step..=step),
                );
            let shape = self.oriented(id, pose);
            let bin = self.case.usable.expand(-self.case.guard);
            let max = bin.max - shape.bounds.size();
            if max.0 < bin.min.0 || max.1 < bin.min.1 {
                continue;
            }
            let anchor = clip_point(
                center - shape.bounds.size().scale(0.5),
                Aabb { min: bin.min, max },
            );
            block[j].1 = Placed::at_lower_left(shape, pose, anchor);
            match self.block_cost(&block) {
                Some(value) if value <= cost => cost = value,
                _ => block[j].1 = old,
            }
        }
        (cost == 0.).then_some(block)
    }

    #[cfg(feature = "fcmaes")]
    fn de_block(&mut self, base: &[(usize, Placed)]) -> Option<Vec<(usize, Placed)>> {
        let stride = if self.case.rules.poses == Poses::Free {
            3
        } else {
            2
        };
        let mut best = None;
        let seed = self.rng.r#gen();
        super::fcmaes::de_search(base.len() * stride, seed, 72, Some(0.0), |x| {
            let mut block = Vec::with_capacity(base.len());
            for (j, (id, p)) in base.iter().enumerate() {
                let offset = j * stride;
                let Point(w, h) = p.bounds.size();
                let radius = w.min(h).max(self.case.gap) * 0.8;
                let mut pose = p.pose;
                if stride == 3 {
                    pose.rot = Pose::canonical_angle(pose.rot + x[offset + 2] * 35.);
                }
                let shape = self.oriented(*id, pose);
                let center = p.bounds.center() + Point(x[offset], x[offset + 1]).scale(radius);
                pose = pose.with_translation(center - shape.bounds.center());
                block.push((*id, Placed::new(pose, shape)));
            }
            let cost = self.block_cost(&block)?;
            self.stats.fcmaes_evaluations += 1;
            if cost == 0. {
                best = Some(block);
            }
            Some(cost)
        });
        best
    }
}

fn conflict_weight(a: Aabb, b: Aabb, gap: f64) -> f64 {
    let dx = a.max.0.min(b.max.0) - a.min.0.max(b.min.0);
    let dy = a.max.1.min(b.max.1) - a.min.1.max(b.min.1);
    1.0 + (dx + gap).min(dy + gap).max(0.)
}
