//! 数字を桁ごとの升の中で縦に回すフレーム列．

use std::collections::{BTreeMap, BTreeSet};

use crate::{CoreError, Font, Glyph, IndexedCanvas, Result};

/// 数字が回る向き．
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum RollDirection {
    /// 古い字が上へ抜け，新しい字が下から入る．
    #[default]
    Up,
    /// 古い字が下へ抜け，新しい字が上から入る．
    Down,
}

/// 数字の回転に使う設定．
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct RollOptions {
    /// 1 回の字の切り替えに使う区間数．出力は区間数 + 1 枚になる．
    pub frames: u32,
    pub direction: RollDirection,
    pub letter_spacing: u32,
}

/// `from` から `to` まで，桁ごとの升で数字を回す．
pub fn roll(
    font: &Font,
    from: &str,
    to: &str,
    options: &RollOptions,
) -> Result<Vec<IndexedCanvas>> {
    if options.frames == 0 {
        return Err(CoreError::RollNoFrames);
    }
    let from = from.chars().collect::<Vec<_>>();
    let to = to.chars().collect::<Vec<_>>();
    if from.len() != to.len() {
        return Err(CoreError::RollLengthMismatch {
            from: from.len(),
            to: to.len(),
        });
    }

    let paths = from
        .iter()
        .copied()
        .zip(to.iter().copied())
        .enumerate()
        .map(|(position, (from, to))| digit_path(from, to, position + 1, options.direction))
        .collect::<Result<Vec<_>>>()?;
    let glyphs = font
        .glyphs
        .iter()
        .filter(|glyph| glyph.variant.is_none())
        .map(|glyph| (glyph.character, glyph))
        .collect::<BTreeMap<_, _>>();
    let required = paths.iter().flatten().copied().collect::<BTreeSet<_>>();
    let mut transparent = None;
    for character in required {
        let glyph = glyphs
            .get(&character)
            .ok_or(CoreError::RollMissingGlyph { character })?;
        let actual = glyph
            .canvas
            .transparent()
            .ok_or(CoreError::RollMissingTransparentIndex { character })?;
        match transparent {
            None => transparent = Some(actual),
            Some(expected) if expected != actual => {
                return Err(CoreError::RollInconsistentTransparentIndex {
                    character,
                    expected,
                    actual,
                });
            }
            Some(_) => {}
        }
    }
    // 空の文字列でも，生成する画布の透明添字は明示する．
    let transparent = transparent.unwrap_or(0);

    let cell_widths = paths
        .iter()
        .map(|path| {
            path.iter()
                .map(|character| glyphs[character].effective_advance(&font.meta))
                .max()
                .unwrap_or(0)
        })
        .collect::<Vec<_>>();
    let cells_width = cell_widths
        .iter()
        .try_fold(0u32, |sum, width| sum.checked_add(*width))
        .ok_or(CoreError::RollDimensionsOverflow)?;
    let gaps = u32::try_from(paths.len().saturating_sub(1))
        .map_err(|_| CoreError::RollDimensionsOverflow)?;
    let width = cells_width
        .checked_add(
            options
                .letter_spacing
                .checked_mul(gaps)
                .ok_or(CoreError::RollDimensionsOverflow)?,
        )
        .ok_or(CoreError::RollDimensionsOverflow)?;
    let transitions = paths
        .iter()
        .map(|path| path.len().saturating_sub(1))
        .max()
        .unwrap_or(0);
    let frame_count = transitions
        .checked_mul(options.frames as usize)
        .and_then(|count| count.checked_add(1))
        .ok_or(CoreError::RollDimensionsOverflow)?;
    let mut frames = Vec::with_capacity(frame_count);

    if transitions == 0 {
        frames.push(draw_frame(
            font,
            &glyphs,
            &paths,
            &cell_widths,
            width,
            transparent,
            options,
            0,
            0,
        ));
        return Ok(frames);
    }

    for segment in 0..transitions {
        let first_k = usize::from(segment != 0);
        for k in first_k..=options.frames as usize {
            frames.push(draw_frame(
                font,
                &glyphs,
                &paths,
                &cell_widths,
                width,
                transparent,
                options,
                segment,
                k as u32,
            ));
        }
    }
    Ok(frames)
}

