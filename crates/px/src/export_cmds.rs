//! `px export` — 正規 JSON から出力先の形式へ変換する (設計書 4.4)．
//!
//! **元の絵は見ない．** 読むのは [`px_core::tilejson::TilesetDoc`] だけである —
//! 設計書 4.4 が «独自 JSON がエンジン非依存の正規出力．Godot / Tiled はアダプタ»
//! と定めているとおりの形にする．
//!
//! > [!warning] **Godot はまだ書かない．**
//! > terrain set の «peering bit» がこちらの 8 近傍 bitmask と 1 対 1 に対応すると
//! > 確かめられていない．**推測で書いた出力は，使う側が壊れて初めて分かる**
//! > (設計書 6.8 の «47 枚が静かに壊れる» と同じ) ．仕様を引いてから書くこと．

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use px_core::export::{tiled_tmx, tiled_tsx};
use px_core::sheet::SheetDoc;
use px_core::tilejson::TilesetDoc;

#[derive(Subcommand)]
pub enum ExportCommand {
    /// Tiled の `.tsx` / `.tmx` を書く
    Tiled {
        #[command(flatten)]
        args: TiledArgs,
    },
}

#[derive(Args, Clone)]
pub struct TiledArgs {
    /// 正規 JSON (`px tileset extract --map` / `px tileset autotile --map` の出力)
    pub input: PathBuf,
    /// `.tsx` の出力先
    #[arg(long)]
    pub tsx: Option<PathBuf>,
    /// `.tmx` の出力先．**正規 JSON が `map` の節を持つときだけ書ける**
    #[arg(long)]
    pub tmx: Option<PathBuf>,
    /// `px sheet pack` が書いた並べ方 (JSON)．**`.tsx` を書くならこれが要る**
    ///
    /// 列数 ・升 ・隙間 ・画像の名前と寸法は**すべてここから引く**．
    /// こちらで並べ方を決めたり，利用者に聞き直したりしない (D110) ．
    #[arg(long)]
    pub sheet: Option<PathBuf>,
    #[arg(long, default_value = "tileset")]
    pub name: String,
    /// `.tmx` が持つ先頭 GID (Tiled は 0 を «空の升» に使う)
    #[arg(long, default_value_t = 1)]
    pub first_gid: u32,
}

pub fn export(command: ExportCommand) -> Result<()> {
    match command {
        ExportCommand::Tiled { args } => tiled(&args),
    }
}

fn tiled(args: &TiledArgs) -> Result<()> {
    let text = std::fs::read_to_string(&args.input)
        .with_context(|| format!("{} を読めない", args.input.display()))?;
    let doc = TilesetDoc::from_json(&text)?;
    println!(
        "{} — タイル {} 画素 ・{} 枚{}{}",
        args.input.display(),
        doc.tile,
        doc.tiles,
        if doc.terrain.is_empty() {
            String::new()
        } else {
            format!(" ・terrain {} 通り", doc.terrain.len())
        },
        match &doc.map {
            Some(m) => format!(" ・地図 {}x{}", m.cols, m.rows),
            None => String::new(),
        }
    );

    if let Some(path) = &args.tsx {
        // **並べ方は `px sheet pack` から引く．** 無いなら書かない —
        // ここで «たぶん 8 列» と決めると，シートと `.tsx` が黙って食い違う
        let Some(sheet_path) = &args.sheet else {
            anyhow::bail!(
                ".tsx を書くには --sheet が要る (px sheet pack が書いた JSON)．\n\
                 並べ方を決めるのは sheet pack であって export ではない"
            );
        };
        let text = std::fs::read_to_string(sheet_path)
            .with_context(|| format!("{} を読めない", sheet_path.display()))?;
        let sheet = SheetDoc::from_json(&text)?;
        let out = tiled_tsx(&doc, &sheet, &args.name)?;
        std::fs::write(path, out).with_context(|| format!("{} を書き出せない", path.display()))?;
        println!(
            "  {} へ書き出した (並べ方は {} から引いた — {}x{} 列 ・{}x{} 画素)",
            path.display(),
            sheet_path.display(),
            sheet.columns,
            sheet.rows,
            sheet.width,
            sheet.height
        );
    }
    if let Some(path) = &args.tmx {
        let source = args
            .tsx
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("{}.tsx", args.name));
        let out = tiled_tmx(&doc, &source, args.first_gid)?;
        std::fs::write(path, out).with_context(|| format!("{} を書き出せない", path.display()))?;
        println!("  {} へ書き出した", path.display());
    }
    if args.tsx.is_none() && args.tmx.is_none() {
        println!("  何も書いていない (--tsx か --tmx を指定すること)");
    }

    // **書いていないものを黙らない** (D92 の作法)
    if !doc.terrain.is_empty() {
        println!(
            "  書いていない: terrain ({} 通り) は Tiled の Wang set へ写していない\n\
             (対応が確かめられていないため．正規 JSON には残っている)",
            doc.terrain.len()
        );
    }
    Ok(())
}
