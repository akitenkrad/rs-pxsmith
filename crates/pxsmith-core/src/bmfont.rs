//! AngelCode BMFont のテキスト記述とページ画像を組み立てる．

use std::collections::BTreeSet;

use crate::font::{Font, FontError, FontMode, Glyph};
use crate::{Palette, Rgba8, RgbaCanvas};

/// BMFont のページ梱包と記述生成に使う設定．
#[derive(Clone, Debug)]
pub struct BmFontOptions<'a> {
    pub page_size: u32,
    pub page_file_stem: &'a str,
    pub characters: Option<&'a BTreeSet<char>>,
    pub palette: Option<&'a Palette>,
}

impl<'a> BmFontOptions<'a> {
    pub fn new(page_size: u32, page_file_stem: &'a str) -> Self {
        Self {
            page_size,
            page_file_stem,
            characters: None,
            palette: None,
        }
    }

    pub fn with_characters(mut self, characters: &'a BTreeSet<char>) -> Self {
        self.characters = Some(characters);
        self
    }

    pub fn with_palette(mut self, palette: &'a Palette) -> Self {
        self.palette = Some(palette);
        self
    }
}

/// BMFont へ書いた字とページ，除外した縦書き字形の件数．
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BmFontReport {
    pub glyphs: usize,
    pub pages: usize,
    pub skipped_vertical: usize,
}

/// BMFont のテキスト記述と RGBA ページ列．
#[derive(Clone, Debug, PartialEq)]
pub struct BmFontOutput {
    pub pages: Vec<RgbaCanvas>,
    pub descriptor: String,
    pub report: BmFontReport,
}

/// BMFont を作れなかった理由．
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BmFontError {
    #[error("フォントが有効でない: {0}")]
    InvalidFont(#[from] FontError),
    #[error("page_size {page_size} は 2 の冪でなければならない")]
    InvalidPageSize { page_size: u32 },
    #[error(
        "字 '{character}' の墨 {width}x{height} は 1 px の外周を含めて {page_size}x{page_size} ページに入らない"
    )]
    GlyphTooLarge {
        character: char,
        width: u32,
        height: u32,
        page_size: u32,
    },
    #[error("フォントに無い字がある: {characters:?}")]
    MissingGlyphs { characters: Vec<char> },
    #[error("多色フォントの BMFont 出力にパレットが無い")]
    MissingPalette,
    #[error("字 '{character}' の添字 {index} がパレットの範囲外")]
    PaletteIndex { character: char, index: u8 },
}

#[derive(Clone, Debug)]
struct PackedGlyph<'a> {
    glyph: &'a Glyph,
    source_x: u32,
    source_y: u32,
    width: u32,
    height: u32,
    page: usize,
    x: u32,
    y: u32,
}

