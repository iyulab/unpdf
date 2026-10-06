//! Type 1 font programs: the `/FontFile` a font descriptor embeds (ISO 32000-1 §9.9), read
//! as the Adobe Type 1 Font Format (1990) defines them.
//!
//! A program is a PostScript cleartext part — `/FontMatrix`, the built-in `/Encoding` — then,
//! after `eexec`, a part encrypted with key 55665 holding the private dictionary: `/Subrs`
//! and `/CharStrings`, each charstring encrypted again with key 4330. A charstring is a small
//! stack program that draws a glyph's outline: [`Type1Font::outline`] runs it, `seac`
//! accented characters, `callsubr` subroutines and the flex mechanism included. Hints only
//! steer rasterization at small sizes and are skipped.
//!
//! Text extraction reads only the cleartext part — the built-in encoding
//! ([`builtin_encoding_chars`]); the outlines are for the rasterizer.
#![cfg_attr(not(feature = "raster"), allow(dead_code))]

use std::collections::HashMap;

use super::encoding::{glyph_name_to_unicode, BaseEncoding};

/// Where an outline goes: glyph-space path operations, as a charstring draws them.
pub(crate) trait OutlineSink {
    fn move_to(&mut self, x: f32, y: f32);
    fn line_to(&mut self, x: f32, y: f32);
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32);
    fn close(&mut self);
}

/// A program's built-in encoding: what each code is when the PDF font gives no `/Encoding`.
#[derive(Debug, Clone, PartialEq)]
enum BuiltinEncoding {
    Standard,
    Custom(HashMap<u8, String>),
}

/// A parsed Type 1 program. Glyphs are numbered in the order the program defines them.
#[derive(Debug, Clone)]
pub(crate) struct Type1Font {
    /// Glyph space → text space; `[0.001 0 0 0.001 0 0]` unless the program says otherwise.
    pub font_matrix: [f32; 6],
    encoding: BuiltinEncoding,
    charstrings: Vec<Vec<u8>>,
    by_name: HashMap<String, u16>,
    by_char: HashMap<char, u16>,
    subrs: Vec<Vec<u8>>,
}

/// Charstrings nest subroutine calls and `seac` components; deeper than this is a loop.
const MAX_CALL_DEPTH: usize = 16;

impl Type1Font {
    /// `None` when `data` is not a Type 1 program this reads.
    pub fn parse(data: &[u8]) -> Option<Self> {
        let eexec = find(data, b"eexec")?;
        let clear = &data[..eexec];
        let font_matrix = font_matrix(clear).unwrap_or([0.001, 0.0, 0.0, 0.001, 0.0, 0.0]);
        let encoding = builtin_encoding(clear);

        let private = decrypt(&encrypted_part(&data[eexec + 5..]), 55665, 4);
        let len_iv = integer_after(&private, b"/lenIV").unwrap_or(4);
        let (subrs, glyphs) = private_programs(&private);
        let decrypt_cs = |cs: Vec<u8>| match usize::try_from(len_iv) {
            Ok(skip) => decrypt(&cs, 4330, skip),
            Err(_) => cs,
        };
        let subrs: Vec<Vec<u8>> = subrs.into_iter().map(decrypt_cs).collect();
        let (names, charstrings): (Vec<String>, Vec<Vec<u8>>) = glyphs
            .into_iter()
            .map(|(name, cs)| (name, decrypt_cs(cs)))
            .unzip();
        if names.is_empty() || names.len() > usize::from(u16::MAX) {
            return None;
        }
        let by_name: HashMap<String, u16> = names
            .iter()
            .enumerate()
            .map(|(gid, name)| (name.clone(), gid as u16))
            .collect();
        let mut by_char = HashMap::new();
        for (gid, name) in names.iter().enumerate() {
            if let Some(c) = glyph_name_to_unicode(name) {
                by_char.entry(c).or_insert(gid as u16);
            }
        }
        Some(Self {
            font_matrix,
            encoding,
            charstrings,
            by_name,
            by_char,
            subrs,
        })
    }

    /// The glyph named `name`.
    pub fn glyph_by_name(&self, name: &str) -> Option<u16> {
        self.by_name.get(name).copied()
    }

