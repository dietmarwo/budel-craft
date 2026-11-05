use super::{
    Aabb, Location, Point, Pose, edges, locate, orientation, point_segment2, ray_edge,
    segment_within,
};
use crate::model::{Case, Geometry};
use serde::Serialize;
use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};
use std::mem::size_of;
use std::sync::{Arc, OnceLock};

#[derive(Clone, Copy)]
enum Node {
    Leaf { start: usize, end: usize },
    Fork { left: usize, right: usize },
}

struct Topology {
    order: Vec<usize>,
    nodes: Vec<Node>,
}

impl Topology {
    fn new(ring: &[Point]) -> Self {
        fn split(
            centers: &[Point],
            order: &mut [usize],
            start: usize,
            nodes: &mut Vec<Node>,
        ) -> usize {
            let id = nodes.len();
            nodes.push(Node::Leaf {
                start,
                end: start + order.len(),
            });
            if order.len() > 6 {
                let bounds = order
                    .iter()
                    .fold(Aabb::new(centers[order[0]], centers[order[0]]), |bb, &i| {
                        bb.union(Aabb::new(centers[i], centers[i]))
                    });
                let size = bounds.size();
                let middle = order.len() / 2;
                order.select_nth_unstable_by(middle, |&a, &b| {
                    let coord = |i: usize| {
                        if size.0 >= size.1 {
                            centers[i].0
                        } else {
                            centers[i].1
                        }
                    };
                    coord(a).total_cmp(&coord(b)).then(a.cmp(&b))
                });
                let (a, b) = order.split_at_mut(middle);
                let left = split(centers, a, start, nodes);
                let right = split(centers, b, start + middle, nodes);
                nodes[id] = Node::Fork { left, right };
            }
            id
        }
        let centers: Vec<_> = edges(ring).map(|(a, b)| (a + b).scale(0.5)).collect();
        let mut order: Vec<_> = (0..ring.len()).collect();
        let mut nodes = Vec::new();
        split(&centers, &mut order, 0, &mut nodes);
        Self { order, nodes }
    }
}

fn convex_hull(ring: &[Point]) -> Vec<Point> {
    let mut points = ring.to_vec();
    points.sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    fn chain(points: impl Iterator<Item = Point>) -> Vec<Point> {
        let mut result: Vec<Point> = Vec::new();
        for p in points {
            while result.len() >= 2
                && orientation(result[result.len() - 2], result[result.len() - 1], p)
                    != Ordering::Greater
            {
                result.pop();
            }
            result.push(p);
        }
        result.pop();
        result
    }
    let mut hull = chain(points.iter().copied());
    hull.extend(chain(points.into_iter().rev()));
    hull
}

#[derive(Clone, Copy, Debug)]
pub struct Disc {
    pub center: Point,
    pub radius: f64,
}

pub struct ShapeIndex {
    pub geometry: Arc<Geometry>,
    pub hull: Vec<Point>,
    topology: Topology,
    discs: Vec<Disc>,
    guard: f64,
    poles: OnceLock<Vec<Disc>>,
}

impl ShapeIndex {
    fn new(geometry: Arc<Geometry>, guard: f64) -> Self {
        let hull = convex_hull(&geometry.ring);
        let topology = Topology::new(&geometry.ring);
        let mut discs = Vec::new();
        let Point(w, h) = geometry.bounds.size();
        let average = geometry
            .ring
            .iter()
            .copied()
            .fold(Point::default(), |sum, p| sum + p)
            .scale(1.0 / geometry.ring.len() as f64);
        let center = geometry.bounds.center();
        for p in [
            Point::default(),
            average,
            center,
            center + Point(w * 0.2, 0.),
            center - Point(w * 0.2, 0.),
            center + Point(0., h * 0.2),
            center - Point(0., h * 0.2),
        ] {
            if locate(p, &geometry.ring) == Location::Inside {
                let radius = edges(&geometry.ring)
                    .map(|(a, b)| point_segment2(p, a, b))
                    .fold(f64::INFINITY, f64::min)
                    .sqrt()
                    - guard * 4.0;
                if radius > 0.0 {
                    discs.push(Disc { center: p, radius });
                }
            }
        }
        discs.sort_unstable_by(|a, b| b.radius.total_cmp(&a.radius));
        if let Some(first) = discs.first().copied() {
            let mut other = discs
                .iter()
                .copied()
                .skip(1)
                .find(|d| (d.center - first.center).norm2() > first.radius.powi(2) * 0.49);
            discs.clear();
            discs.push(first);
            discs.extend(other.take());
        }
        Self {
            geometry,
            hull,
            topology,
            discs,
            guard,
            poles: OnceLock::new(),
        }
    }

