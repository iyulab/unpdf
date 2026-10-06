//! Glyphs for the rasterizer: which glyph a font's code selects, and its outline.
//!
//! Selection follows ISO 32000-1 §9.6.6 (simple fonts) and §9.7.4.2 (CIDFonts), with the
//! fallbacks real files need: a symbolic TrueType font's codes through its `(3,0)` or `(1,0)`
//! cmap, a non-symbolic one's through the character its encoding gives the code, a bare
//! CFF font's through its glyph names or, with no `/Encoding`, its own encoding.

use std::collections::HashMap;

use tiny_skia::{Path, PathBuilder, Transform};

use super::backend::{FontFormat, FontProgram, GlyphSelection};
use super::encoding::glyph_name_to_unicode;

/// A font ready to paint: its program, how codes select glyphs, and outlines already built.
pub(super) struct LoadedFont {
    program: FontProgram,
    /// Glyph space → text space: `1 / unitsPerEm`, or a CFF font's `FontMatrix`.
    pub(super) glyph_to_text: Transform,
    /// For a CFF font with a CID charset: CID → glyph index.
    cid_to_gid: Option<HashMap<u16, u16>>,
    /// For a bare CFF font: the character each glyph name stands for → glyph index.
    cff_by_char: Option<HashMap<char, u16>>,
    outlines: HashMap<u16, Option<Path>>,
}

impl LoadedFont {
    /// `None` when the program does not parse.
    pub(super) fn load(program: FontProgram) -> Option<Self> {
        let (glyph_to_text, cid_to_gid, cff_by_char) = match program.format {
            FontFormat::TrueType | FontFormat::OpenType => {
                let face = ttf_parser::Face::parse(&program.data, 0).ok()?;
                let upem = f32::from(face.units_per_em().max(1));
                let cid = face.tables().cff.as_ref().and_then(cff_cid_map);
                (Transform::from_scale(1.0 / upem, 1.0 / upem), cid, None)
            }
            FontFormat::Cff => {
                let cff = ttf_parser::cff::Table::parse(&program.data)?;
                let m = cff.matrix();
                let matrix = Transform::from_row(m.sx, m.ky, m.kx, m.sy, m.tx, m.ty);
                let by_char = (0..cff.number_of_glyphs())
                    .filter_map(|gid| {
                        let name = cff.glyph_name(ttf_parser::GlyphId(gid))?;
                        Some((glyph_name_to_unicode(name)?, gid))
                    })
                    .fold(HashMap::new(), |mut map, (c, gid)| {
                        map.entry(c).or_insert(gid);
                        map
                    });
                (matrix, cff_cid_map(&cff), Some(by_char))
            }
        };
        Some(Self {
            program,
            glyph_to_text,
            cid_to_gid,
            cff_by_char,
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
                    FontFormat::Cff => {
                        let cff = ttf_parser::cff::Table::parse(&self.program.data)?;
                        name.and_then(|n| cff.glyph_index_by_name(n))
                            .map(|g| g.0)
                            .or_else(|| {
                                ch.and_then(|c| self.cff_by_char.as_ref()?.get(&c).copied())
                            })
                            .or_else(|| cff.glyph_index(code).map(|g| g.0))
                    }
                    FontFormat::TrueType | FontFormat::OpenType => {
                        let face = ttf_parser::Face::parse(&self.program.data, 0).ok()?;
                        let by_unicode = || ch.and_then(|c| face.glyph_index(c)).map(|g| g.0);
                        let by_name =
                            || name.and_then(|n| face.glyph_index_by_name(n)).map(|g| g.0);
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

    /// The glyph's advance in glyph space, from the program itself — for a font whose
    /// dictionary declares no widths.
    pub(super) fn program_advance(&self, gid: u16) -> Option<f32> {
        match self.program.format {
            FontFormat::Cff => {
                let cff = ttf_parser::cff::Table::parse(&self.program.data)?;
                cff.glyph_width(ttf_parser::GlyphId(gid)).map(f32::from)
            }
            _ => {
                let face = ttf_parser::Face::parse(&self.program.data, 0).ok()?;
                face.glyph_hor_advance(ttf_parser::GlyphId(gid))
                    .map(f32::from)
            }
        }
    }

    /// The glyph's outline in glyph space; `None` for a glyph with none (a space).
    pub(super) fn outline(&mut self, gid: u16) -> Option<&Path> {
        if !self.outlines.contains_key(&gid) {
            let mut builder = Outline(PathBuilder::new());
            let id = ttf_parser::GlyphId(gid);
            let drawn = match self.program.format {
                FontFormat::Cff => ttf_parser::cff::Table::parse(&self.program.data)
                    .and_then(|cff| cff.outline(id, &mut builder).ok())
                    .is_some(),
                _ => ttf_parser::Face::parse(&self.program.data, 0)
                    .ok()
                    .and_then(|face| face.outline_glyph(id, &mut builder))
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
fn symbolic_glyph(face: &ttf_parser::Face<'_>, code: u8) -> Option<u16> {
    let cmap = face.tables().cmap?;
    let code = u32::from(code);
    let in_subtable = |platform: ttf_parser::PlatformId, encoding: u16, cp: u32| {
        cmap.subtables
            .into_iter()
            .filter(|t| t.platform_id == platform && t.encoding_id == encoding)
            .find_map(|t| t.glyph_index(cp))
            .map(|g| g.0)
    };
    [code, 0xF000 + code, 0xF100 + code, 0xF200 + code]
        .into_iter()
        .find_map(|cp| in_subtable(ttf_parser::PlatformId::Windows, 0, cp))
        .or_else(|| in_subtable(ttf_parser::PlatformId::Macintosh, 0, code))
}

/// For a CID-keyed CFF font, CID → glyph index (its charset read backwards).
fn cff_cid_map(cff: &ttf_parser::cff::Table<'_>) -> Option<HashMap<u16, u16>> {
    cff.glyph_cid(ttf_parser::GlyphId(0))?;
    Some(
        (0..cff.number_of_glyphs())
            .filter_map(|gid| Some((cff.glyph_cid(ttf_parser::GlyphId(gid))?, gid)))
            .collect(),
    )
}

/// ttf-parser's outline callbacks, building a tiny-skia path in glyph space.
struct Outline(PathBuilder);

impl ttf_parser::OutlineBuilder for Outline {
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
