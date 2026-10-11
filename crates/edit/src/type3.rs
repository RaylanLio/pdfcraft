//! Type 3 replacement fonts drawn from craft-fonts faces: per-glyph outline charprocs with a
//! ToUnicode map for copy and search. Shared by retyped existing text (`text.rs`) and added text
//! in a script family (`added.rs`).
//!
//! No font file is embedded: each glyph's outline becomes its own CharProc, like the Japanese
//! replacement text always has.

use pdfcraft_cos::{Dict, Document, Object, PdfString, Stream};
use pdfcraft_fonts::{CraftFont, GlyphError, japanese_glyph_from};

use crate::EditError;

/// Most glyphs in one Type 3 font: its codes are single bytes, 1–240.
pub(crate) const MAX_TYPE3_GLYPHS: usize = 240;

/// Side bearing (em) given to a respaced glyph on each side.
const SIDE_BEARING: f64 = 0.05;

#[derive(Clone)]
pub(crate) struct Type3Fallback {
    /// The resource name the text operators select.
    pub(crate) name: String,
    /// The family its glyphs come from ("Shippori Mincho").
    pub(crate) family: &'static str,
    pub(crate) codes: Vec<(char, u8, f64)>,
}

/// The craft-fonts faces of one family, its `Bold` first when possible (no weight is invented:
/// a family without a bold face draws bold text from its Regular).
pub(crate) fn family_faces(family: &str, bold: bool) -> Vec<&'static CraftFont> {
    let mut faces: Vec<&CraftFont> = pdfcraft_fonts::CRAFT_FONTS.iter().filter(|f| f.family == family).collect();
    if bold {
        faces.sort_by_key(|f| f.style != "Bold");
    }
    faces
}

/// This build has no script face to draw the replacement text with.
pub(crate) fn no_script_font() -> EditError {
    EditError::Invalid(
        "this text needs a script font that this build doesn't include \
         (official releases do; to build it in, set CRAFT_FONTS_DIR to a craft-fonts checkout)"
            .into(),
    )
}

pub(crate) fn pdf_num(v: f64) -> String {
    if v.fract() == 0.0 { format!("{v:.0}") } else { format!("{v:.4}").trim_end_matches('0').trim_end_matches('.').to_string() }
}

/// The charproc body and advance (em) for one glyph of `face`, drawn from its outline.
fn type3_path(face: &CraftFont, ch: char) -> Result<(Vec<u8>, f64), EditError> {
    let glyph = japanese_glyph_from(face, ch).map_err(|e| match e {
        GlyphError::NoFont => no_script_font(),
        GlyphError::Missing => EditError::Invalid(format!("no fallback font has a glyph for U+{:04X}", ch as u32)),
        GlyphError::TooComplex => EditError::Invalid(format!("fallback glyph U+{:04X} is too complex", ch as u32)),
    })?;
    let (dx, width) = proportional(ch, &glyph);
    let scale = 1000.0;
    // d1 is `wx wy llx lly urx ury` (ISO 32000-2 §9.6.4) with a box enclosing the glyph;
    // Acrobat draws a bullet in place of a glyph whose d1 is malformed.
    let b = if glyph.contours.is_empty() { [0.0; 4] } else { glyph.bbox };
    let mut out = format!(
        "{} 0 {} {} {} {} d1\n",
        pdf_num(width * scale),
        pdf_num(((b[0] + dx) * scale).floor()),
        pdf_num((b[1] * scale).floor()),
        pdf_num(((b[2] + dx) * scale).ceil()),
        pdf_num((b[3] * scale).ceil())
    )
    .into_bytes();
    for contour in glyph.contours {
        let Some(first) = contour.first() else { continue };
        out.extend_from_slice(format!("{} {} m\n", pdf_num((first[0] + dx) * scale), pdf_num(first[1] * scale)).as_bytes());
        for p in contour.iter().skip(1) {
            out.extend_from_slice(format!("{} {} l\n", pdf_num((p[0] + dx) * scale), pdf_num(p[1] * scale)).as_bytes());
        }
        out.extend_from_slice(b"h\n");
    }
    out.extend_from_slice(b"f\n");
    Ok((out, width))
}

/// Shift and advance for a fallback glyph. Japanese faces draw Cyrillic and Greek full-width
/// (one em each, as in JIS X 0208) and have no proportional (`palt`) metrics for them, so a
/// word set that way reads as letter-spaced. Those letters get their ink width plus a side
/// bearing instead; everything else keeps the face's own advance.
fn proportional(ch: char, glyph: &pdfcraft_fonts::GlyphOutline) -> (f64, f64) {
    let respace = matches!(ch, '\u{0370}'..='\u{03FF}' | '\u{0400}'..='\u{052F}' | '\u{1F00}'..='\u{1FFF}');
    let ink = glyph.bbox[2] - glyph.bbox[0];
    if !respace || glyph.contours.is_empty() || !(ink > 0.0 && ink < glyph.width) {
        return (0.0, glyph.width);
    }
    (SIDE_BEARING - glyph.bbox[0], ink + 2.0 * SIDE_BEARING)
}

fn unicode_hex(ch: char) -> String {
    let mut units = [0u16; 2];
    let encoded = ch.encode_utf16(&mut units);
    encoded.iter().map(|u| format!("{u:04X}")).collect()
}

/// The first of `faces` (best match first) that has a glyph for every one of `chars`, else the first.
pub(crate) fn fallback_face<'a>(faces: &[&'a CraftFont], chars: &[char], has_glyph: impl Fn(&CraftFont, char) -> bool) -> Option<&'a CraftFont> {
    faces.iter().copied().find(|face| chars.iter().all(|ch| has_glyph(face, *ch))).or_else(|| faces.first().copied())
}