/// `font` から BMFont のテキスト記述とページ画像を決定的に組み立てる．
pub fn build_bmfont(font: &Font, options: &BmFontOptions<'_>) -> Result<BmFontOutput, BmFontError> {
    font.validate()?;
    if !options.page_size.is_power_of_two() {
        return Err(BmFontError::InvalidPageSize {
            page_size: options.page_size,
        });
    }

    let base_characters = font
        .glyphs
        .iter()
        .filter(|glyph| glyph.variant.is_none())
        .map(|glyph| glyph.character)
        .collect::<BTreeSet<_>>();
    let selected_characters = match options.characters {
        Some(requested) => {
            let missing = requested
                .difference(&base_characters)
                .copied()
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                return Err(BmFontError::MissingGlyphs {
                    characters: missing,
                });
            }
            let mut selected = requested.clone();
            if base_characters.contains(&' ') {
                selected.insert(' ');
            }
            selected
        }
        None => base_characters,
    };
    let skipped_vertical = font
        .glyphs
        .iter()
        .filter(|glyph| {
            glyph.variant.as_deref() == Some("vert")
                && options
                    .characters
                    .is_none_or(|characters| characters.contains(&glyph.character))
        })
        .count();

    let mut packed = font
        .glyphs
        .iter()
        .filter(|glyph| glyph.variant.is_none() && selected_characters.contains(&glyph.character))
        .map(|glyph| {
            let bbox = glyph.canvas.opaque_bbox();
            PackedGlyph {
                glyph,
                source_x: bbox.map_or(0, |rect| rect.x as u32),
                source_y: bbox.map_or(0, |rect| rect.y as u32),
                width: bbox.map_or(0, |rect| rect.w),
                height: bbox.map_or(0, |rect| rect.h),
                page: 0,
                x: 0,
                y: 0,
            }
        })
        .collect::<Vec<_>>();
    packed.sort_by_key(|item| (std::cmp::Reverse(item.height), item.glyph.character));
    pack_glyphs(&mut packed, options.page_size)?;

    let page_count = packed
        .iter()
        .filter(|item| item.width != 0 && item.height != 0)
        .map(|item| item.page + 1)
        .max()
        .unwrap_or(1);
    let mut pages = (0..page_count)
        .map(|_| RgbaCanvas::filled(options.page_size, options.page_size, Rgba8::TRANSPARENT))
        .collect::<Vec<_>>();
    draw_pages(font, options.palette, &packed, &mut pages)?;

    packed.sort_by_key(|item| item.glyph.character);
    let descriptor = descriptor(font, options, &packed, page_count, &selected_characters);
    let report = BmFontReport {
        glyphs: packed.len(),
        pages: page_count,
        skipped_vertical,
    };
    Ok(BmFontOutput {
        pages,
        descriptor,
        report,
    })
}