    /// The glyph whose name stands for `c`.
    pub fn glyph_by_char(&self, c: char) -> Option<u16> {
        self.by_char.get(&c).copied()
    }

    /// The glyph the program's own encoding gives `code`.
    pub fn glyph_by_builtin_code(&self, code: u8) -> Option<u16> {
        match &self.encoding {
            BuiltinEncoding::Custom(map) => self.glyph_by_name(map.get(&code)?),
            BuiltinEncoding::Standard => {
                self.glyph_by_char(BaseEncoding::Standard.decode_char(code)?)
            }
        }
    }

    /// The glyph's advance width in glyph space, as its `hsbw`/`sbw` sets it.
    pub fn advance(&self, gid: u16) -> Option<f32> {
        let mut sink = NoOutline;
        let mut run = Interpreter::new(self, &mut sink);
        run.glyph(gid, 0.0, 0.0, 0)?;
        run.width
    }

    /// Draw the glyph's outline into `sink`, in glyph space. `None` when the glyph does not
    /// exist or its charstring cannot be run.
    pub fn outline(&self, gid: u16, sink: &mut dyn OutlineSink) -> Option<()> {
        let mut run = Interpreter::new(self, sink);
        run.glyph(gid, 0.0, 0.0, 0)
    }
}

/// An outline sink that discards everything — for reading a glyph's metrics.
struct NoOutline;

impl OutlineSink for NoOutline {
    fn move_to(&mut self, _: f32, _: f32) {}
    fn line_to(&mut self, _: f32, _: f32) {}
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
    fn close(&mut self) {}
}

/// Runs charstrings. One interpreter draws one glyph, `seac` components included.
struct Interpreter<'a> {
    font: &'a Type1Font,
    sink: &'a mut dyn OutlineSink,
    stack: Vec<f32>,
    /// What `callothersubr` leaves for `pop`.
    ps_stack: Vec<f32>,
    x: f32,
    y: f32,
    /// Where the glyph being drawn is placed — a `seac` accent is drawn offset.
    origin: (f32, f32),
    open: bool,
    /// Points collected between flex start and end.
    flex: Option<Vec<(f32, f32)>>,
    width: Option<f32>,
    done: bool,
}

impl<'a> Interpreter<'a> {
    fn new(font: &'a Type1Font, sink: &'a mut dyn OutlineSink) -> Self {
        Self {
            font,
            sink,
            stack: Vec::new(),
            ps_stack: Vec::new(),
            x: 0.0,
            y: 0.0,
            origin: (0.0, 0.0),
            open: false,
            flex: None,
            width: None,
            done: false,
        }
    }

    /// Draw glyph `gid` with its origin at `(ox, oy)`.
    fn glyph(&mut self, gid: u16, ox: f32, oy: f32, depth: usize) -> Option<()> {
        let charstring = self.font.charstrings.get(usize::from(gid))?;
        self.origin = (ox, oy);
        self.x = ox;
        self.y = oy;
        self.stack.clear();
        self.done = false;
        self.run(charstring, depth)?;
        self.close_path();
        Some(())
    }

    fn move_to(&mut self, x: f32, y: f32) {
        self.close_path();
        self.x = x;
        self.y = y;
        self.sink.move_to(x, y);
        self.open = true;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.ensure_open();
        self.x = x;
        self.y = y;
        self.sink.line_to(x, y);
    }

    fn curve_to(&mut self, d: [f32; 6]) {
        self.ensure_open();
        let (x1, y1) = (self.x + d[0], self.y + d[1]);
        let (x2, y2) = (x1 + d[2], y1 + d[3]);
        let (x3, y3) = (x2 + d[4], y2 + d[5]);
        self.sink.curve_to(x1, y1, x2, y2, x3, y3);
        self.x = x3;
        self.y = y3;
    }

    /// A drawing operator with no `moveto` before it starts at the current point.
    fn ensure_open(&mut self) {
        if !self.open {
            self.sink.move_to(self.x, self.y);
            self.open = true;
        }
    }

    fn close_path(&mut self) {
        if self.open {
            self.sink.close();
            self.open = false;
        }
    }