    /// Interior discs in local coordinates, largest first. They are computed on first
    /// use because only the overlap-tolerant separator and pocket search need them.
    /// Each disc lies strictly inside the body (radius reduced by four guards), so the
    /// discs give a lower bound on the body and a penetration proxy for overlapping pairs.
    pub fn poles(&self) -> &[Disc] {
        self.poles
            .get_or_init(|| interior_poles(&self.geometry.ring, self.geometry.bounds, self.guard))
    }

    pub fn bounds(&self, rot: f64, mirror: bool) -> Aabb {
        let rotation = super::Rotation::new(rot, mirror);
        let first = rotation.apply(self.hull[0]);
        self.hull
            .iter()
            .skip(1)
            .fold(Aabb::new(first, first), |bb, &p| {
                let p = rotation.apply(p);
                bb.union(Aabb::new(p, p))
            })
    }
}

/// Greedy pole-of-inaccessibility cover on a bounded sample grid.
fn interior_poles(ring: &[Point], bounds: Aabb, guard: f64) -> Vec<Disc> {
    const CELLS: f64 = 22.0;
    const MAX_POLES: usize = 12;
    let Point(w, h) = bounds.size();
    let step = w.max(h) / CELLS;
    if step.is_nan() || step <= 0.0 {
        return Vec::new();
    }
    let mut samples = Vec::new();
    let mut y = bounds.min.1 + step * 0.5;
    while y < bounds.max.1 {
        let mut x = bounds.min.0 + step * 0.5;
        while x < bounds.max.0 {
            let p = Point(x, y);
            if locate(p, ring) == Location::Inside {
                let d = edges(ring)
                    .map(|(a, b)| point_segment2(p, a, b))
                    .fold(f64::INFINITY, f64::min)
                    .sqrt()
                    - guard * 4.0;
                if d > 0.0 {
                    samples.push((p, d));
                }
            }
            x += step;
        }
        y += step;
    }
    let mut poles: Vec<Disc> = Vec::new();
    while poles.len() < MAX_POLES {
        let best = samples
            .iter()
            .filter(|(p, _)| {
                poles
                    .iter()
                    .all(|d| (*p - d.center).norm2() > d.radius * d.radius)
            })
            .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.0.total_cmp(&a.0.0)));
        let Some(&(center, radius)) = best else {
            break;
        };
        if poles
            .first()
            .is_some_and(|first| radius < first.radius * 0.12 || radius < step * 0.35)
        {
            break;
        }
        poles.push(Disc { center, radius });
    }
    poles
}

pub fn prepare(
    case: &Case,
    check: impl Fn() -> crate::Result<()>,
) -> crate::Result<Vec<Arc<ShapeIndex>>> {
    let mut indices = Vec::new();
    for item in &case.items {
        if item.geometry.key == indices.len() {
            check()?;
            indices.push(Arc::new(ShapeIndex::new(
                Arc::clone(&item.geometry),
                case.guard,
            )));
        }
    }
    Ok(indices)
}

pub struct Oriented {
    source: Arc<ShapeIndex>,
    pub points: Vec<Point>,
    pub bounds: Aabb,
    edge_bounds: Vec<Aabb>,
    node_bounds: Vec<Aabb>,
    discs: Vec<Disc>,
    rotation: super::Rotation,
    poles: OnceLock<Vec<Disc>>,
}

impl Oriented {
    pub fn new(source: Arc<ShapeIndex>, rot: f64, mirror: bool) -> Self {
        let rotation = super::Rotation::new(rot, mirror);
        let points: Vec<_> = source
            .geometry
            .ring
            .iter()
            .map(|&p| rotation.apply(p))
            .collect();
        let bounds = Aabb::of(&points);
        let edge_bounds: Vec<_> = edges(&points).map(|(a, b)| Aabb::new(a, b)).collect();
        let mut node_bounds = vec![bounds; source.topology.nodes.len()];
        for (i, node) in source.topology.nodes.iter().enumerate().rev() {
            node_bounds[i] = match *node {
                Node::Leaf { start, end } => {
                    let order = &source.topology.order[start..end];
                    order[1..]
                        .iter()
                        .fold(edge_bounds[order[0]], |bb, &j| bb.union(edge_bounds[j]))
                }
                Node::Fork { left, right } => node_bounds[left].union(node_bounds[right]),
            };
        }
        let discs = source
            .discs
            .iter()
            .map(|d| Disc {
                center: rotation.apply(d.center),
                radius: d.radius,
            })
            .collect();
        Self {
            source,
            points,
            bounds,
            edge_bounds,
            node_bounds,
            discs,
            rotation,
            poles: OnceLock::new(),
        }
    }

