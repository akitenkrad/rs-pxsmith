//! ビットマップフォントの純粋な組版処理．

use std::collections::{BTreeMap, BTreeSet};

use unicode_linebreak::{BreakClass, break_property, linebreaks};

use crate::{Font, Glyph, IndexedCanvas, VerticalOrientation, vertical_orientation};

/// 組版の書字方向．
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum WritingDirection {
    #[default]
    Horizontal,
    Vertical,
}

/// 行を画布の幅へ寄せる方法．
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// CJ 類の行頭禁則の強さ．
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Kinsoku {
    #[default]
    Normal,
    Strict,
}

/// フォント組版の設定．
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontLayoutOptions {
    pub direction: WritingDirection,
    pub letter_spacing: u32,
    pub line_spacing: u32,
    pub align: TextAlign,
    pub wrap: Option<u32>,
    pub kern: bool,
    pub kinsoku: Kinsoku,
    pub hang: bool,
    pub missing: Option<char>,
    pub tcy: Option<u32>,
}

impl Default for FontLayoutOptions {
    fn default() -> Self {
        Self {
            direction: WritingDirection::Horizontal,
            letter_spacing: 0,
            line_spacing: 0,
            align: TextAlign::Left,
            wrap: None,
            kern: true,
            kinsoku: Kinsoku::Normal,
            hang: false,
            missing: None,
            tcy: None,
        }
    }
}

/// 組版中に起きた代替や例外的な折り返しの集計．
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FontLayoutReport {
    pub lines: Vec<String>,
    pub replacements: BTreeMap<char, usize>,
    pub forced_breaks: usize,
    pub hanging_punctuation: usize,
    pub unconverted_cj: usize,
    pub vertical_variants: usize,
    pub rotated_r: usize,
    pub rotated_tr: usize,
    pub upright_tu: usize,
    pub tcy_groups: usize,
    pub tcy_fallbacks: usize,
}

/// 組版済みの画布と報告．
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontLayoutResult {
    pub canvas: IndexedCanvas,
    pub report: FontLayoutReport,
}

/// フォントを組版できなかった理由．
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FontLayoutError {
    #[error("フォントに無い字がある: {characters:?}")]
    MissingGlyphs { characters: Vec<char> },
    #[error("代替字 '{character}' がフォントに無い")]
    MissingReplacement { character: char },
    #[error("横書きに使える基底字形が 1 字も無い")]
    NoBaseGlyphs,
    #[error("字 '{character}' に透明添字が明示されていない")]
    MissingTransparentIndex { character: char },
    #[error(
        "字 '{character}' の透明添字 {actual} が同じフォントの透明添字 {expected} と一致しない"
    )]
    InconsistentTransparentIndex {
        character: char,
        expected: u8,
        actual: u8,
    },
    #[error("組版した画布の寸法が扱える範囲を超えた")]
    DimensionsOverflow,
}

