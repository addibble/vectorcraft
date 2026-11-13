//! Golden bits of [`LiveShape::to_path`]. A format v6 file leaves a live shape's path out when
//! its parameters rebuild it bit for bit, and reading the file rebuilds it with `to_path`. So
//! every bit `to_path` gives for a set of parameters is part of the file format: change one and
//! every saved deck with that shape redraws (slightly, or not at all where it shows, but its
//! path is no longer the one it was saved with).
//!
//! A failure here is a format decision, not a test to update. Either keep the builders' bits as
//! they were, or bump the format version and keep the old builders for files saved before it
//! (or stop leaving out the paths of the shapes concerned). Never record new bits to pass.

use super::*;
use vectorcraft_geom::kurbo::{Affine, Point};
use vectorcraft_geom::shapes::{CornerKind, star};
use vectorcraft_geom::{AnchorKind, PathData};

/// FNV-1a over every bit `path` holds: its subpaths, whether each is closed, and each anchor's
/// point, handles and kind.
fn fingerprint(path: &PathData) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |v: u64| {
        for b in v.to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    eat(path.subpaths.len() as u64);
    for sp in &path.subpaths {
        eat(u64::from(sp.closed));
        eat(sp.anchors.len() as u64);
        for a in &sp.anchors {
            for p in [a.p, a.h_in, a.h_out] {
                eat(p.x.to_bits());
                eat(p.y.to_bits());
            }
            eat(u64::from(a.kind == AnchorKind::Smooth));
        }
    }
    h
}

fn cases() -> Vec<(&'static str, LiveShape)> {
    use CornerKind::{Chamfer, InvertedRound, Round};
    let moved = Affine::translate((12.5, -7.25));
    let turned = Affine::translate((40.0, 30.0)) * Affine::rotate(0.5235987755982988);
    let sheared = Affine::new([1.0, 0.0, 0.3, 1.0, 3.0, 4.0]);
    let rect = |w, h, radii, kinds, radii_relative, xf| LiveShape::Rectangle { w, h, radii, kinds, radii_relative, xf };
    let poly = |radius, sides, xf, radii: Vec<f64>, kinds: Vec<CornerKind>, radii_relative| LiveShape::Polygon {
        radius,
        sides,
        xf,
        radii,
        kinds,
        radii_relative,
    };
    let star_base = star(Point::new(50.0, 50.0), 40.0, 18.0, 5, 0.0);
    let pen = PathData::single(vectorcraft_geom::SubPath::new(
        vec![
            vectorcraft_geom::Anchor::corner(Point::new(0.0, 0.0)),
            vectorcraft_geom::Anchor::corner(Point::new(80.0, 10.0)),
            vectorcraft_geom::Anchor::corner(Point::new(60.0, 70.0)),
            vectorcraft_geom::Anchor::corner(Point::new(-5.0, 45.0)),
        ],
        true,
    ));
    vec![
        ("rectangle", rect(100.0, 50.0, [0.0; 4], [Round; 4], false, moved)),
        ("rectangle turned", rect(72.0, 36.0, [0.0; 4], [Round; 4], false, turned)),
        ("rectangle sheared", rect(30.0, 60.0, [0.0; 4], [Round; 4], false, sheared)),
        ("rectangle rounded", rect(100.0, 50.0, [10.0, 0.0, 5.0, 20.0], [Round; 4], false, moved)),
        ("rectangle rounded turned", rect(64.0, 64.0, [8.0; 4], [Round; 4], false, turned)),
        ("rectangle inverted round", rect(90.0, 40.0, [6.0; 4], [InvertedRound; 4], false, Affine::IDENTITY)),
        ("rectangle chamfer", rect(90.0, 40.0, [6.0, 12.0, 0.0, 3.0], [Chamfer; 4], false, moved)),
        ("rectangle mixed corners", rect(90.0, 40.0, [6.0, 7.0, 8.0, 9.0], [Round, InvertedRound, Chamfer, Round], false, turned)),
        ("rectangle relative radii", rect(120.0, 45.0, [0.5, 1.0, 0.25, 0.0], [Round; 4], true, moved)),
        ("rectangle radii past half", rect(20.0, 10.0, [50.0; 4], [Round; 4], false, Affine::IDENTITY)),
        ("ellipse", LiveShape::Ellipse { w: 80.0, h: 40.0, pie: (0.0, 360.0), xf: moved }),
        ("circle turned", LiveShape::Ellipse { w: 33.3, h: 33.3, pie: (0.0, 360.0), xf: turned }),
        ("ellipse pie", LiveShape::Ellipse { w: 80.0, h: 40.0, pie: (30.0, 250.0), xf: moved }),
        ("ellipse pie quarter", LiveShape::Ellipse { w: 50.0, h: 70.0, pie: (0.0, 90.0), xf: Affine::IDENTITY }),
        ("ellipse pie wrapping", LiveShape::Ellipse { w: 50.0, h: 70.0, pie: (300.0, 45.0), xf: sheared }),
        ("polygon triangle", poly(30.0, 3, moved, vec![], vec![], false)),
        ("polygon hexagon turned", poly(40.0, 6, turned, vec![], vec![], false)),
        ("polygon many sides", poly(25.0, 37, Affine::IDENTITY, vec![], vec![], false)),
        ("polygon rounded", poly(40.0, 5, moved, vec![5.0; 5], vec![], false)),
        ("polygon mixed corners", poly(40.0, 4, turned, vec![3.0, 6.0, 0.0, 9.0], vec![Round, Chamfer, InvertedRound, Round], false)),
        ("polygon relative radii", poly(40.0, 8, sheared, vec![0.5; 8], vec![], true)),
        ("line", LiveShape::Line { a: Point::new(1.5, 2.5), b: Point::new(-40.0, 73.25) }),
        ("star rounded", LiveShape::Path { base: star_base.clone(), radii: vec![2.0; 10], kinds: vec![], radii_relative: false }),
        (
            "star mixed corners",
            LiveShape::Path { base: star_base, radii: vec![3.0, 0.0, 1.5], kinds: vec![Chamfer, Round, InvertedRound], radii_relative: false },
        ),
        ("pen path relative radii", LiveShape::Path { base: pen, radii: vec![0.3, 0.6, 1.0, 0.0], kinds: vec![], radii_relative: true }),
    ]
}

