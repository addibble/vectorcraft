//! Image regions (IPTC Photo Metadata's Image Region, stored in XMP as `Iptc4xmpExt:ImageRegion`;
//! <https://iptc.org/std/photometadata/specification/IPTC-PhotoMetadata#image-region>): an image's
//! main subject, the region whose role is `mainSubjectArea`
//! (<https://cv.iptc.org/newscodes/imageregionrole/>), read from an XMP sidecar (the `.xmp` file of
//! the same name beside the image, where tools that won't rewrite a file keep its metadata) or
//! from the XMP inside the image: JPEG's APP1 packet, PNG's `iTXt` chunk `XML:com.adobe.xmp`,
//! WebP's `XMP ` chunk. A rectangle, circle or polygon boundary, in relative or pixel units, is
//! read as the rectangle around it, in fractions of the image.

use vectorcraft_doc::links::ImageRegion;

/// The namespace of IPTC's extension schema.
const IPTC_EXT: &str = "http://iptc.org/std/Iptc4xmpExt/2008-02-29/";
const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
/// The role a main subject's region has (its URI ends with it).
const MAIN_SUBJECT: &str = "mainSubjectArea";
/// The most XML nodes a packet is read with: real packets hold hundreds.
const MAX_NODES: u32 = 100_000;
/// The largest packet read: Adobe's own pad a packet to a few kilobytes.
const MAX_PACKET: usize = 4 << 20;
const JPEG_XMP: &[u8] = b"http://ns.adobe.com/xap/1.0/\0"; // brand-ok: the XMP marker JPEG files use

/// The main subject of an image `bytes` of `pixels` (width, height), from its sidecar `sidecar`
/// when it has one that names a main subject, else from its own XMP.
pub fn main_subject(bytes: &[u8], sidecar: Option<&[u8]>, pixels: (u32, u32)) -> Option<ImageRegion> {
    sidecar.and_then(|x| main_subject_in(x, pixels)).or_else(|| main_subject_in(&embedded_xmp(bytes)?, pixels))
}

/// The main subject an XMP packet names, as fractions of an image of `pixels` (for regions in
/// pixel units).
pub fn main_subject_in(xmp: &[u8], pixels: (u32, u32)) -> Option<ImageRegion> {
    if xmp.len() > MAX_PACKET {
        return None;
    }
    let text = std::str::from_utf8(xmp).ok()?.trim_start_matches('\u{feff}');
    let opts = roxmltree::ParsingOptions { nodes_limit: MAX_NODES, ..roxmltree::ParsingOptions::default() };
    let doc = roxmltree::Document::parse_with_options(text, opts).ok()?;
    let regions = doc.descendants().filter(|n| is(n, IPTC_EXT, "ImageRegion"));
    // Each region: an item of the bag in each ImageRegion.
    let items = regions.flat_map(|r| {
        r.descendants()
            .filter(move |n| is(n, RDF, "li") && n.parent().is_some_and(|p| is(&p, RDF, "Bag")) && n.parent().and_then(|p| p.parent()) == Some(r))
    });
    items.filter(|li| role_is_main(li)).find_map(|li| boundary(&structure(li), pixels))
}

/// `n` is element `name` of namespace `ns`.
fn is(n: &roxmltree::Node, ns: &str, name: &str) -> bool {
    n.is_element() && n.tag_name().name() == name && n.tag_name().namespace() == Some(ns)
}

/// The node holding a structure's fields: `n` itself (`rdf:parseType="Resource"`, or fields as
/// its attributes), or the `rdf:Description` inside it.
fn structure<'a, 'input>(n: roxmltree::Node<'a, 'input>) -> roxmltree::Node<'a, 'input> {
    n.children().find(|c| is(c, RDF, "Description")).unwrap_or(n)
}

/// Field `name` of IPTC's extension schema in structure `s`: an attribute, or a child element's
/// text.
fn text_field<'a>(s: &roxmltree::Node<'a, '_>, name: &str) -> Option<&'a str> {
    if let Some(v) = s.attributes().find(|a| a.name() == name && a.namespace() == Some(IPTC_EXT)) {
        return Some(v.value());
    }
    s.children().find(|c| is(c, IPTC_EXT, name)).and_then(|c| c.text()).map(str::trim)
}

/// Structure field `name` of `s`.
fn struct_field<'a, 'input>(s: &roxmltree::Node<'a, 'input>, name: &str) -> Option<roxmltree::Node<'a, 'input>> {
    s.children().find(|c| is(c, IPTC_EXT, name)).map(structure)
}