fn pack_glyphs(items: &mut [PackedGlyph<'_>], page_size: u32) -> Result<(), BmFontError> {
    let inner_limit = page_size.saturating_sub(1);
    let mut page = 0;
    let mut x = 1;
    let mut y = 1;
    let mut shelf_height = 0;
    for item in items
        .iter_mut()
        .filter(|item| item.width != 0 && item.height != 0)
    {
        if item.width > page_size.saturating_sub(2) || item.height > page_size.saturating_sub(2) {
            return Err(BmFontError::GlyphTooLarge {
                character: item.glyph.character,
                width: item.width,
                height: item.height,
                page_size,
            });
        }
        if x + item.width > inner_limit {
            x = 1;
            y += shelf_height + 1;
            shelf_height = 0;
        }
        if y + item.height > inner_limit {
            page += 1;
            x = 1;
            y = 1;
            shelf_height = 0;
        }
        item.page = page;
        item.x = x;
        item.y = y;
        x += item.width + 1;
        shelf_height = shelf_height.max(item.height);
    }
    Ok(())
}

fn draw_pages(
    font: &Font,
    palette: Option<&Palette>,
    items: &[PackedGlyph<'_>],
    pages: &mut [RgbaCanvas],
) -> Result<(), BmFontError> {
    let palette = match font.meta.mode {
        FontMode::Mono => None,
        FontMode::Color => Some(palette.ok_or(BmFontError::MissingPalette)?),
    };
    for item in items
        .iter()
        .filter(|item| item.width != 0 && item.height != 0)
    {
        for dy in 0..item.height {
            for dx in 0..item.width {
                let index = item
                    .glyph
                    .canvas
                    .get((item.source_x + dx) as i32, (item.source_y + dy) as i32)
                    .expect("外接矩形は字の画布内にある");
                if item.glyph.canvas.transparent() == Some(index) {
                    continue;
                }
                let color = match palette {
                    None => Rgba8::rgb(255, 255, 255),
                    Some(palette) => palette.get(index).ok_or(BmFontError::PaletteIndex {
                        character: item.glyph.character,
                        index,
                    })?,
                };
                pages[item.page].set((item.x + dx) as i32, (item.y + dy) as i32, color);
            }
        }
    }
    Ok(())
}

fn descriptor(
    font: &Font,
    options: &BmFontOptions<'_>,
    items: &[PackedGlyph<'_>],
    page_count: usize,
    selected_characters: &BTreeSet<char>,
) -> String {
    use std::fmt::Write;

    let mut output = String::new();
    let face = font.meta.name.replace('"', "");
    writeln!(
        output,
        "info face=\"{face}\" size={} bold=0 italic=0 charset=\"\" unicode=1 stretchH=100 smooth=0 aa=1 padding=0,0,0,0 spacing=1,1 outline=0",
        font.meta.line_height
    )
    .expect("String への書き込みは失敗しない");
    writeln!(
        output,
        "common lineHeight={} base={} scaleW={} scaleH={} pages={page_count} packed=0 alphaChnl=0 redChnl=0 greenChnl=0 blueChnl=0",
        font.meta.line_height, font.meta.baseline, options.page_size, options.page_size
    )
    .expect("String への書き込みは失敗しない");
    for page in 0..page_count {
        writeln!(
            output,
            "page id={page} file=\"{}_{page}.png\"",
            options.page_file_stem
        )
        .expect("String への書き込みは失敗しない");
    }
    writeln!(output, "chars count={}", items.len()).expect("String への書き込みは失敗しない");
    for item in items {
        writeln!(
            output,
            "char id={} x={} y={} width={} height={} xoffset={} yoffset={} xadvance={} page={} chnl=15",
            item.glyph.character as u32,
            item.x,
            item.y,
            item.width,
            item.height,
            item.source_x,
            item.source_y,
            item.glyph.effective_advance(&font.meta),
            item.page
        )
        .expect("String への書き込みは失敗しない");
    }

    let kerns = font
        .kerns
        .iter()
        .filter_map(|kern| {
            let pair = kern.pair.chars().collect::<Vec<_>>();
            (selected_characters.contains(&pair[0]) && selected_characters.contains(&pair[1]))
                .then_some((pair[0], pair[1], kern.amount))
        })
        .collect::<Vec<_>>();
    if !kerns.is_empty() {
        writeln!(output, "kernings count={}", kerns.len())
            .expect("String への書き込みは失敗しない");
        for (first, second, amount) in kerns {
            writeln!(
                output,
                "kerning first={} second={} amount={amount}",
                first as u32, second as u32
            )
            .expect("String への書き込みは失敗しない");
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use crate::font::{FORMAT_VERSION, Font, FontMeta, FontMode, Glyph, Kern};
    use crate::{IndexedCanvas, Palette, Rgba8};

    use super::*;

    fn glyph(character: char, advance: u32, rows: &[&str]) -> Glyph {
        let width = rows.first().map_or(0, |row| row.len()) as u32;
        let pixels = rows
            .iter()
            .flat_map(|row| row.bytes().map(|pixel| u8::from(pixel == b'#')))
            .collect();
        Glyph {
            character,
            variant: None,
            advance: Some(advance),
            canvas: IndexedCanvas::from_pixels(width, rows.len() as u32, pixels)
                .unwrap()
                .with_transparent(Some(0)),
        }
    }

    fn font(glyphs: Vec<Glyph>) -> Font {
        Font {
            meta: FontMeta {
                format: FORMAT_VERSION,
                name: "test".into(),
                mode: FontMode::Mono,
                line_height: 4,
                baseline: 3,
                default_advance: 4,
                source: None,
                copyright: None,
                palette: None,
            },
            glyphs,
            kerns: vec![],
        }
    }

    fn character_line(descriptor: &str, character: char) -> &str {
        let prefix = format!("char id={} ", character as u32);
        descriptor
            .lines()
            .find(|line| line.starts_with(&prefix))
            .unwrap_or_else(|| panic!("{character:?} の char 行が無い:\n{descriptor}"))
    }

    #[test]
    fn crops_to_ink_bbox_and_records_offsets() {
        let font = font(vec![glyph('A', 5, &["....", ".##.", ".#..", "...."])]);
        let output = build_bmfont(&font, &BmFontOptions::new(8, "ui")).unwrap();

        assert!(output.descriptor.contains(
            "char id=65 x=1 y=1 width=2 height=2 xoffset=1 yoffset=1 xadvance=5 page=0 chnl=15"
        ));
        assert_eq!(output.pages[0].get(1, 1), Some(Rgba8::rgb(255, 255, 255)));
        assert_eq!(output.pages[0].get(2, 1), Some(Rgba8::rgb(255, 255, 255)));
        assert_eq!(output.pages[0].get(0, 0), Some(Rgba8::TRANSPARENT));
    }

    #[test]
    fn blank_glyph_has_only_advance_and_no_packed_rectangle() {
        let font = font(vec![glyph(' ', 3, &["....", "....", "....", "...."])]);
        let output = build_bmfont(&font, &BmFontOptions::new(8, "ui")).unwrap();

        assert_eq!(
            character_line(&output.descriptor, ' '),
            "char id=32 x=0 y=0 width=0 height=0 xoffset=0 yoffset=0 xadvance=3 page=0 chnl=15"
        );
        assert!(
            output.pages[0]
                .pixels()
                .iter()
                .all(|pixel| *pixel == Rgba8::TRANSPARENT)
        );
    }

    #[test]
    fn shelf_packing_is_deterministic_for_input_order() {
        let a = glyph('A', 4, &["#...", "#...", "#...", "...."]);
        let b = glyph('B', 4, &["##..", "##..", "....", "...."]);
        let c = glyph('C', 4, &["###.", "....", "....", "...."]);
        let forward = build_bmfont(
            &font(vec![a.clone(), b.clone(), c.clone()]),
            &BmFontOptions::new(8, "ui"),
        )
        .unwrap();
        let reverse = build_bmfont(&font(vec![c, b, a]), &BmFontOptions::new(8, "ui")).unwrap();

        assert_eq!(forward, reverse);
        assert!(character_line(&forward.descriptor, 'A').contains("x=1 y=1 width=1 height=3"));
        assert!(character_line(&forward.descriptor, 'B').contains("x=3 y=1 width=2 height=2"));
        assert!(character_line(&forward.descriptor, 'C').contains("x=1 y=5 width=3 height=1"));
    }

    #[test]
    fn starts_a_new_page_when_the_next_shelf_does_not_fit() {
        let font = font(vec![
            glyph('A', 6, &["######", "######", "######", "######"]),
            glyph('B', 6, &["######", "######", "######", "######"]),
        ]);
        let output = build_bmfont(&font, &BmFontOptions::new(8, "ui")).unwrap();

        assert_eq!(output.pages.len(), 2);
        assert_eq!(output.report.pages, 2);
        assert!(character_line(&output.descriptor, 'A').contains("x=1 y=1 width=6 height=4"));
        assert!(character_line(&output.descriptor, 'B').contains("x=1 y=1 width=6 height=4"));
        assert!(character_line(&output.descriptor, 'B').contains("page=1"));
    }

    #[test]
    fn rejects_a_glyph_that_cannot_fit_between_page_edges() {
        let font = font(vec![glyph(
            'A',
            7,
            &["#######", "#######", "#######", "#######"],
        )]);
        let error = build_bmfont(&font, &BmFontOptions::new(8, "ui")).unwrap_err();
        assert!(matches!(
            error,
            BmFontError::GlyphTooLarge {
                character: 'A',
                width: 7,
                height: 4,
                page_size: 8
            }
        ));
    }

    #[test]
    fn descriptor_uses_requested_format_and_sanitizes_face() {
        let mut font = font(vec![glyph('𐀀', 4, &["#...", "....", "....", "...."])]);
        font.meta.name = "quo\"ted".into();
        let output = build_bmfont(&font, &BmFontOptions::new(8, "ui atlas")).unwrap();
        let lines = output.descriptor.lines().collect::<Vec<_>>();

        assert_eq!(
            lines[0],
            "info face=\"quoted\" size=4 bold=0 italic=0 charset=\"\" unicode=1 stretchH=100 smooth=0 aa=1 padding=0,0,0,0 spacing=1,1 outline=0"
        );
        assert_eq!(
            lines[1],
            "common lineHeight=4 base=3 scaleW=8 scaleH=8 pages=1 packed=0 alphaChnl=0 redChnl=0 greenChnl=0 blueChnl=0"
        );
        assert_eq!(lines[2], "page id=0 file=\"ui atlas_0.png\"");
        assert_eq!(lines[3], "chars count=1");
        assert!(lines[4].starts_with("char id=65536 "));
        assert!(!output.descriptor.contains("kernings count="));
    }

    #[test]
    fn writes_kerning_and_omits_vertical_variants() {
        let mut vertical = glyph('ー', 4, &[".#..", ".#..", ".#..", "...."]);
        vertical.variant = Some("vert".into());
        let mut font = font(vec![
            glyph('A', 4, &["#...", "....", "....", "...."]),
            glyph('V', 4, &["#...", "....", "....", "...."]),
            vertical,
        ]);
        font.kerns.push(Kern {
            pair: "AV".into(),
            amount: -1,
        });
        let output = build_bmfont(&font, &BmFontOptions::new(8, "ui")).unwrap();

        assert_eq!(output.report.glyphs, 2);
        assert_eq!(output.report.skipped_vertical, 1);
        assert!(!output.descriptor.contains("id=12540"));
        assert!(output.descriptor.contains("kernings count=1\n"));
        assert!(
            output
                .descriptor
                .contains("kerning first=65 second=86 amount=-1\n")
        );
    }

    #[test]
    fn color_pages_keep_palette_colors_and_clear_transparency() {
        let mut font = font(vec![Glyph {
            character: '★',
            variant: None,
            advance: Some(2),
            canvas: IndexedCanvas::from_pixels(2, 4, vec![1, 2, 0, 1, 0, 0, 0, 0])
                .unwrap()
                .with_transparent(Some(0)),
        }]);
        font.meta.mode = FontMode::Color;
        font.meta.palette = Some(crate::FontPalette {
            reference: "colors.hex".into(),
            map: Default::default(),
        });
        let palette = Palette::new(vec![
            Rgba8::rgb(9, 9, 9),
            Rgba8::rgb(255, 0, 0),
            Rgba8::rgb(255, 255, 0),
        ])
        .unwrap();
        let output =
            build_bmfont(&font, &BmFontOptions::new(8, "ui").with_palette(&palette)).unwrap();

        assert_eq!(output.pages[0].get(1, 1), Some(Rgba8::rgb(255, 0, 0)));
        assert_eq!(output.pages[0].get(2, 1), Some(Rgba8::rgb(255, 255, 0)));
        assert_eq!(output.pages[0].get(1, 2), Some(Rgba8::TRANSPARENT));
        assert_eq!(output.pages[0].get(0, 0), Some(Rgba8::TRANSPARENT));
    }

    #[test]
    fn selected_characters_include_space_and_reject_missing() {
        let font = font(vec![
            glyph(' ', 2, &["....", "....", "....", "...."]),
            glyph('A', 4, &["#...", "....", "....", "...."]),
            glyph('B', 4, &["#...", "....", "....", "...."]),
        ]);
        let selected = BTreeSet::from(['A']);
        let output = build_bmfont(
            &font,
            &BmFontOptions::new(8, "ui").with_characters(&selected),
        )
        .unwrap();
        assert_eq!(output.report.glyphs, 2);
        assert!(output.descriptor.contains("char id=32 "));
        assert!(output.descriptor.contains("char id=65 "));
        assert!(!output.descriptor.contains("char id=66 "));

        let missing = BTreeSet::from(['A', 'Z']);
        let error = build_bmfont(
            &font,
            &BmFontOptions::new(8, "ui").with_characters(&missing),
        )
        .unwrap_err();
        assert_eq!(
            error,
            BmFontError::MissingGlyphs {
                characters: vec!['Z']
            }
        );
    }

    #[test]
    fn page_size_must_be_a_power_of_two() {
        let font = font(vec![]);
        for page_size in [0, 3, 255] {
            assert_eq!(
                build_bmfont(&font, &BmFontOptions::new(page_size, "ui")).unwrap_err(),
                BmFontError::InvalidPageSize { page_size }
            );
        }
    }
}
