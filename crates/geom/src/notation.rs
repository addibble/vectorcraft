//! Geometry in SVG's notation: a path's anchors as SVG path data and an affine transform as an
//! SVG transform, the forms a saved file holds them in ([`crate::saved`]).
//!
//! Both are exact: a string this module writes reads back to the same anchors (count, points,
//! handles and kinds) and the same transform, bit for bit, because every number is written as
//! the shortest decimal that parses back to the same `f64`. What the notation cannot say exactly
//! is written in the structured form instead ([`path_data`] and [`transform`] return `None`).
//!
//! # What is written
//!
//! ```text
//! number    = ["-"] digits ["." digits] ["e" ["-"] digits]
//!               ; the shortest text that parses back to the same f64: Rust's `{}` (Display),
//!               ; or `{:e}` (LowerExp) when that is shorter; "-0" for negative zero; never
//!               ; NaN or infinite (such values keep the structured form)
//! pair      = number " " number                 ; x then y
//!
//! d         = { subpath }                       ; nothing between subpaths; "" is no subpath
//! subpath   = "M" pair { segment } [ closing ]
//! segment   = "L" pair                          ; a straight segment to the next anchor
//!           | "C" pair " " pair " " pair        ; out handle, next anchor's in handle, next anchor
//! closing   = [ segment ] "Z"
//!
//! kinds     = run { " " run }                   ; one run per subpath, in order
//! run       = { "c" | "s" }                     ; one letter per anchor: corner, smooth
//!
//! transform = "translate(" number [ " " number ] ")"   ; [1 0 0 1 e f], f left out when +0
//!           | "scale(" number [ " " number ] ")"       ; [a 0 0 d 0 0], d left out when equal to a
//!           | "rotate(90)" | "rotate(-90)"             ; [0 1 -1 0 0 0], [0 -1 1 0 0 0]
//!           | "matrix(" number 5*( " " number ) ")"    ; [a b c d e f]
//! ```
//!
//! Commands are absolute and every one is written (no implicit repeats); numbers are separated by
//! one space, and a command letter follows the number before it directly (`M0 0L10 0`).
//!
//! **Segments.** A segment from anchor A to anchor B is `L` when A's out handle and B's in handle
//! are exactly their anchors' points (no handles), else `C` with both handles. An anchor's in
//! handle comes from the segment that ends at it, its out handle from the segment that starts at
//! it; a handle no segment carries is at its anchor.
//!
//! **Closed subpaths.** `Z` closes a subpath. The closing segment, from the last anchor back to
//! the first, is left out when it is straight and the last anchor is neither at the first
//! anchor's point nor within [`crate::path::CLOSE_FOLD`] (1e-7) of it; otherwise it is written,
//! ending exactly at the first anchor's point. So a reader, on `Z`, takes a last anchor at
//! exactly the first anchor's point (same number text) for the end of the closing segment, not
//! an anchor: it drops it and gives its in handle to the first anchor
//! ([`crate::SubPath::fold_closing`]). A file's reader folds only that exact point, whatever
//! commands the data uses, so a file's anchors are what its `d` says: files written before the
//! writer wrote the near case hold real last anchors within 1e-7 of the first, and `kinds` written
//! by hand counts them. Path data written by hand for a command ([`parse_path_data_by_hand`])
//! folds within 1e-7, as [`crate::PathData::from_bezpath`] does: relative data, as optimizers and
//! other apps write it, often ends its closing segment a rounding error off the start. The writer
//! writes the near case's closing segment so that a saved `d` given to such a command keeps its
//! anchors. A second `Z` right after one
//! closes nothing and is left out (`ZZ`), and so is an arc back to its own start that would open
//! a subpath after `Z`; numbers after `Z` are refused (`Z 5 5`), since only a command may follow
//! it.
//!
//! **Not written as `d`** (the structured `subpaths` form is kept): a subpath without anchors, an
//! open subpath whose first anchor has an in handle or whose last anchor has an out handle (SVG
//! has no segment to carry them), and coordinates that are not finite.
//!
//! **Anchor kinds.** `d` says nothing about how an anchor's handles behave when edited
//! ([`AnchorKind`]). Without `kinds`, an anchor reads as a corner, except the joins the reader
//! knows are smooth: those inside an arc (`A`) and those `S` or `T` reflect a control point
//! through (see below); with it, every anchor's kind is its letter. It is written only
//! when some anchor is smooth, and then has one letter per anchor (`d`'s anchors, after the
//! closing rule above), subpaths separated by one space.
//!
//! **Transforms** use the shortest exact form among translate, scale, rotate and matrix, the
//! first of them on a tie (so the identity is `scale(1)`). The entries a form implies are exact:
//! `translate` only when a and d are 1 and b and c are positive zero, and so on.
//!
//! # What is read
//!
//! Readers take everything above and, for data written by hand, a wider part of SVG: relative
//! commands (`m l h v c z`), `H` and `V`, implicit repeats (pairs after `M` are lines), commas
//! and any whitespace between numbers, `E` and `+` in numbers, a command after `Z` without `M`
//! (it starts at the subpath's first point), and the rest of SVG's path commands, each read into
//! the anchors above:
//!
//! - `S` and `T` reflect the previous segment's control point as SVG says (the current point when
//!   the previous command was not `C`/`S`, or `Q`/`T`);
//! - `Q` and `T` become the cubic with the same curve (handles two thirds of the way to the
//!   control point);
//! - `A` becomes one cubic per quarter turn, with half a degree of grace (an arc of up to 90.5°
//!   is one cubic, up to 180.5° two, and so on to four: from the span alone, whatever rounding
//!   its angles carry), each the standard approximation of its part of the ellipse (within 0.03%
//!   of the radius), starting and ending exactly at the arc's end points; angles within 1e-5° of
//!   a quarter turn are drawn at it, so a half circle's points are exact. As SVG says (SVG 1.1
//!   F.6.2, F.6.6), an arc with a zero radius is a line, one to its own start point is left out,
//!   and radii too small to reach are scaled up; an arc with a radius at or below 1e-5 (in the
//!   path's units) is exactly `L`, to its own start too, as kurbo (and so SVG import) reads it,
//!   and radii one of which is more than the largest number times the other are refused. An arc whose span is zero but for rounding (its
//!   ends far closer than its radii) is its chord when small and the whole turn round when
//!   large; one whose ends are a smallest step apart is its chord. A coordinate that leaves the
//!   finite range is refused, as everywhere. Flags need no separator (`a10 10 0 1110 10`).
//!
//! Anchors read from `d` count against [`crate::path::with_anchor_budget`] as anchors read one
//! by one do.
//!
//! What they read to is written back as `L` and `C`, so a file stays exact from then on. The
//! joins inside an arc are smooth anchors, and so is the join an `S` or `T` reflects a control
//! point through (its handles are collinear); the rest are corners. `kinds`, when given, says
//! every anchor's kind instead, and counts an arc's anchors like any other. Transforms
//! read SVG transform lists: `matrix`, `translate`, `scale`, `rotate(angle [cx cy])` (quarter
//! turns exact), `skewX` and `skewY`, separated by whitespace or commas, applied right to left as
//! SVG does.

use kurbo::{Affine, Point};

use crate::path::{Anchor, AnchorKind, PathData, SubPath};

/// The shortest text that parses back to `x` (finite): its shortest round-trip digits, as zmij
/// (serde_json's float writer) finds them, laid out plain (`{}`'s form) or with an exponent
/// (`{:e}`'s), whichever is shorter, without allocating: a file's every coordinate goes through
/// here.
pub fn push_number(out: &mut String, x: f64) {
    if x.is_finite() {
        // zmij writes most numbers plain, as `{}` does but for an integer's `.0`. Plain is the
        // shorter form unless it ends in zeros or starts `0.0`: those take the layout below.
        let mut buf = zmij::Buffer::new();
        let text = buf.format_finite(x);
        let text = text.strip_suffix(".0").unwrap_or(text);
        let unsigned = text.strip_prefix('-').unwrap_or(text);
        if !text.contains('e') && !unsigned.ends_with("00") && !unsigned.starts_with("0.0") {
            out.push_str(text);
            return;
        }
    }
    let Some(n) = Shortest::of(x) else {
        out.push_str(&x.to_string());
        return;
    };
    let digits = n.digits.get(..n.len).unwrap_or_default();
    let len = digits.len() as i64;
    let pow_len = if n.pow < 0 { 1 } else { 0 } + decimal_len(n.pow.unsigned_abs());
    let exp_len = i64::from(n.neg) + len + i64::from(len > 1) + 1 + pow_len;
    let plain_len = i64::from(n.neg)
        + if n.pow >= len - 1 {
            n.pow + 1
        } else if n.pow >= 0 {
            len + 1
        } else {
            1 - n.pow + len
        };
    if n.neg {
        out.push('-');
    }
    let push_digits = |out: &mut String, d: &[u8]| d.iter().for_each(|&b| out.push(char::from(b)));
    if exp_len < plain_len {
        let (first, rest) = digits.split_at(1.min(digits.len()));
        push_digits(out, first);
        if !rest.is_empty() {
            out.push('.');
            push_digits(out, rest);
        }
        out.push('e');
        if n.pow < 0 {
            out.push('-');
        }
        push_decimal(out, n.pow.unsigned_abs());
    } else if n.pow >= len - 1 {
        push_digits(out, digits);
        (0..n.pow - (len - 1)).for_each(|_| out.push('0'));
    } else if n.pow >= 0 {
        let (int, frac) = digits.split_at((n.pow as usize + 1).min(digits.len()));
        push_digits(out, int);
        out.push('.');
        push_digits(out, frac);
    } else {
        out.push_str("0.");
        (0..-n.pow - 1).for_each(|_| out.push('0'));
        push_digits(out, digits);
    }
}

/// A finite `f64`'s shortest round-trip decimal digits (ASCII, no leading or trailing zeros,
/// none for zero) and the power of ten of the first: `x = ±d.ddd × 10^pow`.
struct Shortest {
    neg: bool,
    digits: [u8; 17],
    len: usize,
    pow: i64,
}

impl Shortest {
    fn of(x: f64) -> Option<Shortest> {
        if !x.is_finite() {
            return None;
        }
        let mut buf = zmij::Buffer::new();
        let text = buf.format_finite(x).as_bytes();
        let (neg, text) = match text.split_first() {
            Some((b'-', rest)) => (true, rest),
            _ => (false, text),
        };
        let (mantissa, exp) = match text.iter().position(|&b| b == b'e') {
            Some(i) => (text.get(..i)?, std::str::from_utf8(text.get(i + 1..)?).ok()?.parse::<i64>().ok()?),
            None => (text, 0),
        };
        let int_len = mantissa.iter().position(|&b| b == b'.').unwrap_or(mantissa.len());
        let mut n = Shortest { neg, digits: [0; 17], len: 0, pow: int_len as i64 + exp - 1 };
        for &b in mantissa.iter().filter(|b| b.is_ascii_digit()) {
            if n.len == 0 && b == b'0' {
                n.pow -= 1;
                continue;
            }
            *n.digits.get_mut(n.len)? = b;
            n.len += 1;
        }
        while n.len > 0 && n.digits.get(n.len - 1) == Some(&b'0') {
            n.len -= 1;
        }
        if n.len == 0 {
            n.pow = 0;
        }
        Some(n)
    }
}

