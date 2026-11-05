//! Free-space targeting: rasterize the working layout, compute a Euclidean distance
//! map of free cells, and try missing items where the clearance can hold their largest
//! interior pole. The raster only proposes positions; `fits` decides feasibility.
use super::{Candidate, Engine};
use crate::geometry::{Aabb, Point, Pose, edges};
use crate::layout::{Placed, Snapshot};
use rand::Rng;
use std::sync::Arc;

/// Target number of raster cells over the usable rectangle.
const CELLS: f64 = 160_000.0;
const BIG: f64 = 1e20;

pub(super) struct FreeMap {
    origin: Point,
    cell: f64,
    w: usize,
    h: usize,
    occupied: Vec<bool>,
    /// Conservative clearance from each cell centre to the nearest occupied cell or border.
    pub(super) dist: Vec<f64>,
    scratch: Scratch,
}

#[derive(Default)]
struct Scratch {
    grid: Vec<f64>,
    f: Vec<f64>,
    d: Vec<f64>,
    v: Vec<usize>,
    z: Vec<f64>,
}

impl FreeMap {
    pub(super) fn new(usable: Aabb, min_cell: f64) -> Self {
        let Point(uw, uh) = usable.size();
        let cell = ((uw * uh) / CELLS).sqrt().max(min_cell).max(1e-9);
        let w = ((uw / cell).ceil() as usize).max(1);
        let h = ((uh / cell).ceil() as usize).max(1);
        Self {
            origin: usable.min,
            cell,
            w,
            h,
            occupied: vec![false; w * h],
            dist: vec![0.0; w * h],
            scratch: Scratch::default(),
        }
    }

    pub(super) fn center(&self, idx: usize) -> Point {
        let (x, y) = (idx % self.w, idx / self.w);
        Point(
            self.origin.0 + (x as f64 + 0.5) * self.cell,
            self.origin.1 + (y as f64 + 0.5) * self.cell,
        )
    }

    fn mark(&mut self, p: Point) {
        let x = ((p.0 - self.origin.0) / self.cell).floor();
        let y = ((p.1 - self.origin.1) / self.cell).floor();
        if x >= 0.0 && y >= 0.0 && (x as usize) < self.w && (y as usize) < self.h {
            self.occupied[y as usize * self.w + x as usize] = true;
        }
    }

    /// Marks cells whose centre lies inside the translated ring, and every cell that
    /// holds a vertex so thin curls are not lost between cell centres.
    pub(super) fn paint(&mut self, ring: &[Point], offset: Point) {
        let bb = Aabb::of(ring).shift(offset);
        let y0 = ((bb.min.1 - self.origin.1) / self.cell - 0.5)
            .ceil()
            .max(0.0) as usize;
        let y1 = ((bb.max.1 - self.origin.1) / self.cell - 0.5).floor();
        let mut xs = Vec::new();
        if y1 >= 0.0 {
            let y1 = (y1 as usize).min(self.h - 1);
            for y in y0..=y1 {
                let cy = self.origin.1 + (y as f64 + 0.5) * self.cell;
                xs.clear();
                for (a, b) in edges(ring) {
                    let (a, b) = (a + offset, b + offset);
                    if (a.1 <= cy) != (b.1 <= cy) {
                        xs.push(a.0 + (cy - a.1) * (b.0 - a.0) / (b.1 - a.1));
                    }
                }
                xs.sort_unstable_by(f64::total_cmp);
                for pair in xs.chunks_exact(2) {
                    let xa = ((pair[0] - self.origin.0) / self.cell - 0.5)
                        .ceil()
                        .max(0.0);
                    let xb = ((pair[1] - self.origin.0) / self.cell - 0.5)
                        .floor()
                        .min(self.w as f64 - 1.0);
                    if xb >= xa {
                        for x in xa as usize..=xb as usize {
                            self.occupied[y * self.w + x] = true;
                        }
                    }
                }
            }
        }
        for &p in ring {
            self.mark(p + offset);
        }
    }

    /// Exact squared Euclidean distance transform (Felzenszwalb–Huttenlocher) on a
    /// grid padded with an occupied border, converted to a conservative clearance.
    pub(super) fn compute(&mut self) {
        let (pw, ph) = (self.w + 2, self.h + 2);
        let s = &mut self.scratch;
        s.grid.clear();
        s.grid.resize(pw * ph, 0.0);
        for y in 0..self.h {
            for x in 0..self.w {
                if !self.occupied[y * self.w + x] {
                    s.grid[(y + 1) * pw + x + 1] = BIG;
                }
            }
        }
        let n = pw.max(ph);
        s.f.resize(n, 0.0);
        s.d.resize(n, 0.0);
        s.v.resize(n, 0);
        s.z.resize(n + 1, 0.0);
        for x in 0..pw {
            for y in 0..ph {
                s.f[y] = s.grid[y * pw + x];
            }
            edt_1d(&s.f[..ph], &mut s.d[..ph], &mut s.v, &mut s.z);
            for y in 0..ph {
                s.grid[y * pw + x] = s.d[y];
            }
        }
        for y in 0..ph {
            s.f[..pw].copy_from_slice(&s.grid[y * pw..(y + 1) * pw]);
            edt_1d(&s.f[..pw], &mut s.d[..pw], &mut s.v, &mut s.z);
            s.grid[y * pw..(y + 1) * pw].copy_from_slice(&s.d[..pw]);
        }
        let slack = self.cell * std::f64::consts::FRAC_1_SQRT_2;
        for y in 0..self.h {
            for x in 0..self.w {
                let d2 = s.grid[(y + 1) * pw + x + 1];
                self.dist[y * self.w + x] = (d2.sqrt() * self.cell - slack).max(0.0);
            }
        }
    }
}

