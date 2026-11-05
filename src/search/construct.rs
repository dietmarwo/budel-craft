use super::{Candidate, Engine, clip_point, random_point};
use crate::geometry::{Aabb, Point, Pose};
use crate::layout::{Placed, Snapshot};
use crate::model::Poses;
use rand::Rng;

struct Rectangles {
    free: Vec<Aabb>,
    pad: f64,
    cap: usize,
}

impl Rectangles {
    fn new(bin: Aabb, pad: f64, cap: usize) -> Self {
        let bin = Aabb {
            min: bin.min,
            max: bin.max + Point(pad, pad),
        };
        Self {
            free: vec![bin],
            pad,
            cap,
        }
    }

    fn cut(&mut self, used: Aabb) {
        let used = Aabb {
            min: used.min,
            max: used.max + Point(self.pad, self.pad),
        };
        let mut next = Vec::with_capacity(self.free.len() + 8);
        for bb in self.free.drain(..) {
            if used.min.0 >= bb.max.0
                || used.max.0 <= bb.min.0
                || used.min.1 >= bb.max.1
                || used.max.1 <= bb.min.1
            {
                next.push(bb);
                continue;
            }
            if used.min.0 > bb.min.0 {
                next.push(Aabb {
                    min: bb.min,
                    max: Point(used.min.0, bb.max.1),
                });
            }
            if used.max.0 < bb.max.0 {
                next.push(Aabb {
                    min: Point(used.max.0, bb.min.1),
                    max: bb.max,
                });
            }
            if used.min.1 > bb.min.1 {
                next.push(Aabb {
                    min: bb.min,
                    max: Point(bb.max.0, used.min.1),
                });
            }
            if used.max.1 < bb.max.1 {
                next.push(Aabb {
                    min: Point(bb.min.0, used.max.1),
                    max: bb.max,
                });
            }
        }
        next.sort_unstable_by(|a, b| {
            b.area()
                .total_cmp(&a.area())
                .then(a.min.1.total_cmp(&b.min.1))
                .then(a.min.0.total_cmp(&b.min.0))
        });
        for bb in next {
            let Point(w, h) = bb.size();
            if w > self.pad && h > self.pad && self.free.iter().all(|other| !other.contains(bb)) {
                self.free.push(bb);
                if self.free.len() >= self.cap {
                    break;
                }
            }
        }
    }
}