fn decimal_len(mut v: u64) -> i64 {
    let mut len = 1;
    while v >= 10 {
        v /= 10;
        len += 1;
    }
    len
}

fn push_decimal(out: &mut String, v: u64) {
    if v >= 10 {
        push_decimal(out, v / 10);
    }
    out.push(char::from(b'0' + (v % 10) as u8));
}

fn push_pair(out: &mut String, p: Point) {
    push_number(out, p.x);
    out.push(' ');
    push_number(out, p.y);
}

/// The same point bit for bit (so -0 is not 0): what reads back from the same text.
pub(crate) fn same(a: Point, b: Point) -> bool {
    a.x.to_bits() == b.x.to_bits() && a.y.to_bits() == b.y.to_bits()
}

fn push_segment(out: &mut String, a: &Anchor, b: &Anchor) {
    if same(a.h_out, a.p) && same(b.h_in, b.p) {
        out.push('L');
        push_pair(out, b.p);
    } else {
        out.push('C');
        push_pair(out, a.h_out);
        out.push(' ');
        push_pair(out, b.h_in);
        out.push(' ');
        push_pair(out, b.p);
    }
}

/// `path` as SVG path data and, when some anchor is smooth, its anchors' kinds (see the module
/// documentation for both grammars). `None` when the notation cannot hold it exactly.
pub fn path_data(path: &PathData) -> Option<(String, Option<String>)> {
    // About a curve's worth of text per anchor, so the string is seldom grown.
    let mut d = String::with_capacity(48 * path.subpaths.iter().map(|s| s.anchors.len()).sum::<usize>());
    for sp in &path.subpaths {
        let (first, last) = (sp.anchors.first()?, sp.anchors.last()?);
        if !sp.closed && (!same(first.h_in, first.p) || !same(last.h_out, last.p)) {
            return None;
        }
        if !sp.anchors.iter().all(|a| a.p.is_finite() && a.h_in.is_finite() && a.h_out.is_finite()) {
            return None;
        }
        d.push('M');
        push_pair(&mut d, first.p);
        for w in sp.anchors.windows(2) {
            if let [a, b] = w {
                push_segment(&mut d, a, b);
            }
        }
        if sp.closed {
            let straight = same(last.h_out, last.p) && same(first.h_in, first.p);
            if !straight || same(last.p, first.p) || last.p.distance(first.p) < crate::path::CLOSE_FOLD {
                push_segment(&mut d, last, first);
            }
            d.push('Z');
        }
    }
    let smooth = path.subpaths.iter().flat_map(|s| &s.anchors).any(|a| a.kind == AnchorKind::Smooth);
    let kinds = smooth.then(|| {
        let runs: Vec<String> =
            path.subpaths.iter().map(|s| s.anchors.iter().map(|a| if a.kind == AnchorKind::Smooth { 's' } else { 'c' }).collect()).collect();
        runs.join(" ")
    });
    Some((d, kinds))
}

/// Reads SVG path data (and anchor kinds, see the module documentation) into anchors, as a
/// saved file's path data is read: a closing anchor folds into the start only at exactly its
/// point, and the kinds are `kinds`' or, without it, the commands' (corners but for the joins
/// inside an arc and those `S` or `T` reflect through).
pub fn parse_path_data(d: &str, kinds: Option<&str>) -> Result<PathData, String> {
    read_path_data(d, kinds, None)
}

/// SVG path data written by hand, without kinds, read as [`crate::PathData::from_bezpath`] reads
/// kurbo's path of it: a closing anchor within [`crate::path::CLOSE_FOLD`] of the start folds into
/// it, and the kinds are taken from the handles ([`PathData::infer_kinds_from_handles`]). The
/// geometry is [`parse_path_data`]'s otherwise.
pub fn parse_path_data_by_hand(d: &str) -> Result<PathData, String> {
    let mut path = read_path_data(d, None, Some(crate::path::CLOSE_FOLD))?;
    path.infer_kinds_from_handles();
    Ok(path)
}

fn read_path_data(d: &str, kinds: Option<&str>, fold: Option<f64>) -> Result<PathData, String> {
    let mut path = PathData { subpaths: Parser { s: d.as_bytes(), i: 0, fold }.subpaths()? };
    if let Some(kinds) = kinds {
        let runs: Vec<&str> = kinds.split(' ').collect();
        if runs.len() != path.subpaths.len() {
            return Err(format!("`kinds` has {} runs of letters but `d` has {} subpaths", runs.len(), path.subpaths.len()));
        }
        for (i, (sp, run)) in path.subpaths.iter_mut().zip(runs).enumerate() {
            if run.len() != sp.anchors.len() {
                return Err(format!("`kinds` gives subpath {i} {} letters but it has {} anchors", run.len(), sp.anchors.len()));
            }
            for (a, k) in sp.anchors.iter_mut().zip(run.bytes()) {
                a.kind = match k {
                    b'c' => AnchorKind::Corner,
                    b's' => AnchorKind::Smooth,
                    _ => return Err("`kinds` holds only `c` (corner), `s` (smooth) and a space between subpaths".into()),
                };
            }
        }
    }
    Ok(path)
}

/// Spend from the read's budget what the `read` anchors held now add to those `spent` so far
/// (the reader's count never falls: the anchor a `Z` may fold in is not counted until it stays).
fn settle_anchor_budget(read: usize, spent: &mut usize) -> Result<(), String> {
    if read > *spent {
        crate::path::spend_anchor_budget(read - *spent)?;
        *spent = read;
    }
    Ok(())
}

/// A join whose handles `S` or `T` made collinear by reflection is smooth, as the curve is there.
fn smooth_join(sp: &mut SubPath, reflected: bool) {
    if reflected && let Some(a) = sp.anchors.last_mut() {
        a.kind = AnchorKind::Smooth;
    }
}

/// `deg` on the nearest quarter turn when it is within a hundred-thousandth of a degree of it,
/// so that an arc's quarter turns are drawn with exact sines and cosines (and, for the span, so
/// that a span that is zero but for rounding is zero). It decides no piece count.
fn snap_quarter(deg: f64) -> f64 {
    let quarter = (deg / 90.0).round() * 90.0;
    if (deg - quarter).abs() <= 1e-5 { quarter } else { deg }
}

/// The grace, in degrees, an arc's span has over each quarter turn before it takes another cubic:
/// far above any rounding of the angles, so that the count is the span's alone, and a cubic of
/// 90.5° still stays within 0.03% of the radius.
const ARC_PIECE_GRACE: f64 = 0.5;

/// The largest radius an arc is a straight line at, exactly `L` (to its own start too): kurbo's
/// rule (`SvgArc::is_straight_line`, 0.13 `src/svg.rs`), which `path.create` and SVG import
/// (through usvg) have always read arcs by, kept for parity with them. SVG itself makes only a
/// zero radius a line. The threshold is in the path's own units: a half circle of radius 5e-6 is
/// its chord.
const ARC_LINE_RADIUS: f64 = 1e-5;

/// An arc read as cubics `[c1, c2, end]`, the first `len` of them.
struct ArcCubics {
    cubics: [[Point; 3]; 4],
    len: usize,
}

/// SVG's elliptical arc from `from` to `to` (two distinct points; radii `rx`, `ry` positive and
/// finite, the ellipse's x axis turned `rotation` degrees) as cubics: one per quarter turn of its
/// span with half a degree of grace ([`ARC_PIECE_GRACE`], [`crate::shapes::arc_pieces`]), each
/// [`crate::shapes::arc_handle`]'s approximation of its part of the ellipse, the first starting
/// at `from` and the last ending exactly at `to`. `Ok(None)` when the arc is its chord: a small
/// arc whose span is zero but for rounding, or ends a smallest step apart, too close to have a
/// direction. A large arc whose span is zero but for rounding is the whole turn round. `Err` when
/// the ellipse itself (radii scaled up past the largest number) leaves the finite range, or the
/// radii's ratio, either way round, passes the largest number (when the ends are not one point
/// but for rounding, which is the chord whatever the radii).
///
/// The center comes from SVG's endpoint-to-center conversion (SVG 1.1 Appendix F.6.5, SVG 2
/// "Implementation notes"), radii too small to reach scaled up as F.6.6 says. It is worked in
/// the unit circle the ellipse maps to, as a unit direction and a distance, and each point is
/// placed from the start point, so that tiny or huge numbers neither overflow nor underflow on
/// the way and a far center's rounding does not throw a point off. A point that leaves the finite
/// range all the same is refused by the reader's own check of every coordinate.
fn arc_cubics(from: Point, to: Point, rx: f64, ry: f64, rotation: f64, large_arc: bool, sweep: bool) -> Result<Option<ArcCubics>, String> {
    let (sin, cos) = crate::sin_cos_deg(rotation);
    // Half the chord, each end halved first so that far-apart ends do not overflow.
    let half = (from.x * 0.5 - to.x * 0.5, from.y * 0.5 - to.y * 0.5);
    // The start point from the chord's midpoint, in the ellipse's axes.
    let (x1, y1) = (cos * half.0 + sin * half.1, -sin * half.0 + cos * half.1);
    if x1 == 0.0 && y1 == 0.0 {
        return Ok(None);
    }
    // The radii's ratio both ways round. Neither is zero (the radii are finite and positive), but
    // one can pass the largest number, and the ellipse then has no direction to work in.
    let (q, ratio) = (ry / rx, rx / ry);
    if !(q.is_finite() && ratio.is_finite()) {
        return Err("an arc's radii are out of range: one is more than the largest number times the other".into());
    }
    // Its direction in unit-circle terms, (x1/rx, y1/ry) normalized, found as (x1, y1·rx/ry) or
    // (x1·ry/rx, y1) over the larger coordinate so that it neither overflows nor underflows away.
    let (ua, ub) = {
        let m = x1.abs().max(y1.abs());
        let (a, b) = if q >= 1.0 { (x1 / m, y1 / m / q) } else { (x1 / m * q, y1 / m) };
        // Never zero: the larger coordinate's share is ±1 when q ≥ 1 or that coordinate is y1,
        // and ±q, which is not zero, otherwise.
        let n = a.hypot(b);
        (a / n, b / n)
    };
    let (mut rx, mut ry) = (rx, ry);
    let d = (x1 / rx).hypot(y1 / ry);
    // The center's offset from the midpoint, in unit-circle terms, along the perpendicular of the
    // start point's direction, and the start point's distance: at the midpoint, the start one
    // radius away, when the radii have to grow to reach (F.6.6).
    let (s, dist) = if d >= 1.0 {
        if d.is_finite() {
            rx *= d;
            ry *= d;
        } else {
            // rx·d and ry·d without the overflow of d.
            (rx, ry) = (x1.hypot(y1 * ratio), (x1 / ratio).hypot(y1));
        }
        if !(rx.is_finite() && ry.is_finite()) {
            return Err("an arc's ellipse is out of range: its radii, scaled up to reach its end points, pass the largest number".into());
        }
        (0.0, 1.0)
    } else {
        let s = ((1.0 - d) * (1.0 + d)).sqrt();
        (if large_arc == sweep { -s } else { s }, d)
    };
    let (cu, cv) = (s * ub, -s * ua);
    let (pa, pb) = (dist * ua, dist * ub);
    let theta1 = (pb - cv).atan2(pa - cu).to_degrees();
    let mut delta = snap_quarter((-pb - cv).atan2(-pa - cu).to_degrees() - theta1);
    if !delta.is_finite() {
        return Err("an arc's angles are out of range".into());
    }
    if sweep && delta < 0.0 {
        delta += 360.0;
    } else if !sweep && delta > 0.0 {
        delta -= 360.0;
    }
    // A span far below the radius leaves the two angles equal once rounded: the small arc is
    // then the chord and the large one the whole turn round.
    if delta == 0.0 {
        if !large_arc {
            return Ok(None);
        }
        delta = if sweep { 360.0 } else { -360.0 };
    }
    let pieces = crate::shapes::arc_pieces((delta.abs() - ARC_PIECE_GRACE).max(0.0));
    let theta1 = snap_quarter(theta1);
    let step = delta / f64::from(pieces);
    let k = crate::shapes::arc_handle(step);
    // `p` plus a vector of the unit circle's plane in the path's coordinates; on a radius near
    // the largest number, a vector that overflows alone though the point it reaches does not is
    // added in quarters.
    let place = |p: Point, u: f64, v: f64| {
        let along = |u: f64, v: f64| {
            let (x, y) = (u * rx, v * ry);
            kurbo::Vec2::new(cos * x - sin * y, sin * x + cos * y)
        };
        let q = p + along(u, v);
        if q.is_finite() {
            return q;
        }
        let h = along(u * 0.25, v * 0.25);
        p + h + h + h + h
    };
    let (s1, c1) = crate::sin_cos_deg(theta1);
    let exact = |deg: f64| crate::quarter_turn(deg).is_some();
    // The point at `t` degrees, from the start point: cos t − cos θ1 and sin t − sin θ1 taken
    // directly between quarter turns (exact), else by the sum-to-product identities (no
    // cancellation when the two are close, as on a huge radius).
    let point = |t: f64, st: f64, ct: f64| {
        if exact(t) && exact(theta1) {
            return place(from, ct - c1, st - s1);
        }
        let (sm, cm) = ((t + theta1) / 2.0).to_radians().sin_cos();
        let sh = ((t - theta1) / 2.0).to_radians().sin();
        place(from, -2.0 * sm * sh, 2.0 * cm * sh)
    };
    let mut out = ArcCubics { cubics: [[Point::ZERO; 3]; 4], len: 0 };
    let (mut start, mut s0, mut c0) = (from, s1, c1);
    for (i, slot) in (1..=pieces).zip(out.cubics.iter_mut()) {
        let t = theta1 + step * f64::from(i);
        let (st, ct) = crate::sin_cos_deg(t);
        let end = if i == pieces { to } else { point(t, st, ct) };
        *slot = [place(start, -s0 * k, c0 * k), place(end, st * k, -ct * k), end];
        (start, s0, c0) = (end, st, ct);
        out.len += 1;
    }
    Ok(Some(out))
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    /// How a closing anchor folds into the start ([`SubPath::fold_closing`]): `None`, only at
    /// exactly its point (files); `Some(t)`, nearer than `t` (path data written by hand).
    fold: Option<f64>,
}