/// Whether region `li` has the main subject's role: an identifier in its roles ending with it.
fn role_is_main(li: &roxmltree::Node) -> bool {
    let region = structure(*li);
    let Some(roles) = region.children().find(|c| is(c, IPTC_EXT, "rRole")) else { return false };
    roles.descendants().any(|n| {
        let value = if n.is_text() { n.text() } else { n.attribute((RDF, "resource")) };
        value.is_some_and(|v| v.trim().trim_end_matches('/').ends_with(MAIN_SUBJECT))
    })
}

/// The rectangle around region `region`'s boundary, in fractions of an image of `pixels`.
fn boundary(region: &roxmltree::Node, pixels: (u32, u32)) -> Option<ImageRegion> {
    let b = struct_field(region, "RegionBoundary")?;
    let num = |n: &roxmltree::Node, k: &str| text_field(n, k).and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite());
    // Pixel units are fractions of the image's pixels; relative ones already are.
    let (sx, sy) = match text_field(&b, "rbUnit").unwrap_or("relative") {
        "relative" => (1.0, 1.0),
        "pixel" if pixels.0 > 0 && pixels.1 > 0 => (pixels.0 as f64, pixels.1 as f64),
        _ => return None,
    };
    let (l, t, r, bo) = match text_field(&b, "rbShape").unwrap_or("rectangle") {
        "rectangle" => {
            let (x, y) = (num(&b, "rbX")?, num(&b, "rbY")?);
            (x, y, x + num(&b, "rbW")?, y + num(&b, "rbH")?)
        }
        // The radius is in the image's width units: pixels down as across, or, relative, the
        // width's fraction (the height's is that times the aspect ratio).
        "circle" => {
            let (x, y, rx) = (num(&b, "rbX")?, num(&b, "rbY")?, num(&b, "rbRx")?);
            let ry = if sx > 1.0 || pixels.1 == 0 { rx } else { rx * pixels.0 as f64 / pixels.1 as f64 };
            (x - rx, y - ry, x + rx, y + ry)
        }
        "polygon" => {
            let vertices = struct_field(&b, "rbVertices")?;
            let points: Vec<(f64, f64)> = vertices
                .descendants()
                .filter(|n| is(n, RDF, "li"))
                .filter_map(|li| {
                    let p = structure(li);
                    Some((num(&p, "rbX")?, num(&p, "rbY")?))
                })
                .collect();
            if points.len() < 3 {
                return None;
            }
            let fold = |f: fn(f64, f64) -> f64, k: fn(&(f64, f64)) -> f64, start: f64| points.iter().map(k).fold(start, f);
            (
                fold(f64::min, |p| p.0, f64::INFINITY),
                fold(f64::min, |p| p.1, f64::INFINITY),
                fold(f64::max, |p| p.0, f64::NEG_INFINITY),
                fold(f64::max, |p| p.1, f64::NEG_INFINITY),
            )
        }
        _ => return None,
    };
    ImageRegion::checked(l / sx, t / sy, r / sx, bo / sy)
}

/// The XMP packet inside image `bytes` (JPEG, PNG or WebP), if it has one.
pub fn embedded_xmp(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.starts_with(&[0xff, 0xd8]) {
        jpeg_xmp(bytes)
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        png_xmp(bytes)
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        webp_xmp(bytes)
    } else {
        None
    }
}

fn jpeg_xmp(b: &[u8]) -> Option<Vec<u8>> {
    let mut at = 2usize;
    loop {
        let (&ff, &marker) = (b.get(at)?, b.get(at + 1)?);
        // Start of scan or end of image: no more metadata segments.
        if ff != 0xff || marker == 0xda || marker == 0xd9 {
            return None;
        }
        let len = u16::from_be_bytes([*b.get(at + 2)?, *b.get(at + 3)?]) as usize;
        let data = b.get(at + 4..(at + 2).checked_add(len)?)?;
        if marker == 0xe1 && data.starts_with(JPEG_XMP) {
            return data.get(JPEG_XMP.len()..).map(<[u8]>::to_vec);
        }
        at = at.checked_add(2 + len)?;
    }
}

