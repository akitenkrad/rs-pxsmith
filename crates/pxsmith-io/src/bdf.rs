//! BDF 2.1 を升全体の単色グリフへ取り込む．

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::OnceLock;

use pxsmith_core::IndexedCanvas;
use pxsmith_core::font::{FORMAT_VERSION, Font, FontMeta, FontMode, Glyph};

use crate::error::{IoError, Result};

const JIS0208: &str = include_str!("../data/JIS0208.TXT");

/// BDF 取り込みの指定．
#[derive(Clone, Debug, Default)]
pub struct BdfImportOptions {
    /// 升からはみ出す BBX を切る．既定では取り込み全体を失敗させる．
    pub clip: bool,
    /// 指定したときは，Unicode へ写した後の字が集合に含まれるものだけ取り込む．
    pub chars: Option<BTreeSet<char>>,
}

/// BDF から何を取り込み，何を飛ばしたか．
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BdfImportReport {
    pub declared_glyphs: usize,
    pub imported_glyphs: usize,
    pub skipped_unencoded: usize,
    pub unmapped: usize,
    pub filtered_out: usize,
    pub clipped: usize,
}

/// BDF 取り込みの結果．
#[derive(Clone, Debug)]
pub struct BdfImport {
    pub font: Font,
    pub report: BdfImportReport,
}

#[derive(Debug)]
struct RawGlyph {
    name: String,
    line: usize,
    encoding: i64,
    dwidth: i32,
    bbx: (i32, i32, i32, i32),
    bitmap: Vec<(usize, String)>,
}

