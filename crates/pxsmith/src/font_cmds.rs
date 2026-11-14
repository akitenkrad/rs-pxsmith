//! `pxsmith font` — フォントの取り込み，字の不足検査，文字列の焼き込み．

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use pxsmith_core::font::{Font, FontMode, Glyph};
use pxsmith_core::{
    FontLayoutError, FontLayoutOptions, IndexedCanvas, Kinsoku, Palette, Rgba8, TextAlign,
    VerticalOrientation, WritingDirection, layout_text, vertical_orientation,
};
use pxsmith_io::bdf::{BdfImportOptions, import_bdf};
use pxsmith_io::pxfont::PxFontDocument;
use pxsmith_io::ttf::{TtfImportOptions, import_ttf};

#[derive(Subcommand)]
pub enum FontCommand {
    /// BDF を `.pxfont.toml` へ取り込む
    Import(ImportArgs),
    /// 文言に必要な字と字形を検査する
    Check(CheckArgs),
    /// 文字列を横書きまたは縦書きで画布へ焼き込む
    Render(RenderArgs),
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
    /// TTF/OTF/TTC の 1 em の画素数
    #[arg(long)]
    pub size: Option<u32>,
    /// 埋め込みビットマップがあっても輪郭をラスタライズする
    #[arg(long)]
    pub outline: bool,
    /// TTC の face 添字（既定 0）
    #[arg(long)]
    pub face: Option<u32>,
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

#[derive(Copy, Clone, Debug, Default, ValueEnum)]
pub enum AlignArg {
    #[default]
    Left,
    Center,
    Right,
}

impl From<AlignArg> for TextAlign {
    fn from(value: AlignArg) -> Self {
        match value {
            AlignArg::Left => Self::Left,
            AlignArg::Center => Self::Center,
            AlignArg::Right => Self::Right,
        }
    }
}

#[derive(Copy, Clone, Debug, Default, ValueEnum)]
pub enum KinsokuArg {
    #[default]
    Normal,
    Strict,
}

impl From<KinsokuArg> for Kinsoku {
    fn from(value: KinsokuArg) -> Self {
        match value {
            KinsokuArg::Normal => Self::Normal,
            KinsokuArg::Strict => Self::Strict,
        }
    }
}

#[derive(Args)]
pub struct RenderArgs {
    /// `.pxfont.toml` フォント
    pub font: PathBuf,
    /// `TEXT OUTPUT`，または --text-file 使用時の `OUTPUT`
    #[arg(value_names = ["TEXT", "OUTPUT"], num_args = 1..=2)]
    pub text_and_output: Vec<OsString>,
    /// UTF-8 の文言ファイル（位置引数の文字列と排他）
    #[arg(long)]
    pub text_file: Option<PathBuf>,
    /// 出力に使う `.hex` パレット
    #[arg(long)]
    pub palette: Option<PathBuf>,
    /// 単色フォントの墨に使うパレット添字
    #[arg(long)]
    pub ink: Option<u8>,
    /// 字間（px）
    #[arg(long, default_value_t = 0)]
    pub letter_spacing: u32,
    /// 行間（px）
    #[arg(long, default_value_t = 0)]
    pub line_spacing: u32,
    /// 行の寄せ方（縦書きでは left = 上，center = 中央，right = 下）
    #[arg(long, value_enum, default_value_t = AlignArg::Left)]
    pub align: AlignArg,
    /// 折り返す行幅，または縦書きの列の高さ（px）
    #[arg(long)]
    pub wrap: Option<u32>,
    /// カーニングを無効にする
    #[arg(long)]
    pub no_kern: bool,
    /// 小書き仮名の行頭禁則
    #[arg(long, value_enum, default_value_t = KinsokuArg::Normal)]
    pub kinsoku: KinsokuArg,
    /// 行末の句読点を 1 字だけ行幅の外へ出す
    #[arg(long)]
    pub hang: bool,
    /// フォントに無い字を置き換える字（1 字または U+XXXX）
    #[arg(long, value_parser = parse_character)]
    pub missing: Option<char>,
    /// 上から下，右から左へ縦書きする
    #[arg(long)]
    pub vertical: bool,
    /// N 桁以下の半角数字を 1 字分へ横組みする（--vertical が必要）
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..), requires = "vertical")]
    pub tcy: Option<u32>,
}

pub fn font(command: FontCommand) -> Result<()> {
    match command {
        FontCommand::Import(args) => import(&args),
        FontCommand::Check(args) => check(&args),
        FontCommand::Render(args) => render(&args),
    }
}

