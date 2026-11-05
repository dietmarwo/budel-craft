use crate::geometry::{
    Aabb, Point, Pose,
    index::{KernelStats, Oriented, conflict},
};
use crate::model::{Answer, Case, Placement};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

type Cell = (i64, i64);
const MAX_QUERY_CELLS: i128 = 4096;

struct Grid {
    cell: f64,
    buckets: HashMap<Cell, Vec<usize>>,
    oversized: Vec<usize>,
    all: Vec<usize>,
    seen: Vec<u32>,
    epoch: u32,
    neighbors: Vec<usize>,
}

impl Grid {
    fn new(cell: f64, items: usize) -> Self {
        Self {
            cell,
            buckets: HashMap::new(),
            oversized: Vec::new(),
            all: Vec::new(),
            seen: vec![0; items],
            epoch: 0,
            neighbors: Vec::new(),
        }
    }

    fn cells(&self, bb: Aabb) -> Option<(Cell, Cell)> {
        let coord = |v: f64| (v / self.cell).floor() as i64;
        let lo = (coord(bb.min.0), coord(bb.min.1));
        let hi = (coord(bb.max.0), coord(bb.max.1));
        let count = (i128::from(hi.0) - i128::from(lo.0) + 1)
            .saturating_mul(i128::from(hi.1) - i128::from(lo.1) + 1);
        (count <= MAX_QUERY_CELLS).then_some((lo, hi))
    }

    fn add(&mut self, id: usize, bb: Aabb) {
        self.all.push(id);
        if let Some((lo, hi)) = self.cells(bb) {
            for x in lo.0..=hi.0 {
                for y in lo.1..=hi.1 {
                    self.buckets.entry((x, y)).or_default().push(id);
                }
            }
        } else {
            self.oversized.push(id);
        }
    }

    fn remove(&mut self, id: usize, bb: Aabb) {
        self.all.retain(|&i| i != id);
        if let Some((lo, hi)) = self.cells(bb) {
            for x in lo.0..=hi.0 {
                for y in lo.1..=hi.1 {
                    let key = (x, y);
                    if let Some(bucket) = self.buckets.get_mut(&key) {
                        bucket.retain(|&i| i != id);
                        if bucket.is_empty() {
                            self.buckets.remove(&key);
                        }
                    }
                }
            }
        } else {
            self.oversized.retain(|&i| i != id);
        }
    }

    fn query(&mut self, bb: Aabb) -> &[usize] {
        self.neighbors.clear();
        let Some((lo, hi)) = self.cells(bb) else {
            return &self.all;
        };
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.seen.fill(0);
            self.epoch = 1;
        }
        for x in lo.0..=hi.0 {
            for y in lo.1..=hi.1 {
                if let Some(bucket) = self.buckets.get(&(x, y)) {
                    for &i in bucket {
                        if self.seen[i] != self.epoch {
                            self.seen[i] = self.epoch;
                            self.neighbors.push(i);
                        }
                    }
                }
            }
        }
        self.neighbors.extend_from_slice(&self.oversized);
        &self.neighbors
    }
}

#[derive(Clone)]
pub struct Placed {
    pub pose: Pose,
    pub shape: Arc<Oriented>,
    pub bounds: Aabb,
}

impl Placed {
    pub fn new(pose: Pose, shape: Arc<Oriented>) -> Self {
        let bounds = shape.bounds.shift(pose.translation());
        Self {
            pose,
            shape,
            bounds,
        }
    }

    pub fn at_lower_left(shape: Arc<Oriented>, mut pose: Pose, point: Point) -> Self {
        pose = pose.with_translation(point - shape.bounds.min);
        Self::new(pose, shape)
    }
}

pub struct Layout {
    pub entries: Vec<Option<Placed>>,
    pub count: usize,
    pub area: f64,
    grid: Grid,
    scratch: Vec<usize>,
}

impl Layout {
    pub fn new(case: &Case) -> Self {
        let mut widths: Vec<_> = case
            .items
            .iter()
            .map(|item| {
                let Point(w, h) = item.geometry.bounds.size();
                (w * h).sqrt()
            })
            .collect();
        widths.sort_unstable_by(f64::total_cmp);
        let cell = widths
            .get(widths.len() / 2)
            .copied()
            .unwrap_or(1.0)
            .max(case.gap * 2.0)
            .max(0.01);
        Self {
            entries: vec![None; case.items.len()],
            count: 0,
            area: 0.,
            grid: Grid::new(cell, case.items.len()),
            scratch: Vec::new(),
        }
    }