/// What each case's path was when format v6 began leaving live shapes' paths out of files.
const GOLDEN: &[(&str, u64)] = &[
    ("rectangle", 0x724d8564fd7245c1),
    ("rectangle turned", 0x7435cd1c29336208),
    ("rectangle sheared", 0x7fc2b38d043e7c35),
    ("rectangle rounded", 0x6bf0adc6970faf82),
    ("rectangle rounded turned", 0x00d9d9f8f1b00d44),
    ("rectangle inverted round", 0xdf29732f6d23b4b5),
    ("rectangle chamfer", 0x00ee17e3e382d3fa),
    ("rectangle mixed corners", 0xce67f9471d7ecce7),
    ("rectangle relative radii", 0xba32c2b90efd18f0),
    ("rectangle radii past half", 0x339b6acc665c031d),
    ("ellipse", 0x8830626f9117173e),
    ("circle turned", 0xdd9ac77bd3564233),
    ("ellipse pie", 0x59811b25e910352d),
    ("ellipse pie quarter", 0x7383ca9d003db755),
    ("ellipse pie wrapping", 0x1daab8f972a673a7),
    ("polygon triangle", 0x9371e5d255db01b4),
    ("polygon hexagon turned", 0xbee354f3f1b7c9b9),
    ("polygon many sides", 0x057856eeeb6be1ed),
    ("polygon rounded", 0x22c04c16126b1778),
    ("polygon mixed corners", 0x22ecc24ee3bf9987),
    ("polygon relative radii", 0x33aad3fdc5206ecc),
    ("line", 0x382acd7a241a70a5),
    ("star rounded", 0xf6b1df0df3230cfd),
    ("star mixed corners", 0xe0065f3e262879b4),
    ("pen path relative radii", 0xa40ce91488f8b0f1),
];

/// A failure means decks saved in format v6 would redraw: see the module's note. The fix is a
/// format decision, never new bits here.
#[test]
fn live_shapes_rebuild_the_paths_format_v6_files_left_out_bit_for_bit() {
    let got: Vec<(&str, u64)> = cases().iter().map(|(name, shape)| (*name, fingerprint(&shape.to_path()))).collect();
    // Every kind of live shape is among the cases (a new kind fails to build here until it is).
    let kind = |s: &LiveShape| match s {
        LiveShape::Rectangle { .. } => 0,
        LiveShape::Ellipse { .. } => 1,
        LiveShape::Polygon { .. } => 2,
        LiveShape::Line { .. } => 3,
        LiveShape::Path { .. } => 4,
    };
    let kinds: std::collections::BTreeSet<i32> = cases().iter().map(|(_, s)| kind(s)).collect();
    assert_eq!(kinds.len(), 5, "a case for every kind of live shape");
    assert_eq!(
        got, GOLDEN,
        "LiveShape::to_path's bits changed: decks saved in format v6 leave these paths out and would redraw. This is a format decision (keep the old builders' bits, or a new format version), not a golden to update"
    );
}