impl Parser<'_> {
    fn skip_separators(&mut self) {
        while self.s.get(self.i).is_some_and(|c| c.is_ascii_whitespace() || *c == b',') {
            self.i += 1;
        }
    }

    /// Whether a number starts here (after separators).
    fn at_number(&mut self) -> bool {
        self.skip_separators();
        self.s.get(self.i).is_some_and(|c| c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.'))
    }

    fn number(&mut self) -> Result<f64, String> {
        self.skip_separators();
        let start = self.i;
        let digits = |p: &mut Self| {
            let from = p.i;
            while p.s.get(p.i).is_some_and(u8::is_ascii_digit) {
                p.i += 1;
            }
            p.i > from
        };
        if matches!(self.s.get(self.i), Some(b'-' | b'+')) {
            self.i += 1;
        }
        let mut any = digits(self);
        if self.s.get(self.i) == Some(&b'.') {
            self.i += 1;
            any |= digits(self);
        }
        if !any {
            return Err(format!("a number was expected at byte {start}"));
        }
        if matches!(self.s.get(self.i), Some(b'e' | b'E')) {
            let mark = self.i;
            self.i += 1;
            if matches!(self.s.get(self.i), Some(b'-' | b'+')) {
                self.i += 1;
            }
            if !digits(self) {
                self.i = mark;
            }
        }
        let text = std::str::from_utf8(self.s.get(start..self.i).unwrap_or_default()).map_err(|e| e.to_string())?;
        let x: f64 = text.parse().map_err(|_| format!("`{text}` is not a number"))?;
        if !x.is_finite() {
            return Err(format!("`{text}` is out of range"));
        }
        Ok(x)
    }

    /// An arc's flag: `0` or `1`, which needs no separator after it (`a10 10 0 1110 10`).
    fn flag(&mut self) -> Result<bool, String> {
        self.skip_separators();
        let flag = match self.s.get(self.i) {
            Some(b'0') => false,
            Some(b'1') => true,
            _ => return Err(format!("an arc's flag (0 or 1) was expected at byte {}", self.i)),
        };
        self.i += 1;
        Ok(flag)
    }

    fn pair(&mut self, rel: bool, from: Point) -> Result<Point, String> {
        let p = Point::new(self.number()?, self.number()?);
        Ok(if rel { from + p.to_vec2() } else { p })
    }

    fn subpaths(&mut self) -> Result<Vec<SubPath>, String> {
        let mut out: Vec<SubPath> = Vec::new();
        let mut cur: Option<SubPath> = None;
        // The current point, and the first point of the current (or last closed) subpath.
        let (mut at, mut start) = (Point::ZERO, Point::ZERO);
        let mut command: Option<u8> = None;
        // The control point a following `S` (after `C` or `S`) or `T` (after `Q` or `T`) reflects.
        let (mut cubic_c2, mut quad_c): (Option<Point>, Option<Point>) = (None, None);
        // Whether the open subpath began after `Z` without `M`.
        let mut opened_after_z = false;
        // Anchors in finished subpaths, and anchors spent from the read's budget so far: each
        // command adds at most five (an arc's four after the start a command after `Z` takes),
        // spent before the next is read, all but the open subpath's last, which a `Z` may fold
        // into its start; that one is spent once the subpath goes on or ends, so the count never
        // falls and nothing is given back.
        let (mut done, mut spent) = (0usize, 0usize);
        loop {
            let read = done + cur.as_ref().map_or(0, |sp| sp.anchors.len().saturating_sub(1));
            settle_anchor_budget(read, &mut spent)?;
            // Every command but those four leaves nothing to reflect.
            let (prev_c2, prev_q) = (cubic_c2.take(), quad_c.take());
            self.skip_separators();
            let Some(&c) = self.s.get(self.i) else { break };
            let cmd = if c.is_ascii_alphabetic() {
                self.i += 1;
                c
            } else {
                // Implicit repeat: pairs after a moveto are linetos.
                match command {
                    Some(b'M') => b'L',
                    Some(b'm') => b'l',
                    Some(c) if !matches!(c, b'Z' | b'z') => c,
                    _ => return Err(format!("a command (M, L, H, V, C, S, Q, T, A or Z) was expected at byte {}", self.i)),
                }
            };
            command = Some(cmd);
            let rel = cmd.is_ascii_lowercase();
            match cmd.to_ascii_uppercase() {
                b'M' => {
                    if let Some(sp) = cur.take() {
                        done += sp.anchors.len();
                        out.push(sp);
                    }
                    at = self.pair(rel, at)?;
                    start = at;
                    opened_after_z = false;
                    cur = Some(SubPath { anchors: vec![Anchor::corner(at)], closed: false });
                }
                b'Z' => {
                    let Some(mut sp) = cur.take() else {
                        // A second Z right after one closes an empty subpath at the start: nothing.
                        if out.last().is_some_and(|sp| sp.closed) {
                            continue;
                        }
                        return Err("`Z` closes nothing: a subpath starts with `M`".into());
                    };
                    sp.fold_closing(self.fold);
                    sp.closed = true;
                    done += sp.anchors.len();
                    out.push(sp);
                    at = start;
                }
                upper @ (b'L' | b'H' | b'V' | b'C' | b'S' | b'Q' | b'T' | b'A') => {
                    if cur.is_none() {
                        if out.last().is_none_or(|s| !s.closed) {
                            return Err("path data starts with `M`".into());
                        }
                        // After `Z`, the next subpath starts where the closed one did.
                        cur = Some(SubPath { anchors: vec![Anchor::corner(start)], closed: false });
                        opened_after_z = true;
                    }
                    let Some(sp) = cur.as_mut() else { continue };
                    let from = at;
                    match upper {
                        b'L' => at = self.pair(rel, at)?,
                        b'H' => at.x = if rel { at.x + self.number()? } else { self.number()? },
                        b'V' => at.y = if rel { at.y + self.number()? } else { self.number()? },
                        b'C' | b'S' => {
                            // `S`'s first control point is the previous `C` or `S`'s second reflected
                            // through the current point, or the current point after anything else.
                            let reflected = if upper == b'S' { prev_c2.filter(|c| *c != from) } else { None };
                            let c1 = if upper == b'C' { self.pair(rel, from)? } else { reflected.map_or(from, |c| from + (from - c)) };
                            let c2 = self.pair(rel, from)?;
                            at = self.pair(rel, from)?;
                            smooth_join(sp, reflected.is_some());
                            sp.push_cubic(c1, c2, at);
                            cubic_c2 = Some(c2);
                            continue;
                        }
                        b'Q' | b'T' => {
                            // A quadratic is the cubic whose handles lie two thirds of the way to its
                            // control point; `T` reflects the previous `Q` or `T`'s control point.
                            let reflected = if upper == b'T' { prev_q.filter(|c| *c != from) } else { None };
                            let q = if upper == b'Q' { self.pair(rel, from)? } else { reflected.map_or(from, |c| from + (from - c)) };
                            at = self.pair(rel, from)?;
                            smooth_join(sp, reflected.is_some());
                            let (c1, c2) = crate::path::quad_handles(from, q, at);
                            sp.push_cubic(c1, c2, at);
                            quad_c = Some(q);
                            continue;
                        }
                        _ => {
                            let (rx, ry) = (self.number()?.abs(), self.number()?.abs());
                            let rotation = self.number()?;
                            let (large_arc, sweep) = (self.flag()?, self.flag()?);
                            at = self.pair(rel, from)?;
                            // A radius at or below ARC_LINE_RADIUS (zero included, SVG F.6.2) is
                            // exactly `L`, to its own start too, as kurbo reads it.
                            if rx <= ARC_LINE_RADIUS || ry <= ARC_LINE_RADIUS {
                                sp.anchors.push(Anchor::corner(at));
                                continue;
                            }
                            // SVG F.6.2: an arc to its own start point is left out.
                            if from == at {
                                // Nor is the subpath it would have started after `Z`.
                                if opened_after_z && sp.anchors.len() == 1 {
                                    cur = None;
                                }
                                continue;
                            }
                            if let Some(arc) = arc_cubics(from, at, rx, ry, rotation, large_arc, sweep)? {
                                let joins = arc.len.saturating_sub(1);
                                for (i, [c1, c2, p]) in arc.cubics.into_iter().take(arc.len).enumerate() {
                                    sp.push_cubic(c1, c2, p);
                                    // The joins inside one arc are smooth, as the ellipse is there.
                                    if i < joins
                                        && let Some(a) = sp.anchors.last_mut()
                                    {
                                        a.kind = AnchorKind::Smooth;
                                    }
                                }
                                continue;
                            }
                        }
                    }
                    sp.anchors.push(Anchor::corner(at));
                }
                _ => return Err(format!("`{}` is not path data: SVG's commands are M, L, H, V, C, S, Q, T, A and Z (either case)", char::from(cmd))),
            }
        }
        if let Some(sp) = cur {
            done += sp.anchors.len();
            out.push(sp);
        }
        settle_anchor_budget(done, &mut spent)?;
        // Relative coordinates can add up past the largest number.
        if !out.iter().flat_map(|s| &s.anchors).all(|a| a.p.is_finite() && a.h_in.is_finite() && a.h_out.is_finite()) {
            return Err("a coordinate is out of range".into());
        }
        Ok(out)
    }
}