    pub fn insert(&mut self, case: &Case, id: usize, item: Placed) {
        assert!(self.entries[id].is_none());
        self.grid.add(id, item.bounds);
        self.entries[id] = Some(item);
        self.count += 1;
        self.area += case.items[id].score_area;
    }

    pub fn remove(&mut self, case: &Case, id: usize) -> Option<Placed> {
        let item = self.entries[id].take()?;
        self.grid.remove(id, item.bounds);
        self.count -= 1;
        self.area -= case.items[id].score_area;
        Some(item)
    }

    pub fn fits(
        &mut self,
        case: &Case,
        item: &Placed,
        tree: bool,
        stats: &mut KernelStats,
    ) -> bool {
        stats.candidates += 1;
        if !case.usable.contains(item.bounds) {
            return false;
        }
        // The final original-coordinate check uses case.gap. Search keeps an additional
        // guard for the change from relative coordinates to exported world coordinates.
        let gap = case.gap + case.guard;
        for &i in self.grid.query(item.bounds.expand(gap + case.guard)) {
            let other = self.entries[i]
                .as_ref()
                .expect("grid entries have placements");
            if conflict(
                &item.shape,
                &other.shape,
                other.pose.translation() - item.pose.translation(),
                gap,
                tree,
                stats,
            ) {
                return false;
            }
        }
        true
    }

    pub fn nearby(&mut self, bounds: Aabb) -> Vec<usize> {
        self.grid.query(bounds).to_vec()
    }

    /// Writes nearby placed IDs into a reusable buffer.
    pub fn nearby_into(&mut self, bounds: Aabb, out: &mut Vec<usize>) {
        out.clear();
        out.extend_from_slice(self.grid.query(bounds));
    }

    /// All placed IDs (other than `skip`) that violate clearance with `item`, uncapped.
    /// Used by the separator, where temporarily overlapping placements are allowed.
    pub fn colliders(
        &mut self,
        case: &Case,
        item: &Placed,
        skip: usize,
        tree: bool,
        stats: &mut KernelStats,
        out: &mut Vec<usize>,
    ) {
        out.clear();
        let gap = case.gap + case.guard;
        let mut near = std::mem::take(&mut self.scratch);
        near.clear();
        near.extend_from_slice(self.grid.query(item.bounds.expand(gap + case.guard)));
        for &i in &near {
            if i == skip {
                continue;
            }
            let other = self.entries[i]
                .as_ref()
                .expect("grid entries have placements");
            if conflict(
                &item.shape,
                &other.shape,
                other.pose.translation() - item.pose.translation(),
                gap,
                tree,
                stats,
            ) {
                out.push(i);
            }
        }
        self.scratch = near;
    }

    pub fn blockers(
        &mut self,
        case: &Case,
        item: &Placed,
        tree: bool,
        cap: usize,
        stats: &mut KernelStats,
    ) -> Option<Vec<usize>> {
        stats.candidates += 1;
        if !case.usable.contains(item.bounds) {
            return None;
        }
        let mut result = Vec::new();
        let gap = case.gap + case.guard;
        for &i in self.grid.query(item.bounds.expand(gap + case.guard)) {
            let other = self.entries[i]
                .as_ref()
                .expect("grid entries have placements");
            if conflict(
                &item.shape,
                &other.shape,
                other.pose.translation() - item.pose.translation(),
                gap,
                tree,
                stats,
            ) {
                if result.len() >= cap {
                    return None;
                }
                result.push(i);
            }
        }
        Some(result)
    }

    pub fn present(&self) -> impl Iterator<Item = (usize, &Placed)> {
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(i, p)| p.as_ref().map(|p| (i, p)))
    }

    pub fn snapshot(&self, case: &Case) -> Snapshot {
        let poses: Vec<_> = self.present().map(|(i, p)| (i, p.pose)).collect();
        let area = poses.iter().map(|(i, _)| case.items[*i].score_area).sum();
        Snapshot { poses, area }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Snapshot {
    pub poses: Vec<(usize, Pose)>,
    pub area: f64,
}

impl Snapshot {
    pub fn better_than(&self, other: &Self, case: &Case) -> bool {
        if case.rules.must_seat_all && self.poses.len() != other.poses.len() {
            return self.poses.len() > other.poses.len();
        }
        self.area > other.area + 1e-9
    }

    pub fn answer(&self, case: &Case) -> Answer {
        let mut ans = Answer::new(case);
        ans.placements = self
            .poses
            .iter()
            .map(|&(idx, mut pose)| {
                pose.rot = Pose::canonical_angle(pose.rot);
                Placement {
                    id: case.items[idx].id.clone(),
                    pose,
                }
            })
            .collect();
        ans
    }
}