/// `font` の基底字形を使って `text` を組版する．
pub fn layout_text(
    font: &Font,
    text: &str,
    options: &FontLayoutOptions,
) -> Result<FontLayoutResult, FontLayoutError> {
    let glyphs = font
        .glyphs
        .iter()
        .enumerate()
        .filter(|(_, glyph)| glyph.variant.is_none())
        .map(|(index, glyph)| (glyph.character, index))
        .collect::<BTreeMap<_, _>>();
    let mut transparent = None;
    for (&character, &index) in &glyphs {
        let actual = font.glyphs[index]
            .canvas
            .transparent()
            .ok_or(FontLayoutError::MissingTransparentIndex { character })?;
        match transparent {
            None => transparent = Some(actual),
            Some(expected) if expected != actual => {
                return Err(FontLayoutError::InconsistentTransparentIndex {
                    character,
                    expected,
                    actual,
                });
            }
            Some(_) => {}
        }
    }
    let transparent = transparent.ok_or(FontLayoutError::NoBaseGlyphs)?;
    let normalized = text.replace("\r\n", "\n");
    let missing = normalized
        .chars()
        .filter(|character| *character != '\n' && !glyphs.contains_key(character))
        .collect::<BTreeSet<_>>();
    if !missing.is_empty() && options.missing.is_none() {
        return Err(FontLayoutError::MissingGlyphs {
            characters: missing.into_iter().collect(),
        });
    }
    if let Some(replacement) = options.missing
        && !glyphs.contains_key(&replacement)
    {
        return Err(FontLayoutError::MissingReplacement {
            character: replacement,
        });
    }

    let mut report = FontLayoutReport::default();
    let resolved = normalized
        .chars()
        .map(|character| {
            if character == '\n' || glyphs.contains_key(&character) {
                character
            } else {
                *report.replacements.entry(character).or_default() += 1;
                options.missing.expect("代替字は検査済み")
            }
        })
        .collect::<String>();
    if options.direction == WritingDirection::Vertical {
        return layout_vertical(font, &resolved, options, transparent, &glyphs, report);
    }
    let kerns = if options.kern {
        font.kerns
            .iter()
            .filter_map(|kern| {
                let mut pair = kern.pair.chars();
                Some(((pair.next()?, pair.next()?), kern.amount))
            })
            .collect::<BTreeMap<_, _>>()
    } else {
        BTreeMap::new()
    };

    let mut lines = Vec::new();
    for paragraph in resolved.split('\n') {
        let chars = paragraph
            .chars()
            .map(|character| ResolvedChar {
                character,
                glyph: glyphs[&character],
            })
            .collect::<Vec<_>>();
        let (breaks, unconverted_cj) = break_positions(paragraph, options.kinsoku);
        report.unconverted_cj += unconverted_cj;
        wrap_paragraph(
            font,
            &chars,
            &breaks,
            &kerns,
            options,
            &mut report,
            &mut lines,
        )?;
    }

    let width = lines.iter().map(|line| line.width).max().unwrap_or(0);
    let line_count = u32::try_from(lines.len()).map_err(|_| FontLayoutError::DimensionsOverflow)?;
    let height = font
        .meta
        .line_height
        .checked_mul(line_count)
        .and_then(|height| {
            options
                .line_spacing
                .checked_mul(line_count.saturating_sub(1))
                .and_then(|spacing| height.checked_add(spacing))
        })
        .ok_or(FontLayoutError::DimensionsOverflow)?;
    let mut canvas =
        IndexedCanvas::filled(width, height, transparent).with_transparent(Some(transparent));

    for (line_index, line) in lines.iter().enumerate() {
        let align_x = match options.align {
            TextAlign::Left => 0,
            TextAlign::Center => (width - line.width) / 2,
            TextAlign::Right => width - line.width,
        };
        let y = u32::try_from(line_index)
            .ok()
            .and_then(|line| {
                font.meta
                    .line_height
                    .checked_add(options.line_spacing)
                    .and_then(|step| step.checked_mul(line))
            })
            .ok_or(FontLayoutError::DimensionsOverflow)?;
        draw_line(
            &mut canvas,
            font,
            &line.characters,
            &kerns,
            options.letter_spacing,
            align_x,
            y,
        );
        report
            .lines
            .push(line.characters.iter().map(|item| item.character).collect());
    }

    Ok(FontLayoutResult { canvas, report })
}

#[derive(Clone, Debug)]
struct ResolvedChar {
    character: char,
    glyph: usize,
}

#[derive(Clone, Debug)]
struct Line {
    characters: Vec<ResolvedChar>,
    width: u32,
}

fn break_positions(text: &str, kinsoku: Kinsoku) -> (BTreeSet<usize>, usize) {
    let mut unconverted = 0;
    let tailored;
    let source = if kinsoku == Kinsoku::Normal {
        tailored = text
            .chars()
            .map(|character| {
                if break_property(character as u32) != BreakClass::ConditionalJapaneseStarter {
                    return character;
                }
                match character.len_utf8() {
                    3 => 'あ',
                    4 => '𠀀',
                    _ => {
                        unconverted += 1;
                        character
                    }
                }
            })
            .collect::<String>();
        tailored.as_str()
    } else {
        text
    };
    let byte_ends = text
        .char_indices()
        .map(|(offset, character)| offset + character.len_utf8())
        .collect::<Vec<_>>();
    let breaks = linebreaks(source)
        .filter_map(|(byte, _)| byte_ends.binary_search(&byte).ok().map(|index| index + 1))
        .collect();
    (breaks, unconverted)
}

fn wrap_paragraph(
    font: &Font,
    chars: &[ResolvedChar],
    breaks: &BTreeSet<usize>,
    kerns: &BTreeMap<(char, char), i32>,
    options: &FontLayoutOptions,
    report: &mut FontLayoutReport,
    output: &mut Vec<Line>,
) -> Result<(), FontLayoutError> {
    if chars.is_empty() {
        output.push(Line {
            characters: Vec::new(),
            width: 0,
        });
        return Ok(());
    }
    let Some(limit) = options.wrap else {
        output.push(make_line(font, chars, kerns, options.letter_spacing)?);
        return Ok(());
    };

    let mut start = 0;
    while start < chars.len() {
        let candidates = breaks.range((start + 1)..).copied().collect::<Vec<_>>();
        let mut fitting = None;
        let mut hanging = None;
        for end in candidates.iter().copied() {
            let trimmed = trim_end(&chars[start..end]);
            let width = measure(font, trimmed, kerns, options.letter_spacing)?;
            if width <= limit {
                fitting = Some(end);
                continue;
            }
            if options.hang
                && trimmed
                    .last()
                    .is_some_and(|item| is_hanging(item.character))
                && measure(
                    font,
                    &trimmed[..trimmed.len() - 1],
                    kerns,
                    options.letter_spacing,
                )? <= limit
            {
                hanging = Some(end);
                break;
            }
            if fitting.is_some() {
                break;
            }
        }

        let end = if let Some(end) = hanging {
            report.hanging_punctuation += 1;
            end
        } else if let Some(end) = fitting {
            end
        } else {
            report.forced_breaks += 1;
            let boundary = candidates.first().copied().unwrap_or(chars.len());
            let mut forced = start + 1;
            for end in (start + 1)..boundary {
                if measure(
                    font,
                    trim_end(&chars[start..end]),
                    kerns,
                    options.letter_spacing,
                )? <= limit
                {
                    forced = end;
                } else {
                    break;
                }
            }
            forced
        };
        output.push(make_line(
            font,
            &chars[start..end],
            kerns,
            options.letter_spacing,
        )?);
        start = end;
    }
    Ok(())
}