fn pos_zero(x: f64) -> bool {
    x.to_bits() == 0
}

/// `a` as an SVG transform: the shortest exact form (see the module documentation). `None` when
/// an entry is not finite.
pub fn transform(a: Affine) -> Option<String> {
    let [m0, m1, m2, m3, m4, m5] = a.as_coeffs();
    if !a.as_coeffs().iter().all(|x| x.is_finite()) {
        return None;
    }
    let call = |name: &str, args: &[f64]| {
        let mut s = format!("{name}(");
        for (i, x) in args.iter().enumerate() {
            if i > 0 {
                s.push(' ');
            }
            push_number(&mut s, *x);
        }
        s.push(')');
        s
    };
    let mut forms = Vec::with_capacity(4);
    if m0 == 1.0 && pos_zero(m1) && pos_zero(m2) && m3 == 1.0 {
        forms.push(if pos_zero(m5) { call("translate", &[m4]) } else { call("translate", &[m4, m5]) });
    }
    if pos_zero(m1) && pos_zero(m2) && pos_zero(m4) && pos_zero(m5) {
        forms.push(if m0.to_bits() == m3.to_bits() { call("scale", &[m0]) } else { call("scale", &[m0, m3]) });
    }
    if pos_zero(m0) && pos_zero(m3) && pos_zero(m4) && pos_zero(m5) {
        if m1 == 1.0 && m2 == -1.0 {
            forms.push("rotate(90)".into());
        } else if m1 == -1.0 && m2 == 1.0 {
            forms.push("rotate(-90)".into());
        }
    }
    forms.push(call("matrix", &a.as_coeffs()));
    // The first of the shortest.
    forms.into_iter().reduce(|best, f| if f.len() < best.len() { f } else { best })
}

/// Reads an SVG transform list (see the module documentation).
pub fn parse_transform(s: &str) -> Result<Affine, String> {
    let mut p = Parser { s: s.as_bytes(), i: 0, fold: None };
    let mut out: Option<Affine> = None;
    loop {
        p.skip_separators();
        if p.i >= p.s.len() {
            break;
        }
        let from = p.i;
        while p.s.get(p.i).is_some_and(u8::is_ascii_alphabetic) {
            p.i += 1;
        }
        let name = std::str::from_utf8(p.s.get(from..p.i).unwrap_or_default()).unwrap_or_default().to_string();
        while p.s.get(p.i).is_some_and(u8::is_ascii_whitespace) {
            p.i += 1;
        }
        if p.s.get(p.i) != Some(&b'(') {
            return Err(format!("a transform function such as translate(x y) was expected at byte {from}"));
        }
        p.i += 1;
        let mut args = Vec::new();
        while p.at_number() {
            if args.len() == 6 {
                return Err(format!("`{name}` takes at most 6 numbers"));
            }
            args.push(p.number()?);
        }
        p.skip_separators();
        if p.s.get(p.i) != Some(&b')') {
            return Err(format!("`{name}(` is not closed with `)`"));
        }
        p.i += 1;
        let m = function(&name, &args)?;
        // A single function is taken as is (multiplying by the identity would turn -0 into 0).
        out = Some(match out {
            Some(o) => o * m,
            None => m,
        });
    }
    let out = out.ok_or("a transform needs a function, such as translate(10 20) or matrix(1 0 0 1 0 0)")?;
    if !out.as_coeffs().iter().all(|x| x.is_finite()) {
        return Err("the transform is out of range".into());
    }
    Ok(out)
}

fn function(name: &str, args: &[f64]) -> Result<Affine, String> {
    let wrong = |takes: &str| Err(format!("`{name}` takes {takes}, not {} numbers", args.len()));
    Ok(match (name, args) {
        ("matrix", &[a, b, c, d, e, f]) => Affine::new([a, b, c, d, e, f]),
        ("matrix", _) => return wrong("6"),
        ("translate", &[x]) => Affine::new([1.0, 0.0, 0.0, 1.0, x, 0.0]),
        ("translate", &[x, y]) => Affine::new([1.0, 0.0, 0.0, 1.0, x, y]),
        ("translate", _) => return wrong("1 or 2"),
        ("scale", &[k]) => Affine::new([k, 0.0, 0.0, k, 0.0, 0.0]),
        ("scale", &[x, y]) => Affine::new([x, 0.0, 0.0, y, 0.0, 0.0]),
        ("scale", _) => return wrong("1 or 2"),
        ("rotate", &[deg]) => rotation(deg),
        ("rotate", &[deg, cx, cy]) => Affine::translate((cx, cy)) * rotation(deg) * Affine::translate((-cx, -cy)),
        ("rotate", _) => return wrong("1 or 3"),
        ("skewX", &[deg]) => Affine::new([1.0, 0.0, deg.to_radians().tan(), 1.0, 0.0, 0.0]),
        ("skewY", &[deg]) => Affine::new([1.0, deg.to_radians().tan(), 0.0, 1.0, 0.0, 0.0]),
        ("skewX" | "skewY", _) => return wrong("1"),
        _ => return Err(format!("`{name}` is not a transform: matrix, translate, scale, rotate, skewX and skewY are")),
    })
}

/// SVG's `rotate(deg)`, exact for quarter turns (`[0 1 -1 0 0 0]` for 90).
/// Not [`crate::sin_cos_deg`]: a half turn here has a positive zero sine (that one's is
/// negative), and saved files' transforms read through this must keep the matrices they always
/// gave, bit for bit.
fn rotation(deg: f64) -> Affine {
    let turn = deg.rem_euclid(360.0);
    let (sin, cos) = if turn == 0.0 {
        (0.0, 1.0)
    } else if turn == 90.0 {
        (1.0, 0.0)
    } else if turn == 180.0 {
        (0.0, -1.0)
    } else if turn == 270.0 {
        (-1.0, 0.0)
    } else {
        deg.to_radians().sin_cos()
    };
    Affine::new([cos, sin, -sin, cos, 0.0, 0.0])
}

/// The schema of a transform field ([`affine`]): an SVG transform string, or the six numbers.
pub struct Transform;

impl schemars::JsonSchema for Transform {
    fn schema_name() -> String {
        "Transform".into()
    }
    fn json_schema(g: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        use schemars::schema::{InstanceType, Schema, SchemaObject, SingleOrVec, SubschemaValidation};
        let mut text = SchemaObject { instance_type: Some(InstanceType::String.into()), ..Default::default() };
        text.metadata().description = Some(
            "An SVG transform, as saved: translate(x y), scale(sx sy), rotate(90), matrix(a b c d e f), or a list of SVG transform functions.".into(),
        );
        let mut numbers = SchemaObject { instance_type: Some(InstanceType::Array.into()), ..Default::default() };
        numbers.array().items = Some(SingleOrVec::Single(Box::new(g.subschema_for::<f64>())));
        numbers.array().min_items = Some(6);
        numbers.array().max_items = Some(6);
        numbers.metadata().description = Some("The matrix [a, b, c, d, e, f] (x' = a·x + c·y + e, y' = b·x + d·y + f).".into());
        let mut o = SchemaObject {
            subschemas: Some(Box::new(SubschemaValidation { any_of: Some(vec![text.into(), numbers.into()]), ..Default::default() })),
            ..Default::default()
        };
        o.metadata().description = Some(
            "An affine transform. Saved files hold it as an SVG transform string (the shortest exact one); other answers give the six numbers [a, b, c, d, e, f]. Either is read."
                .into(),
        );
        Schema::Object(o)
    }
}

/// Serde for an [`Affine`] field (`#[serde(with = "vectorcraft_geom::notation::affine")]`, also for
/// `Box<Affine>`): an SVG transform string while the model is serialized as saved
/// ([`crate::saved::saving`]), else the six numbers as kurbo writes them; either is read.
pub mod affine {
    use std::borrow::Borrow;

    use kurbo::Affine;
    use serde::{Deserializer, Serialize, Serializer};

    pub fn serialize<T: Borrow<Affine>, S: Serializer>(a: &T, s: S) -> Result<S::Ok, S::Error> {
        let a: &Affine = a.borrow();
        match crate::saved::svg_geometry().then(|| super::transform(*a)).flatten() {
            Some(text) => s.serialize_str(&text),
            None => a.serialize(s),
        }
    }

    pub fn deserialize<'de, T: From<Affine>, D: Deserializer<'de>>(d: D) -> Result<T, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Affine;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a transform: an SVG transform string or [a, b, c, d, e, f]")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Affine, E> {
                super::parse_transform(v).map_err(E::custom)
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut a: A) -> Result<Affine, A::Error> {
                let mut m = [0.0; 6];
                for (i, x) in m.iter_mut().enumerate() {
                    *x = a.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(i, &self))?;
                }
                if a.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::invalid_length(7, &self));
                }
                Ok(Affine::new(m))
            }
            // kurbo writes the matrix as a newtype around the array.
            fn visit_newtype_struct<D: Deserializer<'de>>(self, d: D) -> Result<Affine, D::Error> {
                d.deserialize_any(self)
            }
        }
        d.deserialize_any(V).map(T::from)
    }
}

impl serde::Serialize for PathData {
    /// While the model is serialized as saved ([`crate::saved::saving`]): `{"d": …, "kinds"?: …}`
    /// when the notation holds the path exactly; else `{"subpaths": […]}`.
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        if crate::saved::svg_geometry()
            && let Some((d, kinds)) = path_data(self)
        {
            let mut st = s.serialize_struct("PathData", 1 + usize::from(kinds.is_some()))?;
            st.serialize_field("d", &d)?;
            match &kinds {
                Some(k) => st.serialize_field("kinds", k)?,
                None => st.skip_field("kinds")?,
            }
            return st.end();
        }
        let mut st = s.serialize_struct("PathData", 1)?;
        st.serialize_field("subpaths", &self.subpaths)?;
        st.end()
    }
}

impl<'de> serde::Deserialize<'de> for PathData {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = PathData;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a path: {\"d\": SVG path data, \"kinds\"?: …} or {\"subpaths\": […]}")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut a: A) -> Result<PathData, A::Error> {
                let (mut subpaths, mut d, mut kinds) = (None::<Vec<SubPath>>, None::<String>, None::<String>);
                while let Some(k) = a.next_key::<std::borrow::Cow<'de, str>>()? {
                    match k.as_ref() {
                        "subpaths" => subpaths = Some(a.next_value()?),
                        "d" => d = Some(a.next_value()?),
                        "kinds" => kinds = a.next_value()?,
                        _ => {
                            a.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }
                match (subpaths, d) {
                    (Some(_), Some(_)) => Err(serde::de::Error::custom("a path has `d` or `subpaths`, not both")),
                    (Some(subpaths), None) => match kinds {
                        Some(_) => Err(serde::de::Error::custom("`kinds` goes with `d`; `subpaths` give each anchor's kind")),
                        None => Ok(PathData { subpaths }),
                    },
                    (None, Some(d)) => parse_path_data(&d, kinds.as_deref()).map_err(serde::de::Error::custom),
                    (None, None) => Err(serde::de::Error::missing_field("d")),
                }
            }
        }
        d.deserialize_map(V)
    }
}

