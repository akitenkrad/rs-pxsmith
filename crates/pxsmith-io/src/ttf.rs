//! TrueType/OpenType の Unicode 字形を単色の画素フォントへ取り込む．

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use pxsmith_core::IndexedCanvas;
use pxsmith_core::font::{FORMAT_VERSION, Font, FontMeta, FontMode, Glyph, Kern};
use ttf_parser::gpos::{PairAdjustment, PositioningSubtable};
use ttf_parser::gsub::{SingleSubstitution, SubstitutionSubtable};
use ttf_parser::{Face, GlyphId, RasterGlyphImage, RasterImageFormat, Tag};

use crate::error::{IoError, Result};
use crate::raster::Outline;

/// TTF/OTF/TTC 取り込みの指定．
#[derive(Clone, Debug)]
pub struct TtfImportOptions {
    /// 1 em の画素数．
    pub size: u16,
    /// TTC の face 添字．単一 face のフォントでは 0．
    pub face: u32,
    /// 埋め込みビットマップを使わず，必ず輪郭を使う．
    pub outline: bool,
    /// 升からはみ出す墨を切る．既定では取り込み全体を失敗させる．
    pub clip: bool,
    /// 指定したときは，Unicode の字が集合に含まれるものだけ取り込む．
    pub chars: Option<BTreeSet<char>>,
}

/// TTF/OTF/TTC から何を取り込み，どの代替経路を使ったか．
#[derive(Clone, Debug, Default)]
pub struct TtfImportReport {
    pub mapped_characters: usize,
    pub imported_characters: usize,
    pub filtered_out: usize,
    pub embedded_bitmaps: usize,
    pub outline_glyphs: usize,
    pub bitmap_missing: usize,
    pub bitmap_wrong_ppem: usize,
    pub bitmap_unsupported_format: usize,
    pub clipped: usize,
    pub rounded_advances: usize,
    pub off_grid_glyphs: usize,
    pub vertical_glyphs: usize,
    pub unsupported_vertical_lookups: usize,
    pub kern_pairs: usize,
    pub line_height_rounding: f64,
    pub baseline_rounding: f64,
}

/// TTF/OTF/TTC 取り込みの結果．
#[derive(Clone, Debug)]
pub struct TtfImport {
    pub font: Font,
    pub report: TtfImportReport,
}