/// BDF 2.1 本文を読み，単色フォントへ変換する．
pub fn import_bdf(text: &str, path: &Path, options: &BdfImportOptions) -> Result<BdfImport> {
    let lines = text.lines().collect::<Vec<_>>();
    if lines.first().copied() != Some("STARTFONT 2.1") {
        return parse_error(path, 1, "BDF 2.1 の STARTFONT が無い");
    }

    let mut ascent = None;
    let mut descent = None;
    let mut registry = None;
    let mut family = None;
    let mut font_name = None;
    let mut copyright = None;
    let mut declared = None;
    for (line_index, line) in lines.iter().enumerate() {
        if let Some(value) = line.strip_prefix("FONT_ASCENT ") {
            ascent = Some(parse_i32(value, path, line_index + 1, "FONT_ASCENT")?);
        } else if let Some(value) = line.strip_prefix("FONT_DESCENT ") {
            descent = Some(parse_i32(value, path, line_index + 1, "FONT_DESCENT")?);
        } else if let Some(value) = line.strip_prefix("CHARSET_REGISTRY ") {
            registry = Some(unquote(value));
        } else if let Some(value) = line.strip_prefix("FAMILY_NAME ") {
            family = Some(unquote(value));
        } else if let Some(value) = line.strip_prefix("COPYRIGHT ") {
            copyright = Some(unquote(value));
        } else if let Some(value) = line.strip_prefix("NOTICE ") {
            copyright.get_or_insert_with(|| unquote(value));
        } else if let Some(value) = line.strip_prefix("CHARS ") {
            declared = Some(parse_usize(value, path, line_index + 1, "CHARS")?);
        } else if let Some(value) = line.strip_prefix("FONT ") {
            font_name = Some(value.to_string());
        }
    }

    let ascent = ascent.ok_or_else(|| parse(path, 0, "FONT_ASCENT が無い"))?;
    let descent = descent.ok_or_else(|| parse(path, 0, "FONT_DESCENT が無い"))?;
    if ascent < 0 || descent < 0 {
        return parse_error(
            path,
            0,
            "FONT_ASCENT / FONT_DESCENT は 0 以上でなければならない",
        );
    }
    let line_height = ascent
        .checked_add(descent)
        .and_then(|height| u32::try_from(height).ok())
        .ok_or_else(|| parse(path, 0, "FONT_ASCENT + FONT_DESCENT が大きすぎる"))?;
    let baseline = ascent as u32;
    let registry = registry.ok_or_else(|| parse(path, 0, "CHARSET_REGISTRY が無い"))?;
    let charset = registry.to_ascii_uppercase();
    if !charset.starts_with("ISO10646") && !charset.starts_with("JISX0208") {
        return parse_error(
            path,
            0,
            format!("CHARSET_REGISTRY '{registry}' には対応していない"),
        );
    }

    let raw = parse_glyphs(&lines, path)?;
    let declared = declared.unwrap_or(raw.len());
    if declared != raw.len() {
        return parse_error(
            path,
            0,
            format!("CHARS は {declared} だが STARTCHAR は {} 件ある", raw.len()),
        );
    }

    let mut report = BdfImportReport {
        declared_glyphs: declared,
        ..BdfImportReport::default()
    };
    let mut glyphs = Vec::new();
    let mut outside = Vec::new();
    for raw_glyph in raw {
        if raw_glyph.encoding == -1 {
            report.skipped_unencoded += 1;
            continue;
        }
        let character = map_encoding(raw_glyph.encoding, &charset);
        let Some(character) = character else {
            report.unmapped += 1;
            continue;
        };
        if options
            .chars
            .as_ref()
            .is_some_and(|chars| !chars.contains(&character))
        {
            report.filtered_out += 1;
            continue;
        }
        if raw_glyph.dwidth < 0 {
            return parse_error(
                path,
                raw_glyph.line,
                format!("字 {} の DWIDTH が負である", shown(character)),
            );
        }

        let overflow = overflow(raw_glyph.bbx, raw_glyph.dwidth, line_height, baseline);
        if overflow != (0, 0, 0, 0) {
            if options.clip {
                report.clipped += 1;
            } else {
                outside.push(outside_detail(character, overflow));
                continue;
            }
        }
        let canvas = draw_glyph(&raw_glyph, line_height, baseline, path)?;
        glyphs.push(Glyph {
            character,
            variant: None,
            advance: None,
            canvas,
        });
    }
    if !outside.is_empty() {
        return Err(IoError::BdfOutside {
            count: outside.len(),
            details: outside,
        });
    }

    report.imported_glyphs = glyphs.len();
    let default_advance = most_common_width(&glyphs);
    for glyph in &mut glyphs {
        if glyph.canvas.width() != default_advance {
            glyph.advance = Some(glyph.canvas.width());
        }
    }
    let name = family
        .filter(|name| !name.is_empty())
        .or(font_name)
        .or_else(|| {
            path.file_stem()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "font".to_string());
    let mut font = Font {
        meta: FontMeta {
            format: FORMAT_VERSION,
            name,
            mode: FontMode::Mono,
            line_height,
            baseline,
            default_advance,
            source: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
            copyright,
            palette: None,
        },
        glyphs,
        kerns: Vec::new(),
    };
    font.validate()?;
    font.canonicalize();
    Ok(BdfImport { font, report })
}

fn parse_glyphs(lines: &[&str], path: &Path) -> Result<Vec<RawGlyph>> {
    let mut output = Vec::new();
    let mut at = 0;
    while at < lines.len() {
        let Some(name) = lines[at].strip_prefix("STARTCHAR ") else {
            at += 1;
            continue;
        };
        let start_line = at + 1;
        let mut encoding = None;
        let mut dwidth = None;
        let mut bbx = None;
        let mut bitmap = Vec::new();
        let mut in_bitmap = false;
        at += 1;
        while at < lines.len() && lines[at] != "ENDCHAR" {
            let line = lines[at];
            if in_bitmap {
                bitmap.push((at + 1, line.to_string()));
            } else if line == "BITMAP" {
                in_bitmap = true;
            } else if let Some(value) = line.strip_prefix("ENCODING ") {
                let first = value.split_whitespace().next().unwrap_or("");
                encoding = Some(parse_i64(first, path, at + 1, "ENCODING")?);
            } else if let Some(value) = line.strip_prefix("DWIDTH ") {
                let values = parse_numbers(value, path, at + 1, "DWIDTH", 2)?;
                dwidth = Some(values[0]);
            } else if let Some(value) = line.strip_prefix("BBX ") {
                let values = parse_numbers(value, path, at + 1, "BBX", 4)?;
                bbx = Some((values[0], values[1], values[2], values[3]));
            }
            at += 1;
        }
        if at == lines.len() {
            return parse_error(
                path,
                start_line,
                format!("STARTCHAR '{name}' に ENDCHAR が無い"),
            );
        }
        let encoding = encoding.ok_or_else(|| parse(path, start_line, "ENCODING が無い"))?;
        let dwidth = dwidth.ok_or_else(|| parse(path, start_line, "DWIDTH が無い"))?;
        let bbx = bbx.ok_or_else(|| parse(path, start_line, "BBX が無い"))?;
        if !in_bitmap {
            return parse_error(path, start_line, "BITMAP が無い");
        }
        output.push(RawGlyph {
            name: name.to_string(),
            line: start_line,
            encoding,
            dwidth,
            bbx,
            bitmap,
        });
        at += 1;
    }
    Ok(output)
}

fn draw_glyph(
    raw: &RawGlyph,
    line_height: u32,
    baseline: u32,
    path: &Path,
) -> Result<IndexedCanvas> {
    let (width, height, x_offset, y_offset) = raw.bbx;
    if width < 0 || height < 0 {
        return parse_error(
            path,
            raw.line,
            format!("字 '{}' の BBX の大きさが負である", raw.name),
        );
    }
    if raw.bitmap.len() != height as usize {
        return parse_error(
            path,
            raw.line,
            format!(
                "字 '{}' の BITMAP は {} 行だが BBX の高さは {height}",
                raw.name,
                raw.bitmap.len()
            ),
        );
    }
    let mut canvas =
        IndexedCanvas::filled(raw.dwidth as u32, line_height, 0).with_transparent(Some(0));
    let top = baseline as i64 - (y_offset as i64 + height as i64);
    let bytes_per_row = (width as usize).div_ceil(8);
    for (row, (line, hex)) in raw.bitmap.iter().enumerate() {
        if hex.len() != bytes_per_row * 2 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return parse_error(
                path,
                *line,
                format!(
                    "字 '{}' の BITMAP 行 '{hex}' のバイト数が BBX 幅 {width} と合わない",
                    raw.name
                ),
            );
        }
        let bytes = (0..bytes_per_row)
            .map(|index| u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| {
                parse(
                    path,
                    *line,
                    format!("字 '{}' の BITMAP を読めない", raw.name),
                )
            })?;
        for x in 0..width as usize {
            if bytes[x / 8] & (0x80 >> (x % 8)) != 0 {
                let placed_x = i64::from(x_offset) + x as i64;
                let placed_y = top + row as i64;
                if placed_x >= 0
                    && placed_x < i64::from(raw.dwidth)
                    && placed_y >= 0
                    && placed_y < i64::from(line_height)
                {
                    let _ = canvas.set(placed_x as i32, placed_y as i32, 1);
                }
            }
        }
    }
    Ok(canvas)
}

