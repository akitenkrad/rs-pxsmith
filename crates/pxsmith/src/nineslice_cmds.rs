//! `pxsmith nineslice preview` — L0 の 9-slice 宣言どおりに確認画像を書く．

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Subcommand;
use pxsmith_core::{NineSliceAxis, render_nine_slice, visible_seams};
use pxsmith_io::l0::L0Document;

#[derive(Subcommand)]
pub(crate) enum NinesliceCommand {
    /// L0 の 9-slice 宣言どおりに伸ばした PNG を書く
    Preview {
        /// 入力 (`.px.toml` / `.toml` / `.aseprite` / `.ase`)
        input: PathBuf,
        output: PathBuf,
        /// 描画先の大きさ (`W`x`H`)．複数指定できる
        #[arg(long = "size", required = true, value_parser = parse_size)]
        sizes: Vec<OutputSize>,
    },
}

#[derive(Copy, Clone, Debug)]
pub(crate) struct OutputSize {
    width: u32,
    height: u32,
}

pub(crate) fn parse_size(value: &str) -> std::result::Result<OutputSize, String> {
    let Some((width, height)) = value.split_once('x') else {
        return Err("大きさは W x H の形で指定する (例: 64x32)".to_string());
    };
    if height.contains('x') {
        return Err("大きさは W x H の形で指定する (例: 64x32)".to_string());
    }
    let width = width
        .parse::<u32>()
        .map_err(|_| format!("幅 '{width}' は整数でない"))?;
    let height = height
        .parse::<u32>()
        .map_err(|_| format!("高さ '{height}' は整数でない"))?;
    if width == 0 || height == 0 {
        return Err("幅と高さは 1 以上でなければならない".to_string());
    }
    Ok(OutputSize { width, height })
}

pub(crate) fn nineslice(command: NinesliceCommand) -> Result<()> {
    match command {
        NinesliceCommand::Preview {
            input,
            output,
            sizes,
        } => preview(&input, &output, &sizes),
    }
}

fn preview(input: &Path, output: &Path, sizes: &[OutputSize]) -> Result<()> {
    let document = L0Document::read(input)
        .with_context(|| format!("{} を L0 として読めない", input.display()))?;
    let nine_slice = document
        .meta
        .nine_slice
        .context("L0 の [meta] に nine_slice が無い")?;
    let frames = document
        .to_frames(input)
        .with_context(|| format!("{} を作業層へ変換できない", input.display()))?;
    let frame = frames.first().context("L0 にフレームが 1 つも無い")?;
    let source = frame
        .layers
        .first()
        .and_then(|layer| layer.surface.as_indexed())
        .context("L0 の先頭フレームにインデックスカラーのレイヤが無い")?;

    for axis in visible_seams(source, &nine_slice)? {
        let axis = match axis {
            NineSliceAxis::Horizontal => "横軸",
            NineSliceAxis::Vertical => "縦軸",
        };
        eprintln!("助言: {axis}の中央帯は両端の画素が違うため，タイルの継ぎ目が見える");
    }

    for size in sizes {
        let destination = if sizes.len() == 1 {
            output.to_path_buf()
        } else {
            size_suffixed_path(output, *size)
        };
        let rendered = render_nine_slice(source, &nine_slice, size.width, size.height)?;
        pxsmith_io::png::write_indexed(&destination, &rendered, &frame.palette)
            .with_context(|| format!("{} を書き出せない", destination.display()))?;
        println!(
            "{} -> {} ({}x{})",
            input.display(),
            destination.display(),
            size.width,
            size.height
        );
    }
    Ok(())
}

pub(crate) fn size_suffixed_path(output: &Path, size: OutputSize) -> PathBuf {
    let stem = output
        .file_stem()
        .map(|stem| stem.to_string_lossy())
        .unwrap_or_else(|| "preview".into());
    let extension = output
        .extension()
        .map(|extension| format!(".{}", extension.to_string_lossy()))
        .unwrap_or_default();
    output.with_file_name(format!(
        "{stem}-{}x{}{}",
        size.width, size.height, extension
    ))
}