fn make_line(
    font: &Font,
    characters: &[ResolvedChar],
    kerns: &BTreeMap<(char, char), i32>,
    letter_spacing: u32,
) -> Result<Line, FontLayoutError> {
    let characters = trim_end(characters).to_vec();
    let width = measure(font, &characters, kerns, letter_spacing)?;
    Ok(Line { characters, width })
}

fn trim_end(chars: &[ResolvedChar]) -> &[ResolvedChar] {
    let end = chars
        .iter()
        .rposition(|item| break_property(item.character as u32) != BreakClass::Space)
        .map_or(0, |index| index + 1);
    &chars[..end]
}

fn measure(
    font: &Font,
    chars: &[ResolvedChar],
    kerns: &BTreeMap<(char, char), i32>,
    letter_spacing: u32,
) -> Result<u32, FontLayoutError> {
    let Some(first) = chars.first() else {
        return Ok(0);
    };
    let mut width = i64::from(font.glyphs[first.glyph].effective_advance(&font.meta));
    for pair in chars.windows(2) {
        let previous = &font.glyphs[pair[0].glyph];
        let current = &font.glyphs[pair[1].glyph];
        width = width
            .checked_add(i64::from(letter_spacing))
            .and_then(|width| {
                width.checked_add(i64::from(
                    kerns
                        .get(&(previous.character, current.character))
                        .copied()
                        .unwrap_or(0),
                ))
            })
            .and_then(|width| width.checked_add(i64::from(current.effective_advance(&font.meta))))
            .ok_or(FontLayoutError::DimensionsOverflow)?;
    }
    u32::try_from(width).map_err(|_| FontLayoutError::DimensionsOverflow)
}

fn draw_line(
    output: &mut IndexedCanvas,
    font: &Font,
    chars: &[ResolvedChar],
    kerns: &BTreeMap<(char, char), i32>,
    letter_spacing: u32,
    start_x: u32,
    y: u32,
) {
    let mut pen = i64::from(start_x);
    let mut previous: Option<&Glyph> = None;
    for item in chars {
        let glyph = &font.glyphs[item.glyph];
        if let Some(previous) = previous {
            pen += i64::from(previous.effective_advance(&font.meta));
            pen += i64::from(letter_spacing);
            pen += i64::from(
                kerns
                    .get(&(previous.character, glyph.character))
                    .copied()
                    .unwrap_or(0),
            );
        }
        for glyph_y in 0..glyph.canvas.height() as i32 {
            for glyph_x in 0..glyph.canvas.width() as i32 {
                let Some(index) = glyph.canvas.get(glyph_x, glyph_y) else {
                    continue;
                };
                if glyph.canvas.transparent() == Some(index) {
                    continue;
                }
                if let Ok(x) = i32::try_from(pen + i64::from(glyph_x)) {
                    output.set(x, y as i32 + glyph_y, index);
                }
            }
        }
        previous = Some(glyph);
    }
}

#[derive(Clone, Debug)]
enum VerticalItemKind {
    Glyph { glyph: usize, rotated: bool },
    Tcy { glyphs: Vec<usize> },
}

#[derive(Clone, Debug)]
struct VerticalItem {
    text: String,
    end: usize,
    kind: VerticalItemKind,
    width: u32,
    height: u32,
}

#[derive(Clone, Debug)]
enum RawVerticalItem {
    Ready(VerticalItem),
    TcyCandidate {
        characters: Vec<ResolvedChar>,
        start: usize,
        end: usize,
    },
}

#[derive(Clone, Debug)]
struct Column {
    items: Vec<VerticalItem>,
    height: u32,
}