fn png_xmp(b: &[u8]) -> Option<Vec<u8>> {
    const KEY: &[u8] = b"XML:com.adobe.xmp\0"; // brand-ok: the iTXt keyword PNG files use
    let (_, data) = super::ppi::png_chunks(b).take_while(|(ty, _)| *ty != b"IEND").find(|(ty, d)| *ty == b"iTXt" && d.starts_with(KEY))?;
    // Compression flag and method, then the language tag and translated keyword, each ended by a
    // NUL; then the text. XMP's packet is stored uncompressed, so a compressed one is not read.
    let rest = data.get(KEY.len()..)?;
    if rest.first() != Some(&0) {
        return None;
    }
    let rest = rest.get(2..)?;
    let lang_end = rest.iter().position(|&c| c == 0)?;
    let rest = rest.get(lang_end + 1..)?;
    let translated_end = rest.iter().position(|&c| c == 0)?;
    rest.get(translated_end + 1..).map(<[u8]>::to_vec)
}

fn webp_xmp(b: &[u8]) -> Option<Vec<u8>> {
    let mut at = 12usize;
    while let Some(head) = b.get(at..at + 8) {
        let len = u32::from_le_bytes(head.get(4..8)?.try_into().ok()?) as usize;
        let data = b.get(at + 8..(at + 8).checked_add(len)?)?;
        if head.starts_with(b"XMP ") {
            return Some(data.to_vec());
        }
        // Chunks are padded to an even length.
        at = at.checked_add(8 + len + (len & 1))?;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(region: &str) -> String {
        format!(
            r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?><x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="{RDF}"><rdf:Description rdf:about="" xmlns:Iptc4xmpExt="{IPTC_EXT}"><Iptc4xmpExt:ImageRegion><rdf:Bag>{region}</rdf:Bag></Iptc4xmpExt:ImageRegion></rdf:Description></rdf:RDF></x:xmpmeta><?xpacket end="r"?>"#
        )
    }

    fn region(role: &str, boundary: &str) -> String {
        format!(
            r#"<rdf:li rdf:parseType="Resource"><Iptc4xmpExt:RegionBoundary rdf:parseType="Resource">{boundary}</Iptc4xmpExt:RegionBoundary><Iptc4xmpExt:rRole><rdf:Bag><rdf:li rdf:parseType="Resource"><Iptc4xmpExt:Identifier><rdf:Bag><rdf:li>http://cv.iptc.org/newscodes/imageregionrole/{role}</rdf:li></rdf:Bag></Iptc4xmpExt:Identifier></rdf:li></rdf:Bag></Iptc4xmpExt:rRole></rdf:li>"#
        )
    }

    const RECT: &str = "<Iptc4xmpExt:rbShape>rectangle</Iptc4xmpExt:rbShape><Iptc4xmpExt:rbUnit>relative</Iptc4xmpExt:rbUnit><Iptc4xmpExt:rbX>0.25</Iptc4xmpExt:rbX><Iptc4xmpExt:rbY>0.1</Iptc4xmpExt:rbY><Iptc4xmpExt:rbW>0.5</Iptc4xmpExt:rbW><Iptc4xmpExt:rbH>0.4</Iptc4xmpExt:rbH>";

    fn near(r: Option<ImageRegion>, want: [f64; 4]) {
        let r = r.expect("a region");
        let got = [r.left, r.top, r.right, r.bottom];
        assert!(got.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-9), "{got:?} != {want:?}");
    }

    #[test]
    fn the_main_subject_is_the_region_with_its_role() {
        let xmp = packet(&(region("subjectArea", RECT.replace("0.25", "0.6").as_str()) + &region("mainSubjectArea", RECT)));
        near(main_subject_in(xmp.as_bytes(), (100, 100)), [0.25, 0.1, 0.75, 0.5]);
        assert_eq!(main_subject_in(packet(&region("cropping", RECT)).as_bytes(), (100, 100)), None);
    }

    #[test]
    fn fields_as_attributes_and_pixel_units_read_too() {
        let li = r#"<rdf:li><rdf:Description><Iptc4xmpExt:RegionBoundary Iptc4xmpExt:rbShape="rectangle" Iptc4xmpExt:rbUnit="pixel" Iptc4xmpExt:rbX="50" Iptc4xmpExt:rbY="20" Iptc4xmpExt:rbW="100" Iptc4xmpExt:rbH="40"/><Iptc4xmpExt:rRole><rdf:Bag><rdf:li rdf:resource="http://cv.iptc.org/newscodes/imageregionrole/mainSubjectArea"/></rdf:Bag></Iptc4xmpExt:rRole></rdf:Description></rdf:li>"#;
        near(main_subject_in(packet(li).as_bytes(), (200, 80)), [0.25, 0.25, 0.75, 0.75]);
    }

    #[test]
    fn a_circle_and_a_polygon_are_the_rectangles_around_them() {
        let circle = "<Iptc4xmpExt:rbShape>circle</Iptc4xmpExt:rbShape><Iptc4xmpExt:rbUnit>pixel</Iptc4xmpExt:rbUnit><Iptc4xmpExt:rbX>100</Iptc4xmpExt:rbX><Iptc4xmpExt:rbY>50</Iptc4xmpExt:rbY><Iptc4xmpExt:rbRx>20</Iptc4xmpExt:rbRx>";
        near(main_subject_in(packet(&region("mainSubjectArea", circle)).as_bytes(), (200, 100)), [0.4, 0.3, 0.6, 0.7]);
        let vertex = |x: f64, y: f64| {
            format!(r#"<rdf:li rdf:parseType="Resource"><Iptc4xmpExt:rbX>{x}</Iptc4xmpExt:rbX><Iptc4xmpExt:rbY>{y}</Iptc4xmpExt:rbY></rdf:li>"#)
        };
        let polygon = format!(
            "<Iptc4xmpExt:rbShape>polygon</Iptc4xmpExt:rbShape><Iptc4xmpExt:rbUnit>relative</Iptc4xmpExt:rbUnit><Iptc4xmpExt:rbVertices><rdf:Seq>{}{}{}</rdf:Seq></Iptc4xmpExt:rbVertices>",
            vertex(0.2, 0.3),
            vertex(0.6, 0.1),
            vertex(0.5, 0.8)
        );
        near(main_subject_in(packet(&region("mainSubjectArea", &polygon)).as_bytes(), (10, 10)), [0.2, 0.1, 0.6, 0.8]);
    }

    #[test]
    fn a_flat_or_unknown_region_and_broken_xml_are_none() {
        assert_eq!(main_subject_in(packet(&region("mainSubjectArea", &RECT.replace("0.5<", "0<"))).as_bytes(), (1, 1)), None);
        assert_eq!(main_subject_in(packet(&region("mainSubjectArea", &RECT.replace("rectangle", "star"))).as_bytes(), (1, 1)), None);
        assert_eq!(main_subject_in(b"<x:xmpmeta", (1, 1)), None);
        assert_eq!(main_subject_in(&[0xff, 0xfe, 0x00], (1, 1)), None);
    }

    #[test]
    fn the_sidecar_comes_before_the_image_s_own() {
        let own = packet(&region("mainSubjectArea", RECT));
        let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe1];
        let len = (2 + JPEG_XMP.len() + own.len()) as u16;
        jpeg.extend(len.to_be_bytes());
        jpeg.extend(JPEG_XMP);
        jpeg.extend(own.as_bytes());
        jpeg.extend([0xff, 0xda, 0, 2]);
        near(main_subject(&jpeg, None, (1, 1)), [0.25, 0.1, 0.75, 0.5]);
        let sidecar = packet(&region("mainSubjectArea", &RECT.replace("0.25", "0.4")));
        near(main_subject(&jpeg, Some(sidecar.as_bytes()), (1, 1)), [0.4, 0.1, 0.9, 0.5]);
        // A sidecar naming no main subject leaves the image's own.
        near(main_subject(&jpeg, Some(b"<x/>"), (1, 1)), [0.25, 0.1, 0.75, 0.5]);
    }

    #[test]
    fn png_and_webp_packets_are_found() {
        let own = packet(&region("mainSubjectArea", RECT));
        let mut itxt = b"XML:com.adobe.xmp\0\0\0\0\0".to_vec();
        itxt.extend(own.as_bytes());
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend((itxt.len() as u32).to_be_bytes());
        png.extend(b"iTXt");
        png.extend(&itxt);
        png.extend([0; 4]);
        assert_eq!(embedded_xmp(&png).as_deref(), Some(own.as_bytes()));
        let mut webp = b"RIFF\0\0\0\0WEBP".to_vec();
        webp.extend(b"VP8X");
        webp.extend(10u32.to_le_bytes());
        webp.extend([0; 10]);
        webp.extend(b"XMP ");
        webp.extend((own.len() as u32).to_le_bytes());
        webp.extend(own.as_bytes());
        assert_eq!(embedded_xmp(&webp).as_deref(), Some(own.as_bytes()));
        assert_eq!(embedded_xmp(b"GIF89a"), None);
    }
}
