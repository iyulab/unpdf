//! Glyphs for the rasterizer: which glyph a font's code selects, and its outline.
//!
//! Selection follows ISO 32000-1 §9.6.6 (simple fonts) and §9.7.4.2 (CIDFonts), with the
//! fallbacks real files need: a symbolic TrueType font's codes through its `(3,0)` or `(1,0)`
//! cmap, a non-symbolic one's through the character its encoding gives the code, a bare
//! CFF or Type 1 font's through its glyph names or, with no `/Encoding`, its own encoding.
//!
//! TrueType and OpenType programs are read with `skrifa`, bare CFF ones with `read-fonts`'
//! PostScript reader, Type 1 ones with this crate's own interpreter.

use std::cell::OnceCell;
use std::collections::HashMap;

use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::ps::cff::CffFontRef;
use skrifa::raw::tables::cmap::PlatformId;
use skrifa::raw::TableProvider;
use skrifa::{FontRef, GlyphId, MetadataProvider, Tag};
use tiny_skia::{Path, PathBuilder, Transform};

use super::backend::{FontFormat, FontProgram, GlyphSelection};
use super::encoding::glyph_name_to_unicode;
use super::type1::{OutlineSink, Type1Font};

/// A font ready to paint: its program, how codes select glyphs, and outlines already built.
pub(super) struct LoadedFont {
    program: FontProgram,
    /// Glyph space → text space: `1 / unitsPerEm`, or a Type 1 font's `FontMatrix`.
    pub(super) glyph_to_text: Transform,
    /// For a CID-keyed CFF program (bare or in an OpenType font): CID → glyph index.
    cid_to_gid: Option<HashMap<u16, u16>>,
    /// For a bare CFF font: the character each glyph name stands for → glyph index.
    cff_by_char: Option<HashMap<char, u16>>,
    /// Glyph name → glyph index, built the first time a name is looked up.
    by_name: OnceCell<HashMap<String, u16>>,
    /// A Type 1 program, parsed once.
    type1: Option<Type1Font>,
    outlines: HashMap<u16, Option<Path>>,
}

impl LoadedFont {
    /// `None` when the program does not parse.
    pub(super) fn load(program: FontProgram) -> Option<Self> {
        let mut type1 = None;
        let (glyph_to_text, cid_to_gid, cff_by_char) = match program.format {
            FontFormat::Type1 => {
                let font = Type1Font::parse(&program.data)?;
                let [a, b, c, d, e, f] = font.font_matrix;
                type1 = Some(font);
                (Transform::from_row(a, b, c, d, e, f), None, None)
            }
            FontFormat::TrueType | FontFormat::OpenType => {
                let face = FontRef::new(&program.data).ok()?;
                let upem = f32::from(face.head().ok()?.units_per_em().max(1));
                let cid = face
                    .table_data(Tag::new(b"CFF "))
                    .and_then(|data| CffFontRef::new_cff(data.as_bytes(), 0, None).ok())
                    .and_then(|cff| cff_cid_map(&cff));
                (Transform::from_scale(1.0 / upem, 1.0 / upem), cid, None)
            }
            FontFormat::Cff => {
                // Outlines and advances come out in the font's units per em with its
                // `FontMatrix` (and a CID font's per-dictionary ones) already applied.
                let cff = CffFontRef::new_cff(&program.data, 0, None).ok()?;
                let upem = cff.upem().max(1) as f32;
                let by_char = cff_glyph_names(&cff)
                    .filter_map(|(gid, name)| Some((glyph_name_to_unicode(&name)?, gid)))
                    .fold(HashMap::new(), |mut map, (c, gid)| {
                        map.entry(c).or_insert(gid);
                        map
                    });
                let scale = Transform::from_scale(1.0 / upem, 1.0 / upem);
                (scale, cff_cid_map(&cff), Some(by_char))
            }
        };
        Some(Self {
            program,
            glyph_to_text,
            cid_to_gid,
            cff_by_char,
            by_name: OnceCell::new(),
            type1,
            outlines: HashMap::new(),
        })
    }

    /// How many bytes one code takes.
    pub(super) fn code_bytes(&self) -> usize {
        match self.program.selection {
            GlyphSelection::Cid { .. } => 2,
            GlyphSelection::Simple { .. } => 1,
        }
    }