fn layout_vertical(
    font: &Font,
    text: &str,
    options: &FontLayoutOptions,
    transparent: u8,
    base_glyphs: &BTreeMap<char, usize>,
    mut report: FontLayoutReport,
) -> Result<FontLayoutResult, FontLayoutError> {
    let variants = font
        .glyphs
        .iter()
        .enumerate()
        .filter(|(_, glyph)| glyph.variant.as_deref() == Some("vert"))
        .map(|(index, glyph)| (glyph.character, index))
        .collect::<BTreeMap<_, _>>();
    let paragraphs = text
        .split('\n')
        .map(|paragraph| {
            let characters = paragraph
                .chars()
                .map(|character| ResolvedChar {
                    character,
                    glyph: base_glyphs[&character],
                })
                .collect::<Vec<_>>();
            let (breaks, unconverted_cj) = break_positions(paragraph, options.kinsoku);
            report.unconverted_cj += unconverted_cj;
            let raw = raw_vertical_items(
                font,
                &characters,
                &variants,
                transparent,
                options.tcy,
                &mut report,
            )?;
            Ok((raw, breaks))
        })
        .collect::<Result<Vec<_>, FontLayoutError>>()?;

    let mut column_width = paragraphs
        .iter()
        .flat_map(|(items, _)| items)
        .filter_map(|item| match item {
            RawVerticalItem::Ready(item) => Some(item.width),
            RawVerticalItem::TcyCandidate { .. } => None,
        })
        .max()
        .unwrap_or(font.meta.line_height);
    if column_width == 0 {
        column_width = font.meta.line_height;
    }

    let paragraphs = paragraphs
        .into_iter()
        .map(|(raw, breaks)| {
            finish_vertical_items(font, raw, column_width, &mut report).map(|items| (items, breaks))
        })
        .collect::<Result<Vec<_>, FontLayoutError>>()?;
    column_width = column_width.max(
        paragraphs
            .iter()
            .flat_map(|(items, _)| items.iter().map(|item| item.width))
            .max()
            .unwrap_or(0),
    );

    let mut columns = Vec::new();
    for (items, breaks) in paragraphs {
        wrap_vertical_paragraph(&items, &breaks, options, &mut report, &mut columns)?;
    }

    let column_count =
        u32::try_from(columns.len()).map_err(|_| FontLayoutError::DimensionsOverflow)?;
    let width = column_width
        .checked_mul(column_count)
        .and_then(|width| {
            options
                .line_spacing
                .checked_mul(column_count.saturating_sub(1))
                .and_then(|spacing| width.checked_add(spacing))
        })
        .ok_or(FontLayoutError::DimensionsOverflow)?;
    let height = columns
        .iter()
        .map(|column| column.height)
        .max()
        .unwrap_or(0);
    let mut canvas =
        IndexedCanvas::filled(width, height, transparent).with_transparent(Some(transparent));

    for (index, column) in columns.iter().enumerate() {
        let index = u32::try_from(index).map_err(|_| FontLayoutError::DimensionsOverflow)?;
        let reverse = column_count
            .checked_sub(index + 1)
            .ok_or(FontLayoutError::DimensionsOverflow)?;
        let column_x = column_width
            .checked_add(options.line_spacing)
            .and_then(|step| step.checked_mul(reverse))
            .ok_or(FontLayoutError::DimensionsOverflow)?;
        let mut y = match options.align {
            TextAlign::Left => 0,
            TextAlign::Center => (height - column.height) / 2,
            TextAlign::Right => height - column.height,
        };
        for item in &column.items {
            let x = column_x + (column_width - item.width) / 2;
            draw_vertical_item(&mut canvas, font, item, x, y);
            y = y
                .checked_add(item.height)
                .and_then(|y| y.checked_add(options.letter_spacing))
                .ok_or(FontLayoutError::DimensionsOverflow)?;
        }
        report
            .lines
            .push(column.items.iter().map(|item| item.text.as_str()).collect());
    }

    Ok(FontLayoutResult { canvas, report })
}

fn raw_vertical_items(
    font: &Font,
    characters: &[ResolvedChar],
    variants: &BTreeMap<char, usize>,
    transparent: u8,
    tcy: Option<u32>,
    report: &mut FontLayoutReport,
) -> Result<Vec<RawVerticalItem>, FontLayoutError> {
    let tcy = tcy.and_then(|value| usize::try_from(value).ok());
    let mut output = Vec::new();
    let mut index = 0;
    while index < characters.len() {
        if characters[index].character.is_ascii_digit() {
            let end = characters[index..]
                .iter()
                .position(|item| !item.character.is_ascii_digit())
                .map_or(characters.len(), |offset| index + offset);
            if tcy.is_some_and(|maximum| maximum != 0 && end - index <= maximum) {
                output.push(RawVerticalItem::TcyCandidate {
                    characters: characters[index..end].to_vec(),
                    start: index,
                    end,
                });
            } else {
                for (offset, item) in characters[index..end].iter().enumerate() {
                    output.push(RawVerticalItem::Ready(vertical_glyph_item(
                        font,
                        item,
                        variants,
                        transparent,
                        index + offset + 1,
                        report,
                    )?));
                }
            }
            index = end;
            continue;
        }
        output.push(RawVerticalItem::Ready(vertical_glyph_item(
            font,
            &characters[index],
            variants,
            transparent,
            index + 1,
            report,
        )?));
        index += 1;
    }
    Ok(output)
}