/// フォントデータから指定 face を読み，単色の画素フォントへ変換する．
pub fn import_ttf(data: &[u8], path: &Path, options: &TtfImportOptions) -> Result<TtfImport> {
    if options.size == 0 {
        return Err(IoError::TtfInvalidSize);
    }
    let faces = ttf_parser::fonts_in_collection(data).unwrap_or(1);
    if options.face >= faces {
        return Err(IoError::TtfFaceOutOfRange {
            path: path.to_path_buf(),
            face: options.face,
            faces,
        });
    }
    let face = Face::parse(data, options.face).map_err(|error| IoError::TtfParse {
        path: path.to_path_buf(),
        detail: error.to_string(),
    })?;
    let units_per_em = face.units_per_em();
    let ascender = rounded(i32::from(face.ascender()), options.size, units_per_em);
    let descender = rounded(i32::from(face.descender()), options.size, units_per_em);
    let line_height_exact = (f64::from(face.ascender()) - f64::from(face.descender()))
        * f64::from(options.size)
        / f64::from(units_per_em);
    let line_height = ascender.value - descender.value;
    if line_height <= 0 || ascender.value < 0 || ascender.value > line_height {
        return Err(IoError::TtfParse {
            path: path.to_path_buf(),
            detail: format!(
                "hhea の ascender {} / descender {} から有効な行高を作れない",
                face.ascender(),
                face.descender()
            ),
        });
    }

    let all_mappings = unicode_mappings(&face);
    let mut report = TtfImportReport {
        mapped_characters: all_mappings.len(),
        line_height_rounding: f64::from(line_height) - line_height_exact,
        baseline_rounding: ascender.delta,
        ..TtfImportReport::default()
    };
    let mappings = all_mappings
        .into_iter()
        .filter(|(character, _)| {
            let keep = options
                .chars
                .as_ref()
                .is_none_or(|chars| chars.contains(character));
            report.filtered_out += usize::from(!keep);
            keep
        })
        .collect::<Vec<_>>();
    report.imported_characters = mappings.len();

    let vertical = vertical_substitutions(&face, &mut report);
    let has_bitmaps = has_bitmap_tables(&face);
    let mut rendered = Vec::new();
    let mut outside = Vec::new();
    let mut base_advances = Vec::new();
    for &(character, glyph_id) in &mappings {
        let advance = glyph_advance(&face, glyph_id, options, &mut report);
        base_advances.push(advance);
        let (glyph, overflow, off_grid) = render_glyph(
            &face,
            glyph_id,
            character,
            None,
            advance,
            line_height as u32,
            ascender.value as u32,
            has_bitmaps,
            options,
            &mut report,
        );
        if off_grid {
            report.off_grid_glyphs += 1;
        }
        if overflow {
            if options.clip {
                report.clipped += 1;
            } else {
                outside.push(shown(character));
            }
        }
        rendered.push((glyph, advance));

        if let Some(&vertical_id) = vertical.get(&glyph_id.0) {
            let vertical_advance = glyph_advance_without_report(&face, vertical_id, options);
            let (glyph, overflow, _) = render_glyph(
                &face,
                vertical_id,
                character,
                Some("vert"),
                vertical_advance,
                line_height as u32,
                ascender.value as u32,
                has_bitmaps,
                options,
                &mut report,
            );
            if overflow {
                if options.clip {
                    report.clipped += 1;
                } else {
                    outside.push(format!("{} の vert", shown(character)));
                }
            }
            report.vertical_glyphs += 1;
            rendered.push((glyph, vertical_advance));
        }
    }
    if !outside.is_empty() {
        return Err(IoError::TtfOutside {
            count: outside.len(),
            details: outside,
        });
    }

    let default_advance = most_common(&base_advances);
    let glyphs = rendered
        .into_iter()
        .map(|(mut glyph, advance)| {
            if advance != default_advance {
                glyph.advance = Some(advance);
            }
            glyph
        })
        .collect::<Vec<_>>();
    let kerns = kerning(&face, &mappings, options, &mut report);
    let family_name = name(&face, ttf_parser::name_id::FAMILY)
        .or_else(|| {
            path.file_stem()
                .map(|value| value.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "font".to_string());
    let copyright = [
        name(&face, ttf_parser::name_id::COPYRIGHT_NOTICE),
        name(&face, ttf_parser::name_id::LICENSE),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("\n");
    let mut font = Font {
        meta: FontMeta {
            format: FORMAT_VERSION,
            name: family_name,
            mode: FontMode::Mono,
            line_height: line_height as u32,
            baseline: ascender.value as u32,
            default_advance,
            source: path
                .file_name()
                .map(|value| value.to_string_lossy().into_owned()),
            copyright: (!copyright.is_empty()).then_some(copyright),
            palette: None,
        },
        glyphs,
        kerns,
    };
    font.validate()?;
    font.canonicalize();
    Ok(TtfImport { font, report })
}

fn unicode_mappings(face: &Face<'_>) -> Vec<(char, GlyphId)> {
    let mut characters = BTreeSet::new();
    if let Some(cmap) = face.tables().cmap {
        for subtable in cmap
            .subtables
            .into_iter()
            .filter(|table| table.is_unicode())
        {
            subtable.codepoints(|codepoint| {
                if let Some(character) = char::from_u32(codepoint) {
                    characters.insert(character);
                }
            });
        }
    }
    characters
        .into_iter()
        .filter_map(|character| face.glyph_index(character).map(|glyph| (character, glyph)))
        .collect()
}

fn has_bitmap_tables(face: &Face<'_>) -> bool {
    let tables = face.tables();
    tables.sbix.is_some() || tables.bdat.is_some() || tables.ebdt.is_some() || tables.cbdt.is_some()
}

fn glyph_advance(
    face: &Face<'_>,
    glyph_id: GlyphId,
    options: &TtfImportOptions,
    report: &mut TtfImportReport,
) -> u32 {
    let value = rounded(
        i32::from(face.glyph_hor_advance(glyph_id).unwrap_or(0)),
        options.size,
        face.units_per_em(),
    );
    report.rounded_advances += usize::from(value.changed);
    value.value.max(0) as u32
}

fn glyph_advance_without_report(
    face: &Face<'_>,
    glyph_id: GlyphId,
    options: &TtfImportOptions,
) -> u32 {
    rounded(
        i32::from(face.glyph_hor_advance(glyph_id).unwrap_or(0)),
        options.size,
        face.units_per_em(),
    )
    .value
    .max(0) as u32
}

#[allow(clippy::too_many_arguments)]
fn render_glyph(
    face: &Face<'_>,
    glyph_id: GlyphId,
    character: char,
    variant: Option<&str>,
    advance: u32,
    line_height: u32,
    baseline: u32,
    has_bitmaps: bool,
    options: &TtfImportOptions,
    report: &mut TtfImportReport,
) -> (Glyph, bool, bool) {
    if !options.outline {
        if let Some(image) = raster_image(face, glyph_id, options.size) {
            if image.pixels_per_em != options.size {
                report.bitmap_wrong_ppem += 1;
            } else if matches!(
                image.format,
                RasterImageFormat::BitmapMono | RasterImageFormat::BitmapMonoPacked
            ) {
                report.embedded_bitmaps += 1;
                let (canvas, overflow) = draw_bitmap(image, advance, line_height, baseline);
                return (
                    Glyph {
                        character,
                        variant: variant.map(str::to_owned),
                        advance: None,
                        canvas,
                    },
                    overflow,
                    false,
                );
            } else {
                report.bitmap_unsupported_format += 1;
            }
        } else if has_bitmaps {
            report.bitmap_missing += 1;
        }
    }

    report.outline_glyphs += 1;
    let mut outline = Outline::new(f64::from(options.size) / f64::from(face.units_per_em()));
    let _ = face.outline_glyph(glyph_id, &mut outline);
    let off_grid = outline.is_off_grid();
    let (canvas, overflow) = outline.rasterize(advance, line_height, baseline);
    (
        Glyph {
            character,
            variant: variant.map(str::to_owned),
            advance: None,
            canvas,
        },
        overflow,
        off_grid,
    )
}

fn raster_image<'a>(face: &Face<'a>, glyph_id: GlyphId, size: u16) -> Option<RasterGlyphImage<'a>> {
    let tables = face.tables();
    let candidates = [
        tables
            .sbix
            .and_then(|table| table.best_strike(size))
            .and_then(|strike| strike.get(glyph_id)),
        tables.bdat.and_then(|table| table.get(glyph_id, size)),
        tables.ebdt.and_then(|table| table.get(glyph_id, size)),
        tables.cbdt.and_then(|table| table.get(glyph_id, size)),
    ];
    candidates
        .iter()
        .flatten()
        .find(|image| image.pixels_per_em == size)
        .copied()
        .or_else(|| candidates.into_iter().flatten().next())
}

fn draw_bitmap(
    image: RasterGlyphImage<'_>,
    width: u32,
    height: u32,
    baseline: u32,
) -> (IndexedCanvas, bool) {
    let mut canvas = IndexedCanvas::filled(width, height, 0).with_transparent(Some(0));
    let mut overflow = false;
    let row_stride = usize::from(image.width).div_ceil(8);
    for row in 0..usize::from(image.height) {
        for column in 0..usize::from(image.width) {
            let bit = match image.format {
                RasterImageFormat::BitmapMono => row * row_stride * 8 + column,
                RasterImageFormat::BitmapMonoPacked => row * usize::from(image.width) + column,
                _ => continue,
            };
            if image
                .data
                .get(bit / 8)
                .is_none_or(|byte| byte & (0x80 >> (bit % 8)) == 0)
            {
                continue;
            }
            let x = i64::from(image.x) + column as i64;
            let y = i64::from(baseline) - i64::from(image.y) - i64::from(image.height) + row as i64;
            if x >= 0 && x < i64::from(width) && y >= 0 && y < i64::from(height) {
                let _ = canvas.set(x as i32, y as i32, 1);
            } else {
                overflow = true;
            }
        }
    }
    (canvas, overflow)
}

fn vertical_substitutions(face: &Face<'_>, report: &mut TtfImportReport) -> BTreeMap<u16, GlyphId> {
    let mut output = BTreeMap::new();
    let Some(gsub) = face.tables().gsub else {
        return output;
    };
    for tag in [Tag::from_bytes(b"vert"), Tag::from_bytes(b"vrt2")] {
        let Some(feature) = gsub.features.find(tag) else {
            continue;
        };
        for lookup_index in feature.lookup_indices {
            let Some(lookup) = gsub.lookups.get(lookup_index) else {
                continue;
            };
            for subtable in lookup.subtables.into_iter::<SubstitutionSubtable<'_>>() {
                let SubstitutionSubtable::Single(single) = subtable else {
                    report.unsupported_vertical_lookups += 1;
                    continue;
                };
                for glyph in 0..face.number_of_glyphs() {
                    let glyph = GlyphId(glyph);
                    let Some(index) = single.coverage().get(glyph) else {
                        continue;
                    };
                    let substitute = match single {
                        SingleSubstitution::Format1 { delta, .. } => {
                            let value = i32::from(glyph.0) + i32::from(delta);
                            u16::try_from(value).ok().map(GlyphId)
                        }
                        SingleSubstitution::Format2 { substitutes, .. } => substitutes.get(index),
                    };
                    if let Some(substitute) = substitute {
                        output.insert(glyph.0, substitute);
                    }
                }
            }
        }
    }
    output
}