    /// The glyph `code` selects, if the program has one for it.
    pub(super) fn glyph(&self, code: u32) -> Option<u16> {
        match &self.program.selection {
            GlyphSelection::Cid { cid_to_gid } => {
                let cid = u16::try_from(code).ok()?;
                if let Some(map) = &self.cid_to_gid {
                    return map.get(&cid).copied();
                }
                match cid_to_gid {
                    Some(table) => table.get(cid as usize).copied(),
                    None => Some(cid),
                }
            }
            GlyphSelection::Simple {
                chars,
                names,
                symbolic,
            } => {
                let code = u8::try_from(code).ok()?;
                let name = names.get(&code);
                let ch = name
                    .and_then(|n| glyph_name_to_unicode(n))
                    .or_else(|| chars.as_ref().and_then(|m| m.get(&code).copied()));
                match self.program.format {
                    // §9.6.6.2: a `/Differences` name, else what the font's `/Encoding`
                    // says the code is, else the program's own encoding.
                    FontFormat::Type1 => {
                        let font = self.type1.as_ref()?;
                        name.and_then(|n| font.glyph_by_name(n))
                            .or_else(|| {
                                chars
                                    .as_ref()
                                    .and_then(|m| m.get(&code))
                                    .and_then(|&c| font.glyph_by_char(c))
                            })
                            .or_else(|| font.glyph_by_builtin_code(code))
                    }
                    FontFormat::Cff => {
                        let cff = CffFontRef::new_cff(&self.program.data, 0, None).ok()?;
                        name.and_then(|n| self.glyph_by_name(n))
                            .or_else(|| {
                                ch.and_then(|c| self.cff_by_char.as_ref()?.get(&c).copied())
                            })
                            .or_else(|| {
                                let gid = cff.encoding()?.map(code)?;
                                u16::try_from(gid.to_u32()).ok()
                            })
                    }
                    FontFormat::TrueType | FontFormat::OpenType => {
                        let face = FontRef::new(&self.program.data).ok()?;
                        let by_unicode = || {
                            let gid = face.charmap().map(ch?)?;
                            u16::try_from(gid.to_u32()).ok()
                        };
                        let by_name = || name.and_then(|n| self.glyph_by_name(n));
                        let by_code = || symbolic_glyph(&face, code);
                        if *symbolic {
                            by_code().or_else(by_unicode).or_else(by_name)
                        } else {
                            by_unicode().or_else(by_name).or_else(by_code)
                        }
                    }
                }
            }
        }
    }