fn vertical_glyph_item(
    font: &Font,
    item: &ResolvedChar,
    variants: &BTreeMap<char, usize>,
    transparent: u8,
    end: usize,
    report: &mut FontLayoutReport,
) -> Result<VerticalItem, FontLayoutError> {
    let orientation = vertical_orientation(item.character as u32);
    let (glyph, rotated) = if let Some(&variant) = variants.get(&item.character) {
        ensure_vertical_transparency(font, variant, transparent)?;
        report.vertical_variants += 1;
        (variant, false)
    } else {
        match orientation {
            VerticalOrientation::U => (item.glyph, false),
            VerticalOrientation::R => {
                report.rotated_r += 1;
                (item.glyph, true)
            }
            VerticalOrientation::Tu => {
                report.upright_tu += 1;
                (item.glyph, false)
            }
            VerticalOrientation::Tr => {
                report.rotated_tr += 1;
                (item.glyph, true)
            }
        }
    };
    let advance = font.glyphs[glyph].effective_advance(&font.meta);
    let (width, height) = if rotated {
        (font.meta.line_height, advance)
    } else {
        (advance, font.meta.line_height)
    };
    Ok(VerticalItem {
        text: item.character.to_string(),
        end,
        kind: VerticalItemKind::Glyph { glyph, rotated },
        width,
        height,
    })
}

fn ensure_vertical_transparency(
    font: &Font,
    glyph: usize,
    expected: u8,
) -> Result<(), FontLayoutError> {
    let character = font.glyphs[glyph].character;
    let actual = font.glyphs[glyph]
        .canvas
        .transparent()
        .ok_or(FontLayoutError::MissingTransparentIndex { character })?;
    if actual != expected {
        return Err(FontLayoutError::InconsistentTransparentIndex {
            character,
            expected,
            actual,
        });
    }
    Ok(())
}

fn finish_vertical_items(
    font: &Font,
    raw: Vec<RawVerticalItem>,
    column_width: u32,
    report: &mut FontLayoutReport,
) -> Result<Vec<VerticalItem>, FontLayoutError> {
    let mut output = Vec::new();
    for item in raw {
        match item {
            RawVerticalItem::Ready(item) => output.push(item),
            RawVerticalItem::TcyCandidate {
                characters,
                start,
                end,
            } => {
                let width = characters.iter().try_fold(0_u32, |width, item| {
                    width
                        .checked_add(font.glyphs[item.glyph].effective_advance(&font.meta))
                        .ok_or(FontLayoutError::DimensionsOverflow)
                })?;
                if width <= column_width {
                    report.tcy_groups += 1;
                    output.push(VerticalItem {
                        text: characters.iter().map(|item| item.character).collect(),
                        end,
                        kind: VerticalItemKind::Tcy {
                            glyphs: characters.iter().map(|item| item.glyph).collect(),
                        },
                        width,
                        height: font.meta.line_height,
                    });
                } else {
                    report.tcy_fallbacks += 1;
                    for (offset, item) in characters.into_iter().enumerate() {
                        let width = font.glyphs[item.glyph].effective_advance(&font.meta);
                        output.push(VerticalItem {
                            text: item.character.to_string(),
                            end: start + offset + 1,
                            kind: VerticalItemKind::Glyph {
                                glyph: item.glyph,
                                rotated: false,
                            },
                            width,
                            height: font.meta.line_height,
                        });
                    }
                }
            }
        }
    }
    Ok(output)
}

fn wrap_vertical_paragraph(
    items: &[VerticalItem],
    breaks: &BTreeSet<usize>,
    options: &FontLayoutOptions,
    report: &mut FontLayoutReport,
    output: &mut Vec<Column>,
) -> Result<(), FontLayoutError> {
    if items.is_empty() {
        output.push(Column {
            items: Vec::new(),
            height: 0,
        });
        return Ok(());
    }
    let Some(limit) = options.wrap else {
        output.push(make_column(items, options.letter_spacing)?);
        return Ok(());
    };
    let break_items = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| breaks.contains(&item.end).then_some(index + 1))
        .collect::<BTreeSet<_>>();
    let mut start = 0;
    while start < items.len() {
        let candidates = break_items
            .range((start + 1)..)
            .copied()
            .collect::<Vec<_>>();
        let mut fitting = None;
        let mut hanging = None;
        for end in candidates.iter().copied() {
            let trimmed = trim_vertical_end(&items[start..end]);
            let height = measure_vertical(trimmed, options.letter_spacing)?;
            if height <= limit {
                fitting = Some(end);
                continue;
            }
            if options.hang
                && trimmed
                    .last()
                    .and_then(|item| item.text.chars().last())
                    .is_some_and(is_hanging)
                && measure_vertical(&trimmed[..trimmed.len() - 1], options.letter_spacing)? <= limit
            {
                hanging = Some(end);
                break;
            }
            if fitting.is_some() {
                break;
            }
        }
        let end = if let Some(end) = hanging {
            report.hanging_punctuation += 1;
            end
        } else if let Some(end) = fitting {
            end
        } else {
            report.forced_breaks += 1;
            let boundary = candidates.first().copied().unwrap_or(items.len());
            let mut forced = start + 1;
            for end in (start + 1)..boundary {
                if measure_vertical(
                    trim_vertical_end(&items[start..end]),
                    options.letter_spacing,
                )? <= limit
                {
                    forced = end;
                } else {
                    break;
                }
            }
            forced
        };
        output.push(make_column(&items[start..end], options.letter_spacing)?);
        start = end;
    }
    Ok(())
}