impl Engine<'_> {
    pub(super) fn stock_order(&mut self, attempt: usize, missing_only: bool) -> Vec<usize> {
        let policy = self.cfg.policy;
        let mut keys = Vec::with_capacity(self.case.items.len());
        for i in 0..self.case.items.len() {
            if missing_only && self.layout.entries[i].is_some() {
                continue;
            }
            let area = self.case.items[i].score_area;
            let bb = self.hint(i).compact.bb;
            let Point(w, h) = bb.size();
            let key = match attempt % 5 {
                1 => bb.area().ln(),
                2 => w.max(h).ln() * 1.9 + area.ln() * 0.05,
                3 => area.ln(),
                _ => {
                    policy.stock_area * area.ln()
                        + policy.box_area * bb.area().ln()
                        + policy.extent * (w.max(h) * w.max(h)).ln()
                }
            };
            let jitter = if attempt == 0 {
                0.0
            } else {
                self.rng.gen_range(-0.10..0.10)
            };
            keys.push((i, key + jitter));
        }
        keys.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        keys.into_iter().map(|(i, _)| i).collect()
    }

    pub(super) fn construct(
        &mut self,
        attempt: usize,
        best: &mut Snapshot,
        publish: &mut impl FnMut(Candidate),
    ) {
        let case = self.case;
        let safe = case.usable.expand(-case.guard * 2.0);
        let pad = case.gap + case.guard * 4.0;
        let mut free = Rectangles::new(safe, pad, if case.items.len() > 500 { 192 } else { 384 });
        let scale = case.usable.size().0.min(case.usable.size().1).max(1.0);
        let order = self.stock_order(attempt, false);
        // Adjacent identical bodies share angle preparation and make repeat pairs cheap.
        let mut repeat = None;
        for i in order {
            if !self.budget.can_search() {
                break;
            }
            let key = case.items[i].geometry.key;
            let mut best_placement = None;
            let mut best_cost = f64::INFINITY;
            for rect in &free.free {
                let Point(rw, rh) = rect.size();
                for hint in &self.hint(i).rects {
                    let Point(w, h) = hint.bb.size() + Point(pad, pad);
                    let (dx, dy) = (rw - w, rh - h);
                    if dx < 0.0 || dy < 0.0 {
                        continue;
                    }
                    let short = dx.min(dy) + 0.06 * dx.max(dy);
                    let waste = (rw * rh - w * h) / scale;
                    let high = rect.min.1 + h + 0.12 * (rect.min.0 + w);
                    let cost = match attempt % 4 {
                        1 => short + 0.08 * high,
                        2 => waste + 0.06 * high + 0.10 * short,
                        3 => high + 0.15 * short,
                        _ => {
                            short
                                + self.cfg.policy.rect_waste * waste
                                + self.cfg.policy.rect_height * high
                        }
                    };
                    if cost < best_cost {
                        best_cost = cost;
                        best_placement = Some((hint.pose, rect.min));
                    }
                }
            }
            if let Some((mut pose, point)) = best_placement {
                let flip = case.rules.mirror_allowed
                    && (attempt > 0 && self.rng.gen_bool(0.45)
                        || (self.cfg.pair_hints && repeat == Some(key)));
                if flip {
                    // Reflect about world Y after the rotation, preserving AABB size.
                    pose.rot = Pose::canonical_angle(-pose.rot);
                    pose.mirror = true;
                }
                let p = self.at(i, pose, point);
                if self.fits(&p) {
                    free.cut(p.bounds);
                    self.layout.insert(case, i, p);
                    self.stats.rectangle_insertions += 1;
                    if self.layout.count.is_multiple_of(128) {
                        self.emit(best, "rectangle-checkpoint", publish);
                    }
                    repeat = Some(key);
                }
            }
        }
        self.emit(best, "rectangles", publish);
    }

    pub(super) fn fill_gaps(
        &mut self,
        trials: usize,
        best: &mut Snapshot,
        publish: &mut impl FnMut(Candidate),
    ) {
        let order = self.stock_order(0, true);
        for i in order {
            if !self.budget.can_search() {
                break;
            }
            if let Some(p) = self.find(i, trials, None) {
                self.layout.insert(self.case, i, p);
                self.stats.gap_insertions += 1;
                if self.stats.gap_insertions.is_multiple_of(32) {
                    self.emit(best, "gap-checkpoint", publish);
                }
            }
        }
        self.emit(best, "gap-fill", publish);
    }

    pub(super) fn proposal(&mut self, i: usize, k: usize, focus: Option<Point>) -> Option<Placed> {
        let h = self.hint(i);
        let mut pose = if k < 4 {
            h.compact.pose
        } else if k.is_multiple_of(11) && self.case.rules.poses == Poses::Free {
            Pose {
                rot: self.rng.gen_range(0.0..360.0),
                mirror: self.case.rules.mirror_allowed && self.rng.gen_bool(0.5),
                ..Pose::default()
            }
        } else {
            let n = h.samples.len();
            let j = self.rng.gen_range(0..n);
            self.hint(i).samples[j].pose
        };
        if k == 1 && self.case.rules.mirror_allowed {
            pose.rot = Pose::canonical_angle(-pose.rot);
            pose.mirror = true;
        }
        let shape = self.oriented(i, pose);
        let size = shape.bounds.size();
        let usable = self.case.usable.expand(-self.case.guard * 2.0);
        let max = usable.max - size;
        if max.0 < usable.min.0 || max.1 < usable.min.1 {
            return None;
        }
        let range = Aabb {
            min: usable.min,
            max,
        };
        let anchor = if let Some(center) = focus.filter(|_| k < 4) {
            center - size.scale(0.5)
        } else if k < 4 {
            [
                range.min,
                Point(range.max.0, range.min.1),
                Point(range.min.0, range.max.1),
                range.max,
            ][k]
        } else if let Some(center) = focus.filter(|_| !k.is_multiple_of(4)) {
            let spread = size.0.max(size.1) + self.case.gap;
            center - size.scale(0.5)
                + Point(
                    self.rng.gen_range(-spread..=spread),
                    self.rng.gen_range(-spread..=spread),
                )
        } else if self.layout.count > 0 && !k.is_multiple_of(3) {
            let len = self.layout.entries.len();
            let start = self.rng.gen_range(0..len);
            let neighbor = (0..len).find_map(|j| self.layout.entries[(start + j) % len].as_ref());
            if let Some(neighbor) = neighbor {
                if self.rng.gen_bool(self.cfg.policy.contact_rate) {
                    let q = neighbor.shape.points
                        [self.rng.gen_range(0..neighbor.shape.points.len())]
                        + neighbor.pose.translation();
                    let p = shape.points[self.rng.gen_range(0..shape.points.len())];
                    let angle = self.rng.gen_range(0.0..std::f64::consts::TAU);
                    let (sin, cos) = angle.sin_cos();
                    q - p + shape.bounds.min + Point(cos, sin).scale(self.case.gap + 0.03)
                } else {
                    let bb = neighbor.bounds;
                    let w = bb.size();
                    let gap = self.case.gap + self.case.guard * 4.0;
                    match k % 5 {
                        0 => Point(
                            bb.min.0 - gap - size.0,
                            bb.min.1 + self.rng.gen_range(-size.1..=w.1),
                        ),
                        1 => Point(bb.max.0 + gap, bb.min.1 + self.rng.gen_range(-size.1..=w.1)),
                        2 => Point(bb.min.0 + self.rng.gen_range(-size.0..=w.0), bb.max.1 + gap),
                        3 => Point(
                            bb.min.0 + self.rng.gen_range(-size.0..=w.0),
                            bb.min.1 - gap - size.1,
                        ),
                        _ => {
                            bb.center() - size.scale(0.5)
                                + Point(
                                    self.rng.gen_range(-w.0..=w.0),
                                    self.rng.gen_range(-w.1..=w.1),
                                )
                        }
                    }
                }
            } else {
                random_point(&mut self.rng, range)
            }
        } else {
            random_point(&mut self.rng, range)
        };
        Some(Placed::at_lower_left(
            shape,
            pose,
            clip_point(anchor, range),
        ))
    }

    /// First-fit (`best_of == 1`) or best-of-k feasible placement. With best-of-k each
    /// feasible proposal is slid to contact and scored by its lower-left extent.
    pub(super) fn find(&mut self, i: usize, trials: usize, focus: Option<Point>) -> Option<Placed> {
        let want = self.cfg.policy.best_of.max(1);
        let slide = self.cfg.features.slide && self.cfg.algorithm.persistent();
        let mut chosen: Option<(f64, Placed)> = None;
        let mut found = 0;
        for k in 0..trials {
            if !self.budget.can_search() {
                break;
            }
            if let Some(p) = self.proposal(i, k, focus)
                && self.fits(&p)
            {
                if want == 1 {
                    return Some(p);
                }
                let p = if slide {
                    self.slide_one(p, [Point(0., -1.), Point(-1., 0.)])
                } else {
                    p
                };
                let score = p.bounds.max.1 + 0.35 * p.bounds.max.0;
                if chosen.as_ref().is_none_or(|c| score < c.0) {
                    chosen = Some((score, p));
                }
                found += 1;
                if found >= want {
                    break;
                }
            }
        }
        chosen.map(|c| c.1)
    }
}