fn render(args: &RenderArgs) -> Result<()> {
    let (text, output) = render_input(args)?;
    validate_render_extension(&output)?;
    let document = PxFontDocument::read(&args.font)
        .with_context(|| format!("{} を読めない", args.font.display()))?;
    let font = document
        .to_font(&args.font)
        .with_context(|| format!("{} は有効な pxfont ではない", args.font.display()))?;
    let options = FontLayoutOptions {
        direction: if args.vertical {
            WritingDirection::Vertical
        } else {
            WritingDirection::Horizontal
        },
        letter_spacing: args.letter_spacing,
        line_spacing: args.line_spacing,
        align: args.align.into(),
        wrap: args.wrap,
        kern: !args.no_kern && !args.vertical,
        kinsoku: args.kinsoku.into(),
        hang: args.hang,
        missing: args.missing,
        tcy: args.tcy,
    };
    let laid_out = layout_text(&font, &text, &options).map_err(layout_error)?;
    let drawn_characters = laid_out
        .report
        .lines
        .iter()
        .flat_map(|line| line.chars())
        .collect::<BTreeSet<_>>();
    let (canvas, palette) = prepare_output(
        &font,
        &args.font,
        laid_out.canvas,
        &drawn_characters,
        args.palette.as_deref(),
        args.ink,
    )?;
    crate::color_cmds::write_indexed(&output, &canvas, &palette)?;

    for (character, count) in &laid_out.report.replacements {
        println!(
            "置換: {} -> {} {count} 件",
            shown(*character),
            shown(args.missing.expect("置換があれば代替字がある"))
        );
    }
    if laid_out.report.forced_breaks != 0 {
        println!(
            "語の途中を字単位で折り返した: {} 件",
            laid_out.report.forced_breaks
        );
    }
    if laid_out.report.hanging_punctuation != 0 {
        println!(
            "句読点を行幅の外へぶら下げた: {} 件",
            laid_out.report.hanging_punctuation
        );
    }
    if laid_out.report.unconverted_cj != 0 {
        println!(
            "normal 禁則用の同じ UTF-8 長の ID 類が無かった: {} 件",
            laid_out.report.unconverted_cj
        );
    }
    if laid_out.report.vertical_variants != 0 {
        println!(
            "縦書き変種を使った: {} 件",
            laid_out.report.vertical_variants
        );
    }
    if laid_out.report.rotated_r != 0 {
        println!("R を回した: {} 件", laid_out.report.rotated_r);
    }
    if laid_out.report.rotated_tr != 0 {
        println!(
            "Tr の変種が無いため回した: {} 件",
            laid_out.report.rotated_tr
        );
    }
    if laid_out.report.upright_tu != 0 {
        println!(
            "Tu の変種が無いため正立にした: {} 件",
            laid_out.report.upright_tu
        );
    }
    if laid_out.report.tcy_groups != 0 {
        println!("縦中横: {} 組", laid_out.report.tcy_groups);
    }
    if laid_out.report.tcy_fallbacks != 0 {
        println!("縦中横を諦めた: {} 組", laid_out.report.tcy_fallbacks);
    }
    let unit = if args.vertical { "列" } else { "行" };
    println!(
        "焼き込み: {} {}，{}x{} -> {}",
        laid_out.report.lines.len(),
        unit,
        canvas.width(),
        canvas.height(),
        output.display()
    );
    Ok(())
}

fn render_input(args: &RenderArgs) -> Result<(String, PathBuf)> {
    match (&args.text_file, args.text_and_output.as_slice()) {
        (None, [text, output]) => Ok((
            text.to_str()
                .context("位置引数の文字列が UTF-8 でない")?
                .to_string(),
            PathBuf::from(output),
        )),
        (Some(path), [output]) => Ok((
            std::fs::read_to_string(path)
                .with_context(|| format!("{} を UTF-8 として読めない", path.display()))?
                .replace("\r\n", "\n"),
            PathBuf::from(output),
        )),
        (None, _) => bail!("位置引数に TEXT と OUTPUT を指定すること"),
        (Some(_), _) => bail!("--text-file と位置引数の TEXT は同時に指定できない"),
    }
}

fn layout_error(error: FontLayoutError) -> anyhow::Error {
    match error {
        FontLayoutError::MissingGlyphs { characters } => anyhow::anyhow!(
            "フォントに無い字 {} 件: {}",
            characters.len(),
            shown_chars(&characters, usize::MAX)
        ),
        other => anyhow::Error::new(other),
    }
}