fn make_column(items: &[VerticalItem], letter_spacing: u32) -> Result<Column, FontLayoutError> {
    let items = trim_vertical_end(items).to_vec();
    let height = measure_vertical(&items, letter_spacing)?;
    Ok(Column { items, height })
}

fn trim_vertical_end(items: &[VerticalItem]) -> &[VerticalItem] {
    let end = items
        .iter()
        .rposition(|item| {
            item.text
                .chars()
                .last()
                .is_some_and(|character| break_property(character as u32) != BreakClass::Space)
        })
        .map_or(0, |index| index + 1);
    &items[..end]
}

fn measure_vertical(items: &[VerticalItem], letter_spacing: u32) -> Result<u32, FontLayoutError> {
    let count = u32::try_from(items.len()).map_err(|_| FontLayoutError::DimensionsOverflow)?;
    items
        .iter()
        .try_fold(0_u32, |height, item| height.checked_add(item.height))
        .and_then(|height| {
            letter_spacing
                .checked_mul(count.saturating_sub(1))
                .and_then(|spacing| height.checked_add(spacing))
        })
        .ok_or(FontLayoutError::DimensionsOverflow)
}

fn draw_vertical_item(
    output: &mut IndexedCanvas,
    font: &Font,
    item: &VerticalItem,
    x: u32,
    y: u32,
) {
    match &item.kind {
        VerticalItemKind::Glyph { glyph, rotated } => {
            let glyph = &font.glyphs[*glyph];
            if *rotated {
                draw_rotated_glyph(output, glyph, item.height, x, y);
            } else {
                draw_upright_glyph(output, glyph, item.width, x, y);
            }
        }
        VerticalItemKind::Tcy { glyphs } => {
            let mut pen = x;
            for &glyph in glyphs {
                let glyph = &font.glyphs[glyph];
                let advance = glyph.effective_advance(&font.meta);
                draw_upright_glyph(output, glyph, advance, pen, y);
                pen += advance;
            }
        }
    }
}

fn draw_upright_glyph(output: &mut IndexedCanvas, glyph: &Glyph, advance: u32, x: u32, y: u32) {
    for source_y in 0..glyph.canvas.height() {
        for source_x in 0..advance {
            let Some(index) = glyph.canvas.get(source_x as i32, source_y as i32) else {
                continue;
            };
            if glyph.canvas.transparent() == Some(index) {
                continue;
            }
            let Some(target_x) = i64::from(x)
                .checked_add(i64::from(source_x))
                .and_then(|value| i32::try_from(value).ok())
            else {
                continue;
            };
            let Some(target_y) = i64::from(y)
                .checked_add(i64::from(source_y))
                .and_then(|value| i32::try_from(value).ok())
            else {
                continue;
            };
            output.set(target_x, target_y, index);
        }
    }
}

fn draw_rotated_glyph(output: &mut IndexedCanvas, glyph: &Glyph, advance: u32, x: u32, y: u32) {
    let source_height = glyph.canvas.height();
    for source_y in 0..source_height {
        for source_x in 0..advance {
            let Some(index) = glyph.canvas.get(source_x as i32, source_y as i32) else {
                continue;
            };
            if glyph.canvas.transparent() == Some(index) {
                continue;
            }
            let Some(target_x) = i64::from(x)
                .checked_add(i64::from(source_height - 1 - source_y))
                .and_then(|value| i32::try_from(value).ok())
            else {
                continue;
            };
            let Some(target_y) = i64::from(y)
                .checked_add(i64::from(source_x))
                .and_then(|value| i32::try_from(value).ok())
            else {
                continue;
            };
            output.set(target_x, target_y, index);
        }
    }
}