fn digit_path(
    from: char,
    to: char,
    position: usize,
    direction: RollDirection,
) -> Result<Vec<char>> {
    if from == to {
        return Ok(vec![from]);
    }
    if !from.is_ascii_digit() || !to.is_ascii_digit() {
        return Err(CoreError::RollNonDigitChanged { position, from, to });
    }
    let target = to as u8 - b'0';
    let mut digit = from as u8 - b'0';
    let mut path = vec![from];
    while digit != target {
        digit = match direction {
            RollDirection::Up => (digit + 1) % 10,
            RollDirection::Down => (digit + 9) % 10,
        };
        path.push(char::from(b'0' + digit));
    }
    Ok(path)
}

#[allow(clippy::too_many_arguments)]
fn draw_frame(
    font: &Font,
    glyphs: &BTreeMap<char, &Glyph>,
    paths: &[Vec<char>],
    cell_widths: &[u32],
    width: u32,
    transparent: u8,
    options: &RollOptions,
    segment: usize,
    k: u32,
) -> IndexedCanvas {
    let mut output = IndexedCanvas::filled(width, font.meta.line_height, transparent)
        .with_transparent(Some(transparent));
    let longest = paths
        .iter()
        .map(|path| path.len().saturating_sub(1))
        .max()
        .unwrap_or(0);
    let offset = if k == options.frames {
        font.meta.line_height as i32
    } else {
        (k as f32 * font.meta.line_height as f32 / options.frames as f32).round() as i32
    };
    let mut left = 0i32;
    for (path, &cell_width) in paths.iter().zip(cell_widths) {
        let count = path.len().saturating_sub(1);
        let start = longest - count;
        if count == 0 || segment < start {
            draw_glyph(&mut output, glyphs[&path[0]], left, 0, cell_width);
        } else {
            let local = segment - start;
            let old = glyphs[&path[local]];
            let new = glyphs[&path[local + 1]];
            let (old_y, new_y) = match options.direction {
                RollDirection::Up => (-offset, font.meta.line_height as i32 - offset),
                RollDirection::Down => (offset, -(font.meta.line_height as i32) + offset),
            };
            draw_glyph(&mut output, old, left, old_y, cell_width);
            draw_glyph(&mut output, new, left, new_y, cell_width);
        }
        left += (cell_width + options.letter_spacing) as i32;
    }
    output
}

