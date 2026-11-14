//! `pxsmith font` — フォントの取り込みと字の不足検査．

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use pxsmith_core::font::{Font, Glyph};
use pxsmith_io::bdf::{BdfImportOptions, import_bdf};
use pxsmith_io::pxfont::PxFontDocument;

#[derive(Subcommand)]
pub enum FontCommand {
    /// BDF を `.pxfont.toml` へ取り込む
    Import(ImportArgs),
    /// 文言に必要な字と字形を検査する
    Check(CheckArgs),
}

#[derive(Args)]
pub struct ImportArgs {
    pub input: PathBuf,
    pub output: PathBuf,
    /// このファイルに現れる字だけを取り込む
    #[arg(long)]
    pub chars_file: Option<PathBuf>,
    /// 升からはみ出す BDF の BBX を切る
    #[arg(long)]
    pub clip: bool,
    /// TTF/OTF の 1 em の画素数（TTF/OTF 取り込みは未実装）
    #[arg(long)]
    pub size: Option<u32>,
}

#[derive(Args)]
pub struct CheckArgs {
    pub font: PathBuf,
    /// 検査する文言．複数指定できる
    #[arg(long = "text-file", required = true, num_args = 1..)]
    pub text_files: Vec<PathBuf>,
    /// 縦書きで Tu / Tr の変種も検査する
    #[arg(long)]
    pub vertical: bool,
}

pub fn font(command: FontCommand) -> Result<()> {
    match command {
        FontCommand::Import(args) => import(&args),
        FontCommand::Check(args) => check(&args),
    }
}

fn import(args: &ImportArgs) -> Result<()> {
    let extension = args
        .input
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match extension.as_str() {
        "bdf" => import_bdf_file(args),
        "ttf" | "otf" | "ttc" => {
            let size = args.size.map_or_else(
                || "（--size 未指定）".to_string(),
                |size| format!("（--size {size}）"),
            );
            bail!(
                "{} の {extension} 取り込みは未実装 {size}",
                args.input.display()
            )
        }
        _ => bail!(
            "{} の拡張子には対応していない（BDF は .bdf，TTF/OTF/TTC は未実装）",
            args.input.display()
        ),
    }
}

fn import_bdf_file(args: &ImportArgs) -> Result<()> {
    if args.size.is_some() {
        bail!("--size は TTF/OTF/TTC 専用であり，BDF には指定できない");
    }
    let text = std::fs::read_to_string(&args.input)
        .with_context(|| format!("{} を読めない", args.input.display()))?;
    let chars = args
        .chars_file
        .as_ref()
        .map(|path| read_chars(path))
        .transpose()?;
    let imported = import_bdf(
        &text,
        &args.input,
        &BdfImportOptions {
            clip: args.clip,
            chars,
        },
    )
    .with_context(|| format!("{} を BDF として取り込めない", args.input.display()))?;
    PxFontDocument::from_font(&imported.font)
        .context("取り込んだフォントを文書へ変換できない")?
        .write(&args.output)
        .with_context(|| format!("{} を書き出せない", args.output.display()))?;
    println!(
        "取り込み: {} / {} 字，符号位置なし {} 字，対応なし {} 字，対象外 {} 字，切り取り {} 字",
        imported.report.imported_glyphs,
        imported.report.declared_glyphs,
        imported.report.skipped_unencoded,
        imported.report.unmapped,
        imported.report.filtered_out,
        imported.report.clipped
    );
    println!("出力: {}", args.output.display());
    if let Some(copyright) = &imported.font.meta.copyright {
        println!("著作権表示: {copyright}");
    }
    Ok(())
}

fn read_chars(path: &Path) -> Result<BTreeSet<char>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("{} を読めない", path.display()))?;
    Ok(text
        .chars()
        .filter(|character| !matches!(character, '\n' | '\r'))
        .collect())
}