fn edt_1d(f: &[f64], d: &mut [f64], v: &mut [usize], z: &mut [f64]) {
    let n = f.len();
    let mut k = 0usize;
    v[0] = 0;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    for q in 1..n {
        let fq = f[q] + (q * q) as f64;
        loop {
            let p = v[k];
            let s = (fq - (f[p] + (p * p) as f64)) / (2.0 * (q as f64 - p as f64));
            if s <= z[k] {
                k -= 1;
                continue;
            }
            k += 1;
            v[k] = q;
            z[k] = s;
            z[k + 1] = f64::INFINITY;
            break;
        }
    }
    k = 0;
    for (q, out) in d.iter_mut().enumerate().take(n) {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        let p = v[k];
        *out = (q as f64 - p as f64).powi(2) + f[p];
    }
}

impl Engine<'_> {
    /// Tries missing items at raster pockets whose clearance can hold their largest pole.
    pub(super) fn pocket_fill(&mut self, best: &mut Snapshot, publish: &mut impl FnMut(Candidate)) {
        let case = self.case;
        let mut map = FreeMap::new(case.usable, case.gap * 0.5);
        for (_, p) in self.layout.present() {
            map.paint(&p.shape.points, p.pose.translation());
        }
        map.compute();
        let order = self.stock_order(0, true);
        let trials = self.cfg.policy.pocket_trials.max(1);
        let mut cells = Vec::new();
        let mut inserted = 0;
        for i in order.into_iter().take(96) {
            if !self.budget.can_search() {
                break;
            }
            let index = Arc::clone(self.index(i));
            let Some(pole) = index.poles().first().copied() else {
                continue;
            };
            let need = pole.radius + case.gap + case.guard * 4.0;
            cells.clear();
            cells.extend((0..map.dist.len()).filter(|&k| map.dist[k] >= need));
            if cells.is_empty() {
                continue;
            }
            // Half the trials go to the tightest qualifying pockets, half at random.
            let tight = (trials / 2).min(cells.len());
            if cells.len() > tight && tight > 0 {
                cells.select_nth_unstable_by(tight, |&a, &b| map.dist[a].total_cmp(&map.dist[b]));
            }
            let mut picks: Vec<usize> = cells[..tight].to_vec();
            while picks.len() < trials {
                picks.push(cells[self.rng.gen_range(0..cells.len())]);
            }
            'pick: for k in picks {
                let target = map.center(k);
                for _ in 0..3 {
                    let pose = self.sample_pose(i, Pose::default(), false);
                    let c = pose.rotation().apply(pole.center);
                    let shape = self.oriented(i, pose);
                    let p = Placed::new(pose.with_translation(target - c), shape);
                    self.stats.pocket_trials += 1;
                    if self.fits(&p) {
                        map.paint(&p.shape.points, p.pose.translation());
                        map.compute();
                        self.layout.insert(case, i, p);
                        inserted += 1;
                        self.stats.pocket_insertions += 1;
                        break 'pick;
                    }
                }
            }
        }
        if inserted > 0 {
            if let Some(late) = self.late.as_mut() {
                // Insertions only add area; keep the late-acceptance history current.
                let area = self.layout.area;
                late.accept(area, area);
            }
            self.emit(best, "pockets", publish);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FreeMap;
    use crate::geometry::{Aabb, Point};

    #[test]
    fn distance_map_matches_brute_force_on_a_small_grid() {
        let usable = Aabb::new(Point(0.0, 0.0), Point(40.0, 30.0));
        let mut map = FreeMap::new(usable, 1.0);
        let square = [
            Point(10.0, 10.0),
            Point(14.0, 10.0),
            Point(14.0, 14.0),
            Point(10.0, 14.0),
        ];
        map.paint(&square, Point(0.0, 0.0));
        map.compute();
        let occupied: Vec<Point> = (0..map.dist.len())
            .filter(|&k| map.occupied[k])
            .map(|k| map.center(k))
            .collect();
        assert!(!occupied.is_empty());
        let usable = Aabb::new(Point(0.0, 0.0), Point(40.0, 30.0));
        for k in (0..map.dist.len()).step_by(7) {
            let c = map.center(k);
            let to_items = occupied
                .iter()
                .map(|o| (c - *o).norm2().sqrt())
                .fold(f64::INFINITY, f64::min);
            let to_border = [
                c.0 - usable.min.0 + map.cell * 0.5,
                usable.max.0 - c.0 + map.cell * 0.5,
                c.1 - usable.min.1 + map.cell * 0.5,
                usable.max.1 - c.1 + map.cell * 0.5,
            ]
            .into_iter()
            .fold(f64::INFINITY, f64::min);
            let exact = to_items.min(to_border);
            let expected = (exact - map.cell * std::f64::consts::FRAC_1_SQRT_2).max(0.0);
            let got = if map.occupied[k] { 0.0 } else { map.dist[k] };
            assert!(
                (got - expected).abs() < 1e-6 || map.occupied[k],
                "cell {k}: {got} vs {expected}"
            );
        }
    }
}