    /// A relative move: a point of the flex curve while one is being collected.
    fn r_move(&mut self, dx: f32, dy: f32) {
        let (x, y) = (self.x + dx, self.y + dy);
        match &mut self.flex {
            Some(points) => {
                points.push((x, y));
                self.x = x;
                self.y = y;
            }
            None => self.move_to(x, y),
        }
    }

    fn run(&mut self, code: &[u8], depth: usize) -> Option<()> {
        if depth > MAX_CALL_DEPTH {
            return None;
        }
        let mut i = 0;
        while i < code.len() && !self.done {
            let v = code[i];
            i += 1;
            match v {
                32..=246 => self.stack.push(f32::from(v) - 139.0),
                247..=250 => {
                    let w = *code.get(i)?;
                    i += 1;
                    self.stack
                        .push((f32::from(v) - 247.0) * 256.0 + f32::from(w) + 108.0);
                }
                251..=254 => {
                    let w = *code.get(i)?;
                    i += 1;
                    self.stack
                        .push(-(f32::from(v) - 251.0) * 256.0 - f32::from(w) - 108.0);
                }
                255 => {
                    let b = code.get(i..i + 4)?;
                    i += 4;
                    self.stack
                        .push(i32::from_be_bytes([b[0], b[1], b[2], b[3]]) as f32);
                }
                12 => {
                    let op = *code.get(i)?;
                    i += 1;
                    self.escape(op, depth)?;
                }
                _ => {
                    if self.command(v, depth)? {
                        return Some(());
                    }
                }
            }
        }
        Some(())
    }

    /// Run one command; `true` when it returns from the current charstring.
    fn command(&mut self, op: u8, depth: usize) -> Option<bool> {
        let s = std::mem::take(&mut self.stack);
        let arg = |k: usize| s.get(k).copied().unwrap_or(0.0);
        match op {
            // hstem, vstem: hints.
            1 | 3 => {}
            // vmoveto
            4 => self.r_move(0.0, arg(0)),
            // rlineto
            5 => self.line_to(self.x + arg(0), self.y + arg(1)),
            // hlineto
            6 => self.line_to(self.x + arg(0), self.y),
            // vlineto
            7 => self.line_to(self.x, self.y + arg(0)),
            // rrcurveto
            8 => self.curve_to([arg(0), arg(1), arg(2), arg(3), arg(4), arg(5)]),
            // closepath
            9 => self.close_path(),
            // callsubr
            10 => {
                let index = *s.last()? as usize;
                self.stack = s[..s.len() - 1].to_vec();
                let subr = self.font.subrs.get(index)?;
                self.run(subr, depth + 1)?;
                return Some(false);
            }
            // return
            11 => {
                self.stack = s;
                return Some(true);
            }
            // hsbw: side-bearing point and advance width.
            13 => {
                self.x = self.origin.0 + arg(0);
                self.y = self.origin.1;
                self.width.get_or_insert(arg(1));
            }
            // endchar
            14 => {
                self.close_path();
                self.done = true;
            }
            // rmoveto
            21 => self.r_move(arg(0), arg(1)),
            // hmoveto
            22 => self.r_move(arg(0), 0.0),
            // vhcurveto
            30 => self.curve_to([0.0, arg(0), arg(1), arg(2), arg(3), 0.0]),
            // hvcurveto
            31 => self.curve_to([arg(0), 0.0, arg(1), arg(2), 0.0, arg(3)]),
            _ => {}
        }
        Some(false)
    }