fn overflow(
    bbx: (i32, i32, i32, i32),
    advance: i32,
    line_height: u32,
    baseline: u32,
) -> (u32, u32, u32, u32) {
    let (width, height, x, y_offset) = bbx;
    let top = baseline as i64 - (y_offset as i64 + height as i64);
    let left = (-i64::from(x)).max(0) as u32;
    let right = (i64::from(x) + i64::from(width) - i64::from(advance)).max(0) as u32;
    let above = (-top).max(0) as u32;
    let below = (top + i64::from(height) - i64::from(line_height)).max(0) as u32;
    (left, right, above, below)
}

fn outside_detail(character: char, amounts: (u32, u32, u32, u32)) -> String {
    let labels = ["左", "右", "上", "下"];
    let values = [amounts.0, amounts.1, amounts.2, amounts.3];
    let parts = labels
        .into_iter()
        .zip(values)
        .filter(|(_, amount)| *amount != 0)
        .map(|(label, amount)| format!("{label}へ {amount} px"))
        .collect::<Vec<_>>();
    format!("{}: {}", shown(character), parts.join("，"))
}

fn shown(character: char) -> String {
    if character.is_whitespace() || character.is_control() {
        format!("U+{:04X}", character as u32)
    } else {
        format!("'{character}' (U+{:04X})", character as u32)
    }
}