impl schemars::JsonSchema for PathData {
    fn schema_name() -> String {
        "PathData".into()
    }
    fn json_schema(g: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        use schemars::schema::{InstanceType, Schema, SchemaObject};
        let mut o = SchemaObject { instance_type: Some(InstanceType::Object.into()), ..Default::default() };
        let text = |description: &str| {
            let mut s = SchemaObject { instance_type: Some(InstanceType::String.into()), ..Default::default() };
            s.metadata().description = Some(description.into());
            Schema::Object(s)
        };
        let d = text(
            "The subpaths as SVG path data with absolute M, L, C and Z, as saved. Every SVG path command is read: relative ones, H and V, and S, Q, T and A, which become cubics (an arc one per quarter turn). On Z, a last anchor at exactly the first anchor's point ends the closing segment.",
        );
        let kinds = text(
            "With `d`, when some anchor is smooth: one letter per anchor, c (corner) or s (smooth), a space between subpaths. Without it every anchor is a corner except the joins the reader knows are smooth: inside an arc (A), and where S or T reflects a control point. With it, the letters count every anchor, an arc's included.",
        );
        let mut subpaths = g.subschema_for::<Vec<SubPath>>();
        if let Schema::Object(s) = &mut subpaths {
            s.metadata().description = Some("The subpaths anchor by anchor: what answers outside saved files give, and what a saved file holds when `d` cannot say a path exactly.".into());
        }
        o.object().properties.insert("d".into(), d);
        o.object().properties.insert("kinds".into(), kinds);
        o.object().properties.insert("subpaths".into(), subpaths);
        o.metadata().description = Some(
            "A path: `d` (with `kinds`) or `subpaths`. Saved files hold `d` (SVG path data) when it says the path exactly, else `subpaths`; other answers give `subpaths`. Either is read."
                .into(),
        );
        Schema::Object(o)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corner(x: f64, y: f64) -> Anchor {
        Anchor::corner(Point::new(x, y))
    }

    fn roundtrip(p: &PathData) -> PathData {
        let (d, kinds) = path_data(p).unwrap();
        parse_path_data(&d, kinds.as_deref()).unwrap()
    }

    #[test]
    fn a_square_is_four_moves_and_a_close() {
        let sq =
            PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.5), Point::new(0.0, 10.0)], true));
        assert_eq!(path_data(&sq), Some(("M0 0L10 0L10 10.5L0 10Z".into(), None)));
        assert_eq!(roundtrip(&sq), sq);
    }

    #[test]
    fn curves_closing_curves_and_smooth_kinds() {
        let a = Anchor { p: Point::new(0.0, 0.0), h_in: Point::new(-1.0, 1.0), h_out: Point::new(1.0, -1.0), kind: AnchorKind::Smooth };
        let b = Anchor { p: Point::new(10.0, 0.0), h_in: Point::new(9.0, -1.0), h_out: Point::new(10.0, 0.0), kind: AnchorKind::Corner };
        let p = PathData::new(vec![SubPath::new(vec![a, b], true), SubPath::new(vec![corner(1.0, 1.0), corner(2.0, 2.0)], false)]);
        let (d, kinds) = path_data(&p).unwrap();
        assert_eq!(d, "M0 0C1 -1 9 -1 10 0C10 0 -1 1 0 0ZM1 1L2 2");
        assert_eq!(kinds.as_deref(), Some("sc cc"));
        assert_eq!(roundtrip(&p), p);
    }

    #[test]
    fn an_anchor_on_the_start_point_survives_closing() {
        // A closed path whose last anchor sits on its first: the closing segment is written.
        let p = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(5.0, 0.0), Point::new(0.0, 0.0)], true));
        assert_eq!(path_data(&p).unwrap().0, "M0 0L5 0L0 0L0 0Z");
        assert_eq!(roundtrip(&p), p);
        let one = PathData::single(SubPath::polyline(&[Point::new(3.0, 4.0)], true));
        assert_eq!(path_data(&one).unwrap().0, "M3 4L3 4Z");
        assert_eq!(roundtrip(&one), one);
    }

    #[test]
    fn what_d_cannot_say_keeps_the_structured_form() {
        let dangling = Anchor { p: Point::new(0.0, 0.0), h_in: Point::new(-1.0, 0.0), h_out: Point::new(0.0, 0.0), kind: AnchorKind::Corner };
        assert_eq!(path_data(&PathData::single(SubPath::new(vec![dangling, corner(1.0, 0.0)], false))), None);
        assert_eq!(path_data(&PathData::single(SubPath::default())), None);
        assert_eq!(path_data(&PathData::single(SubPath::polyline(&[Point::new(f64::NAN, 0.0)], false))), None);
        assert_eq!(path_data(&PathData::default()), Some((String::new(), None)));
    }

    #[test]
    fn numbers_are_the_shortest_exact_text() {
        let text = |x: f64| {
            let mut s = String::new();
            push_number(&mut s, x);
            s
        };
        assert_eq!(text(1.0), "1");
        assert_eq!(text(-0.0), "-0");
        assert_eq!(text(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(text(1e-7), "1e-7");
        assert_eq!(text(100000.0), "1e5");
        assert_eq!(text(12345.0), "12345");
        assert_eq!(text(1e300).parse::<f64>().unwrap(), 1e300);
    }

    #[test]
    fn numbers_read_back_exactly_and_are_as_short_as_rusts_two_forms() {
        // Where two texts of the shortest length both read back, zmij and Rust may pick either.
        let shortest = |x: f64| x.to_string().len().min(format!("{x:e}").len());
        let text = |x: f64| {
            let mut s = String::new();
            push_number(&mut s, x);
            s
        };
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut xs = vec![0.0, -0.0, f64::MIN_POSITIVE, f64::MAX, f64::MIN, 5e-324, 0.1, 123.456, 1e21, 1e-5];
        xs.extend((-30..30).map(|p| 10f64.powi(p)));
        for _ in 0..100_000 {
            let r = next();
            xs.push(f64::from_bits(r));
            xs.push((r >> 11) as f64 / (1u64 << 40) as f64 * 2000.0 - 1000.0);
            xs.push(((r % 2_000_001) as f64 - 1_000_000.0) / 1000.0);
        }
        for x in xs.into_iter().filter(|x| x.is_finite()) {
            let t = text(x);
            assert_eq!(t.parse::<f64>().map(f64::to_bits), Ok(x.to_bits()), "{x:?} as {t}");
            assert_eq!(t.len(), shortest(x), "{x:?} as {t}");
            assert!(!t.contains('E') && !t.contains('+'), "{x:?} as {t}");
        }
    }

    #[test]
    fn hand_written_svg_reads() {
        let p = parse_path_data("m 10,10 h 5 v 5 l -5 0 z m 1 1 c 1,0 2,1 2,2", None).unwrap();
        let sq = SubPath::polyline(&[Point::new(10.0, 10.0), Point::new(15.0, 10.0), Point::new(15.0, 15.0), Point::new(10.0, 15.0)], true);
        assert_eq!(p.subpaths[0], sq);
        let c = &p.subpaths[1];
        assert_eq!(c.anchors[0].p, Point::new(11.0, 11.0));
        assert_eq!(c.anchors[0].h_out, Point::new(12.0, 11.0));
        assert_eq!(c.anchors[1].h_in, Point::new(13.0, 12.0));
        assert_eq!(c.anchors[1].p, Point::new(13.0, 13.0));
        // Implicit linetos after M, a closing anchor on the start merged, a command after Z.
        let p = parse_path_data("M0 0 10 0 10 10 0 0Z L 5 5", None).unwrap();
        assert_eq!(p.subpaths[0].anchors.len(), 3);
        assert!(p.subpaths[0].closed);
        assert_eq!(p.subpaths[1].anchors.iter().map(|a| a.p).collect::<Vec<_>>(), vec![Point::new(0.0, 0.0), Point::new(5.0, 5.0)]);
        for bad in ["L1 1", "M0 0R1 1", "M0 0L1", "M0 0Z 1 1", "Z", "M0 0 L1e400 0", "m1e308 0 l1e308 0", "M0 0A5 5 0 2 0 10 0", "M0 0A5 5 0"] {
            assert!(parse_path_data(bad, None).is_err(), "{bad}");
        }
        assert!(parse_path_data("M0 0L1 1", Some("cs c")).is_err());
        assert!(parse_path_data("M0 0L1 1", Some("cx")).is_err());
        assert!(parse_path_data("M0 0L1 1", Some("c")).is_err());
    }

    fn ends(p: &PathData) -> Vec<(Point, Point, Point)> {
        p.subpaths.iter().flat_map(|s| &s.anchors).map(|a| (a.h_in, a.p, a.h_out)).collect()
    }

    fn read(d: &str) -> PathData {
        parse_path_data(d, None).unwrap()
    }

    /// The cubic from `a` to `b` at `t`.
    fn at(a: &Anchor, b: &Anchor, t: f64) -> Point {
        kurbo::ParamCurve::eval(&kurbo::CubicBez::new(a.p, a.h_out, b.h_in, b.p), t)
    }

    #[test]
    fn quadratics_and_smooth_curves_read_as_svg_says() {
        // Q: handles two thirds of the way to the control point; q relative to the current point.
        let q = read("M0 0Q30 30 60 0");
        assert_eq!(ends(&q), ends(&read("M0 0q30 30 60 0")));
        assert_eq!(q.subpaths[0].anchors[0].h_out, Point::new(20.0, 20.0));
        assert_eq!(q.subpaths[0].anchors[1].h_in, Point::new(40.0, 20.0));
        // T reflects the last Q's control point (30 30) through (60 0) to (90 -30); t too; and a
        // T after a T reflects the control point that T implied (90 -30 through 120 0: 150 30).
        let t = read("M0 0Q30 30 60 0T120 0T180 0");
        assert_eq!(ends(&t), ends(&read("M0 0Q30 30 60 0Q90 -30 120 0Q150 30 180 0")));
        assert_eq!(ends(&t), ends(&read("m0 0q30 30 60 0t60 0t60 0")));
        // Implicit repeats: more pairs after T are more Ts.
        assert_eq!(ends(&t), ends(&read("M0 0Q30 30 60 0T120 0 180 0")));
        // S reflects the last C's second control point (50 10) through (60 0) to (70 -10); s
        // too; an S after an S reflects that S's (110 -10 through 120 0: 130 10); repeats too.
        let s = read("M0 0C10 10 50 10 60 0S110 -10 120 0S170 10 180 0");
        assert_eq!(ends(&s), ends(&read("M0 0C10 10 50 10 60 0C70 -10 110 -10 120 0C130 10 170 10 180 0")));
        assert_eq!(ends(&s), ends(&read("m0 0c10 10 50 10 60 0s50 -10 60 0s50 10 60 0")));
        assert_eq!(ends(&s), ends(&read("M0 0C10 10 50 10 60 0S110 -10 120 0 170 10 180 0")));
        // Nothing to reflect, so the first control point is the current point: S after a line or
        // a Q, T after a line or a C.
        for (d, start) in [("M0 0L10 0S20 10 30 0", 1), ("M0 0Q5 5 10 0S20 10 30 0", 1), ("M0 0L10 0T40 0", 1), ("M0 0C1 1 9 1 10 0T40 0", 1)] {
            let a = &read(d).subpaths[0].anchors;
            assert_eq!(a[start].h_out, a[start].p, "{d}");
        }
        // A T after a C is the quadratic whose control point is the current point: its handles lie on the chord, two thirds of the way back.
        let a = &read("M0 0C1 1 9 1 10 0T40 0").subpaths[0].anchors;
        assert_eq!((a[1].h_out, a[2].h_in), (Point::new(10.0, 0.0), Point::new(20.0, 0.0)));
    }

    #[test]
    fn arcs_read_as_one_cubic_per_quarter_turn_on_the_ellipse() {
        // (d, center, rx, ry, rotation in degrees, cubics)
        let cases = [
            ("M0 0A5 5 0 0 1 10 0", (5.0, 0.0), 5.0, 5.0, 0.0, 2),
            ("M0 0A5 5 0 0 0 10 0", (5.0, 0.0), 5.0, 5.0, 0.0, 2),
            // A quarter of a circle, the small and the large arc between the same points.
            ("M10 0A10 10 0 0 1 0 10", (0.0, 0.0), 10.0, 10.0, 0.0, 1),
            ("M10 0A10 10 0 1 0 0 10", (0.0, 0.0), 10.0, 10.0, 0.0, 3),
            // Elliptical, and turned 30 degrees.
            ("M20 0A20 10 0 0 1 -20 0", (0.0, 0.0), 20.0, 10.0, 0.0, 2),
            ("M17.32050807568877 10A20 10 30 0 1 -17.32050807568877 -10", (0.0, 0.0), 20.0, 10.0, 30.0, 2),
            // Radii too small to reach scale up (F.6.6): a half circle on the chord.
            ("M0 0A1 1 0 0 1 10 0", (5.0, 0.0), 5.0, 5.0, 0.0, 2),
            // A large arc of a small circle: three cubics, its center on the side it bulges to.
            ("M0 0A6 6 0 1 1 10 0", (5.0, -3.3166247903554), 6.0, 6.0, 0.0, 3),
        ];
        for (d, (cx, cy), rx, ry, rot, n) in cases {
            let p = read(d);
            let a = &p.subpaths[0].anchors;
            assert_eq!(a.len(), n + 1, "{d}: one cubic per quarter turn or part of one");
            let (sin, cos) = f64::to_radians(rot).sin_cos();
            // How far a point is off the ellipse, as a fraction of the radius.
            let off = |q: Point| {
                let (x, y) = (q.x - cx, q.y - cy);
                let (u, v) = ((cos * x + sin * y) / rx, (-sin * x + cos * y) / ry);
                (u.hypot(v) - 1.0).abs()
            };
            for w in a.windows(2) {
                for i in 0..=16 {
                    let q = at(&w[0], &w[1], f64::from(i) / 16.0);
                    assert!(off(q) < 3e-4, "{d}: {q:?} is {} off the ellipse", off(q));
                }
            }
            // The joins inside the arc are smooth, its ends are corners.
            let kinds: Vec<AnchorKind> = a.iter().map(|x| x.kind).collect();
            assert_eq!(kinds.first(), Some(&AnchorKind::Corner));
            assert_eq!(kinds.last(), Some(&AnchorKind::Corner));
            assert!(kinds[1..n].iter().all(|k| *k == AnchorKind::Smooth), "{d}");
        }
    }

    #[test]
    fn arcs_take_their_side_size_and_svgs_edge_cases() {
        // The sweep flag picks the side: sweep 1 from (0 0) to (10 0) runs through negative y.
        let up = read("M0 0A5 5 0 0 1 10 0");
        assert!(up.subpaths[0].anchors.iter().all(|a| a.p.y <= 1e-9));
        let down = read("M0 0A5 5 0 0 0 10 0");
        assert!(down.subpaths[0].anchors.iter().all(|a| a.p.y >= -1e-9));
        // The count follows from the arc alone, whatever its size.
        for r in ["3.6", "3.7", "0.001", "1e6"] {
            let d = format!("M0 0A{r} {r} 0 0 1 {} 0", 2.0 * r.parse::<f64>().unwrap());
            assert_eq!(read(&d).subpaths[0].anchors.len(), 3, "{d}");
        }
        // Ends exactly at the end point given; relative, flags without separators, implicit
        // repeats (a second arc after the first's numbers).
        assert_eq!(up.subpaths[0].anchors.last().unwrap().p, Point::new(10.0, 0.0));
        assert_eq!(ends(&up), ends(&read("m0 0a5 5 0 0110 0")));
        assert_eq!(ends(&read("M0 0A5 5 0 0 1 10 0A5 5 0 0 1 20 0")), ends(&read("M0 0A5 5 0 0 1 10 0 5 5 0 0 1 20 0")));
        // A zero radius is a line, and so is one at or below 1e-5 (kurbo's rule); one just past it
        // is not. An arc to its own start (-0 is 0) is no segment.
        assert_eq!(ends(&read("M0 0A0 5 0 0 1 10 0")), ends(&read("M0 0L10 0")));
        for d in ["M0 0A1e-5 5 0 0 1 10 0", "M0 0A5 1e-5 0 0 1 10 0", "M0 0A1e-6 1e-6 0 0 1 10 0"] {
            assert_eq!(read(d), read("M0 0L10 0"), "{d}");
        }
        let past = f64::from_bits(1e-5f64.to_bits() + 1);
        assert_eq!(read(&format!("M0 0A{past:e} {past:e} 0 0 1 10 0")).subpaths[0].anchors.len(), 3);
        assert_eq!(read("M0 0A5 5 0 0 1 0 0").subpaths[0].anchors.len(), 1);
        assert_eq!(read("M0 0A5 5 0 0 1 -0 0").subpaths[0].anchors.len(), 1);
        // A tiny span and a huge one: bounded, on the right points, nothing lost before it.
        let tiny = read("M0 0A5 5 0 0 1 1e-200 0");
        assert_eq!(tiny.subpaths[0].anchors.len(), 2);
        assert_eq!(tiny.subpaths[0].anchors[1].p, Point::new(1e-200, 0.0));
        let huge = read("M0 0L5 5A1 1 0 0 1 1e38 0");
        let a = &huge.subpaths[0].anchors;
        assert_eq!(a.len(), 4, "a line, then a half circle in two cubics");
        assert_eq!((a[0].p, a[1].p, a[3].p), (Point::new(0.0, 0.0), Point::new(5.0, 5.0), Point::new(1e38, 0.0)));
    }

    #[test]
    fn arcs_at_the_ends_of_the_number_range() {
        // A huge radius over a short span: the small arc is the chord (its sag is far below a
        // double's precision, and the far center's rounding does not throw its handles off it),
        // and the large arc goes round the huge circle; neither moves the anchors before it.
        let flat = read("M0 0L5 5A1e20 1e20 0 0 0 10 0");
        let a = &flat.subpaths[0].anchors;
        assert_eq!((a.len(), a[1].p, a[2].p), (3, Point::new(5.0, 5.0), Point::new(10.0, 0.0)));
        for h in [a[1].h_out, a[2].h_in] {
            assert!((h.x + h.y - 10.0).abs() < 1e-9 && (5.0..=10.0).contains(&h.x), "{h:?} is off the chord");
        }
        let round = read("M0 0L5 5A1e20 1e20 0 1 0 10 0");
        let a = &round.subpaths[0].anchors;
        assert_eq!((a.len(), a[1].p, a[5].p), (6, Point::new(5.0, 5.0), Point::new(10.0, 0.0)));
        assert!(a[3].p.to_vec2().hypot() > 1e20, "{:?}", a[3].p);
        let far = read("M0 0A1 1 0 0 1 1e100 0");
        assert_eq!((far.subpaths[0].anchors.len(), far.subpaths[0].anchors[0].p), (3, Point::ZERO));
        // Numbers that only an intermediate step could overflow or underflow read as SVG says:
        // a span far below the radii is its chord, radii too small to reach scale up from any
        // size, and far-apart ends are halved before they are subtracted.
        for (d, n) in [
            ("M0 0A1e20 1e20 0 0 0 1e-300 0", 1),
            ("M0 0A1e300 1e300 0 0 1 1e-10 0", 1),
            ("M0 0A5 5 0 0 1 1e-320 0", 1),
            ("M0 0A5 5 0 0 1 5e-324 0", 1),
            ("M0 0A1e-4 1e-4 0 0 1 10 0", 2),
            ("M-1e308 0A1e308 1e308 0 0 1 1e308 0", 2),
            ("M-1e308 0A1e-4 1e-4 0 0 1 1e308 0", 2),
        ] {
            let a = &read(d).subpaths[0].anchors;
            assert_eq!(a.len(), n + 1, "{d}");
        }
        assert_eq!(ends(&read("M0 0A1e-4 1e-4 0 0 1 10 0")), ends(&read("M0 0A5 5 0 0 1 10 0")));
        // A span zero but for rounding: the small arc is its chord (an `L`, -0 kept), the large
        // the whole turn round, by one rule whether or not d itself underflowed; ends a smallest
        // step apart have no direction and are the chord.
        assert_eq!(path_data(&read("M0 0A1e20 1e20 0 0 0 1e-300 0")).unwrap().0, "M0 0L1e-300 0");
        assert!(read("M0 0A1e20 1e20 0 0 1 -0 1e-300").subpaths[0].anchors[1].p.x.is_sign_negative());
        assert_eq!(read("M0 0A5 5 0 0 1 1e-320 0").subpaths[0].anchors.len(), 2);
        assert_eq!(read("M0 0A5 5 0 1 1 1e-320 0").subpaths[0].anchors.len(), 5);
        assert_eq!(read("M0 0A1e20 1e20 0 1 1 1e-300 0").subpaths[0].anchors.len(), 5);
        assert_eq!(read("M0 0A5 5 0 1 1 5e-324 0").subpaths[0].anchors.len(), 2);
        // A radius past half the largest number, whose points are finite though the vectors to
        // them are not.
        let big = read("M-1e308 0A1e308 1e308 0 1 1 0 1e308");
        assert!(big.subpaths[0].anchors.iter().all(|a| a.p.is_finite() && a.h_in.is_finite() && a.h_out.is_finite()));
        // A span of a millionth of a degree on a radius of 500: the small arc is the chord, with no
        // handles; the large one the whole turn round.
        let (s, c) = 1e-6f64.to_radians().sin_cos();
        let d = |large: u8| format!("M500 0A500 500 0 {large} 1 {} {}", 500.0 * c, 500.0 * s);
        let chord = read(&d(0));
        assert!(chord.subpaths[0].anchors.len() == 2 && chord.subpaths[0].anchors.iter().all(|a| a.h_in == a.p && a.h_out == a.p), "{chord:?}");
        // The whole turn round the circle on the far side, its anchors exactly at its quarter turns.
        let whole: Vec<Point> = read(&d(1)).subpaths[0].anchors.iter().map(|a| a.p).collect();
        assert_eq!(whole[..4], [Point::new(500.0, 0.0), Point::new(1000.0, -500.0), Point::new(1500.0, 0.0), Point::new(1000.0, 500.0)]);
        assert_eq!((whole.len(), whole[4]), (5, Point::new(500.0 * c, 500.0 * s)));
        // Only an ellipse past the largest number is refused (a half circle of radius 2.4e308),
        // never read as something else.
        assert!(parse_path_data("M-1.7e308 -1.7e308A1 1 0 0 1 1.7e308 1.7e308", None).is_err());
        let up = read("M0 0A5 5 0 0 1 10 0");
        // Quarter turns are exact: a half circle's middle point, and its handles' run along the
        // axes, carry no arithmetic noise into the document.
        assert_eq!(path_data(&up).unwrap().0, "M0 0C0 -2.761423749153967 2.238576250846033 -5 5 -5C7.761423749153967 -5 10 -2.761423749153967 10 0");
        assert_eq!(read("M0 0A5 5 450 0 1 10 0").subpaths[0].anchors[1].p, Point::new(5.0, -5.0));
        // Read once, written as L and C (the smooth joins as kinds), then exact from there on.
        let (d, kinds) = path_data(&up).unwrap();
        assert!(d.chars().all(|c| !"SQTA".contains(c)), "{d}");
        assert_eq!(kinds.as_deref(), Some("csc"));
        assert_eq!(roundtrip(&up), up);
    }

    #[test]
    fn arc_radii_out_of_range_are_lines_or_refused() {
        // Radii as far apart as the largest number allows read, on finite points ending where
        // they should, whichever way round (ratios 1e307 and 1e308); an infinite ratio is refused,
        // never read as something else; ends one point but for rounding are the chord whatever
        // the radii.
        for d in ["M0 0A1e-4 1e303 0 0 1 0 10", "M0 0A1e-4 1e304 0 0 1 0 10", "M0 0A1e303 1e-4 0 0 1 10 0", "M0 0A1e304 1e-4 0 0 1 10 0"] {
            let a = &read(d).subpaths[0].anchors;
            assert!(a.len() >= 2 && a.iter().all(|x| x.p.is_finite() && x.h_in.is_finite() && x.h_out.is_finite()), "{d}: {a:?}");
            assert!(a[a.len() - 1].p == Point::new(0.0, 10.0) || a[a.len() - 1].p == Point::new(10.0, 0.0), "{d}");
        }
        // (Scaled up across the short radius, 1e304 by 5e4 is past the largest number: refused.)
        assert!(parse_path_data("M0 0A1e304 1e-4 0 0 1 0 10", None).is_err());
        for d in ["M0 0A1e-4 1e305 0 0 1 0 10", "M0 0A1e305 1e-4 0 0 1 10 0", "M0 0A1.7e308 2e-5 0 0 1 10 0"] {
            assert!(parse_path_data(d, None).is_err(), "{d}");
        }
        for d in ["M0 0A1e-4 1e305 0 0 1 5e-324 0", "M0 0A1e305 1e-4 0 1 1 0 5e-324"] {
            let a = &read(d).subpaths[0].anchors;
            assert!(a.len() == 2 && a.iter().all(|x| x.h_in == x.p && x.h_out == x.p), "{d}: {a:?}");
        }
        // Subnormal radii, whose ratios and products left the number range (refusing some, and
        // throwing a middle point off the ellipse), are lines.
        for (d, line) in [
            ("M0 0A1e-10 5e-324 0 0 1 1e300 0", "M0 0L1e300 0"),
            ("M1e-3 0A1e-320 1e-10 0 0 1 -1e-3 0", "M1e-3 0L-1e-3 0"),
            ("M1e299 1e-300A1e-10 5e-324 0 0 1 -1e299 -1e-300", "M1e299 1e-300L-1e299 -1e-300"),
            ("M0 0A5e-324 1e-10 0 0 1 1e-300 1e300", "M0 0L1e-300 1e300"),
            ("M1e-323 10A5e-324 1 0 0 1 0 -10", "M1e-323 10L0 -10"),
        ] {
            assert_eq!(read(d), read(line), "{d}");
        }
    }

    #[test]
    fn small_radius_arcs_read_by_hand_as_kurbo_reads_them() {
        // An arc with a radius at or below 1e-5 is kurbo's line: by hand, the path data reads as
        // from_bezpath of kurbo's own reading, in every form (absolute, relative, to its own start,
        // after `Z`, between other segments, zero and the threshold itself).
        let radii = ["0 0", "0 5", "5 0", "1e-5 1e-5", "1e-5 3", "7 1e-5", "1e-6 1e-6", "5e-324 1", "-1e-5 2"];
        let tails = ["10 0", "0 0", "-3 4", "1e-300 0"];
        let mut checked = 0;
        for r in radii {
            for t in tails {
                for flags in ["0 0", "1 1"] {
                    for d in [
                        format!("M0 0A{r} 30 {flags} {t}"),
                        format!("M0 0a{r} 30 {flags} {t}"),
                        format!("M0 0L10 0A{r} 0 {flags} 10 0L20 0"),
                        format!("M1 2L5 5A{r} 0 {flags} {t}Z"),
                        format!("M1 2L5 5ZA{r} 0 {flags} {t}L9 9"),
                        format!("M1 2L5 5Za{r} 0 {flags} 0 0L9 9"),
                        format!("M3 3A{r} 0 {flags} 3 3A{r} 0 {flags} {t}"),
                    ] {
                        let kurbo = PathData::from_bezpath(&kurbo::BezPath::from_svg(&d).unwrap());
                        assert_eq!(parse_path_data_by_hand(&d).unwrap(), kurbo, "{d}");
                        checked += 1;
                    }
                }
            }
        }
        assert_eq!(checked, 9 * 4 * 2 * 7);
    }

    #[test]
    fn every_half_circle_between_whole_points_is_two_cubics_either_flag() {
        // A half circle's radius is half its chord, so d is 1 within rounding: the piece count
        // must not depend on which side of 1 that rounding falls, nor on the large-arc flag.
        for x in -12i32..=12 {
            for y in -12i32..=12 {
                if x == 0 && y == 0 {
                    continue;
                }
                let r = f64::from(x).hypot(f64::from(y)) / 2.0;
                for flags in ["0 0", "0 1", "1 0", "1 1"] {
                    let d = format!("M0 0A{r} {r} 0 {flags} {x} {y}");
                    let p = read(&d);
                    assert_eq!(p.subpaths[0].anchors.len(), 3, "{d}");
                    // So kinds written for it read back.
                    assert!(parse_path_data(&d, Some("csc")).is_ok(), "{d}");
                }
            }
        }
    }

    #[test]
    fn half_circles_far_from_the_origin_are_two_cubics_either_flag() {
        // Away from the origin d's rounding grows with the coordinates over the radius; the
        // half degree of grace keeps the count the span's.
        let d = "M732.2546341937651 122.63405067305253A1.1996023642006333 1.1996023642006333 0 1 0 731.2650635271451 120.44843102989681";
        assert_eq!(read(d).subpaths[0].anchors.len(), 3);
        for (cx, cy, r) in [(732.25, 122.6, 1.2), (-999.0, 513.5, 0.5), (1000.0, -1000.0, 20.5), (-417.3, -88.8, 7.7)] {
            for t in [0.0f64, 17.0, 33.3, 61.0, 89.0, 123.4, 200.0, 311.1] {
                let (s, c) = t.to_radians().sin_cos();
                let (x0, y0, x1, y1) = (cx + r * c, cy + r * s, cx - r * c, cy - r * s);
                for flags in ["0 0", "0 1", "1 0", "1 1"] {
                    let d = format!("M{x0} {y0}A{r} {r} 0 {flags} {x1} {y1}");
                    assert_eq!(read(&d).subpaths[0].anchors.len(), 3, "{d}");
                }
            }
        }
    }

    #[test]
    fn hand_written_rotations_load_as_before() {
        // Transforms keep `deg.to_radians()` off the quarter turns, negative and past a turn alike.
        for deg in [-30.0f64, 30.0, -1e-20, 390.0, -400.0] {
            let [_, b, ..] = parse_transform(&format!("rotate({deg})")).unwrap().as_coeffs();
            assert_eq!(b.to_bits(), deg.to_radians().sin().to_bits(), "{deg}");
        }
        assert_eq!(parse_transform("rotate(30) rotate(-30)").unwrap(), Affine::rotate(30f64.to_radians()) * Affine::rotate((-30f64).to_radians()));
    }

    #[test]
    fn path_data_counts_against_the_anchor_budget() {
        let lines = format!("M0 0{}", "L1 1".repeat(20));
        assert!(crate::path::with_anchor_budget(10, || parse_path_data(&lines, None)).is_err());
        assert!(crate::path::with_anchor_budget(21, || parse_path_data(&lines, None)).is_ok());
        // Arcs spend their cubics' anchors: three half circles are seven anchors.
        let arcs = "M0 0A5 5 0 0 1 10 0A5 5 0 0 1 20 0A5 5 0 0 1 30 0";
        assert!(crate::path::with_anchor_budget(6, || parse_path_data(arcs, None)).is_err());
        assert!(crate::path::with_anchor_budget(7, || parse_path_data(arcs, None)).is_ok());
        // Outside a budget, nothing is counted.
        assert!(parse_path_data(&lines, None).is_ok());
        // A closing anchor on the start that Z folds in is given back: the writer's own circle is
        // four anchors, a closed triangle three, and a command after Z starts from the start.
        let circle = path_data(&crate::shapes::ellipse(kurbo::Rect::new(0.0, 0.0, 10.0, 10.0))).unwrap().0;
        assert!(circle.ends_with("Z"), "{circle}");
        assert!(crate::path::with_anchor_budget(4, || parse_path_data(&circle, None)).is_ok(), "{circle}");
        assert!(crate::path::with_anchor_budget(3, || parse_path_data(&circle, None)).is_err());
        let triangle = "M0 0L10 0L10 10L0 0Z";
        assert!(crate::path::with_anchor_budget(3, || parse_path_data(triangle, None)).is_ok());
        assert!(crate::path::with_anchor_budget(2, || parse_path_data(triangle, None)).is_err());
        let twice = "M0 0L10 0L10 10L0 0ZL5 5L0 0Z";
        assert!(crate::path::with_anchor_budget(5, || parse_path_data(twice, None)).is_ok());
        assert!(crate::path::with_anchor_budget(4, || parse_path_data(twice, None)).is_err());
        let err = crate::path::with_anchor_budget(10, || parse_path_data(&lines, None)).unwrap_err();
        assert!(err.contains("the paths have more than") && err.contains("anchors"), "the budget's refusal: {err}");
        // A read within another gets no more than the outer has left, and is charged to it.
        let five = "M0 0L1 0L2 0L3 0L4 0";
        let nested = crate::path::with_anchor_budget(3, || crate::path::with_anchor_budget(100, || parse_path_data(five, None)));
        assert!(nested.unwrap_err().contains("the paths have more than"), "the outer read's three bind");
        let (inner, outer) = crate::path::with_anchor_budget(7, || {
            let inner = crate::path::with_anchor_budget(100, || parse_path_data(five, None));
            (inner, parse_path_data("M0 0L1 0L2 0", None))
        });
        assert!(inner.is_ok());
        assert!(outer.is_err(), "the outer read has two of its seven left after the inner one's five");
    }

    #[test]
    fn a_files_closing_point_folds_only_at_the_start_and_z_may_repeat() {
        // As the writer before the near case wrote them (83db73fa1: the closing segment left out
        // when straight and the last point is not the first bit for bit, written to exactly the
        // first point otherwise, `kinds` when some anchor is smooth): read as written, kinds and
        // all, and written back the same.
        for (d, kinds, counts) in [
            ("M0 0L10 0L10 10L0 5e-8Z", Some("cscc"), vec![4]),
            ("M0 0L10 0L10 10L0 5e-8Z", None, vec![4]),
            ("M0 0L10 0L10 10L-0 0Z", None, vec![4]),
            ("M0 0C0 5 10 5 10 0C10 -5 0 -5 0 0Z", Some("ss"), vec![2]),
            ("M0 0L10 0L10 10L1e-8 0ZM20 20L30 20L20 20.00000001Z", Some("ccsc csc"), vec![4, 3]),
            ("M0 0C1 2 3 4 5 5L0 1e-9Z", Some("csc"), vec![3]),
        ] {
            let p = parse_path_data(d, kinds).unwrap();
            assert_eq!(p.subpaths.iter().map(|s| s.anchors.len()).collect::<Vec<_>>(), counts, "{d}");
            assert_eq!(roundtrip(&p), p, "{d}");
        }
        // Written now: the closing segment is written when the last anchor is that near, and reads
        // back exactly.
        let mut near = SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0), Point::new(0.0, 5e-8)], true);
        near.anchors[3].h_in = Point::new(1.0, 1.0);
        let near = PathData::single(near);
        assert!(path_data(&near).unwrap().0.ends_with("L0 0Z"));
        assert_eq!(roundtrip(&near), near);
        // An arc back to its own start right after Z is left out, and so is the subpath it would
        // have opened; a segment after it opens one as before.
        assert_eq!(read("M0 0L10 0L10 10Z A5 5 0 0 1 0 0").subpaths.len(), 1);
        assert_eq!(path_data(&read("M0 0L10 0L10 10Z A5 5 0 0 1 0 0")).unwrap().0, "M0 0L10 0L10 10Z");
        assert_eq!(read("M0 0L10 0L10 10Z A5 5 0 0 1 0 0 L5 5").subpaths[1].anchors.len(), 2);
        assert_eq!(read("M0 0A5 5 0 0 1 0 0").subpaths[0].anchors.len(), 1, "a move's own subpath stays");
        // ZZ and a Z then another before a move: the second Z closes nothing and goes.
        assert_eq!(ends(&read("M0 0L10 0L10 10ZZ")), ends(&read("M0 0L10 0L10 10Z")));
        assert_eq!(read("M0 0L10 0L10 10z z m1 1").subpaths.len(), 2);
        assert!(parse_path_data("M0 0L10 0Z 5 5", None).is_err(), "numbers after Z are refused");
        assert!(parse_path_data("Z", None).is_err());
    }

    #[test]
    fn hand_written_path_data_folds_a_closing_point_a_rounding_error_off_as_from_bezpath() {
        // Relative data back at its start a rounding error off: a file keeps the closing point
        // (what `d` says, whatever its commands, as `kinds` written by hand counts it), data for a
        // command folds it within 1e-7, as from_bezpath does.
        let d = "m0.1 0.2c0.1 -0.2 0.3 0.1 0.2 0.3c-0.1 0.2 -0.2 -0.1 -0.2 -0.3z";
        assert_eq!(read(d).subpaths[0].anchors.len(), 3);
        assert_eq!(parse_path_data("m0 0l10 0l0 10l-10 -9.99999999z", Some("cccc")).unwrap().subpaths[0].anchors.len(), 4);
        assert_eq!(read("M0 0L10 0L10 10L0 1e-8A5 5 0 0 1 0 1e-8Z").subpaths[0].anchors.len(), 4, "an own-start arc changes nothing");
        let p = parse_path_data_by_hand(d).unwrap();
        assert_eq!(p.subpaths[0].anchors.len(), 2);
        assert_eq!(p, PathData::from_bezpath(&kurbo::BezPath::from_svg(d).unwrap()));
    }

    #[test]
    fn kinds_inferred_from_handles_are_from_bezpaths() {
        // Closed and open subpaths, an open one a move ends (kept corners, as from_bezpath keeps
        // them), a trailing open one (inferred), curves whose joins line up.
        for d in [
            "M0 5C0 2.24 2.24 0 5 0C7.76 0 10 2.24 10 5C10 7.76 7.76 10 5 10C2.24 10 0 7.76 0 5Z",
            "M0 0C1 1 9 1 10 0C11 -1 19 -1 20 0",
            "M0 0Q5 5 10 0Q15 -5 20 0",
            "M0 0C0 10 20 10 20 0C20 -10 40 -10 40 0M50 50L60 60",
            "M50 50L60 60M0 0C0 10 20 10 20 0C20 -10 40 -10 40 0",
            "M0 0C0 10 20 10 20 0C20 -10 40 -10 40 0ZM0 0C0 10 20 10 20 0C20 -10 40 -10 40 0M5 5L6 6Z",
            "M0 0L10 0L10 10ZL5 5C5 6 7 6 7 5C7 4 9 4 9 5M1 1L2 2",
        ] {
            let p = parse_path_data_by_hand(d).unwrap();
            assert_eq!(p, PathData::from_bezpath(&kurbo::BezPath::from_svg(d).unwrap()), "{d}");
        }
        // An open subpath a move ends is corners, as from_bezpath makes it, even where the reader
        // made an arc's, an S's or a T's joins smooth.
        for d in
            ["M0 0C0 10 20 10 20 0C20 -10 40 -10 40 0M50 50L60 60", "M0 0A5 5 0 0 1 10 0M50 50L60 60", "M0 0C0 10 20 10 20 0S40 -10 40 0M50 50L60 60"]
        {
            let mut p = read(d);
            p.infer_kinds_from_handles();
            assert!(p.subpaths[0].anchors.iter().all(|a| a.kind == AnchorKind::Corner), "{d}");
        }
    }

    #[test]
    fn kinds_count_an_arcs_anchors_and_override_its_smooth_joins() {
        // An arc of three quarter turns after a line: 1 + 1 + 3 anchors.
        let d = "M0 0L10 0A10 10 0 1 1 20 10";
        let p = read(d);
        let kinds: String = p.subpaths[0].anchors.iter().map(|a| if a.kind == AnchorKind::Smooth { 's' } else { 'c' }).collect();
        assert_eq!(kinds, "ccssc");
        let all_corners = parse_path_data(d, Some("ccccc")).unwrap();
        assert!(all_corners.subpaths[0].anchors.iter().all(|a| a.kind == AnchorKind::Corner));
        assert_eq!(ends(&all_corners), ends(&p));
        assert!(parse_path_data(d, Some("ccc")).is_err(), "the arc's inner anchors count");
    }

    #[test]
    fn a_reflected_join_is_smooth() {
        let kinds = |d: &str| read(d).subpaths[0].anchors.iter().map(|a| a.kind == AnchorKind::Smooth).collect::<Vec<_>>();
        assert_eq!(kinds("M0 0C10 10 50 10 60 0S110 -10 120 0"), [false, true, false]);
        assert_eq!(kinds("M0 0Q30 30 60 0T120 0"), [false, true, false]);
        // Nothing reflected (after a line, or a control point on the join): a corner.
        assert_eq!(kinds("M0 0L10 0S20 10 30 0"), [false, false, false]);
        assert_eq!(kinds("M0 0Q60 0 60 0T120 0"), [false, false, false]);
    }

    #[test]
    fn transforms_take_their_shortest_exact_form() {
        assert_eq!(transform(Affine::IDENTITY).unwrap(), "scale(1)");
        assert_eq!(transform(Affine::translate((10.0, 20.5))).unwrap(), "translate(10 20.5)");
        assert_eq!(transform(Affine::translate((10.0, 0.0))).unwrap(), "translate(10)");
        assert_eq!(transform(Affine::scale_non_uniform(2.0, -3.0)).unwrap(), "scale(2 -3)");
        assert_eq!(transform(Affine::new([0.0, 1.0, -1.0, 0.0, 0.0, 0.0])).unwrap(), "rotate(90)");
        assert_eq!(transform(Affine::new([0.0, -1.0, 1.0, 0.0, 0.0, 0.0])).unwrap(), "rotate(-90)");
        assert_eq!(transform(Affine::new([1.0, -0.0, 0.0, 1.0, 5.0, 0.0])).unwrap(), "matrix(1 -0 0 1 5 0)");
        assert_eq!(transform(Affine::new([2.0, 0.0, 0.0, 2.0, 1.0, 1.0])).unwrap(), "matrix(2 0 0 2 1 1)");
        assert_eq!(transform(Affine::new([f64::INFINITY, 0.0, 0.0, 1.0, 0.0, 0.0])), None);
        for a in [
            Affine::IDENTITY,
            Affine::translate((-0.0, 3.0)),
            Affine::rotate(0.3) * Affine::translate((1.1, 2.2)),
            Affine::new([0.0, 1.0, -1.0, 0.0, 0.0, 0.0]),
            Affine::new([-0.0, -0.0, -0.0, -0.0, -0.0, -0.0]),
        ] {
            let back = parse_transform(&transform(a).unwrap()).unwrap();
            assert_eq!(back.as_coeffs().map(f64::to_bits), a.as_coeffs().map(f64::to_bits), "{a:?}");
        }
    }

    #[test]
    fn transform_lists_read_as_svg_applies_them() {
        let a = parse_transform("translate(10, 20) rotate(90) scale(2)").unwrap();
        assert_eq!(a * Point::new(1.0, 0.0), Point::new(10.0, 22.0));
        assert_eq!(parse_transform("rotate(180 5 5)").unwrap() * Point::new(0.0, 0.0), Point::new(10.0, 10.0));
        assert!((parse_transform("skewX(45)").unwrap() * Point::new(0.0, 1.0)).x - 1.0 < 1e-12);
        for bad in ["", "translate", "translate(1 2 3)", "matrix(1 2)", "spin(3)", "scale(1", "scale(1e300) scale(1e300)"] {
            assert!(parse_transform(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn saved_paths_and_transforms_read_back_and_answers_stay_structured() {
        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct Holder {
            path: PathData,
            #[serde(with = "affine")]
            xf: Affine,
            #[serde(with = "affine")]
            boxed: Box<Affine>,
        }
        let h = Holder {
            path: PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(1.0, 2.0)], false)),
            xf: Affine::translate((3.0, 4.0)),
            boxed: Box::new(Affine::IDENTITY),
        };
        let saved = crate::saved::as_saved(|| serde_json::to_string(&h).unwrap());
        assert_eq!(saved, r#"{"path":{"d":"M0 0L1 2"},"xf":"translate(3 4)","boxed":"scale(1)"}"#);
        let answer = serde_json::to_string(&h).unwrap();
        assert!(answer.contains(r#""subpaths""#) && answer.contains("[1.0,0.0,0.0,1.0,3.0,4.0]"), "{answer}");
        for text in [saved, answer] {
            assert_eq!(serde_json::from_str::<Holder>(&text).unwrap(), h);
        }
        assert!(serde_json::from_str::<PathData>(r#"{"d":"M0 0","subpaths":[]}"#).is_err());
        assert!(serde_json::from_str::<PathData>(r#"{}"#).is_err());
    }
}
