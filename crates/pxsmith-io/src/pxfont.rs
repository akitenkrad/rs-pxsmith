//! `.pxfont.toml` の決定的な読み書き．

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use pxsmith_core::IndexedCanvas;
use pxsmith_core::font::{Font, FontColorKey, FontMeta, FontMode, FontPalette, Glyph, Kern};
use serde::{Deserialize, Serialize};

use crate::error::{IoError, Result};
use crate::l0::{COLOR_KEYS, L0PaletteSpec, RawColorKey, TRANSPARENT_KEY};

/// `[meta]` のファイル表現．`mode = "mono"` は正規形では省く．
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PxFontMeta {
    pub format: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    pub line_height: u32,
    pub baseline: u32,
    pub default_advance: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copyright: Option<String>,
}

/// `[[glyph]]` のファイル表現．
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PxFontGlyph {
    #[serde(rename = "char")]
    pub character: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advance: Option<u32>,
    pub data: String,
}

/// `[[kern]]` のファイル表現．
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PxFontKern {
    pub pair: String,
    pub amount: i32,
}

/// `.pxfont.toml` のファイル表現．
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PxFontDocument {
    pub meta: PxFontMeta,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<L0PaletteSpec>,
    #[serde(rename = "glyph", default)]
    pub glyphs: Vec<PxFontGlyph>,
    #[serde(rename = "kern", default, skip_serializing_if = "Vec::is_empty")]
    pub kerns: Vec<PxFontKern>,
}

impl PxFontDocument {
    /// TOML 本文を読む．意味上の制約は [`Self::to_font`] で検査する．
    pub fn parse(text: &str, path: &Path) -> Result<Self> {
        toml::from_str(text).map_err(|error| IoError::Parse {
            path: path.to_path_buf(),
            line: error
                .span()
                .map_or(0, |span| text[..span.start].lines().count()),
            message: error.message().to_string(),
        })
    }

