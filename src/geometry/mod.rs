use num_bigint::{BigInt, Sign};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::ops::{Add, Sub};

pub mod index;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Point(pub f64, pub f64);

impl Add for Point {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0, self.1 + other.1)
    }
}

impl Sub for Point {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self(self.0 - other.0, self.1 - other.1)
    }
}

impl Point {
    pub fn scale(self, factor: f64) -> Self {
        Self(self.0 * factor, self.1 * factor)
    }

    pub fn dot(self, other: Self) -> f64 {
        self.0 * other.0 + self.1 * other.1
    }

    pub fn cross(self, other: Self) -> f64 {
        self.0 * other.1 - self.1 * other.0
    }

    pub fn norm2(self) -> f64 {
        self.dot(self)
    }

    pub fn finite(self) -> bool {
        self.0.is_finite() && self.1.is_finite()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Aabb {
    pub min: Point,
    pub max: Point,
}

impl Aabb {
    pub fn new(a: Point, b: Point) -> Self {
        Self {
            min: Point(a.0.min(b.0), a.1.min(b.1)),
            max: Point(a.0.max(b.0), a.1.max(b.1)),
        }
    }

    pub fn of(points: &[Point]) -> Self {
        points
            .iter()
            .fold(Self::new(points[0], points[0]), |bb, &p| {
                bb.union(Self::new(p, p))
            })
    }

    pub fn union(self, other: Self) -> Self {
        Self {
            min: Point(self.min.0.min(other.min.0), self.min.1.min(other.min.1)),
            max: Point(self.max.0.max(other.max.0), self.max.1.max(other.max.1)),
        }
    }

    pub fn shift(self, offset: Point) -> Self {
        Self {
            min: self.min + offset,
            max: self.max + offset,
        }
    }

    pub fn expand(self, radius: f64) -> Self {
        let offset = Point(radius, radius);
        Self {
            min: self.min - offset,
            max: self.max + offset,
        }
    }

    pub fn size(self) -> Point {
        self.max - self.min
    }

    pub fn center(self) -> Point {
        (self.min + self.max).scale(0.5)
    }

    pub fn area(self) -> f64 {
        let Point(w, h) = self.size();
        w * h
    }

    pub fn contains(self, other: Self) -> bool {
        self.min.0 <= other.min.0
            && self.min.1 <= other.min.1
            && self.max.0 >= other.max.0
            && self.max.1 >= other.max.1
    }

    pub fn contains_point(self, p: Point) -> bool {
        self.min.0 <= p.0 && p.0 <= self.max.0 && self.min.1 <= p.1 && p.1 <= self.max.1
    }

    pub fn gap2(self, other: Self) -> f64 {
        let dx = (other.min.0 - self.max.0)
            .max(self.min.0 - other.max.0)
            .max(0.0);
        let dy = (other.min.1 - self.max.1)
            .max(self.min.1 - other.max.1)
            .max(0.0);
        dx * dx + dy * dy
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Pose {
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default)]
    pub rot: f64,
    #[serde(default)]
    pub mirror: bool,
}

impl Pose {
    pub fn translation(self) -> Point {
        Point(self.x, self.y)
    }

    pub fn with_translation(mut self, p: Point) -> Self {
        self.x = p.0;
        self.y = p.1;
        self
    }

    pub fn canonical_angle(angle: f64) -> f64 {
        let angle = angle.rem_euclid(360.0);
        if angle == 360.0 || angle == 0.0 {
            0.0
        } else {
            angle
        }
    }

    pub fn rotation(self) -> Rotation {
        Rotation::new(self.rot, self.mirror)
    }

    pub fn apply(self, points: &[Point]) -> Vec<Point> {
        let rot = self.rotation();
        let offset = self.translation();
        points.iter().map(|&p| rot.apply(p) + offset).collect()
    }
}

#[derive(Clone, Copy)]
pub struct Rotation {
    sin: f64,
    cos: f64,
    mirror: bool,
}

impl Rotation {
    pub fn new(angle: f64, mirror: bool) -> Self {
        let (sin, cos) = Pose::canonical_angle(angle).to_radians().sin_cos();
        Self { sin, cos, mirror }
    }

    pub fn apply(self, p: Point) -> Point {
        let x = if self.mirror { -p.0 } else { p.0 };
        Point(x * self.cos - p.1 * self.sin, x * self.sin + p.1 * self.cos)
    }
}

/// Shoelace area around the first vertex, with compensated summation.
pub fn signed_area(points: &[Point]) -> f64 {
    let first = points[0];
    let mut sum = 0.0;
    let mut correction = 0.0;
    for edge in points[1..].windows(2) {
        let term = (edge[0] - first).cross(edge[1] - first) - correction;
        let next = sum + term;
        correction = (next - sum) - term;
        sum = next;
    }
    sum * 0.5
}

fn dyadic(x: f64) -> (i64, i32) {
    let bits = x.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & ((1_u64 << 52) - 1);
    let (mantissa, exponent) = if exponent == 0 {
        (fraction as i64, -1074)
    } else {
        ((fraction | (1_u64 << 52)) as i64, exponent - 1075)
    };
    (if bits >> 63 == 0 { mantissa } else { -mantissa }, exponent)
}

#[cold]
fn orientation_exact(a: Point, b: Point, c: Point) -> Ordering {
    let values = [a.0, a.1, b.0, b.1, c.0, c.1].map(dyadic);
    let base = values
        .iter()
        .filter(|v| v.0 != 0)
        .map(|v| v.1)
        .min()
        .unwrap_or(0);
    let [ax, ay, bx, by, cx, cy] = values.map(|(m, e)| {
        if m == 0 {
            BigInt::from(0)
        } else {
            BigInt::from(m) << (e - base) as usize
        }
    });
    match ((bx - &ax) * (cy - &ay) - (by - ay) * (cx - ax)).sign() {
        Sign::Minus => Ordering::Less,
        Sign::NoSign => Ordering::Equal,
        Sign::Plus => Ordering::Greater,
    }
}

/// Fast floating-point filter and exact dyadic fallback for the supplied f64 points.
pub fn orientation(a: Point, b: Point, c: Point) -> Ordering {
    let u = b - a;
    let v = c - a;
    let p = u.0 * v.1;
    let q = u.1 * v.0;
    let det = p - q;
    let bound = 16.0 * f64::EPSILON * (p.abs() + q.abs());
    if det.is_normal() && det.abs() > bound {
        return if det > 0.0 {
            Ordering::Greater
        } else {
            Ordering::Less
        };
    }
    if a == b || a == c || b == c || (a.0 == b.0 && b.0 == c.0) || (a.1 == b.1 && b.1 == c.1) {
        return Ordering::Equal;
    }
    orientation_exact(a, b, c)
}

pub fn intersects(a: Point, b: Point, c: Point, d: Point) -> bool {
    let ab = Aabb::new(a, b);
    let cd = Aabb::new(c, d);
    if ab.gap2(cd) > 0.0 {
        return false;
    }
    let o1 = orientation(a, b, c);
    let o2 = orientation(a, b, d);
    let o3 = orientation(c, d, a);
    let o4 = orientation(c, d, b);
    let zero = Ordering::Equal;
    (o1 == zero && ab.contains_point(c))
        || (o2 == zero && ab.contains_point(d))
        || (o3 == zero && cd.contains_point(a))
        || (o4 == zero && cd.contains_point(b))
        || (o1 != zero && o2 != zero && o3 != zero && o4 != zero && o1 != o2 && o3 != o4)
}

pub fn point_segment2(p: Point, a: Point, b: Point) -> f64 {
    let u = b - a;
    let length2 = u.norm2();
    let v = p - a;
    if length2 == 0.0 {
        return v.norm2();
    }
    let t = v.dot(u) / length2;
    if t <= 0.0 {
        v.norm2()
    } else if t >= 1.0 {
        (p - b).norm2()
    } else {
        let cross = v.cross(u);
        cross * cross / length2
    }
}

pub fn segment_distance2(a: Point, b: Point, c: Point, d: Point) -> f64 {
    if intersects(a, b, c, d) {
        return 0.0;
    }
    point_segment2(a, c, d)
        .min(point_segment2(b, c, d))
        .min(point_segment2(c, a, b))
        .min(point_segment2(d, a, b))
}

fn segment_within(a: Point, b: Point, c: Point, d: Point, gap2: f64) -> bool {
    intersects(a, b, c, d)
        || point_segment2(a, c, d) <= gap2
        || point_segment2(b, c, d) <= gap2
        || point_segment2(c, a, b) <= gap2
        || point_segment2(d, a, b) <= gap2
}

pub fn edges(points: &[Point]) -> impl Iterator<Item = (Point, Point)> + '_ {
    points
        .iter()
        .copied()
        .zip(points.iter().copied().cycle().skip(1))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Location {
    Outside,
    Boundary,
    Inside,
}

/// 0 = miss, 1 = ray crossing, 2 = boundary contact.
fn ray_edge(p: Point, a: Point, b: Point) -> u8 {
    let crosses_level = (a.1 > p.1) != (b.1 > p.1);
    let in_box = Aabb::new(a, b).contains_point(p);
    if crosses_level || in_box {
        let side = orientation(a, b, p);
        if in_box && side == Ordering::Equal {
            return 2;
        }
        if crosses_level
            && (if b.1 > a.1 {
                side == Ordering::Greater
            } else {
                side == Ordering::Less
            })
        {
            return 1;
        }
    }
    0
}

pub fn locate(p: Point, ring: &[Point]) -> Location {
    let mut inside = false;
    for (a, b) in edges(ring) {
        match ray_edge(p, a, b) {
            2 => return Location::Boundary,
            1 => inside = !inside,
            _ => {}
        }
    }
    if inside {
        Location::Inside
    } else {
        Location::Outside
    }
}

pub fn reference_conflict(a: &[Point], b: &[Point], gap: f64) -> bool {
    let gap2 = gap * gap;
    let ab = Aabb::of(a);
    let bb = Aabb::of(b);
    if ab.gap2(bb) > gap2 {
        return false;
    }
    if (ab.contains_point(b[0]) && locate(b[0], a) != Location::Outside)
        || (bb.contains_point(a[0]) && locate(a[0], b) != Location::Outside)
    {
        return true;
    }
    for (p, q) in edges(a) {
        let box_a = Aabb::new(p, q);
        if box_a.gap2(bb) > gap2 {
            continue;
        }
        for (r, s) in edges(b) {
            if box_a.gap2(Aabb::new(r, s)) <= gap2 && segment_distance2(p, q, r, s) <= gap2 {
                return true;
            }
        }
    }
    false
}

pub fn validate_ring(points: &[Point]) -> Result<f64, &'static str> {
    if !(3..=4096).contains(&points.len()) {
        return Err("ring must contain 3 to 4096 vertices");
    }
    if points
        .iter()
        .any(|p| !p.finite() || p.0.abs().max(p.1.abs()) > 1e9)
    {
        return Err("ring coordinate must be finite and within ±1e9");
    }
    let area = signed_area(points).abs();
    if !area.is_finite() || area == 0.0 {
        return Err("ring must have positive finite area");
    }
    let n = points.len();
    for i in 0..n {
        let a = points[i];
        let b = points[(i + 1) % n];
        let c = points[(i + 2) % n];
        if a == b {
            return Err("zero-length edge");
        }
        if (b - a).dot(c - b) < 0.0 && orientation(a, b, c) == Ordering::Equal {
            return Err("adjacent edges retrace each other");
        }
        let ab = Aabb::new(a, b);
        for j in (i + 2)..n {
            if i == 0 && j == n - 1 {
                continue;
            }
            let c = points[j];
            let d = points[(j + 1) % n];
            if ab.gap2(Aabb::new(c, d)) == 0.0 && intersects(a, b, c, d) {
                return Err("ring self-intersects or self-touches");
            }
        }
    }
    Ok(area)
}