fn is_hanging(character: char) -> bool {
    matches!(character, '、' | '。' | '，' | '．' | '､' | '｡')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::{FORMAT_VERSION, FontMeta, FontMode, Glyph, Kern};

    fn glyph(character: char, advance: u32) -> Glyph {
        Glyph {
            character,
            variant: None,
            advance: Some(advance),
            canvas: IndexedCanvas::filled(advance, 3, 1).with_transparent(Some(0)),
        }
    }

    fn font() -> Font {
        Font {
            meta: FontMeta {
                format: FORMAT_VERSION,
                name: "layout-test".into(),
                mode: FontMode::Mono,
                line_height: 3,
                baseline: 2,
                default_advance: 2,
                source: None,
                copyright: None,
                palette: None,
            },
            glyphs: vec![glyph(' ', 2), glyph('A', 2), glyph('B', 2), glyph('C', 2)],
            kerns: Vec::new(),
        }
    }

    fn japanese_font() -> Font {
        let mut value = font();
        value.meta.default_advance = 8;
        value.glyphs = "あいうっ、「」。"
            .chars()
            .map(|character| glyph(character, 8))
            .collect();
        value
    }

    #[test]
    fn wrap_keeps_exact_width_and_breaks_at_one_pixel_less() {
        let exact = layout_text(
            &font(),
            "A B",
            &FontLayoutOptions {
                wrap: Some(6),
                ..FontLayoutOptions::default()
            },
        )
        .unwrap();
        assert_eq!(exact.report.lines, ["A B"]);
        assert_eq!(exact.canvas.width(), 6);

        let overflow = layout_text(
            &font(),
            "A B",
            &FontLayoutOptions {
                wrap: Some(5),
                ..FontLayoutOptions::default()
            },
        )
        .unwrap();
        assert_eq!(overflow.report.lines, ["A", "B"]);
    }

    #[test]
    fn trailing_spaces_are_neither_drawn_nor_measured() {
        let result = layout_text(&font(), "A  ", &FontLayoutOptions::default()).unwrap();
        assert_eq!(result.report.lines, ["A"]);
        assert_eq!(result.canvas.width(), 2);
        assert_eq!(
            result
                .canvas
                .pixels()
                .iter()
                .filter(|&&pixel| pixel == 1)
                .count(),
            6
        );
    }

    #[test]
    fn an_unbreakable_word_is_split_by_character_and_reported() {
        let result = layout_text(
            &font(),
            "ABC",
            &FontLayoutOptions {
                wrap: Some(3),
                ..FontLayoutOptions::default()
            },
        )
        .unwrap();
        assert_eq!(result.report.lines, ["A", "B", "C"]);
        assert_eq!(result.report.forced_breaks, 2);
    }

    #[test]
    fn explicit_newline_and_line_spacing_determine_height() {
        let result = layout_text(
            &font(),
            "A\nB",
            &FontLayoutOptions {
                line_spacing: 2,
                ..FontLayoutOptions::default()
            },
        )
        .unwrap();
        assert_eq!(result.report.lines, ["A", "B"]);
        assert_eq!(result.canvas.height(), 8);
    }

    #[test]
    fn alignment_uses_the_widest_line_and_center_rounds_down() {
        for (align, expected_x) in [(TextAlign::Center, 1), (TextAlign::Right, 2)] {
            let result = layout_text(
                &font(),
                "A\nBB",
                &FontLayoutOptions {
                    align,
                    ..FontLayoutOptions::default()
                },
            )
            .unwrap();
            assert_eq!(result.canvas.width(), 4);
            assert_eq!(result.canvas.get(expected_x, 0), Some(1));
            assert_eq!(result.canvas.get(expected_x - 1, 0), Some(0));
        }
    }

    #[test]
    fn letter_spacing_is_only_between_characters() {
        let result = layout_text(
            &font(),
            "AB",
            &FontLayoutOptions {
                letter_spacing: 1,
                ..FontLayoutOptions::default()
            },
        )
        .unwrap();
        assert_eq!(result.canvas.width(), 5);
        assert_eq!(result.canvas.get(2, 0), Some(0));
        assert_eq!(result.canvas.get(3, 0), Some(1));
    }

    #[test]
    fn kerning_applies_within_a_line_and_can_be_disabled() {
        let mut value = font();
        value.kerns.push(Kern {
            pair: "AB".into(),
            amount: -1,
        });
        let kerned = layout_text(&value, "AB", &FontLayoutOptions::default()).unwrap();
        assert_eq!(kerned.canvas.width(), 3);

        let separate = layout_text(&value, "A\nB", &FontLayoutOptions::default()).unwrap();
        assert_eq!(separate.canvas.width(), 2);

        let disabled = layout_text(
            &value,
            "AB",
            &FontLayoutOptions {
                kern: false,
                ..FontLayoutOptions::default()
            },
        )
        .unwrap();
        assert_eq!(disabled.canvas.width(), 4);
    }

    #[test]
    fn uax14_keeps_closing_punctuation_off_the_line_head() {
        let result = layout_text(
            &japanese_font(),
            "あい、う",
            &FontLayoutOptions {
                wrap: Some(16),
                ..FontLayoutOptions::default()
            },
        )
        .unwrap();
        assert_eq!(result.report.lines, ["あ", "い、", "う"]);
    }

    #[test]
    fn uax14_keeps_opening_punctuation_off_the_line_end() {
        let result = layout_text(
            &japanese_font(),
            "あ「いう",
            &FontLayoutOptions {
                wrap: Some(16),
                ..FontLayoutOptions::default()
            },
        )
        .unwrap();
        assert_eq!(result.report.lines, ["あ", "「い", "う"]);
    }

    #[test]
    fn normal_allows_cj_at_line_head_while_strict_does_not() {
        let normal = layout_text(
            &japanese_font(),
            "あいっう",
            &FontLayoutOptions {
                wrap: Some(16),
                ..FontLayoutOptions::default()
            },
        )
        .unwrap();
        assert_eq!(normal.report.lines, ["あい", "っう"]);

        let strict = layout_text(
            &japanese_font(),
            "あいっう",
            &FontLayoutOptions {
                wrap: Some(16),
                kinsoku: Kinsoku::Strict,
                ..FontLayoutOptions::default()
            },
        )
        .unwrap();
        assert_eq!(strict.report.lines, ["あ", "いっ", "う"]);
    }

    #[test]
    fn hanging_punctuation_extends_the_canvas_by_one_character() {
        let pushed = layout_text(
            &japanese_font(),
            "あい。",
            &FontLayoutOptions {
                wrap: Some(16),
                ..FontLayoutOptions::default()
            },
        )
        .unwrap();
        assert_eq!(pushed.report.lines, ["あ", "い。"]);
        assert_eq!(pushed.canvas.width(), 16);

        let hung = layout_text(
            &japanese_font(),
            "あい。",
            &FontLayoutOptions {
                wrap: Some(16),
                hang: true,
                ..FontLayoutOptions::default()
            },
        )
        .unwrap();
        assert_eq!(hung.report.lines, ["あい。"]);
        assert_eq!(hung.canvas.width(), 24);
        assert_eq!(hung.report.hanging_punctuation, 1);
    }

    #[test]
    fn missing_characters_are_sorted_or_replaced_and_reported() {
        let error = layout_text(&font(), "ZYA", &FontLayoutOptions::default()).unwrap_err();
        assert_eq!(
            error,
            FontLayoutError::MissingGlyphs {
                characters: vec!['Y', 'Z']
            }
        );

        let replaced = layout_text(
            &font(),
            "ZZA",
            &FontLayoutOptions {
                missing: Some('B'),
                ..FontLayoutOptions::default()
            },
        )
        .unwrap();
        assert_eq!(replaced.report.lines, ["BBA"]);
        assert_eq!(replaced.report.replacements, BTreeMap::from([('Z', 2)]));
    }

    #[test]
    fn vertical_direction_places_the_first_column_on_the_right() {
        let result = layout_text(
            &font(),
            "AB",
            &FontLayoutOptions {
                direction: WritingDirection::Vertical,
                wrap: Some(3),
                ..FontLayoutOptions::default()
            },
        )
        .unwrap();
        assert_eq!(result.report.lines, ["A", "B"]);
        assert_eq!((result.canvas.width(), result.canvas.height()), (6, 2));
        assert_eq!(result.canvas.get(3, 0), Some(1));
        assert_eq!(result.canvas.get(0, 0), Some(1));
    }

    #[test]
    fn a_tcy_fallback_can_widen_the_column() {
        let mut value = font();
        value.glyphs.push(glyph('1', 4));
        let result = layout_text(
            &value,
            "A1",
            &FontLayoutOptions {
                direction: WritingDirection::Vertical,
                tcy: Some(1),
                ..FontLayoutOptions::default()
            },
        )
        .unwrap();
        assert_eq!(result.report.tcy_fallbacks, 1);
        assert_eq!(result.canvas.width(), 4);
    }

    #[test]
    fn empty_text_and_consecutive_newlines_preserve_blank_lines() {
        let empty = layout_text(&font(), "", &FontLayoutOptions::default()).unwrap();
        assert_eq!(empty.report.lines, [""]);
        assert_eq!(empty.canvas.width(), 0);
        assert_eq!(empty.canvas.height(), 3);

        let blank = layout_text(&font(), "A\n\nB\n", &FontLayoutOptions::default()).unwrap();
        assert_eq!(blank.report.lines, ["A", "", "B", ""]);
        assert_eq!(blank.canvas.height(), 12);
    }

    #[test]
    fn normal_tailoring_preserves_four_byte_cj_offsets() {
        let small_katakana_ko = '\u{1b132}';
        assert_eq!(
            break_property(small_katakana_ko as u32),
            BreakClass::ConditionalJapaneseStarter
        );
        let mut value = japanese_font();
        value.glyphs.push(glyph(small_katakana_ko, 8));
        let normal = layout_text(
            &value,
            &format!("あい{small_katakana_ko}う"),
            &FontLayoutOptions {
                wrap: Some(16),
                ..FontLayoutOptions::default()
            },
        )
        .unwrap();
        assert_eq!(normal.report.lines, ["あい", "𛄲う"]);
        assert_eq!(normal.report.unconverted_cj, 0);
    }

    #[test]
    fn negative_kerning_never_moves_the_first_glyph_left_of_origin() {
        let mut value = font();
        value.kerns.push(Kern {
            pair: "AB".into(),
            amount: -1,
        });
        let result = layout_text(&value, "AB", &FontLayoutOptions::default()).unwrap();
        assert_eq!(result.canvas.get(0, 0), Some(1));
    }

    #[test]
    fn a_missing_transparent_index_is_an_error_not_an_assumed_zero() {
        let mut value = font();
        value
            .glyphs
            .iter_mut()
            .find(|glyph| glyph.character == 'A')
            .unwrap()
            .canvas
            .set_transparent(None);
        let error = layout_text(&value, "A", &FontLayoutOptions::default()).unwrap_err();
        assert_eq!(
            error,
            FontLayoutError::MissingTransparentIndex { character: 'A' }
        );
    }
}
