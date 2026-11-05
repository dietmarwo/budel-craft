use budel_craft::geometry::{
    Point, Pose,
    index::{KernelStats, Oriented, PoseCache, conflict, prepare},
    reference_conflict,
};
use budel_craft::layout::{Layout, Placed};
use budel_craft::model::Case;
use rand::{Rng, SeedableRng, rngs::StdRng};
use std::sync::Arc;

#[test]
fn tree_and_reference_agree_on_rotated_case_pairs() {
    let case = Case::parse(include_bytes!("../cases/small.json")).unwrap();
    let indices = prepare(&case, || Ok(())).unwrap();
    let mut rng = StdRng::seed_from_u64(1847);
    let mut stats = KernelStats::default();
    let mut cache = PoseCache::new(200_000);
    for _ in 0..700 {
        let i = rng.gen_range(0..indices.len());
        let j = rng.gen_range(0..indices.len());
        let p = Pose {
            x: 500.,
            y: 530.,
            rot: rng.gen_range(0..16) as f64 * 22.5,
            mirror: rng.gen_bool(0.5),
        };
        let q = Pose {
            x: 500. + rng.gen_range(-350.0..350.0),
            y: 530. + rng.gen_range(-350.0..350.0),
            rot: rng.gen_range(0..16) as f64 * 22.5,
            mirror: rng.gen_bool(0.5),
        };
        let a = cache.get(&indices[i], p.rot, p.mirror, &mut stats);
        let b = cache.get(&indices[j], q.rot, q.mirror, &mut stats);
        let expected = reference_conflict(
            &p.apply(&indices[i].geometry.ring),
            &q.apply(&indices[j].geometry.ring),
            case.gap,
        );
        for tree in [true, false] {
            assert_eq!(
                conflict(
                    &a,
                    &b,
                    q.translation() - p.translation(),
                    case.gap,
                    tree,
                    &mut stats
                ),
                expected,
                "seed 1847, bodies {i},{j}, poses {p:?} {q:?}"
            );
        }
        assert!(cache.bytes <= 200_000);
    }
}

#[test]
fn grid_move_remove_and_rollback() {
    let case = Case::parse(include_bytes!("../cases/tiny.json")).unwrap();
    let indices = prepare(&case, || Ok(())).unwrap();
    let shape = |i: usize| Arc::new(Oriented::new(Arc::clone(&indices[i]), 0., false));
    let p = Placed::new(
        Pose {
            x: 25.,
            y: 25.,
            ..Pose::default()
        },
        shape(0),
    );
    let q = Placed::new(
        Pose {
            x: 40.5,
            y: 25.,
            ..Pose::default()
        },
        shape(1),
    );
    let mut layout = Layout::new(&case);
    let mut stats = KernelStats::default();
    layout.insert(&case, 0, p);
    assert!(!layout.fits(&case, &q, true, &mut stats));
    let removed = layout.remove(&case, 0).unwrap();
    assert!(layout.fits(&case, &q, true, &mut stats));
    layout.insert(&case, 0, removed);
    assert!(!layout.fits(&case, &q, true, &mut stats));
    assert!(layout.nearby(q.bounds.expand(3.)).contains(&0));
    assert_eq!(layout.count, 1);
    assert_eq!(layout.area, 400.);
}

#[test]
fn reflected_pose_changes_occupied_geometry() {
    let case = Case::parse(
        br#"{
      "case":"mirror","plane":{"w":60,"h":60},"rules":{"min_channel":0.5},
      "shapes":[{"id":"L","ring":[[0,0],[20,0],[20,3],[3,3],[3,20],[0,20]]},
                {"id":"block","ring":[[0,0],[7,0],[7,7],[0,7]]}]
    }"#,
    )
    .unwrap();
    let indices = prepare(&case, || Ok(())).unwrap();
    let mut layout = Layout::new(&case);
    layout.insert(
        &case,
        1,
        Placed::new(
            Pose {
                x: 26.,
                y: 20.,
                ..Pose::default()
            },
            Arc::new(Oriented::new(Arc::clone(&indices[1]), 0., false)),
        ),
    );
    let mut stats = KernelStats::default();
    for mirror in [false, true] {
        let pose = Pose {
            x: 21.,
            y: 20.,
            rot: 0.,
            mirror,
        };
        let p = Placed::new(
            pose,
            Arc::new(Oriented::new(Arc::clone(&indices[0]), pose.rot, mirror)),
        );
        let expected = !reference_conflict(
            &pose.apply(&case.items[0].geometry.ring),
            &Pose {
                x: 26.,
                y: 20.,
                ..Pose::default()
            }
            .apply(&case.items[1].geometry.ring),
            case.gap,
        );
        assert_eq!(layout.fits(&case, &p, true, &mut stats), expected);
        assert_eq!(expected, mirror);
    }
    let pos = |mirror| {
        Pose {
            x: 20.,
            y: 20.,
            rot: 0.,
            mirror,
        }
        .apply(&case.items[0].geometry.ring)
    };
    assert_ne!(pos(false), pos(true));
    assert_eq!(pos(false)[1], Point(40., 20.));
    assert_eq!(pos(true)[1], Point(0., 20.));
}