fn kerning(
    face: &Face<'_>,
    mappings: &[(char, GlyphId)],
    options: &TtfImportOptions,
    report: &mut TtfImportReport,
) -> Vec<Kern> {
    let mut values = BTreeMap::<(char, char), i32>::new();
    let used_gpos = gpos_kerning(face, mappings, &mut values);
    if !used_gpos {
        legacy_kerning(face, mappings, &mut values);
    }
    let kerns = values
        .into_iter()
        .filter_map(|((left, right), units)| {
            let amount = rounded(units, options.size, face.units_per_em()).value;
            (amount != 0).then(|| Kern {
                pair: [left, right].into_iter().collect(),
                amount,
            })
        })
        .collect::<Vec<_>>();
    report.kern_pairs = kerns.len();
    kerns
}

fn gpos_kerning(
    face: &Face<'_>,
    mappings: &[(char, GlyphId)],
    values: &mut BTreeMap<(char, char), i32>,
) -> bool {
    let Some(gpos) = face.tables().gpos else {
        return false;
    };
    let Some(feature) = gpos.features.find(Tag::from_bytes(b"kern")) else {
        return false;
    };
    for lookup_index in feature.lookup_indices {
        let Some(lookup) = gpos.lookups.get(lookup_index) else {
            continue;
        };
        for subtable in lookup.subtables.into_iter::<PositioningSubtable<'_>>() {
            let PositioningSubtable::Pair(pair) = subtable else {
                continue;
            };
            for &(left_char, left_glyph) in mappings {
                let Some(coverage_index) = pair.coverage().get(left_glyph) else {
                    continue;
                };
                for &(right_char, right_glyph) in mappings {
                    let adjustment = match pair {
                        PairAdjustment::Format1 { sets, .. } => sets
                            .get(coverage_index)
                            .and_then(|set| set.get(right_glyph))
                            .map(|records| records.0.x_advance),
                        PairAdjustment::Format2 {
                            classes, matrix, ..
                        } => matrix
                            .get((classes.0.get(left_glyph), classes.1.get(right_glyph)))
                            .map(|records| records.0.x_advance),
                    };
                    if let Some(adjustment) = adjustment
                        && adjustment != 0
                    {
                        *values.entry((left_char, right_char)).or_default() +=
                            i32::from(adjustment);
                    }
                }
            }
        }
    }
    true
}