    /// ファイルから読む．
    pub fn read(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|source| IoError::File {
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse(&text, path)
    }

    /// ファイル表現を，検証済みのフォントモデルへ変換する．
    pub fn to_font(&self, path: &Path) -> Result<Font> {
        let mode = match self.meta.mode.as_deref().unwrap_or("mono") {
            "mono" => FontMode::Mono,
            "color" => FontMode::Color,
            other => {
                return parse_error(path, 0, format!("mode '{other}' は mono / color ではない"));
            }
        };

        let palette = match (&mode, &self.palette) {
            (FontMode::Mono, None) => None,
            (FontMode::Mono, Some(_)) => {
                return parse_error(path, 0, "単色フォントに [palette] がある");
            }
            (FontMode::Color, None) => {
                return parse_error(path, 0, "多色フォントに [palette] が無い");
            }
            (FontMode::Color, Some(spec)) => Some(decode_palette(spec, path)?),
        };
        let transparent = palette
            .as_ref()
            .map(|palette| transparent_index(palette, path))
            .transpose()?;

        let meta = FontMeta {
            format: self.meta.format,
            name: self.meta.name.clone(),
            mode,
            line_height: self.meta.line_height,
            baseline: self.meta.baseline,
            default_advance: self.meta.default_advance,
            source: self.meta.source.clone(),
            copyright: self.meta.copyright.clone(),
            palette,
        };
        let mut glyphs = Vec::with_capacity(self.glyphs.len());
        for glyph in &self.glyphs {
            let character = decode_character(&glyph.character, path)?;
            let canvas = decode_data(glyph, &meta, transparent, path)?;
            glyphs.push(Glyph {
                character,
                variant: glyph.variant.clone(),
                advance: glyph.advance,
                canvas,
            });
        }
        let mut font = Font {
            meta,
            glyphs,
            kerns: self
                .kerns
                .iter()
                .map(|kern| Kern {
                    pair: kern.pair.clone(),
                    amount: kern.amount,
                })
                .collect(),
        };
        font.validate()?;
        font.canonicalize();
        Ok(font)
    }

    /// フォントモデルから正規形の文書を作る．
    pub fn from_font(font: &Font) -> Result<Self> {
        font.validate()?;
        let mut font = font.clone();
        font.canonicalize();
        let palette = font.meta.palette.as_ref().map(encode_palette);
        let glyphs = font
            .glyphs
            .iter()
            .map(|glyph| {
                Ok(PxFontGlyph {
                    character: encode_character(glyph.character),
                    variant: glyph.variant.clone(),
                    advance: glyph.advance,
                    data: encode_data(glyph, &font.meta)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            meta: PxFontMeta {
                format: font.meta.format,
                name: font.meta.name,
                mode: (font.meta.mode == FontMode::Color).then(|| "color".to_string()),
                line_height: font.meta.line_height,
                baseline: font.meta.baseline,
                default_advance: font.meta.default_advance,
                source: font.meta.source,
                copyright: font.meta.copyright,
            },
            palette,
            glyphs,
            kerns: font
                .kerns
                .into_iter()
                .map(|kern| PxFontKern {
                    pair: kern.pair,
                    amount: kern.amount,
                })
                .collect(),
        })
    }

    /// 正規形の TOML を作る．`data` は必ず複数行リテラルにする．
    pub fn to_toml(&self) -> Result<String> {
        let font = self.to_font(Path::new("<memory>"))?;
        Self::from_font(&font)?.serialize_canonical()
    }

    /// 正規形を原子的に書く．
    pub fn write(&self, path: impl AsRef<Path>) -> Result<()> {
        crate::atomic::write(path, self.to_toml()?.as_bytes())
    }

    fn serialize_canonical(&self) -> Result<String> {
        const MARKER: &str = "@@PXSMITH_FONT_DATA_";
        let mut shadow = self.clone();
        let data = shadow
            .glyphs
            .iter_mut()
            .enumerate()
            .map(|(index, glyph)| std::mem::replace(&mut glyph.data, format!("{MARKER}{index}@@")))
            .collect::<Vec<_>>();
        let mut text = toml::to_string_pretty(&shadow).map_err(|error| IoError::Parse {
            path: PathBuf::from("<memory>"),
            line: 0,
            message: error.to_string(),
        })?;
        for (index, original) in data.iter().enumerate() {
            let marker = format!("\"{MARKER}{index}@@\"");
            let body = original.trim_end_matches('\n');
            text = text.replace(&marker, &format!("'''\n{body}\n'''"));
        }
        Ok(text)
    }
}

fn parse_error<T>(path: &Path, line: usize, message: impl Into<String>) -> Result<T> {
    Err(IoError::Parse {
        path: path.to_path_buf(),
        line,
        message: message.into(),
    })
}

fn decode_character(value: &str, path: &Path) -> Result<char> {
    if let Some(hex) = value.strip_prefix("U+") {
        if hex.len() < 4 || hex.len() > 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return parse_error(path, 0, format!("char '{value}' の符号位置を読めない"));
        }
        let code = u32::from_str_radix(hex, 16).map_err(|_| IoError::Parse {
            path: path.to_path_buf(),
            line: 0,
            message: format!("char '{value}' の符号位置を読めない"),
        })?;
        return char::from_u32(code).ok_or_else(|| IoError::Parse {
            path: path.to_path_buf(),
            line: 0,
            message: format!("char '{value}' は Unicode の符号位置でない"),
        });
    }
    let mut chars = value.chars();
    match (chars.next(), chars.next()) {
        (Some(character), None) => Ok(character),
        _ => parse_error(path, 0, format!("char '{value}' は 1 文字でない")),
    }
}

fn encode_character(character: char) -> String {
    if character.is_whitespace() || character.is_control() {
        format!("U+{:04X}", character as u32)
    } else {
        character.to_string()
    }
}

fn decode_palette(spec: &L0PaletteSpec, path: &Path) -> Result<FontPalette> {
    let mut map = BTreeMap::new();
    for (key, value) in &spec.map {
        let mut chars = key.chars();
        let (Some(character), None) = (chars.next(), chars.next()) else {
            return parse_error(path, 0, format!("色キー '{key}' が 1 文字でない"));
        };
        if character != TRANSPARENT_KEY && !COLOR_KEYS.contains(character) {
            return parse_error(
                path,
                0,
                format!("色キー '{character}' は 0-9a-zA-Z と '.' 以外である"),
            );
        }
        let value = match value {
            RawColorKey::Index(index) => FontColorKey::Index(*index),
            RawColorKey::Name(name) if name == "transparent" => FontColorKey::Transparent,
            RawColorKey::Name(name) => {
                return parse_error(
                    path,
                    0,
                    format!("色キー '{key}' の値 '{name}' を解釈できない"),
                );
            }
        };
        map.insert(character, value);
    }
    map.entry(TRANSPARENT_KEY)
        .or_insert(FontColorKey::Transparent);
    Ok(FontPalette {
        reference: spec.reference.to_string_lossy().into_owned(),
        map,
    })
}

fn encode_palette(palette: &FontPalette) -> L0PaletteSpec {
    L0PaletteSpec {
        reference: PathBuf::from(&palette.reference),
        map: palette
            .map
            .iter()
            .map(|(key, value)| {
                let value = match value {
                    FontColorKey::Transparent => RawColorKey::Name("transparent".to_string()),
                    FontColorKey::Index(index) => RawColorKey::Index(*index),
                };
                (key.to_string(), value)
            })
            .collect(),
    }
}

fn transparent_index(palette: &FontPalette, path: &Path) -> Result<u8> {
    let used = palette
        .map
        .values()
        .filter_map(|value| match value {
            FontColorKey::Index(index) => Some(*index),
            FontColorKey::Transparent => None,
        })
        .collect::<std::collections::BTreeSet<_>>();
    (0..=u8::MAX)
        .find(|index| !used.contains(index))
        .ok_or_else(|| IoError::Parse {
            path: path.to_path_buf(),
            line: 0,
            message: "透明添字の空きが無い".to_string(),
        })
}

fn decode_data(
    glyph: &PxFontGlyph,
    meta: &FontMeta,
    transparent: Option<u8>,
    path: &Path,
) -> Result<IndexedCanvas> {
    let rows = glyph.data.lines().collect::<Vec<_>>();
    if rows.len() != meta.line_height as usize {
        return parse_error(
            path,
            rows.len().min(meta.line_height as usize) + 1,
            format!(
                "字 '{}' の data は {} 行だが line_height は {}",
                glyph.character,
                rows.len(),
                meta.line_height
            ),
        );
    }
    let width = rows.first().map_or(0, |row| row.chars().count());
    let mut pixels = Vec::with_capacity(width * rows.len());
    for (row_index, row) in rows.iter().enumerate() {
        let actual = row.chars().count();
        if actual != width {
            return parse_error(
                path,
                row_index + 1,
                format!(
                    "字 '{}' の {} 行目が {actual} 文字（1 行目は {width} 文字）",
                    glyph.character,
                    row_index + 1
                ),
            );
        }
        for symbol in row.chars() {
            let index = match meta.mode {
                FontMode::Mono => match symbol {
                    '.' => 0,
                    '#' => 1,
                    _ => {
                        return parse_error(
                            path,
                            row_index + 1,
                            format!("単色の字 '{}' に文字 '{symbol}' がある", glyph.character),
                        );
                    }
                },
                FontMode::Color => {
                    let palette = meta.palette.as_ref().expect("多色は検証済み");
                    match palette.map.get(&symbol) {
                        Some(FontColorKey::Index(index)) => *index,
                        Some(FontColorKey::Transparent) => transparent.expect("透明添字は確保済み"),
                        None => {
                            return parse_error(
                                path,
                                row_index + 1,
                                format!(
                                    "字 '{}' の文字 '{symbol}' が [palette] map に無い",
                                    glyph.character
                                ),
                            );
                        }
                    }
                }
            };
            pixels.push(index);
        }
    }
    let canvas = IndexedCanvas::from_pixels(width as u32, meta.line_height, pixels)?;
    Ok(canvas.with_transparent(Some(transparent.unwrap_or(0))))
}

fn encode_data(glyph: &Glyph, meta: &FontMeta) -> Result<String> {
    let reverse = meta.palette.as_ref().map(|palette| {
        let mut reverse = BTreeMap::new();
        for (symbol, value) in &palette.map {
            if let FontColorKey::Index(index) = value {
                reverse.entry(*index).or_insert(*symbol);
            }
        }
        reverse
    });
    let mut output = String::new();
    for y in 0..glyph.canvas.height() as i32 {
        for x in 0..glyph.canvas.width() as i32 {
            let index = glyph.canvas.get(x, y).expect("升の内側");
            let symbol = if Some(index) == glyph.canvas.transparent() {
                TRANSPARENT_KEY
            } else if meta.mode == FontMode::Mono {
                '#'
            } else {
                reverse
                    .as_ref()
                    .and_then(|map| map.get(&index))
                    .copied()
                    .ok_or_else(|| IoError::Parse {
                        path: PathBuf::from("<memory>"),
                        line: y as usize + 1,
                        message: format!(
                            "字 '{}' の添字 {index} が [palette] map に無い",
                            glyph.character
                        ),
                    })?
            };
            output.push(symbol);
        }
        output.push('\n');
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    const BASIC: &str = r##"
[meta]
format = 1
name = "test"
line_height = 2
baseline = 1
default_advance = 2

[[glyph]]
char = "A"
data = '''
.#
##
'''
"##;

    #[test]
    fn row_count_mismatch_reports_a_line() {
        let text = BASIC.replace(".#\n##", ".#");
        let document = PxFontDocument::parse(&text, Path::new("test.pxfont.toml")).unwrap();
        match document.to_font(Path::new("test.pxfont.toml")).unwrap_err() {
            crate::IoError::Parse { line, message, .. } => {
                assert!(line > 0, "行番号が無い: {message}");
                assert!(message.contains("line_height"), "{message}");
            }
            other => panic!("想定外のエラー: {other}"),
        }
    }

    #[test]
    fn row_width_mismatch_reports_the_data_row() {
        let text = BASIC.replace(".#\n##", ".#\n#");
        let document = PxFontDocument::parse(&text, Path::new("test.pxfont.toml")).unwrap();
        match document.to_font(Path::new("test.pxfont.toml")).unwrap_err() {
            crate::IoError::Parse { line, message, .. } => {
                assert_eq!(line, 2);
                assert!(message.contains("1 文字"), "{message}");
            }
            other => panic!("想定外のエラー: {other}"),
        }
    }

    #[test]
    fn crlf_data_is_accepted_and_mono_transparency_is_explicit() {
        let text = BASIC.replace(".#\n##", ".#\r\n##");
        let font = PxFontDocument::parse(&text, Path::new("test.pxfont.toml"))
            .unwrap()
            .to_font(Path::new("test.pxfont.toml"))
            .unwrap();
        assert_eq!(font.glyphs[0].canvas.transparent(), Some(0));
        assert_eq!(font.glyphs[0].canvas.pixels(), &[0, 1, 1, 1]);
    }

    #[test]
    fn whitespace_is_written_as_uppercase_codepoint() {
        let text = BASIC.replace("char = \"A\"", "char = \"U+0020\"");
        let document = PxFontDocument::parse(&text, Path::new("test.pxfont.toml")).unwrap();
        let font = document.to_font(Path::new("test.pxfont.toml")).unwrap();
        let output = PxFontDocument::from_font(&font).unwrap().to_toml().unwrap();
        assert!(output.contains("char = \"U+0020\""), "{output}");
    }

    #[test]
    fn canonical_advance_is_omitted_only_when_it_matches_the_implicit_advance() {
        let document = PxFontDocument::parse(BASIC, Path::new("test.pxfont.toml")).unwrap();
        let base = document.to_font(Path::new("test.pxfont.toml")).unwrap();

        let mut same_as_width = base.clone();
        same_as_width.glyphs[0].advance = Some(2);
        assert_eq!(
            PxFontDocument::from_font(&same_as_width).unwrap().glyphs[0].advance,
            None
        );

        let mut different_from_width = base.clone();
        different_from_width.glyphs[0].advance = Some(1);
        assert_eq!(
            PxFontDocument::from_font(&different_from_width)
                .unwrap()
                .glyphs[0]
                .advance,
            Some(1)
        );

        let mut zero_width = base;
        zero_width.glyphs[0].canvas = IndexedCanvas::filled(0, 2, 0).with_transparent(Some(0));
        zero_width.glyphs[0].advance = Some(2);
        assert_eq!(
            PxFontDocument::from_font(&zero_width).unwrap().glyphs[0].advance,
            None
        );
        zero_width.glyphs[0].advance = Some(1);
        assert_eq!(
            PxFontDocument::from_font(&zero_width).unwrap().glyphs[0].advance,
            Some(1)
        );
    }
}