fn most_common_width(glyphs: &[Glyph]) -> u32 {
    let mut counts = BTreeMap::<u32, usize>::new();
    for glyph in glyphs {
        *counts.entry(glyph.canvas.width()).or_default() += 1;
    }
    let mut best = (0, 0);
    for (width, count) in counts {
        if count > best.1 {
            best = (width, count);
        }
    }
    best.0
}

fn map_encoding(encoding: i64, charset: &str) -> Option<char> {
    let code = u32::try_from(encoding).ok()?;
    if charset.starts_with("ISO10646") {
        return char::from_u32(code);
    }
    jis0208().get(&code).copied()
}

fn jis0208() -> &'static BTreeMap<u32, char> {
    static TABLE: OnceLock<BTreeMap<u32, char>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = BTreeMap::new();
        for line in JIS0208.lines() {
            let values = line
                .split_whitespace()
                .take_while(|field| field.starts_with("0x"))
                .filter_map(|field| u32::from_str_radix(&field[2..], 16).ok())
                .collect::<Vec<_>>();
            if let [_, jis, unicode, ..] = values.as_slice()
                && let Some(character) = char::from_u32(*unicode)
            {
                table.insert(*jis, character);
            }
        }
        table
    })
}

fn unquote(value: &str) -> String {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value)
        .replace("\\\"", "\"")
        .replace("\\\\", "\\")
}

fn parse_numbers(
    value: &str,
    path: &Path,
    line: usize,
    field: &str,
    count: usize,
) -> Result<Vec<i32>> {
    let values = value
        .split_whitespace()
        .map(|value| parse_i32(value, path, line, field))
        .collect::<Result<Vec<_>>>()?;
    if values.len() != count {
        return parse_error(
            path,
            line,
            format!("{field} は {count} 個の整数でなければならない"),
        );
    }
    Ok(values)
}

fn parse_i32(value: &str, path: &Path, line: usize, field: &str) -> Result<i32> {
    value
        .parse()
        .map_err(|_| parse(path, line, format!("{field} の整数 '{value}' を読めない")))
}

fn parse_i64(value: &str, path: &Path, line: usize, field: &str) -> Result<i64> {
    value
        .parse()
        .map_err(|_| parse(path, line, format!("{field} の整数 '{value}' を読めない")))
}

fn parse_usize(value: &str, path: &Path, line: usize, field: &str) -> Result<usize> {
    value
        .parse()
        .map_err(|_| parse(path, line, format!("{field} の整数 '{value}' を読めない")))
}

fn parse(path: &Path, line: usize, message: impl Into<String>) -> IoError {
    IoError::Parse {
        path: path.to_path_buf(),
        line,
        message: message.into(),
    }
}