fn draw_glyph(output: &mut IndexedCanvas, glyph: &Glyph, left: i32, top: i32, cell_width: u32) {
    for y in 0..glyph.canvas.height() as i32 {
        for x in 0..cell_width as i32 {
            if let Some(pixel) = glyph.canvas.get(x, y)
                && glyph.canvas.transparent() != Some(pixel)
            {
                output.set(left + x, top + y, pixel);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::font::{FORMAT_VERSION, FontMeta, FontMode, Glyph};

    const H: u32 = 3;

    fn glyph(character: char, advance: u32, rows: [&str; H as usize]) -> Glyph {
        let width = rows[0].len() as u32;
        assert!(rows.iter().all(|row| row.len() as u32 == width));
        let pixels = rows
            .iter()
            .flat_map(|row| row.bytes().map(|pixel| u8::from(pixel == b'#')))
            .collect();
        Glyph {
            character,
            variant: None,
            advance: Some(advance),
            canvas: IndexedCanvas::from_pixels(width, H, pixels)
                .unwrap()
                .with_transparent(Some(0)),
        }
    }

    fn font(glyphs: Vec<Glyph>) -> Font {
        Font {
            meta: FontMeta {
                format: FORMAT_VERSION,
                name: "roll-test".into(),
                mode: FontMode::Mono,
                line_height: H,
                baseline: 2,
                default_advance: 2,
                source: None,
                copyright: None,
                palette: None,
            },
            glyphs,
            kerns: vec![],
        }
    }

    fn digits() -> Font {
        font(vec![
            glyph('0', 2, ["##", "#.", "##"]),
            glyph('1', 2, [".#", "##", ".#"]),
            glyph('2', 2, ["##", ".#", "#."]),
            glyph('3', 2, ["##", ".#", "##"]),
            glyph('4', 2, ["#.", "##", ".#"]),
            glyph('5', 2, ["##", "#.", ".#"]),
            glyph('6', 2, ["#.", "##", "##"]),
            // 上下非対称で，移動方向を画素から判定できる形．
            glyph('7', 2, ["##", ".#", "#."]),
            glyph('8', 2, ["#.", ".#", "##"]),
            glyph('9', 2, ["##", "##", ".#"]),
            glyph(':', 1, [".", "#", "#"]),
        ])
    }

    fn base_glyphs(font: &Font) -> BTreeMap<char, &Glyph> {
        font.glyphs
            .iter()
            .filter(|glyph| glyph.variant.is_none())
            .map(|glyph| (glyph.character, glyph))
            .collect()
    }

    /// 製品の roll を使わず，静止した文字列を升ごとに組み立てる．
    fn expected_static(font: &Font, text: &str, widths: &[u32], spacing: u32) -> IndexedCanvas {
        let glyphs = base_glyphs(font);
        let width = widths.iter().sum::<u32>() + spacing * widths.len().saturating_sub(1) as u32;
        let mut out = IndexedCanvas::filled(width, H, 0).with_transparent(Some(0));
        let mut left = 0i32;
        for (character, &cell_width) in text.chars().zip(widths) {
            let source = &glyphs[&character].canvas;
            for y in 0..H as i32 {
                for x in 0..cell_width as i32 {
                    if let Some(pixel) = source.get(x, y)
                        && source.transparent() != Some(pixel)
                    {
                        out.set(left + x, y, pixel);
                    }
                }
            }
            left += (cell_width + spacing) as i32;
        }
        out
    }

    /// 製品の roll を使わず，1 升の 2 字を指定した移動量で合成する．
    fn expected_transition(
        old: &Glyph,
        new: &Glyph,
        cell_width: u32,
        offset: i32,
        direction: RollDirection,
    ) -> IndexedCanvas {
        let mut out = IndexedCanvas::filled(cell_width, H, 0).with_transparent(Some(0));
        let (old_y, new_y) = match direction {
            RollDirection::Up => (-offset, H as i32 - offset),
            RollDirection::Down => (offset, -(H as i32) + offset),
        };
        for (source, top) in [(&old.canvas, old_y), (&new.canvas, new_y)] {
            for sy in 0..H as i32 {
                for x in 0..cell_width as i32 {
                    if let Some(pixel) = source.get(x, sy)
                        && source.transparent() != Some(pixel)
                    {
                        out.set(x, top + sy, pixel);
                    }
                }
            }
        }
        out
    }

    fn options(frames: u32, direction: RollDirection) -> RollOptions {
        RollOptions {
            frames,
            direction,
            letter_spacing: 0,
        }
    }

    #[test]
    fn endpoints_are_exactly_the_old_and_new_glyphs() {
        let font = digits();
        let frames = roll(&font, "7", "8", &options(2, RollDirection::Up)).unwrap();
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0], expected_static(&font, "7", &[2], 0));
        assert_eq!(frames[2], expected_static(&font, "8", &[2], 0));
    }

    #[test]
    fn intermediate_pixels_are_clipped_to_each_cells_advance() {
        let font = font(vec![
            glyph('7', 2, ["###", "..#", "#.."]),
            glyph('8', 2, ["..#", "###", "..#"]),
            glyph('1', 2, [".#", "##", ".#"]),
        ]);
        let frames = roll(&font, "71", "81", &options(2, RollDirection::Up)).unwrap();
        let middle = &frames[1];
        assert_eq!(middle.width(), 4);
        // 1 桁目の canvas の x=2 は 1 桁目の升外なので，2 桁目を汚してはならない．
        for y in 0..H as i32 {
            assert_eq!(middle.get(2, y), font.glyphs[2].canvas.get(0, y));
        }
    }

    #[test]
    fn up_and_down_move_an_asymmetric_glyph_in_opposite_directions() {
        let font = digits();
        let glyphs = base_glyphs(&font);
        let up = roll(&font, "7", "8", &options(2, RollDirection::Up)).unwrap();
        let down = roll(&font, "7", "6", &options(2, RollDirection::Down)).unwrap();
        // round(1 * 3 / 2) = 2．
        assert_eq!(
            up[1],
            expected_transition(glyphs[&'7'], glyphs[&'8'], 2, 2, RollDirection::Up)
        );
        assert_eq!(
            down[1],
            expected_transition(glyphs[&'7'], glyphs[&'6'], 2, 2, RollDirection::Down)
        );
        assert_ne!(up[1], down[1]);
    }

    #[test]
    fn nine_wraps_directly_to_zero_when_rolling_up() {
        let font = digits();
        let frames = roll(&font, "9", "0", &options(3, RollDirection::Up)).unwrap();
        assert_eq!(frames.len(), 4, "9→0 は 1 回の切り替え");
        assert_eq!(frames[0], expected_static(&font, "9", &[2], 0));
        assert_eq!(frames[3], expected_static(&font, "0", &[2], 0));
    }

    #[test]
    fn shorter_digit_paths_are_aligned_to_the_end() {
        let font = digits();
        let frames = roll(&font, "29", "31", &options(2, RollDirection::Up)).unwrap();
        assert_eq!(frames.len(), 5);
        // 前半の切り替えは 1 の位だけで，10 の位は 2 のまま．
        for y in 0..H as i32 {
            for x in 0..2 {
                assert_eq!(frames[1].get(x, y), font.glyphs[2].canvas.get(x, y));
            }
        }
        // 後半では 10 の位も 2→3 の途中に入る．
        let glyphs = base_glyphs(&font);
        let expected = expected_transition(glyphs[&'2'], glyphs[&'3'], 2, 2, RollDirection::Up);
        for y in 0..H as i32 {
            for x in 0..2 {
                assert_eq!(frames[3].get(x, y), expected.get(x, y));
            }
        }
        assert_eq!(frames[4], expected_static(&font, "31", &[2, 2], 0));
    }

    #[test]
    fn different_character_counts_are_rejected() {
        let error = roll(&digits(), "09", "100", &options(2, RollDirection::Up)).unwrap_err();
        assert!(error.to_string().contains("字数"), "{error}");
    }

    #[test]
    fn changing_a_non_digit_is_rejected() {
        let error = roll(&digits(), "1:9", "1;0", &options(2, RollDirection::Up)).unwrap_err();
        assert!(error.to_string().contains("数字でない"), "{error}");
    }

    #[test]
    fn zero_frames_is_rejected_instead_of_dividing_by_zero() {
        let error = roll(&digits(), "7", "8", &options(0, RollDirection::Up)).unwrap_err();
        assert!(error.to_string().contains("1 以上"), "{error}");
    }

    #[test]
    fn empty_inputs_make_one_empty_canvas_with_explicit_transparency() {
        let frames = roll(&digits(), "", "", &options(2, RollDirection::Up)).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].size(), crate::uvec2(0, H));
        assert_eq!(frames[0].transparent(), Some(0));
    }

    #[test]
    fn a_missing_intermediate_digit_is_reported() {
        let font = font(vec![
            glyph('8', 2, ["#.", ".#", "##"]),
            glyph('0', 2, ["##", "#.", "##"]),
        ]);
        let error = roll(&font, "8", "0", &options(2, RollDirection::Up)).unwrap_err();
        assert!(error.to_string().contains("'9'"), "{error}");
    }

    #[test]
    fn cells_use_the_largest_advance_on_their_path_and_keep_letter_spacing() {
        let font = font(vec![
            glyph('0', 1, ["#", "#", "#"]),
            glyph('1', 3, [".#", "##", ".#"]),
            glyph('2', 2, ["##", ".#", "#."]),
        ]);
        let frames = roll(
            &font,
            "02",
            "12",
            &RollOptions {
                frames: 1,
                direction: RollDirection::Up,
                letter_spacing: 2,
            },
        )
        .unwrap();
        assert_eq!(frames[0].width(), 3 + 2 + 2);
        assert_eq!(frames[0], expected_static(&font, "02", &[3, 2], 2));
        assert_eq!(frames[1], expected_static(&font, "12", &[3, 2], 2));
        assert_eq!(frames[0].transparent(), Some(0));
    }
}