fn prepare_output(
    font: &Font,
    font_path: &Path,
    canvas: IndexedCanvas,
    drawn_characters: &BTreeSet<char>,
    palette_path: Option<&Path>,
    ink: Option<u8>,
) -> Result<(IndexedCanvas, Palette)> {
    match font.meta.mode {
        FontMode::Mono => prepare_mono_output(canvas, palette_path, ink),
        FontMode::Color => {
            prepare_color_output(font, font_path, canvas, drawn_characters, palette_path, ink)
        }
    }
}

fn prepare_mono_output(
    mut canvas: IndexedCanvas,
    palette_path: Option<&Path>,
    ink: Option<u8>,
) -> Result<(IndexedCanvas, Palette)> {
    let palette_path = palette_path.context("単色フォントには --palette が必要")?;
    let ink = ink.context("単色フォントには --ink が必要")?;
    let mut palette = pxsmith_io::hex::read(palette_path)
        .with_context(|| format!("{} を読めない", palette_path.display()))?;
    if ink as usize >= palette.len() {
        bail!("--ink {ink} はパレットの範囲外（色数 {}）", palette.len());
    }
    let transparent = ensure_transparent(&mut palette)?;
    let source_transparent = canvas.transparent().context("組版画布に透明添字が無い")?;
    let mut map = (0..=u8::MAX).collect::<Vec<_>>();
    map[source_transparent as usize] = transparent;
    map[1] = ink;
    canvas.remap(&map)?;
    Ok((canvas, palette))
}

fn prepare_color_output(
    font: &Font,
    font_path: &Path,
    mut canvas: IndexedCanvas,
    drawn_characters: &BTreeSet<char>,
    palette_path: Option<&Path>,
    ink: Option<u8>,
) -> Result<(IndexedCanvas, Palette)> {
    if ink.is_some() {
        bail!("多色フォントに --ink は指定できない");
    }
    let spec = font
        .meta
        .palette
        .as_ref()
        .context("多色フォントに [palette] が無い")?;
    let source_path = font_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(&spec.reference);
    let source = pxsmith_io::hex::read(&source_path)
        .with_context(|| format!("{} を読めない", source_path.display()))?;
    let mut output = match palette_path {
        Some(path) => {
            pxsmith_io::hex::read(path).with_context(|| format!("{} を読めない", path.display()))?
        }
        None => source.clone(),
    };

    let used = font
        .glyphs
        .iter()
        .filter(|glyph| glyph.variant.is_none() && drawn_characters.contains(&glyph.character))
        .flat_map(|glyph| {
            glyph
                .canvas
                .pixels()
                .iter()
                .copied()
                .map(move |index| (glyph, index))
        })
        .filter(|(glyph, index)| glyph.canvas.transparent() != Some(*index))
        .map(|(_, index)| index)
        .collect::<BTreeSet<_>>();
    let mut missing = BTreeSet::new();
    let mut remap = (0..=u8::MAX).collect::<Vec<_>>();
    for source_index in used {
        let color = source.get(source_index).with_context(|| {
            format!(
                "多色フォントの添字 {source_index} が {} の範囲外",
                source_path.display()
            )
        })?;
        match output
            .entries()
            .iter()
            .position(|candidate| *candidate == color)
        {
            Some(index) => remap[source_index as usize] = index as u8,
            None => {
                missing.insert(color.to_hex_string());
            }
        }
    }
    if !missing.is_empty() {
        bail!(
            "出力パレットに多色フォントの色が {} 色無い: {}",
            missing.len(),
            missing.into_iter().collect::<Vec<_>>().join(" ")
        );
    }
    let transparent = ensure_transparent(&mut output)?;
    let source_transparent = canvas.transparent().context("組版画布に透明添字が無い")?;
    remap[source_transparent as usize] = transparent;
    canvas.remap(&remap)?;
    Ok((canvas, output))
}

fn ensure_transparent(palette: &mut Palette) -> Result<u8> {
    match palette.entries().iter().position(|color| color.a == 0) {
        Some(index) => Ok(index as u8),
        None => palette
            .push(Rgba8::TRANSPARENT)
            .context("透明背景のためのパレット添字を確保できない"),
    }
}

fn validate_render_extension(path: &Path) -> Result<()> {
    let name = path.to_string_lossy();
    if name.ends_with(".px.toml")
        || path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("aseprite") || extension.eq_ignore_ascii_case("png")
            })
    {
        return Ok(());
    }
    bail!("出力は .px.toml／.aseprite／.png のいずれかにすること")
}