    /// The glyph the program names `name`, from its `post` table or CFF charset.
    fn glyph_by_name(&self, name: &str) -> Option<u16> {
        self.by_name
            .get_or_init(|| {
                let names: Vec<(u16, String)> = match self.program.format {
                    FontFormat::Cff => CffFontRef::new_cff(&self.program.data, 0, None)
                        .map(|cff| cff_glyph_names(&cff).collect())
                        .unwrap_or_default(),
                    FontFormat::TrueType | FontFormat::OpenType => FontRef::new(&self.program.data)
                        .map(|face| {
                            face.glyph_names()
                                .iter()
                                .filter(|(_, name)| !name.is_synthesized())
                                .filter_map(|(gid, name)| {
                                    let gid = u16::try_from(gid.to_u32()).ok()?;
                                    Some((gid, name.as_str().to_string()))
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    FontFormat::Type1 => Vec::new(),
                };
                // The first glyph with a name keeps it.
                names
                    .into_iter()
                    .rev()
                    .map(|(gid, name)| (name, gid))
                    .collect()
            })
            .get(name)
            .copied()
    }

    /// The glyph's advance in glyph space, from the program itself — for a font whose
    /// dictionary declares no widths.
    pub(super) fn program_advance(&self, gid: u16) -> Option<f32> {
        match self.program.format {
            FontFormat::Type1 => self.type1.as_ref()?.advance(gid),
            FontFormat::Cff => {
                let cff = CffFontRef::new_cff(&self.program.data, 0, None).ok()?;
                let id = GlyphId::new(u32::from(gid));
                let subfont = cff.subfont(cff.subfont_index(id)?, &[]).ok()?;
                cff.advance(&subfont, id, &[], None).ok()?
            }
            FontFormat::TrueType | FontFormat::OpenType => {
                let face = FontRef::new(&self.program.data).ok()?;
                face.glyph_metrics(Size::unscaled(), LocationRef::default())
                    .advance_width(GlyphId::new(u32::from(gid)))
            }
        }
    }

    /// The glyph's outline in glyph space; `None` for a glyph with none (a space).
    pub(super) fn outline(&mut self, gid: u16) -> Option<&Path> {
        if !self.outlines.contains_key(&gid) {
            let mut builder = Outline(PathBuilder::new());
            let id = GlyphId::new(u32::from(gid));
            let drawn = match self.program.format {
                FontFormat::Type1 => self
                    .type1
                    .as_ref()
                    .and_then(|font| font.outline(gid, &mut builder))
                    .is_some(),
                FontFormat::Cff => CffFontRef::new_cff(&self.program.data, 0, None)
                    .ok()
                    .and_then(|cff| {
                        let subfont = cff.subfont(cff.subfont_index(id)?, &[]).ok()?;
                        cff.draw(&subfont, id, &[], None, &mut builder).ok()
                    })
                    .is_some(),
                FontFormat::TrueType | FontFormat::OpenType => FontRef::new(&self.program.data)
                    .ok()
                    .and_then(|face| {
                        let glyph = face.outline_glyphs().get(id)?;
                        let settings =
                            DrawSettings::unhinted(Size::unscaled(), LocationRef::default());
                        glyph.draw(settings, &mut builder).ok()
                    })
                    .is_some(),
            };
            let path = if drawn { builder.0.finish() } else { None };
            self.outlines.insert(gid, path);
        }
        self.outlines.get(&gid)?.as_ref()
    }
}

/// A symbolic TrueType font's glyph for a one-byte code: its `(3,0)` cmap, where codes are
/// commonly stored at `0xF000 + code` (also `0xF100`, `0xF200`), then its `(1,0)` cmap.
fn symbolic_glyph(face: &FontRef<'_>, code: u8) -> Option<u16> {
    let cmap = face.cmap().ok()?;
    let code = u32::from(code);
    let in_subtable = |platform: PlatformId, encoding: u16, cp: u32| {
        cmap.encoding_records()
            .iter()
            .filter(|r| r.platform_id() == platform && r.encoding_id() == encoding)
            .filter_map(|r| r.subtable(cmap.offset_data()).ok())
            .find_map(|t| t.map_codepoint(cp))
            .and_then(|g| u16::try_from(g.to_u32()).ok())
    };
    [code, 0xF000 + code, 0xF100 + code, 0xF200 + code]
        .into_iter()
        .find_map(|cp| in_subtable(PlatformId::Windows, 0, cp))
        .or_else(|| in_subtable(PlatformId::Macintosh, 0, code))
}

/// A CFF program's glyph names, glyph by glyph — none for a CID-keyed one.
fn cff_glyph_names<'a>(cff: &CffFontRef<'a>) -> impl Iterator<Item = (u16, String)> + 'a {
    let cff = cff.clone();
    let charset = (!cff.is_cid()).then(|| cff.charset()).flatten();
    charset.into_iter().flat_map(move |charset| {
        let cff = cff.clone();
        charset.iter().filter_map(move |(gid, sid)| {
            let name = std::str::from_utf8(cff.string(sid)?).ok()?;
            Some((u16::try_from(gid.to_u32()).ok()?, name.to_string()))
        })
    })
}

/// For a CID-keyed CFF program, CID → glyph index (its charset read backwards).
fn cff_cid_map(cff: &CffFontRef<'_>) -> Option<HashMap<u16, u16>> {
    if !cff.is_cid() {
        return None;
    }
    Some(
        cff.charset()?
            .iter()
            .filter_map(|(gid, cid)| Some((cid.to_u16(), u16::try_from(gid.to_u32()).ok()?)))
            .collect(),
    )
}

/// Outline callbacks — the font readers' and the Type 1 interpreter's — building a tiny-skia
/// path in glyph space.
struct Outline(PathBuilder);

impl OutlineSink for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.0.cubic_to(x1, y1, x2, y2, x, y);
    }
    fn close(&mut self) {
        self.0.close();
    }
}

impl OutlinePen for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.0.quad_to(x1, y1, x, y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.0.cubic_to(x1, y1, x2, y2, x, y);
    }
    fn close(&mut self) {
        self.0.close();
    }
}