    /// Interior poles in this orientation's local frame, largest first.
    pub fn poles(&self) -> &[Disc] {
        self.poles.get_or_init(|| {
            self.source
                .poles()
                .iter()
                .map(|d| Disc {
                    center: self.rotation.apply(d.center),
                    radius: d.radius,
                })
                .collect()
        })
    }

    /// Radius of the largest interior pole, or a small fallback for slivers.
    pub fn max_radius(&self) -> f64 {
        self.poles().first().map_or_else(
            || {
                let Point(w, h) = self.bounds.size();
                w.min(h) * 0.1
            },
            |d| d.radius,
        )
    }

    fn bytes(&self) -> usize {
        size_of::<Self>()
            + self.points.capacity() * size_of::<Point>()
            + (self.edge_bounds.capacity() + self.node_bounds.capacity()) * size_of::<Aabb>()
            + self.discs.capacity() * size_of::<Disc>()
            + self.source.poles.get().map_or(0, Vec::len) * size_of::<Disc>()
            + 192
    }

    fn segment(&self, i: usize) -> (Point, Point) {
        (self.points[i], self.points[(i + 1) % self.points.len()])
    }

    fn ray(&self, p: Point, node: usize, stats: &mut KernelStats) -> u8 {
        stats.ray_nodes += 1;
        let bb = self.node_bounds[node];
        if p.1 < bb.min.1 || p.1 > bb.max.1 || p.0 > bb.max.0 {
            return 0;
        }
        match self.source.topology.nodes[node] {
            Node::Leaf { start, end } => {
                let mut parity = 0;
                for &i in &self.source.topology.order[start..end] {
                    let (a, b) = self.segment(i);
                    let hit = ray_edge(p, a, b);
                    if hit == 2 {
                        return 2;
                    }
                    parity ^= hit;
                }
                parity
            }
            Node::Fork { left, right } => {
                let a = self.ray(p, left, stats);
                if a == 2 {
                    return 2;
                }
                let b = self.ray(p, right, stats);
                if b == 2 { 2 } else { a ^ b }
            }
        }
    }

    fn covers(&self, p: Point, tree: bool, stats: &mut KernelStats) -> bool {
        self.bounds.contains_point(p)
            && if tree {
                self.ray(p, 0, stats) != 0
            } else {
                locate(p, &self.points) != Location::Outside
            }
    }
}

type Key = (usize, u64, bool);

pub struct PoseCache {
    entries: HashMap<Key, Arc<Oriented>>,
    order: VecDeque<Key>,
    max_bytes: usize,
    pub bytes: usize,
}

impl PoseCache {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            max_bytes,
            bytes: 0,
        }
    }

    pub fn get(
        &mut self,
        index: &Arc<ShapeIndex>,
        rot: f64,
        mirror: bool,
        stats: &mut KernelStats,
    ) -> Arc<Oriented> {
        let rot = Pose::canonical_angle(rot);
        let key = (index.geometry.key, rot.to_bits(), mirror);
        if let Some(shape) = self.entries.get(&key) {
            stats.cache_hits += 1;
            return Arc::clone(shape);
        }
        stats.orientations += 1;
        let oriented = Arc::new(Oriented::new(Arc::clone(index), rot, mirror));
        let bytes = oriented.bytes();
        if bytes <= self.max_bytes {
            while self.bytes + bytes > self.max_bytes {
                if let Some(key) = self.order.pop_front() {
                    self.bytes -= self
                        .entries
                        .remove(&key)
                        .expect("cache order matches entries")
                        .bytes();
                }
            }
            self.bytes += bytes;
            stats.cache_peak_bytes = stats.cache_peak_bytes.max(self.bytes);
            self.order.push_back(key);
            self.entries.insert(key, Arc::clone(&oriented));
        }
        oriented
    }
}

