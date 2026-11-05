use crate::Result;
use crate::geometry::{
    Aabb, Location, Point, Pose, edges, intersects, locate, reference_conflict, validate_ring,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Plane {
    pub w: f64,
    pub h: f64,
    #[serde(default)]
    pub margin: f64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Poses {
    #[default]
    Free,
    Eight,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Body {
    #[default]
    Rest,
    Envelope,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Holes {
    #[default]
    Paper,
    Ink,
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize)]
pub struct Rules {
    #[serde(default)]
    pub margin: Option<f64>,
    #[serde(default)]
    pub min_channel: f64,
    #[serde(default)]
    pub tolerance: f64,
    #[serde(default)]
    pub poses: Poses,
    #[serde(default = "yes")]
    pub mirror_allowed: bool,
    #[serde(default)]
    pub must_seat_all: bool,
    #[serde(default)]
    pub body: Body,
    #[serde(default)]
    pub holes_are: Holes,
    #[serde(default)]
    pub runtime: RuntimeRules,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct RuntimeRules {
    #[serde(default)]
    pub budgets: BTreeMap<String, f64>,
}

#[derive(Deserialize)]
struct RawCase {
    case: String,
    plane: Plane,
    rules: Rules,
    shapes: Vec<RawShape>,
}

#[derive(Deserialize)]
struct RawShape {
    id: String,
    ring: Vec<Point>,
    #[serde(default)]
    holes: Vec<Vec<Point>>,
    #[serde(default)]
    envelope: Option<Vec<Point>>,
    #[serde(default)]
    size_class: Option<f64>,
}

#[derive(Clone)]
pub struct Geometry {
    pub key: usize,
    pub ring: Arc<[Point]>,
    pub bounds: Aabb,
    pub area: f64,
}

#[derive(Clone)]
pub struct Item {
    pub id: String,
    pub artwork: Arc<[Point]>,
    pub holes: Vec<Vec<Point>>,
    pub geometry: Arc<Geometry>,
    pub score_area: f64,
    pub size_class: Option<f64>,
}

pub struct Case {
    pub name: String,
    pub plane: Plane,
    pub usable: Aabb,
    pub rules: Rules,
    pub items: Vec<Item>,
    pub ids: HashMap<String, usize>,
    pub sha256: String,
    pub gap: f64,
    pub guard: f64,
    pub available_area: f64,
}

impl Case {
    pub fn load(path: &Path) -> Result<Self> {
        Self::load_with(path, || Ok(()))
    }

    pub fn load_with(path: &Path, check: impl Fn() -> Result<()>) -> Result<Self> {
        let bytes = read_limited(path, 64 * 1024 * 1024)?;
        check()?;
        Self::parse_with(&bytes, check)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self> {
        Self::parse_with(bytes, || Ok(()))
    }

    fn parse_with(bytes: &[u8], check: impl Fn() -> Result<()>) -> Result<Self> {
        let raw: RawCase = serde_json::from_slice(bytes)?;
        let margin = raw.rules.margin.unwrap_or(raw.plane.margin);
        let Plane { w, h, .. } = raw.plane;
        if raw.case.trim().is_empty()
            || raw.case.len() > 512
            || ![w, h, margin, raw.rules.min_channel, raw.rules.tolerance]
                .iter()
                .all(|x| x.is_finite())
            || w > 1e9
            || h > 1e9
            || margin < 0.0
            || w <= 2.0 * margin
            || h <= 2.0 * margin
            || raw.rules.min_channel < 0.0
            || raw.rules.min_channel > 1e9
            || raw.rules.tolerance < 0.0
        {
            return Err("invalid case name, dimensions, margin or clearance".into());
        }
        if raw
            .rules
            .runtime
            .budgets
            .values()
            .any(|b| !b.is_finite() || *b <= 0.0)
        {
            return Err("runtime budgets must be positive finite seconds".into());
        }
        if raw.shapes.len() > 20_000 {
            return Err("case exceeds 20,000 items".into());
        }
        let vertex_count: usize = raw
            .shapes
            .iter()
            .map(|s| {
                s.ring.len()
                    + s.holes.iter().map(Vec::len).sum::<usize>()
                    + s.envelope.as_ref().map_or(0, Vec::len)
            })
            .sum();
        if vertex_count > 2_000_000 {
            return Err("case exceeds two million vertices".into());
        }
        let mut items = Vec::with_capacity(raw.shapes.len());
        let mut ids = HashMap::new();
        let mut bodies = HashMap::<Vec<[u64; 2]>, Arc<Geometry>>::new();
        let mut coordinate_scale = w.max(h);
        for sh in raw.shapes {
            check()?;
            if sh.id.is_empty()
                || sh.id.len() > 512
                || sh.id.chars().any(char::is_control)
                || ids.insert(sh.id.clone(), items.len()).is_some()
            {
                return Err(format!("invalid or duplicate shape ID {:?}", sh.id).into());
            }
            let ring: Arc<[Point]> = sh.ring.into();
            let body: Arc<[Point]> = match (raw.rules.body, sh.envelope) {
                (Body::Envelope, Some(envelope)) => {
                    validate_ring(&ring).map_err(|msg| format!("{} artwork: {msg}", sh.id))?;
                    envelope.into()
                }
                (_, _) => Arc::clone(&ring),
            };
            let key: Vec<_> = body
                .iter()
                .map(|p| [p.0.to_bits(), p.1.to_bits()])
                .collect();
            let geometry = if let Some(geometry) = bodies.get(&key) {
                Arc::clone(geometry)
            } else {
                let area = validate_ring(&body).map_err(|msg| format!("{} body: {msg}", sh.id))?;
                let bounds = Aabb::of(&body);
                coordinate_scale = coordinate_scale.max(
                    bounds
                        .min
                        .0
                        .abs()
                        .max(bounds.max.0.abs())
                        .max(bounds.min.1.abs())
                        .max(bounds.max.1.abs()),
                );
                let geometry = Arc::new(Geometry {
                    key: bodies.len(),
                    bounds,
                    area,
                    ring: body,
                });
                bodies.insert(key, Arc::clone(&geometry));
                geometry
            };
            let mut score_area = geometry.area;
            for (index, hole) in sh.holes.iter().enumerate() {
                let area =
                    validate_ring(hole).map_err(|msg| format!("{} hole {index}: {msg}", sh.id))?;
                if raw.rules.holes_are == Holes::Paper {
                    if locate(hole[0], &geometry.ring) != Location::Inside
                        || edges(hole).any(|(a, b)| {
                            edges(&geometry.ring).any(|(c, d)| intersects(a, b, c, d))
                        })
                        || sh.holes[..index]
                            .iter()
                            .any(|h| reference_conflict(h, hole, 0.0))
                    {
                        return Err(format!(
                            "{}: paper hole is not strictly internal and disjoint",
                            sh.id
                        )
                        .into());
                    }
                    score_area -= area;
                }
            }
            if score_area <= 0.0 || sh.size_class.is_some_and(|k| !k.is_finite()) {
                return Err(format!("{}: invalid scoring area or size class", sh.id).into());
            }
            items.push(Item {
                id: sh.id,
                artwork: ring,
                holes: sh.holes,
                geometry,
                score_area,
                size_class: sh.size_class,
            });
        }
        let guard = 1e-6_f64.max(64.0 * f64::EPSILON * coordinate_scale);
        let available_area = items.iter().map(|sh| sh.score_area).sum();
        Ok(Self {
            name: raw.case,
            plane: raw.plane,
            usable: Aabb::new(Point(margin, margin), Point(w - margin, h - margin)),
            gap: raw.rules.min_channel + guard,
            guard,
            rules: raw.rules,
            items,
            ids,
            sha256: format!("{:x}", Sha256::digest(bytes)),
            available_area,
        })
    }

    pub fn allowed(&self, pose: Pose) -> bool {
        pose.translation().finite()
            && pose.rot.is_finite()
            && (!pose.mirror || self.rules.mirror_allowed)
            && (self.rules.poses == Poses::Free || Pose::canonical_angle(pose.rot) % 45.0 == 0.0)
    }

    pub fn coverage(&self, area: f64) -> f64 {
        100.0 * area / (self.plane.w * self.plane.h)
    }
}

pub fn read_limited(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(format!("{} exceeds {limit} bytes", path.display()).into());
    }
    Ok(bytes)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Placement {
    pub id: String,
    #[serde(flatten)]
    pub pose: Pose,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Answer {
    pub case: String,
    pub placements: Vec<Placement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_s: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
}

impl Answer {
    pub fn new(case: &Case) -> Self {
        Self {
            case: case.name.clone(),
            placements: Vec::new(),
            runtime_s: None,
            budget: None,
            machine: None,
            author: Some("budel-craft".into()),
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        Ok(serde_json::from_slice(&read_limited(
            path,
            16 * 1024 * 1024,
        )?)?)
    }
}