fn check(args: &CheckArgs) -> Result<()> {
    let document = PxFontDocument::read(&args.font)
        .with_context(|| format!("{} を読めない", args.font.display()))?;
    let font = document
        .to_font(&args.font)
        .with_context(|| format!("{} は有効な pxfont ではない", args.font.display()))?;
    let used = read_texts(&args.text_files)?;
    let base = font
        .glyphs
        .iter()
        .filter(|glyph| glyph.variant.is_none())
        .map(|glyph| glyph.character)
        .collect::<BTreeSet<_>>();
    let vertical = font
        .glyphs
        .iter()
        .filter(|glyph| glyph.variant.as_deref() == Some("vert"))
        .map(|glyph| glyph.character)
        .collect::<BTreeSet<_>>();

    let missing = used
        .iter()
        .copied()
        .filter(|character| !base.contains(character))
        .collect::<Vec<_>>();
    let missing_vertical = if args.vertical {
        used.iter()
            .copied()
            .filter(|character| {
                base.contains(character)
                    && vertical_requires_variant(*character)
                    && !vertical.contains(character)
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    print_shape_advisories(&font);
    if !missing.is_empty() {
        eprintln!(
            "不足字 {} 件: {}",
            missing.len(),
            shown_chars(&missing, usize::MAX)
        );
    }
    if !missing_vertical.is_empty() {
        eprintln!(
            "縦書きの変種不足 {} 件: {}",
            missing_vertical.len(),
            shown_chars(&missing_vertical, usize::MAX)
        );
    }
    if !missing.is_empty() || !missing_vertical.is_empty() {
        bail!(
            "font check で不足を検出した（字 {} 件，縦書き変種 {} 件）",
            missing.len(),
            missing_vertical.len()
        );
    }
    println!("不足字 0 件，縦書きの変種不足 0 件");
    Ok(())
}

fn read_texts(paths: &[PathBuf]) -> Result<BTreeSet<char>> {
    let mut output = BTreeSet::new();
    for path in paths {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("{} を読めない", path.display()))?;
        output.extend(
            text.chars()
                .filter(|character| !matches!(character, '\n' | '\r')),
        );
    }
    Ok(output)
}

fn print_shape_advisories(font: &Font) {
    let mut right_edge = Vec::new();
    let mut empty = Vec::new();
    let mut narrow = Vec::new();
    for glyph in &font.glyphs {
        if has_advisable_right_edge(glyph) {
            right_edge.push(glyph.character);
        }
        if glyph.canvas.opaque_bbox().is_none() && !glyph.character.is_whitespace() {
            empty.push(glyph.character);
        }
        if glyph.effective_advance(&font.meta) < glyph.canvas.width() {
            narrow.push(glyph.character);
        }
    }
    if !right_edge.is_empty() {
        println!(
            "助言: 右端の列に墨がある字 {} 件（罫線・ブロック要素を除く．隣の字とくっつく）（先頭 {}）: {}",
            right_edge.len(),
            right_edge.len().min(20),
            shown_chars(&right_edge, 20)
        );
    }
    print_advisory("墨が 1 画素も無い字", &empty);
    print_advisory("字送りが data の幅より小さい字", &narrow);
}

fn has_advisable_right_edge(glyph: &Glyph) -> bool {
    let code = glyph.character as u32;
    if glyph.variant.as_deref() == Some("vert")
        || (0x2500..=0x257f).contains(&code)
        || (0x2580..=0x259f).contains(&code)
        || glyph.canvas.width() == 0
    {
        return false;
    }
    let transparent = glyph.canvas.transparent();
    let right = glyph.canvas.width() as i32 - 1;
    (0..glyph.canvas.height() as i32).any(|y| {
        glyph
            .canvas
            .get(right, y)
            .is_some_and(|index| Some(index) != transparent)
    })
}

fn print_advisory(label: &str, characters: &[char]) {
    if !characters.is_empty() {
        println!(
            "助言: {label} {} 件（先頭 {}）: {}",
            characters.len(),
            characters.len().min(20),
            shown_chars(characters, 20)
        );
    }
}

fn shown_chars(characters: &[char], limit: usize) -> String {
    characters
        .iter()
        .take(limit)
        .map(|character| shown(*character))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shown(character: char) -> String {
    if character.is_whitespace() || character.is_control() {
        format!("U+{:04X}", character as u32)
    } else {
        format!("{character}(U+{:04X})", character as u32)
    }
}

fn vertical_requires_variant(character: char) -> bool {
    let code = character as u32;
    VERTICAL_VARIANT_RANGES
        .binary_search_by(|&(start, end)| {
            if code < start {
                std::cmp::Ordering::Greater
            } else if code > end {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// Unicode 18.0.0 `VerticalOrientation.txt` の Tu / Tr を機械生成した範囲．
const VERTICAL_VARIANT_RANGES: &[(u32, u32)] = &[
    (0x2018, 0x2018),
    (0x2019, 0x2019),
    (0x201C, 0x201C),
    (0x201D, 0x201D),
    (0x2329, 0x2329),
    (0x232A, 0x232A),
    (0x3001, 0x3002),
    (0x3008, 0x3008),
    (0x3009, 0x3009),
    (0x300A, 0x300A),
    (0x300B, 0x300B),
    (0x300C, 0x300C),
    (0x300D, 0x300D),
    (0x300E, 0x300E),
    (0x300F, 0x300F),
    (0x3010, 0x3010),
    (0x3011, 0x3011),
    (0x3014, 0x3014),
    (0x3015, 0x3015),
    (0x3016, 0x3016),
    (0x3017, 0x3017),
    (0x3018, 0x3018),
    (0x3019, 0x3019),
    (0x301A, 0x301A),
    (0x301B, 0x301B),
    (0x301C, 0x301C),
    (0x301D, 0x301D),
    (0x301E, 0x301F),
    (0x3030, 0x3030),
    (0x3041, 0x3041),
    (0x3043, 0x3043),
    (0x3045, 0x3045),
    (0x3047, 0x3047),
    (0x3049, 0x3049),
    (0x3063, 0x3063),
    (0x3083, 0x3083),
    (0x3085, 0x3085),
    (0x3087, 0x3087),
    (0x308E, 0x308E),
    (0x3095, 0x3096),
    (0x309B, 0x309C),
    (0x30A0, 0x30A0),
    (0x30A1, 0x30A1),
    (0x30A3, 0x30A3),
    (0x30A5, 0x30A5),
    (0x30A7, 0x30A7),
    (0x30A9, 0x30A9),
    (0x30C3, 0x30C3),
    (0x30E3, 0x30E3),
    (0x30E5, 0x30E5),
    (0x30E7, 0x30E7),
    (0x30EE, 0x30EE),
    (0x30F5, 0x30F6),
    (0x30FC, 0x30FC),
    (0x3127, 0x3127),
    (0x31B4, 0x31B7),
    (0x31BB, 0x31BB),
    (0x31F0, 0x31FF),
    (0x32FF, 0x32FF),
    (0x3300, 0x3357),
    (0x337B, 0x337F),
    (0xFE50, 0xFE52),
    (0xFE59, 0xFE59),
    (0xFE5A, 0xFE5A),
    (0xFE5B, 0xFE5B),
    (0xFE5C, 0xFE5C),
    (0xFE5D, 0xFE5D),
    (0xFE5E, 0xFE5E),
    (0xFF01, 0xFF01),
    (0xFF08, 0xFF08),
    (0xFF09, 0xFF09),
    (0xFF0C, 0xFF0C),
    (0xFF0E, 0xFF0E),
    (0xFF1A, 0xFF1B),
    (0xFF1F, 0xFF1F),
    (0xFF3B, 0xFF3B),
    (0xFF3D, 0xFF3D),
    (0xFF3F, 0xFF3F),
    (0xFF5B, 0xFF5B),
    (0xFF5C, 0xFF5C),
    (0xFF5D, 0xFF5D),
    (0xFF5E, 0xFF5E),
    (0xFF5F, 0xFF5F),
    (0xFF60, 0xFF60),
    (0xFFE3, 0xFFE3),
    (0x1B132, 0x1B132),
    (0x1B150, 0x1B152),
    (0x1B155, 0x1B155),
    (0x1B164, 0x1B168),
    (0x1F200, 0x1F201),
];

#[cfg(test)]
mod tests {
    use super::*;
    use pxsmith_core::IndexedCanvas;

    fn glyph(character: char, variant: Option<&str>, rows: &[&str]) -> Glyph {
        let width = rows.first().map_or(0, |row| row.len()) as u32;
        let pixels = rows
            .iter()
            .flat_map(|row| row.bytes().map(|pixel| u8::from(pixel == b'#')))
            .collect();
        Glyph {
            character,
            variant: variant.map(str::to_owned),
            advance: None,
            canvas: IndexedCanvas::from_pixels(width, rows.len() as u32, pixels)
                .unwrap()
                .with_transparent(Some(0)),
        }
    }

    #[test]
    fn right_edge_ink_is_advised() {
        assert!(has_advisable_right_edge(&glyph('R', None, &["..#", "..."])));
    }

    #[test]
    fn left_edge_only_is_not_advised() {
        assert!(!has_advisable_right_edge(&glyph(
            'L',
            None,
            &["#..", "..."]
        )));
    }

    #[test]
    fn box_drawing_and_block_elements_are_not_advised() {
        for character in ['\u{2500}', '\u{257f}', '\u{2580}', '\u{259f}'] {
            assert!(!has_advisable_right_edge(&glyph(
                character,
                None,
                &["###", "..."]
            )));
        }
    }

    #[test]
    fn vertical_variant_is_not_advised() {
        assert!(!has_advisable_right_edge(&glyph(
            'R',
            Some("vert"),
            &["..#", "..."]
        )));
    }

    #[test]
    fn generated_vertical_variant_ranges_match_unicode_data() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/unicode/VerticalOrientation.txt");
        let text = std::fs::read_to_string(path).unwrap();
        let mut source = Vec::new();
        for line in text.lines() {
            let Some((range, rest)) = line.split_once(';') else {
                continue;
            };
            let property = rest.split('#').next().unwrap_or("").trim();
            if !matches!(property, "Tu" | "Tr") {
                continue;
            }
            let range = range.trim();
            let (start, end) = range.split_once("..").unwrap_or((range, range));
            source.push((
                u32::from_str_radix(start, 16).unwrap(),
                u32::from_str_radix(end, 16).unwrap(),
            ));
        }
        assert_eq!(VERTICAL_VARIANT_RANGES, source);
    }

    #[test]
    fn only_tu_and_tr_require_a_vertical_variant() {
        assert!(vertical_requires_variant('、'));
        assert!(vertical_requires_variant('ー'));
        assert!(!vertical_requires_variant('漢'));
        assert!(!vertical_requires_variant('A'));
    }

    #[test]
    fn duplicate_text_characters_are_checked_once() {
        let root = std::env::temp_dir().join(format!("pxsmith-font-check-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("text.txt");
        std::fs::write(&path, "AAA\nA\r\n").unwrap();
        assert_eq!(read_texts(&[path]).unwrap(), BTreeSet::from(['A']));
    }
}