/// A Type 3 font with `text`'s distinct characters as codes 1…, each drawn from the first of
/// `faces` that has every character; registered in `fonts_res` under a name that neither it nor
/// `taken` has. `to_unicode` lets copy and search find each character.
pub(crate) fn type3_font(
    doc: &mut Document,
    fonts_res: &mut Dict,
    taken: &Dict,
    text: &str,
    faces: &[&CraftFont],
) -> Result<Type3Fallback, EditError> {
    let mut chars = Vec::new();
    for ch in text.chars() {
        if !chars.contains(&ch) {
            if chars.len() >= MAX_TYPE3_GLYPHS {
                return Err(EditError::Invalid("the text uses too many different glyphs".into()));
            }
            chars.push(ch);
        }
    }
    if chars.is_empty() {
        return Err(EditError::Invalid("the text is empty".into()));
    }
    // The faces differ in coverage (e.g. Simplified-only characters), so use the best face that
    // has every character. When none has them all, the best face reports the character it lacks.
    let face = fallback_face(faces, &chars, |face, ch| japanese_glyph_from(face, ch).is_ok()).ok_or_else(no_script_font)?;
    let family = face.family;
    let mut codes = Vec::with_capacity(chars.len());
    let mut charprocs = Dict::new();
    let mut widths = Vec::with_capacity(chars.len());
    let mut differences = vec![Object::Int(1)];
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CMapType 2 def\n1 begincodespacerange\n<01> <FF>\nendcodespacerange\n",
    );
    cmap.push_str(&format!("{} beginbfchar\n", chars.len()));
    for (i, ch) in chars.into_iter().enumerate() {
        let code = u8::try_from(i + 1).map_err(|_| EditError::Invalid("the text uses too many different glyphs".into()))?;
        let glyph_name = format!("g{code:02X}");
        let (path, width) = type3_path(face, ch)?;
        let mut pd = Dict::new();
        pd.set(b"Length".to_vec(), path.len() as i64);
        let proc_ref = doc.add(Object::Stream(Stream::from_raw(pd, path)));
        charprocs.set(glyph_name.as_bytes().to_vec(), Object::Ref(proc_ref));
        differences.push(Object::name(&glyph_name));
        widths.push(Object::Real((width * 1000.0).round()));
        cmap.push_str(&format!("<{code:02X}> <{}>\n", unicode_hex(ch)));
        codes.push((ch, code, width));
    }
    cmap.push_str("endbfchar\nendcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    let mut cmap_dict = Dict::new();
    cmap_dict.set(b"Length".to_vec(), cmap.len() as i64);
    let cmap_ref = doc.add(Object::Stream(Stream::from_raw(cmap_dict, cmap.into_bytes())));
    let mut encoding = Dict::new();
    encoding.set(b"Type".to_vec(), Object::name("Encoding"));
    encoding.set(b"Differences".to_vec(), Object::Array(differences));
    let mut font = Dict::new();
    font.set(b"Type".to_vec(), Object::name("Font"));
    font.set(b"Subtype".to_vec(), Object::name("Type3"));
    font.set(b"Name".to_vec(), Object::name("PCType3"));
    let mut descriptor = Dict::new();
    descriptor.set(b"Type".to_vec(), Object::name("FontDescriptor"));
    descriptor.set(b"FontName".to_vec(), Object::name(&format!("{}-{}", face.family, face.style).replace(' ', "")));
    descriptor.set(b"FontFamily".to_vec(), Object::String(PdfString::literal(face.family.as_bytes().to_vec())));
    descriptor.set(b"Flags".to_vec(), Object::Int(if face.family.contains("Mincho") { 6 } else { 4 }));
    descriptor.set(b"ItalicAngle".to_vec(), Object::Int(0));
    // PDF 1.7 tables 5.9 and 5.19: a Type 3 descriptor is indirect; Ascent/Descent may be omitted.
    font.set(b"FontDescriptor".to_vec(), Object::Ref(doc.add(Object::Dict(descriptor))));
    font.set(b"FontBBox".to_vec(), Object::Array(vec![Object::Int(0), Object::Int(-300), Object::Int(1000), Object::Int(1000)]));
    font.set(
        b"FontMatrix".to_vec(),
        Object::Array(vec![Object::Real(0.001), Object::Int(0), Object::Int(0), Object::Real(0.001), Object::Int(0), Object::Int(0)]),
    );
    font.set(b"FirstChar".to_vec(), Object::Int(1));
    font.set(b"LastChar".to_vec(), Object::Int(codes.len() as i64));
    font.set(b"Widths".to_vec(), Object::Array(widths));
    font.set(b"Encoding".to_vec(), Object::Dict(encoding));
    font.set(b"CharProcs".to_vec(), Object::Dict(charprocs));
    font.set(b"ToUnicode".to_vec(), Object::Ref(cmap_ref));
    let mut name = String::from("PCJp");
    let mut suffix = 0usize;
    while fonts_res.contains(name.as_bytes()) || taken.contains(name.as_bytes()) {
        suffix = suffix.saturating_add(1);
        name = format!("PCJp{suffix}");
    }
    fonts_res.set(name.as_bytes().to_vec(), Object::Dict(font));
    Ok(Type3Fallback { name, family, codes })
}

pub(crate) fn type3_encode(fallback: &Type3Fallback, text: &str) -> Option<Vec<u8>> {
    text.chars().map(|ch| fallback.codes.iter().find(|(c, _, _)| *c == ch).map(|(_, code, _)| *code)).collect()
}
