use budel_craft::geometry::{
    Location, Point, Pose, intersects, locate, orientation, reference_conflict, segment_distance2,
};
use budel_craft::model::{Answer, Case, Placement};
use budel_craft::validate::validate;
use rand::{Rng, SeedableRng, rngs::StdRng};
use serde_json::json;
use std::cmp::Ordering;

fn square(x: f64, y: f64, side: f64) -> [Point; 4] {
    [
        Point(x, y),
        Point(x + side, y),
        Point(x + side, y + side),
        Point(x, y + side),
    ]
}

#[test]
fn contact_containment_and_tolerance() {
    let body = square(0., 0., 10.);
    assert!(reference_conflict(&body, &body, 0.));
    assert!(reference_conflict(&body, &square(2., 2., 1.), 0.));
    assert!(reference_conflict(&body, &square(10., 10., 1.), 0.));
    assert!(reference_conflict(&body, &square(11.9, 0., 2.), 2.));
    assert!(!reference_conflict(&body, &square(12.1, 0., 2.), 2.));
    assert_eq!(locate(Point(0., 5.), &body), Location::Boundary);
    assert_eq!(locate(Point(1., 5.), &body), Location::Inside);
    assert_eq!(locate(Point(-1., 5.), &body), Location::Outside);
}

#[test]
fn pose_order_and_roundtrip() {
    let pose = Pose {
        x: 17.,
        y: 23.,
        rot: 90.,
        mirror: true,
    };
    let point = pose.apply(&[Point(3., 5.)])[0];
    assert!((point.0 - 12.).abs() < 1e-13 && (point.1 - 20.).abs() < 1e-13);
    let encoded = serde_json::to_vec(&pose).unwrap();
    assert_eq!(serde_json::from_slice::<Pose>(&encoded).unwrap(), pose);
}

#[test]
fn near_parallel_and_degenerate_contacts() {
    let tiny = f64::from_bits(1);
    assert_eq!(
        orientation(Point(0., 0.), Point(tiny, 0.), Point(0., tiny)),
        Ordering::Greater
    );
    let offset = 1e8;
    assert!(intersects(
        Point(offset, offset),
        Point(offset + 1., offset + 1.),
        Point(offset + 0.5, offset + 0.5),
        Point(offset + 2., offset + 2.)
    ));
    assert!(!intersects(
        Point(0., 0.),
        Point(0., 0.),
        Point(1., 1.),
        Point(2., 2.)
    ));
}

#[test]
fn mandatory_missing_and_finite_input() {
    let case = Case::parse(include_bytes!("../cases/tiny.json")).unwrap();
    let mut answer = Answer::new(&case);
    assert!(!validate(&case, &answer).strict_valid);
    for (id, x) in [("a", 20.), ("b", 45.)] {
        answer.placements.push(Placement {
            id: id.into(),
            pose: Pose {
                x,
                y: 20.,
                ..Pose::default()
            },
        });
    }
    let report = validate(&case, &answer);
    assert!(report.strict_valid && report.complete && report.coverage == 5.0);
    answer.placements[0].pose.x = f64::NAN;
    assert!(!validate(&case, &answer).strict_valid);
    let mut input: serde_json::Value =
        serde_json::from_slice(include_bytes!("../cases/tiny.json")).unwrap();
    input["rules"]["mirror_allowed"] = json!("false");
    assert!(Case::parse(&serde_json::to_vec(&input).unwrap()).is_err());
}

#[test]
fn paper_holes_change_area_without_admitting_stock() {
    let data = json!({
        "case":"holes", "plane":{"w":40,"h":40,"margin":1},
        "rules":{"body":"envelope","holes_are":"paper"},
        "shapes":[{"id":"body","ring":square(0.,0.,8.),"envelope":square(0.,0.,9.),
            "holes":[square(2.,2.,4.)]},{"id":"other","ring":square(0.,0.,1.)}]
    });
    let case = Case::parse(&serde_json::to_vec(&data).unwrap()).unwrap();
    assert_eq!(case.items[0].score_area, 65.0);
    let mut ans = Answer::new(&case);
    for (id, xy) in [("body", 10.), ("other", 13.)] {
        ans.placements.push(Placement {
            id: id.into(),
            pose: Pose {
                x: xy,
                y: xy,
                ..Pose::default()
            },
        });
    }
    assert!(!validate(&case, &ans).strict_valid);
}

#[test]
fn randomized_segment_distance_against_independent_rational_threshold() {
    fn delta(a: (i64, i64), b: (i64, i64)) -> (i64, i64) {
        (a.0 - b.0, a.1 - b.1)
    }
    fn dot(a: (i64, i64), b: (i64, i64)) -> i64 {
        a.0 * b.0 + a.1 * b.1
    }
    fn cross(a: (i64, i64), b: (i64, i64)) -> i64 {
        a.0 * b.1 - a.1 * b.0
    }
    fn near(p: (i64, i64), a: (i64, i64), b: (i64, i64), radius2: i64) -> bool {
        let u = delta(b, a);
        let v = delta(p, a);
        let len = dot(u, u);
        let t = dot(v, u);
        if t <= 0 {
            dot(v, v) <= radius2
        } else if t >= len {
            let w = delta(p, b);
            dot(w, w) <= radius2
        } else {
            cross(v, u).pow(2) <= len * radius2
        }
    }
    fn between(t: i64, denominator: i64) -> bool {
        if denominator > 0 {
            0 <= t && t <= denominator
        } else {
            denominator <= t && t <= 0
        }
    }
    let mut rng = StdRng::seed_from_u64(9215);
    for _ in 0..10_000 {
        let mut p = || (rng.gen_range(-200..=200), rng.gen_range(-200..=200));
        let [a, b, c, d] = [p(), p(), p(), p()];
        let r: i64 = rng.gen_range(1..100);
        let (u, v, w) = (delta(b, a), delta(d, c), delta(c, a));
        let det = cross(u, v);
        let crossing = det != 0 && between(cross(w, v), det) && between(cross(w, u), det);
        let expected = crossing
            || near(a, c, d, r * r)
            || near(b, c, d, r * r)
            || near(c, a, b, r * r)
            || near(d, a, b, r * r);
        let [a, b, c, d] = [a, b, c, d].map(|(x, y)| Point(x as f64, y as f64));
        assert_eq!(
            segment_distance2(a, b, c, d) <= (r * r) as f64 + 1e-11,
            expected
        );
    }
}