#[derive(Clone, Copy, Default, Debug, Serialize)]
pub struct KernelStats {
    pub candidates: u64,
    pub pairs: u64,
    pub nodes: u64,
    pub ray_nodes: u64,
    pub edges: u64,
    pub disc_rejections: u64,
    pub orientations: u64,
    pub cache_hits: u64,
    pub cache_peak_bytes: usize,
}

impl KernelStats {
    pub fn add(&mut self, other: Self) {
        self.candidates += other.candidates;
        self.pairs += other.pairs;
        self.nodes += other.nodes;
        self.ray_nodes += other.ray_nodes;
        self.edges += other.edges;
        self.disc_rejections += other.disc_rejections;
        self.orientations += other.orientations;
        self.cache_hits += other.cache_hits;
        self.cache_peak_bytes = self.cache_peak_bytes.max(other.cache_peak_bytes);
    }
}

struct Pair<'a> {
    a: &'a Oriented,
    b: &'a Oriented,
    offset: Point,
    gap2: f64,
}

impl Pair<'_> {
    fn leaf(&self, a: &[usize], b: &[usize], stats: &mut KernelStats) -> bool {
        for &i in a {
            let bb = self.a.edge_bounds[i];
            let (p, q) = self.a.segment(i);
            for &j in b {
                if bb.gap2(self.b.edge_bounds[j].shift(self.offset)) <= self.gap2 {
                    stats.edges += 1;
                    let (r, s) = self.b.segment(j);
                    if segment_within(p, q, r + self.offset, s + self.offset, self.gap2) {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn visit(&self, i: usize, j: usize, stats: &mut KernelStats) -> bool {
        stats.nodes += 1;
        let aa = self.a.node_bounds[i];
        let bb = self.b.node_bounds[j].shift(self.offset);
        if aa.gap2(bb) > self.gap2 {
            return false;
        }
        let a_node = self.a.source.topology.nodes[i];
        let b_node = self.b.source.topology.nodes[j];
        match (a_node, b_node) {
            (Node::Leaf { start: a, end: ae }, Node::Leaf { start: b, end: be }) => self.leaf(
                &self.a.source.topology.order[a..ae],
                &self.b.source.topology.order[b..be],
                stats,
            ),
            (Node::Fork { left, right }, Node::Leaf { .. }) => {
                self.visit(left, j, stats) || self.visit(right, j, stats)
            }
            (Node::Leaf { .. }, Node::Fork { left, right }) => {
                self.visit(i, left, stats) || self.visit(i, right, stats)
            }
            (Node::Fork { left: l, right: r }, Node::Fork { left, right }) => {
                if aa.area() >= bb.area() {
                    self.visit(l, j, stats) || self.visit(r, j, stats)
                } else {
                    self.visit(i, left, stats) || self.visit(i, right, stats)
                }
            }
        }
    }
}

pub fn conflict(
    a: &Oriented,
    b: &Oriented,
    offset: Point,
    gap: f64,
    tree: bool,
    stats: &mut KernelStats,
) -> bool {
    let gap2 = gap * gap;
    if a.bounds.gap2(b.bounds.shift(offset)) > gap2 {
        return false;
    }
    stats.pairs += 1;
    if tree {
        for x in &a.discs {
            for y in &b.discs {
                if (x.center - y.center - offset).norm2() < (x.radius + y.radius + gap).powi(2) {
                    stats.disc_rejections += 1;
                    return true;
                }
            }
        }
    }
    let b_first = b.points[0] + offset;
    let a_first = a.points[0] - offset;
    if a.covers(b_first, tree, stats) || b.covers(a_first, tree, stats) {
        return true;
    }
    let pair = Pair { a, b, offset, gap2 };
    if tree {
        pair.visit(0, 0, stats)
    } else {
        pair.leaf(&a.source.topology.order, &b.source.topology.order, stats)
    }
}

/// Overlap proxy for a pair already known to conflict: summed pole-pair penetration
/// depth plus a base term, so a conflicting pair never costs zero. Search guidance only;
/// feasibility is always decided by [`conflict`].
pub fn penetration(a: &Oriented, b: &Oriented, offset: Point, gap: f64) -> f64 {
    let mut sum = 0.0;
    for x in a.poles() {
        for y in b.poles() {
            let d = (x.center - y.center - offset).norm2().sqrt();
            let depth = x.radius + y.radius + gap - d;
            if depth > 0.0 {
                sum += depth;
            }
        }
    }
    sum + 0.25 * a.max_radius().min(b.max_radius()) + gap
}