fn parse_error<T>(path: &Path, line: usize, message: impl Into<String>) -> Result<T> {
    Err(parse(path, line, message))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn bdf(registry: &str, encoding: i32, dwidth: i32, bbx: &str, bitmap: &str) -> String {
        format!(
            "STARTFONT 2.1\n\
             FONT test\n\
             STARTPROPERTIES 5\n\
             FAMILY_NAME \"Test Font\"\n\
             CHARSET_REGISTRY \"{registry}\"\n\
             FONT_ASCENT 3\n\
             FONT_DESCENT 1\n\
             COPYRIGHT \"Test Copyright\"\n\
             ENDPROPERTIES\n\
             CHARS 1\n\
             STARTCHAR test\n\
             ENCODING {encoding}\n\
             DWIDTH {dwidth} 0\n\
             BBX {bbx}\n\
             BITMAP\n\
             {bitmap}\
             ENDCHAR\n\
             ENDFONT\n"
        )
    }

    #[test]
    fn places_bbx_from_the_baseline_and_sets_transparent_zero() {
        let text = bdf("ISO10646", 65, 4, "2 2 1 0", "80\n40\n");
        let imported =
            import_bdf(&text, Path::new("test.bdf"), &BdfImportOptions::default()).unwrap();
        let glyph = &imported.font.glyphs[0];
        assert_eq!(glyph.character, 'A');
        assert_eq!(glyph.canvas.transparent(), Some(0));
        assert_eq!(glyph.canvas.get(1, 1), Some(1));
        assert_eq!(glyph.canvas.get(2, 2), Some(1));
        assert_eq!(glyph.canvas.get(0, 0), Some(0));
        assert_eq!(
            imported.font.meta.copyright.as_deref(),
            Some("Test Copyright")
        );
    }

    #[test]
    fn outside_bbx_errors_by_default_and_clip_counts_the_glyph() {
        let text = bdf("ISO10646", 65, 2, "3 1 -1 0", "E0\n");
        let error =
            import_bdf(&text, Path::new("test.bdf"), &BdfImportOptions::default()).unwrap_err();
        assert!(error.to_string().contains("左へ 1"), "{error}");

        let imported = import_bdf(
            &text,
            Path::new("test.bdf"),
            &BdfImportOptions {
                clip: true,
                ..BdfImportOptions::default()
            },
        )
        .unwrap();
        assert_eq!(imported.report.clipped, 1);
        assert_eq!(
            imported.font.glyphs[0].canvas.pixels(),
            &[0, 0, 0, 0, 1, 1, 0, 0]
        );
    }

    #[test]
    fn encoding_minus_one_is_skipped_and_reported() {
        let text = bdf("ISO10646", -1, 2, "1 1 0 0", "80\n");
        let imported =
            import_bdf(&text, Path::new("test.bdf"), &BdfImportOptions::default()).unwrap();
        assert!(imported.font.glyphs.is_empty());
        assert_eq!(imported.report.skipped_unencoded, 1);
    }

    #[test]
    fn jisx0208_encoding_is_mapped_with_the_bundled_unicode_table() {
        let text = bdf("JISX0208.1990", 0x2422, 2, "1 1 0 0", "80\n");
        let imported =
            import_bdf(&text, Path::new("test.bdf"), &BdfImportOptions::default()).unwrap();
        assert_eq!(imported.font.glyphs[0].character, 'あ');
        assert_eq!(imported.report.unmapped, 0);
    }

    #[test]
    fn crlf_bdf_is_accepted() {
        let text = bdf("ISO10646", 65, 2, "1 1 0 0", "80\n").replace('\n', "\r\n");
        let imported =
            import_bdf(&text, Path::new("test.bdf"), &BdfImportOptions::default()).unwrap();
        assert_eq!(imported.font.glyphs.len(), 1);
    }

    #[test]
    fn extreme_offsets_do_not_overflow_when_clipping() {
        let text = bdf("ISO10646", 65, 2, "1 1 -2147483648 0", "80\n");
        let imported = import_bdf(
            &text,
            Path::new("test.bdf"),
            &BdfImportOptions {
                clip: true,
                ..BdfImportOptions::default()
            },
        )
        .unwrap();
        assert_eq!(imported.report.clipped, 1);
        assert!(
            imported.font.glyphs[0]
                .canvas
                .pixels()
                .iter()
                .all(|&pixel| pixel == 0)
        );
    }
}