fn parse_character(value: &str) -> std::result::Result<char, String> {
    if let Some(hex) = value.strip_prefix("U+") {
        if !(4..=6).contains(&hex.len()) || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!("'{value}' を U+XXXX の符号位置として読めない"));
        }
        let code = u32::from_str_radix(hex, 16)
            .map_err(|_| format!("'{value}' を U+XXXX の符号位置として読めない"))?;
        return char::from_u32(code)
            .ok_or_else(|| format!("'{value}' は Unicode の符号位置でない"));
    }
    let mut characters = value.chars();
    match (characters.next(), characters.next()) {
        (Some(character), None) => Ok(character),
        _ => Err(format!("'{value}' は 1 文字でない")),
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
        "ttf" | "otf" | "ttc" => import_ttf_file(args),
        _ => bail!(
            "{} の拡張子には対応していない（.bdf / .ttf / .otf / .ttc を指定すること）",
            args.input.display()
        ),
    }
}

fn import_bdf_file(args: &ImportArgs) -> Result<()> {
    if args.size.is_some() {
        bail!("--size は TTF/OTF/TTC 専用であり，BDF には指定できない");
    }
    if args.outline {
        bail!("--outline は TTF/OTF/TTC 専用であり，BDF には指定できない");
    }
    if args.face.is_some() {
        bail!("--face は TTC 専用であり，BDF には指定できない");
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

fn import_ttf_file(args: &ImportArgs) -> Result<()> {
    let size = args
        .size
        .ok_or_else(|| anyhow::anyhow!("TTF/OTF/TTC の取り込みには --size が必要"))?;
    let size = u16::try_from(size)
        .ok()
        .filter(|size| *size != 0)
        .ok_or_else(|| anyhow::anyhow!("--size は 1..=65535 で指定すること"))?;
    let data = std::fs::read(&args.input)
        .with_context(|| format!("{} を読めない", args.input.display()))?;
    let chars = args
        .chars_file
        .as_ref()
        .map(|path| read_chars(path))
        .transpose()?;
    let imported = import_ttf(
        &data,
        &args.input,
        &TtfImportOptions {
            size,
            face: args.face.unwrap_or(0),
            outline: args.outline,
            clip: args.clip,
            chars,
        },
    )
    .with_context(|| {
        format!(
            "{} を OpenType フォントとして取り込めない",
            args.input.display()
        )
    })?;
    PxFontDocument::from_font(&imported.font)
        .context("取り込んだフォントを文書へ変換できない")?
        .write(&args.output)
        .with_context(|| format!("{} を書き出せない", args.output.display()))?;
    println!(
        "取り込み: {} / {} 字，対象外 {} 字，切り取り {} 字",
        imported.report.imported_characters,
        imported.report.mapped_characters,
        imported.report.filtered_out,
        imported.report.clipped
    );
    println!(
        "字形経路: 埋め込み {} 字，輪郭 {} 字，strike に無い {} 字，ppem 不一致 {} 字，非1ビット {} 字",
        imported.report.embedded_bitmaps,
        imported.report.outline_glyphs,
        imported.report.bitmap_missing,
        imported.report.bitmap_wrong_ppem,
        imported.report.bitmap_unsupported_format
    );
    println!(
        "寸法: line_height {}（丸め差 {:+.3} px），baseline {}（丸め差 {:+.3} px），advance 丸め {} 字",
        imported.font.meta.line_height,
        imported.report.line_height_rounding,
        imported.font.meta.baseline,
        imported.report.baseline_rounding,
        imported.report.rounded_advances
    );
    println!(
        "OpenType: 縦書き {} 字（未対応 lookup {} 件），カーニング {} 組",
        imported.report.vertical_glyphs,
        imported.report.unsupported_vertical_lookups,
        imported.report.kern_pairs
    );
    println!(
        "輪郭格子外: {} / {} 字",
        imported.report.off_grid_glyphs, imported.report.imported_characters
    );
    if imported.report.imported_characters != 0 {
        let ratio =
            imported.report.off_grid_glyphs as f64 / imported.report.imported_characters as f64;
        if ratio > 0.05 {
            println!(
                "助言: 輪郭の点が画素格子に乗らない字が {:.1}% あるため，ドット絵用でない輪郭はつぶれる可能性がある",
                ratio * 100.0
            );
        }
    }
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
    matches!(
        vertical_orientation(character as u32),
        VerticalOrientation::Tu | VerticalOrientation::Tr
    )
}

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