fn legacy_kerning(
    face: &Face<'_>,
    mappings: &[(char, GlyphId)],
    values: &mut BTreeMap<(char, char), i32>,
) {
    let Some(kern) = face.tables().kern else {
        return;
    };
    let mut characters = BTreeMap::<u16, Vec<char>>::new();
    for &(character, glyph) in mappings {
        characters.entry(glyph.0).or_default().push(character);
    }
    for subtable in kern.subtables {
        if !subtable.horizontal || subtable.variable || subtable.has_cross_stream {
            continue;
        }
        let ttf_parser::kern::Format::Format0(format) = subtable.format else {
            continue;
        };
        for pair in format.pairs {
            let (Some(left), Some(right)) = (
                characters.get(&pair.left().0),
                characters.get(&pair.right().0),
            ) else {
                continue;
            };
            for &left_char in left {
                for &right_char in right {
                    *values.entry((left_char, right_char)).or_default() += i32::from(pair.value);
                }
            }
        }
    }
}

fn most_common(values: &[u32]) -> u32 {
    let mut counts = BTreeMap::<u32, usize>::new();
    for &value in values {
        *counts.entry(value).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by_key(|&(value, count)| (count, std::cmp::Reverse(value)))
        .map_or(0, |(value, _)| value)
}

fn name(face: &Face<'_>, id: u16) -> Option<String> {
    face.names()
        .into_iter()
        .filter(|name| name.name_id == id)
        .find_map(|name| name.to_string())
        .filter(|name| !name.is_empty())
}

fn shown(character: char) -> String {
    if character.is_whitespace() || character.is_control() {
        format!("U+{:04X}", character as u32)
    } else {
        format!("{character}(U+{:04X})", character as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(path: &str) -> Vec<u8> {
        std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../testdata/fonts")
                .join(path),
        )
        .unwrap()
    }

    fn options(size: u16) -> TtfImportOptions {
        TtfImportOptions {
            size,
            face: 0,
            outline: false,
            clip: false,
            chars: Some(BTreeSet::from(['A'])),
        }
    }

    #[test]
    fn rounding_delta_reports_the_signed_pixel_difference() {
        let exact = rounded(600, 8, 800);
        assert_eq!(exact.value, 6);
        assert_eq!(exact.delta, 0.0);
        assert!(!exact.changed);

        let down = rounded(649, 8, 800);
        assert_eq!(down.value, 6);
        assert!((down.delta + 0.49).abs() < 1.0e-9);
        assert!(down.changed);

        let half_up = rounded(650, 8, 800);
        assert_eq!(half_up.value, 7);
        assert!((half_up.delta - 0.5).abs() < 1.0e-9);
        assert!(half_up.changed);
    }

    #[test]
    fn face_out_of_range_is_an_error_instead_of_a_panic() {
        let data = fixture("synthetic/pxsynth-gpos.ttf");
        let mut options = options(8);
        options.face = 1;
        let error = import_ttf(&data, Path::new("one-face.ttf"), &options).unwrap_err();
        assert!(error.to_string().contains("face 1"), "{error}");
    }

    #[test]
    fn zero_pixels_per_em_is_rejected() {
        let data = fixture("synthetic/pxsynth-gpos.ttf");
        let error = import_ttf(&data, Path::new("zero.ttf"), &options(0)).unwrap_err();
        assert!(error.to_string().contains("--size"), "{error}");
    }

    #[test]
    fn a_nonmatching_bitmap_strike_falls_back_to_outline() {
        let data = fixture("misaki/misaki_gothic.ttf");
        let imported = import_ttf(&data, Path::new("misaki_gothic.ttf"), &options(7)).unwrap();
        assert_eq!(imported.report.embedded_bitmaps, 0);
        assert_eq!(imported.report.outline_glyphs, 1);
        assert_eq!(imported.report.bitmap_wrong_ppem, 1);
    }
}

#[derive(Clone, Copy, Debug)]
struct Rounded {
    value: i32,
    delta: f64,
    changed: bool,
}

fn rounded(units: i32, size: u16, units_per_em: u16) -> Rounded {
    let exact = f64::from(units) * f64::from(size) / f64::from(units_per_em);
    let value = exact.round() as i32;
    let delta = f64::from(value) - exact;
    Rounded {
        value,
        delta,
        changed: delta.abs() > 1.0e-9,
    }
}
