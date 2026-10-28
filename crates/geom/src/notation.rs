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
//! the first, is left out when it is straight and the last anchor is not at the first anchor's
//! point; otherwise it is written, ending exactly at the first anchor's point. So a reader, on
//! `Z`, takes a last anchor at exactly the first anchor's point (same number text) for the end of
//! the closing segment, not an anchor: it drops it and gives its in handle to the first anchor.
//!
//! **Not written as `d`** (the structured `subpaths` form is kept): a subpath without anchors, an
//! open subpath whose first anchor has an in handle or whose last anchor has an out handle (SVG
//! has no segment to carry them), and coordinates that are not finite.
//!
//! **Anchor kinds.** `d` says nothing about how an anchor's handles behave when edited
//! ([`AnchorKind`]). Every anchor reads as a corner unless `kinds` is given; it is written only
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
//! and any whitespace between numbers, `E` and `+` in numbers, and a command after `Z` without
//! `M` (it starts at the subpath's first point). `S`, `Q`, `T` and `A` are refused. Transforms
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
fn same(a: Point, b: Point) -> bool {
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
            if !straight || same(last.p, first.p) {
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

/// Reads SVG path data (and anchor kinds, see the module documentation) into anchors.
pub fn parse_path_data(d: &str, kinds: Option<&str>) -> Result<PathData, String> {
    let mut path = PathData { subpaths: Parser { s: d.as_bytes(), i: 0 }.subpaths()? };
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

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
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
        loop {
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
                    _ => return Err(format!("a command (M, L, H, V, C or Z) was expected at byte {}", self.i)),
                }
            };
            command = Some(cmd);
            let rel = cmd.is_ascii_lowercase();
            match cmd.to_ascii_uppercase() {
                b'M' => {
                    if let Some(sp) = cur.take() {
                        out.push(sp);
                    }
                    at = self.pair(rel, at)?;
                    start = at;
                    cur = Some(SubPath { anchors: vec![Anchor::corner(at)], closed: false });
                }
                b'Z' => {
                    let Some(mut sp) = cur.take() else { return Err("`Z` closes nothing: a subpath starts with `M`".into()) };
                    if sp.anchors.len() > 1
                        && let (Some(first), Some(last)) = (sp.anchors.first(), sp.anchors.last())
                        && same(first.p, last.p)
                        && let Some(end) = sp.anchors.pop()
                        && let Some(first) = sp.anchors.first_mut()
                    {
                        first.h_in = end.h_in;
                    }
                    sp.closed = true;
                    out.push(sp);
                    at = start;
                }
                upper @ (b'L' | b'H' | b'V' | b'C') => {
                    if cur.is_none() {
                        if out.last().is_none_or(|s| !s.closed) {
                            return Err("path data starts with `M`".into());
                        }
                        // After `Z`, the next subpath starts where the closed one did.
                        cur = Some(SubPath { anchors: vec![Anchor::corner(start)], closed: false });
                    }
                    let Some(sp) = cur.as_mut() else { continue };
                    match upper {
                        b'L' => at = self.pair(rel, at)?,
                        b'H' => at.x = if rel { at.x + self.number()? } else { self.number()? },
                        b'V' => at.y = if rel { at.y + self.number()? } else { self.number()? },
                        _ => {
                            let from = at;
                            let c1 = self.pair(rel, from)?;
                            let c2 = self.pair(rel, from)?;
                            at = self.pair(rel, from)?;
                            if let Some(a) = sp.anchors.last_mut() {
                                a.h_out = c1;
                            }
                            sp.anchors.push(Anchor { p: at, h_in: c2, h_out: at, kind: AnchorKind::Corner });
                            continue;
                        }
                    }
                    sp.anchors.push(Anchor::corner(at));
                }
                _ => return Err(format!("`{}` is not read: path data here takes M, L, H, V, C and Z (either case)", char::from(cmd))),
            }
        }
        if let Some(sp) = cur {
            out.push(sp);
        }
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
    let mut p = Parser { s: s.as_bytes(), i: 0 };
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
            "The subpaths as SVG path data with absolute M, L, C and Z, as saved (relative commands, H and V are read too). On Z, a last anchor at exactly the first anchor's point ends the closing segment.",
        );
        let kinds = text(
            "With `d`, when some anchor is smooth: one letter per anchor, c (corner) or s (smooth), a space between subpaths. Without it every anchor is a corner.",
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
        for bad in ["L1 1", "M0 0Q1 1 2 2", "M0 0L1", "M0 0Z 1 1", "Z", "M0 0 L1e400 0", "m1e308 0 l1e308 0"] {
            assert!(parse_path_data(bad, None).is_err(), "{bad}");
        }
        assert!(parse_path_data("M0 0L1 1", Some("cs c")).is_err());
        assert!(parse_path_data("M0 0L1 1", Some("cx")).is_err());
        assert!(parse_path_data("M0 0L1 1", Some("c")).is_err());
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