    fn escape(&mut self, op: u8, depth: usize) -> Option<()> {
        let s = std::mem::take(&mut self.stack);
        let arg = |k: usize| s.get(k).copied().unwrap_or(0.0);
        match op {
            // dotsection, vstem3, hstem3: hints.
            0..=2 => {}
            // seac: an accented character built from two others in StandardEncoding.
            6 => {
                let (asb, adx, ady) = (arg(0), arg(1), arg(2));
                let standard = |code: f32| {
                    let code = u8::try_from(code as i64).ok()?;
                    self.font
                        .glyph_by_char(BaseEncoding::Standard.decode_char(code)?)
                };
                let base = standard(arg(3))?;
                let accent = standard(arg(4))?;
                let (ox, oy) = self.origin;
                let width = self.width;
                self.close_path();
                self.glyph(base, ox, oy, depth + 1)?;
                self.glyph(accent, ox + adx - asb, oy + ady, depth + 1)?;
                self.width = width;
                self.done = true;
            }
            // sbw
            7 => {
                self.x = self.origin.0 + arg(0);
                self.y = self.origin.1 + arg(1);
                self.width.get_or_insert(arg(2));
            }
            // div
            12 => {
                let mut rest = s;
                let b = rest.pop()?;
                let a = rest.pop()?;
                rest.push(if b == 0.0 { 0.0 } else { a / b });
                self.stack = rest;
            }
            // callothersubr: othersubr# and its argument count on top, the arguments below.
            16 => {
                let mut rest = s;
                let index = rest.pop()? as i32;
                let count = (rest.pop()? as usize).min(rest.len());
                let args = rest.split_off(rest.len() - count);
                self.stack = rest;
                self.other_subr(index, &args);
            }
            // pop: a value an othersubr left.
            17 => {
                self.stack = s;
                let value = self.ps_stack.pop().unwrap_or(0.0);
                self.stack.push(value);
            }
            // setcurrentpoint
            33 => {
                self.x = arg(0);
                self.y = arg(1);
            }
            _ => {}
        }
        Some(())
    }

    /// The othersubrs every Type 1 font defines: flex (0–2) and hint replacement (3).
    fn other_subr(&mut self, index: i32, args: &[f32]) {
        match index {
            // Flex start: the next moves are the curve's reference point and control points.
            1 => self.flex = Some(Vec::new()),
            // A flex point: the move before it already recorded it.
            2 => {}
            // Flex end: two curves through the collected points; the end point is left for
            // `pop pop setcurrentpoint`.
            0 => {
                let points = self.flex.take().unwrap_or_default();
                if points.len() >= 7 {
                    let p = &points[1..7];
                    self.ensure_open();
                    self.sink
                        .curve_to(p[0].0, p[0].1, p[1].0, p[1].1, p[2].0, p[2].1);
                    self.sink
                        .curve_to(p[3].0, p[3].1, p[4].0, p[4].1, p[5].0, p[5].1);
                    self.x = p[5].0;
                    self.y = p[5].1;
                }
                // `pop` returns x, then y.
                self.ps_stack.push(self.y);
                self.ps_stack.push(self.x);
            }
            // Hint replacement, and anything unknown: hand the arguments back, so `pop`
            // returns them in order (`subr# 1 3 callothersubr pop callsubr`).
            _ => self.ps_stack.extend(args.iter().rev()),
        }
    }
}

/// The encrypted part, as bytes: binary as written, or hexadecimal (the program's
/// first four bytes all hex digits) decoded.
fn encrypted_part(after_eexec: &[u8]) -> Vec<u8> {
    let start = after_eexec
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(after_eexec.len());
    let body = &after_eexec[start..];
    let is_hex = body.len() >= 4 && body[..4].iter().all(u8::is_ascii_hexdigit);
    if !is_hex {
        return body.to_vec();
    }
    let digits: Vec<u8> = body
        .iter()
        .copied()
        .filter(|b| !b.is_ascii_whitespace())
        .take_while(u8::is_ascii_hexdigit)
        .collect();
    let (pairs, _) = digits.as_chunks::<2>();
    pairs
        .iter()
        .filter_map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect()
}

/// Type 1 decryption (Adobe Type 1 Font Format §7): `key` 55665 for the eexec part, 4330
/// for charstrings; the first `skip` plain bytes are random and dropped.
fn decrypt(cipher: &[u8], key: u16, skip: usize) -> Vec<u8> {
    const C1: u16 = 52845;
    const C2: u16 = 22719;
    let mut r = key;
    let plain: Vec<u8> = cipher
        .iter()
        .map(|&c| {
            let p = c ^ (r >> 8) as u8;
            r = (u16::from(c).wrapping_add(r))
                .wrapping_mul(C1)
                .wrapping_add(C2);
            p
        })
        .collect();
    plain.get(skip..).map(<[u8]>::to_vec).unwrap_or_default()
}

