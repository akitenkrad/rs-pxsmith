//! `pxsmith sheet pack` — スプライトシートと JSON メタ (設計書 5 章 `op = "sheet.pack"`)．
//!
//! **並べ方を決めるのはここだけである．** `pxsmith export tiled` は `--sheet` でこの
//! メタを読み，列数 ・升の大きさ ・隙間 ・画像の寸法をすべてそこから引く —
//! 利用者に聞き直したり，数値を書き写したりしない (D110 の «正規出力が 2 つ
//! あるのは正規出力が無いのと同じ» と同じ話) ．

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use pxsmith_core::sheet::{PackOptions, SheetItem, pack};

use crate::color_cmds::write_indexed;

#[derive(Subcommand)]
pub enum SheetCommand {
    /// フレームを 1 枚のシートへ並べ，並べ方を JSON メタに書く
    Pack {
        #[command(flatten)]
        args: PackArgs,
    },
}

#[derive(Args, Clone)]
pub struct PackArgs {
    /// シート画像の出力先 (`.png` か `.aseprite`)
    pub output: PathBuf,
    /// 並べる絵．**フレームを持つものは全フレームが順に載る**
    #[arg(long = "input", num_args = 1.., required = true)]
    pub inputs: Vec<PathBuf>,
    /// 並べ方を書く JSON．省略すると出力の隣に `.sheet.json` で置く
    ///
    /// **`.json` にしない．** `pxsmith tileset extract --map` の正規 JSON が
    /// `tiles.json` を使うので，`tiles.png` の隣に `tiles.json` を書くと
    /// **黙って上書きする** (端から端まで通して見つけた) ．
    #[arg(long)]
    pub meta: Option<PathBuf>,
    /// 列数．**省略時は空き升が最小になる並びを選ぶ** (数え上げ．校正の対象ではない)
    #[arg(long)]
    pub columns: Option<u32>,
    /// 升と升の間の空き (Tiled の `spacing`)．**既定 0**
    #[arg(long, default_value_t = 0)]
    pub padding: u32,
    /// 外周の空き (Tiled の `margin`)．**既定 0**
    #[arg(long, default_value_t = 0)]
    pub margin: u32,
}

pub fn sheet(command: SheetCommand) -> Result<()> {
    match command {
        SheetCommand::Pack { args } => pack_cmd(&args),
    }
}

fn stem(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().replace(".px.toml", ""))
        .map(|s| {
            s.trim_end_matches(".aseprite")
                .trim_end_matches(".png")
                .to_string()
        })
        .unwrap_or_else(|| "frame".to_string())
}

fn pack_cmd(args: &PackArgs) -> Result<()> {
    let mut items: Vec<SheetItem> = Vec::new();
    for path in &args.inputs {
        if crate::color_cmds::is_png(path) {
            bail!(
                "{} は PNG なのでインデックスカラーとして読めない．\n\
                 先に pxsmith quantize か pxsmith palette apply を通すこと",
                path.display()
            );
        }
        let frames = crate::load_frames(path)?;
        if frames.is_empty() {
            bail!("{} にフレームが 1 つも無い", path.display());
        }
        let base = stem(path);
        for (i, frame) in frames.iter().enumerate() {
            let canvas = flatten(frame).with_context(|| {
                format!(
                    "{} のフレーム {i} にインデックスカラーのレイヤが無い",
                    path.display()
                )
            })?;
            // 1 枚しか無いならファイル名そのまま．複数なら添字を付ける
            let name = if frames.len() == 1 {
                base.clone()
            } else {
                format!("{base}_{i:03}")
            };
            items.push(SheetItem {
                name,
                canvas,
                palette: frame.palette.clone(),
                duration_ms: frame.duration_ms,
            });
        }
    }

    let opts = PackOptions {
        columns: args.columns,
        padding: args.padding,
        margin: args.margin,
    };
    let (canvas, palette, mut doc, report) = pack(&items, &opts)?;

    let image = args
        .output
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "sheet.png".to_string());
    doc.image = image;

    write_indexed(&args.output, &canvas, &palette)?;
    let meta = args
        .meta
        .clone()
        .unwrap_or_else(|| args.output.with_extension("sheet.json"));
    // **別の種類の文書を黙って潰さない．** 自分が前に書いたシートなら上書きしてよい
    // (冪等) が，タイルセットの正規 JSON を踏むと «並べたら地図が消えた» になる
    if let Ok(existing) = std::fs::read_to_string(&meta)
        && pxsmith_core::sheet::SheetDoc::from_json(&existing).is_err()
    {
        bail!(
            "{} は既にあり，シートのメタではない．\n\
             上書きするとその文書が消える (--meta で別の名前を指定すること)",
            meta.display()
        );
    }
    std::fs::write(&meta, doc.to_json()?)
        .with_context(|| format!("{} を書き出せない", meta.display()))?;

    println!(
        "{} — {} 枚を {}x{} 列に並べた ({}x{} 画素 ・升 {}x{} ・{} 色)",
        args.output.display(),
        report.items,
        doc.columns,
        doc.rows,
        doc.width,
        doc.height,
        doc.cell_w,
        doc.cell_h,
        report.colors
    );
    println!(
        "  空き升 {} ・無駄になった面積 {:.1}%{}",
        report.empty_cells,
        report.waste * 100.0,
        if report.smaller_than_cell > 0 {
            format!(" (升より小さい絵が {} 枚)", report.smaller_than_cell)
        } else {
            String::new()
        }
    );
    if args.columns.is_none() {
        println!("  列数は枚数から決めた (--columns で明示できる)");
    }
    println!("  並べ方 -> {}", meta.display());
    // **書いていないものを黙らない** (D92 の作法)
    println!(
        "  書いていない: 絵ごとに切り詰めて詰め込む «非一様な梱包» は実装していない\n\
         (Tiled も Godot も一様格子しか受け取らないため)"
    );
    Ok(())
}

/// レイヤを下から順に載せて 1 枚にする．
fn flatten(frame: &pxsmith_core::Frame) -> Option<pxsmith_core::IndexedCanvas> {
    let first = frame.layers.first()?.surface.as_indexed()?;
    let transparent = first.transparent().unwrap_or(0);
    let mut out = pxsmith_core::IndexedCanvas::filled(frame.size.x, frame.size.y, transparent)
        .with_transparent(Some(transparent));
    for layer in &frame.layers {
        if let Some(c) = layer.surface.as_indexed() {
            out.blit(c, pxsmith_core::ivec2(0, 0), true);
        }
    }
    Some(out)
}