/// A glyph name and its (still encrypted) charstring.
type NamedCharstring = (String, Vec<u8>);

/// The subroutines (by index) and charstrings (by name, in definition order) of the
/// private dictionary: every `<n> RD <n bytes>` (or `-|`) after `/Subrs` and after
/// `/CharStrings`.
fn private_programs(private: &[u8]) -> (Vec<Vec<u8>>, Vec<NamedCharstring>) {
    let mut subrs: Vec<Vec<u8>> = Vec::new();
    let mut glyphs: Vec<NamedCharstring> = Vec::new();
    let mut in_charstrings = false;
    // The last two tokens before the current one.
    let mut back: [Vec<u8>; 2] = [Vec::new(), Vec::new()];
    let mut i = 0;
    while i < private.len() {
        if private[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        while i < private.len() && !private[i].is_ascii_whitespace() && private[i] != b'/' {
            i += 1;
        }
        let token = &private[start..i];
        match token {
            b"/CharStrings" => in_charstrings = true,
            b"RD" | b"-|" => {
                let Some(len) = parse_int(&back[1]).and_then(|n| usize::try_from(n).ok()) else {
                    continue;
                };
                // One space separates the operator from the binary data.
                let data_start = i + 1;
                let Some(data) = private.get(data_start..data_start + len) else {
                    break;
                };
                if in_charstrings {
                    if let Some(name) = back[0].strip_prefix(b"/") {
                        glyphs.push((String::from_utf8_lossy(name).into_owned(), data.to_vec()));
                    }
                } else if let Some(index) =
                    parse_int(&back[0]).and_then(|n| usize::try_from(n).ok())
                {
                    if index < 65_536 {
                        if subrs.len() <= index {
                            subrs.resize(index + 1, Vec::new());
                        }
                        subrs[index] = data.to_vec();
                    }
                }
                i = data_start + len;
                back = [Vec::new(), Vec::new()];
                continue;
            }
            _ => {}
        }
        back = [std::mem::take(&mut back[1]), token.to_vec()];
    }
    (subrs, glyphs)
}

/// What each code is in a Type 1 program's built-in encoding, by its glyph names — the
/// encoding a font with no `/Encoding` uses, and the base its `/Differences` apply to when
/// it names none (ISO 32000-1 §9.6.6.1). Only the cleartext part is read. `None` when
/// `data` is not a Type 1 program.
pub(crate) fn builtin_encoding_chars(data: &[u8]) -> Option<HashMap<u8, char>> {
    let eexec = find(data, b"eexec")?;
    Some(match builtin_encoding(&data[..eexec]) {
        BuiltinEncoding::Standard => (0..=255u8)
            .filter_map(|code| Some((code, BaseEncoding::Standard.decode_char(code)?)))
            .collect(),
        BuiltinEncoding::Custom(names) => names
            .into_iter()
            .filter_map(|(code, name)| Some((code, glyph_name_to_unicode(&name)?)))
            .collect(),
    })
}

/// `/FontMatrix [a b c d e f]` in the cleartext.
fn font_matrix(clear: &[u8]) -> Option<[f32; 6]> {
    let at = find(clear, b"/FontMatrix")?;
    let rest = &clear[at + 11..];
    let open = rest.iter().position(|&b| b == b'[' || b == b'{')?;
    let close = rest[open..].iter().position(|&b| b == b']' || b == b'}')? + open;
    let numbers: Vec<f32> = std::str::from_utf8(&rest[open + 1..close])
        .ok()?
        .split_ascii_whitespace()
        .filter_map(|t| t.parse().ok())
        .collect();
    let m: [f32; 6] = numbers.try_into().ok()?;
    (m[0] != 0.0 || m[1] != 0.0).then_some(m)
}

/// The cleartext's `/Encoding`: `StandardEncoding`, or the codes its `dup <code> /<name> put`
/// entries name.
fn builtin_encoding(clear: &[u8]) -> BuiltinEncoding {
    let Some(at) = find(clear, b"/Encoding") else {
        return BuiltinEncoding::Standard;
    };
    let rest = &clear[at + 9..];
    let first = rest
        .split(u8::is_ascii_whitespace)
        .find(|t| !t.is_empty())
        .unwrap_or_default();
    if first == b"StandardEncoding" {
        return BuiltinEncoding::Standard;
    }
    // Up to the end of the definition.
    let end = find(rest, b"readonly def")
        .or_else(|| find(rest, b" def"))
        .unwrap_or(rest.len());
    let text = String::from_utf8_lossy(&rest[..end]);
    let tokens: Vec<&str> = text.split_ascii_whitespace().collect();
    let mut map = HashMap::new();
    for w in tokens.windows(4) {
        if w[0] == "dup" && w[3] == "put" {
            if let (Ok(code), Some(name)) = (w[1].parse::<u8>(), w[2].strip_prefix('/')) {
                map.insert(code, name.to_string());
            }
        }
    }
    BuiltinEncoding::Custom(map)
}

/// The integer after `key`, as in `/lenIV 4 def`.
fn integer_after(data: &[u8], key: &[u8]) -> Option<i64> {
    let at = find(data, key)?;
    let rest = &data[at + key.len()..];
    let token = rest
        .split(u8::is_ascii_whitespace)
        .find(|t| !t.is_empty())?;
    parse_int(token)
}

fn parse_int(token: &[u8]) -> Option<i64> {
    std::str::from_utf8(token).ok()?.parse().ok()
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encrypt as the Type 1 format does — the inverse of [`decrypt`] — with `skip` leading
    /// bytes of padding.
    fn encrypt(plain: &[u8], key: u16, skip: usize) -> Vec<u8> {
        let mut r = key;
        std::iter::repeat_n(0u8, skip)
            .chain(plain.iter().copied())
            .map(|p| {
                let c = p ^ (r >> 8) as u8;
                r = (u16::from(c).wrapping_add(r))
                    .wrapping_mul(52845)
                    .wrapping_add(22719);
                c
            })
            .collect()
    }

    /// A charstring number.
    fn num(n: i32) -> Vec<u8> {
        match n {
            -107..=107 => vec![(n + 139) as u8],
            108..=1131 => {
                let v = n - 108;
                vec![(v / 256 + 247) as u8, (v % 256) as u8]
            }
            -1131..=-108 => {
                let v = -n - 108;
                vec![(v / 256 + 251) as u8, (v % 256) as u8]
            }
            _ => {
                let mut b = vec![255];
                b.extend(n.to_be_bytes());
                b
            }
        }
    }

    fn cs(parts: &[&[u8]]) -> Vec<u8> {
        parts.concat()
    }

    /// A Type 1 program with the given charstrings and subrs, encrypted the way files are.
    fn program(
        glyphs: &[(&str, Vec<u8>)],
        subrs: &[Vec<u8>],
        encoding: &str,
        hex: bool,
    ) -> Vec<u8> {
        let mut private = b"dup /Private 8 dict dup begin /RD{string currentfile exch readstring pop}executeonly def /lenIV 4 def ".to_vec();
        private.extend(format!("/Subrs {} array\n", subrs.len()).as_bytes());
        for (i, s) in subrs.iter().enumerate() {
            let e = encrypt(s, 4330, 4);
            private.extend(format!("dup {i} {} RD ", e.len()).as_bytes());
            private.extend(&e);
            private.extend(b" NP\n");
        }
        private.extend(
            format!("ND\n2 index /CharStrings {} dict dup begin\n", glyphs.len()).as_bytes(),
        );
        for (name, g) in glyphs {
            let e = encrypt(g, 4330, 4);
            private.extend(format!("/{name} {} -| ", e.len()).as_bytes());
            private.extend(&e);
            private.extend(b" |-\n");
        }
        private.extend(b"end end readonly put noaccess put dup /FontName get exch definefont pop mark currentfile closefile\n");
        let mut out = format!(
            "%!PS-AdobeFont-1.0: Test\n/FontMatrix [0.001 0 0 0.001 0 0] readonly def\n/Encoding {encoding}\ncurrentfile eexec\n"
        )
        .into_bytes();
        let enc = encrypt(&private, 55665, 4);
        if hex {
            for chunk in enc.chunks(32) {
                for b in chunk {
                    out.extend(format!("{b:02x}").as_bytes());
                }
                out.push(b'\n');
            }
        } else {
            out.extend(enc);
        }
        out.extend(b"\n0000000000000000\ncleartomark\n");
        out
    }

    #[derive(Default)]
    struct Recorder(Vec<String>);

    impl OutlineSink for Recorder {
        fn move_to(&mut self, x: f32, y: f32) {
            self.0.push(format!("M{x} {y}"));
        }
        fn line_to(&mut self, x: f32, y: f32) {
            self.0.push(format!("L{x} {y}"));
        }
        fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
            self.0.push(format!("C{x1} {y1} {x2} {y2} {x} {y}"));
        }
        fn close(&mut self) {
            self.0.push("Z".into());
        }
    }

    /// A square: hsbw 50 600, moveto (50,0), three lines, closepath, endchar.
    fn square() -> Vec<u8> {
        cs(&[
            &num(50),
            &num(600),
            &[13],
            &num(0),
            &num(0),
            &[21],
            &num(500),
            &[6],
            &num(500),
            &[7],
            &num(-500),
            &[6],
            &[9, 14],
        ])
    }

    fn outline_of(font: &Type1Font, name: &str) -> Vec<String> {
        let mut rec = Recorder::default();
        font.outline(font.glyph_by_name(name).unwrap(), &mut rec)
            .unwrap();
        rec.0
    }

    #[test]
    fn a_binary_program_is_decrypted_and_drawn() {
        let data = program(
            &[("A", square())],
            &[],
            "StandardEncoding readonly def",
            false,
        );
        let font = Type1Font::parse(&data).expect("parses");
        assert_eq!(font.font_matrix, [0.001, 0.0, 0.0, 0.001, 0.0, 0.0]);
        assert_eq!(
            outline_of(&font, "A"),
            ["M50 0", "L550 0", "L550 500", "L50 500", "Z"]
        );
        assert_eq!(font.advance(0), Some(600.0));
    }

    #[test]
    fn a_hexadecimal_program_reads_the_same() {
        let data = program(
            &[("A", square())],
            &[],
            "StandardEncoding readonly def",
            true,
        );
        let font = Type1Font::parse(&data).expect("parses");
        assert_eq!(outline_of(&font, "A").len(), 5);
    }

    #[test]
    fn subroutines_are_called_and_return() {
        // Subr 0 draws a line and returns; the glyph calls it.
        let subr = cs(&[&num(100), &num(0), &[5], &[11]]);
        let glyph = cs(&[
            &num(0),
            &num(300),
            &[13],
            &num(10),
            &num(10),
            &[21],
            &num(0),
            &[10],
            &num(0),
            &num(100),
            &[5],
            &[9, 14],
        ]);
        let data = program(
            &[("B", glyph)],
            &[subr],
            "StandardEncoding readonly def",
            false,
        );
        let font = Type1Font::parse(&data).unwrap();
        assert_eq!(
            outline_of(&font, "B"),
            ["M10 10", "L110 10", "L110 110", "Z"]
        );
    }

    #[test]
    fn curves_and_large_numbers_decode() {
        let glyph = cs(&[
            &num(0),
            &num(1000),
            &[13],
            &num(0),
            &num(0),
            &[21],
            &num(200),
            &num(0),
            &num(300),
            &num(400),
            &num(0),
            &num(2000),
            &[8],
            &num(-1000),
            &[6],
            &[9, 14],
        ]);
        let data = program(&[("C", glyph)], &[], "StandardEncoding readonly def", false);
        let font = Type1Font::parse(&data).unwrap();
        assert_eq!(
            outline_of(&font, "C"),
            ["M0 0", "C200 0 500 400 500 2400", "L-500 2400", "Z"]
        );
    }

    #[test]
    fn an_accented_character_is_built_from_its_parts() {
        // `seac`: the accent is placed at (adx - asb, ady) from the base's origin.
        let e = cs(&[
            &num(0),
            &num(500),
            &[13],
            &num(0),
            &num(0),
            &[21],
            &num(100),
            &[6],
            &[9, 14],
        ]);
        let acute = cs(&[
            &num(20),
            &num(300),
            &[13],
            &num(0),
            &num(0),
            &[21],
            &num(50),
            &[7],
            &[9, 14],
        ]);
        // StandardEncoding: 0x65 'e', 0xC2 'acute'.
        let eacute = cs(&[
            &num(0),
            &num(500),
            &[13],
            &num(20),
            &num(200),
            &num(150),
            &num(0x65),
            &num(0xC2),
            &[12, 6],
        ]);
        let data = program(
            &[("e", e), ("acute", acute), ("eacute", eacute)],
            &[],
            "StandardEncoding readonly def",
            false,
        );
        let font = Type1Font::parse(&data).unwrap();
        assert_eq!(
            outline_of(&font, "eacute"),
            ["M0 0", "L100 0", "Z", "M200 150", "L200 200", "Z"]
        );
        assert_eq!(
            font.advance(font.glyph_by_name("eacute").unwrap()),
            Some(500.0)
        );
    }

    #[test]
    fn a_flex_draws_two_curves() {
        // 0 1 callothersubr; seven rmovetos (reference point, then six); 50 x y 3 0 callothersubr;
        // pop pop setcurrentpoint.
        let mut parts: Vec<Vec<u8>> = vec![num(0), num(500), vec![13], num(0), num(0), vec![21]];
        parts.extend([num(0), num(1), vec![12, 16]]);
        for (dx, dy) in [
            (10, 0),
            (10, 10),
            (10, 0),
            (10, 0),
            (10, 0),
            (10, -10),
            (10, 0),
        ] {
            parts.extend([num(dx), num(dy), vec![21]]);
            parts.extend([num(0), num(2), vec![12, 16]]);
        }
        parts.extend([num(50), num(70), num(0), num(3), num(0), vec![12, 16]]);
        parts.extend([vec![12, 17], vec![12, 17], vec![12, 33]]);
        parts.extend([num(-70), vec![6], vec![9, 14]]);
        let glyph = parts.concat();
        let data = program(&[("F", glyph)], &[], "StandardEncoding readonly def", false);
        let font = Type1Font::parse(&data).unwrap();
        assert_eq!(
            outline_of(&font, "F"),
            [
                "M0 0",
                "C20 10 30 10 40 10",
                "C50 10 60 0 70 0",
                "L0 0",
                "Z"
            ]
        );
    }

    #[test]
    fn a_custom_builtin_encoding_selects_glyphs_by_code() {
        let data = program(
            &[("A", square()), ("alpha", square())],
            &[],
            "256 array 0 1 255 {1 index exch /.notdef put} for dup 65 /alpha put dup 66 /A put readonly def",
            false,
        );
        let font = Type1Font::parse(&data).unwrap();
        assert_eq!(font.glyph_by_builtin_code(65), font.glyph_by_name("alpha"));
        assert_eq!(font.glyph_by_builtin_code(66), font.glyph_by_name("A"));
    }

    #[test]
    fn the_standard_encoding_selects_glyphs_through_their_names() {
        let data = program(
            &[("A", square())],
            &[],
            "StandardEncoding readonly def",
            false,
        );
        let font = Type1Font::parse(&data).unwrap();
        assert_eq!(font.glyph_by_builtin_code(b'A'), Some(0));
        assert_eq!(font.glyph_by_builtin_code(b'B'), None);
    }

    #[test]
    fn a_charstring_that_calls_itself_stops() {
        // Subr 0 calls subr 0.
        let subr = cs(&[&num(0), &[10]]);
        let glyph = cs(&[&num(0), &num(500), &[13], &num(0), &[10], &[14]]);
        let data = program(
            &[("L", glyph)],
            &[subr],
            "StandardEncoding readonly def",
            false,
        );
        let font = Type1Font::parse(&data).unwrap();
        let mut rec = Recorder::default();
        assert!(font.outline(0, &mut rec).is_none());
    }

    #[test]
    fn something_else_is_not_a_type1_program() {
        assert!(Type1Font::parse(b"%PDF-1.7 not a font").is_none());
        assert!(Type1Font::parse(b"").is_none());
    }
}
