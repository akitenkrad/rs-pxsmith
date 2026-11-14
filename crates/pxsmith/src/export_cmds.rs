//! `pxsmith export` — シート／タイルセットのメタから出力先の形式へ変換する．
//!
//! **元の絵は見ない．** Tiled は [`pxsmith_core::tilejson::TilesetDoc`] と `SheetDoc`，
//! Godot StyleBox は `SheetDoc` からだけ引く．出力先ごとに並べ方や矩形を決め直さない．
//!
//! Godot の `StyleBoxTexture` は `SheetDoc` の 9-slice 宣言から書ける．terrain set の
//! «peering bit» は，こちらの 8 近傍 bitmask との対応を確かめられていないため，
//! 引き続き書かない．

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use pxsmith_core::export::{godot_styleboxes, tiled_tmx, tiled_tsx};
use pxsmith_core::sheet::SheetDoc;
use pxsmith_core::tilejson::TilesetDoc;

#[derive(Subcommand)]
pub enum ExportCommand {
    /// Tiled の `.tsx` / `.tmx` を書く
    Tiled {
        #[command(flatten)]
        args: TiledArgs,
    },
    /// Godot 4.7 の `StyleBoxTexture` (`.tres`) を書く
    Godot {
        #[command(flatten)]
        args: GodotArgs,
    },
}

#[derive(Args, Clone)]
pub struct GodotArgs {
    /// `pxsmith sheet pack` が書いたシートメタ
    #[arg(long)]
    pub sheet: PathBuf,
    /// `<升名>.stylebox.tres` を置くディレクトリ
    #[arg(long)]
    pub out_dir: PathBuf,
    /// Godot プロジェクト内でシート画像を置くディレクトリ (`res://` で始める)
    #[arg(long)]
    pub res_dir: String,
}

#[derive(Args, Clone)]
pub struct TiledArgs {
    /// 正規 JSON (`pxsmith tileset extract --map` / `pxsmith tileset autotile --map` の出力)
    pub input: PathBuf,
    /// `.tsx` の出力先
    #[arg(long)]
    pub tsx: Option<PathBuf>,
    /// `.tmx` の出力先．**正規 JSON が `map` の節を持つときだけ書ける**
    #[arg(long)]
    pub tmx: Option<PathBuf>,
    /// `pxsmith sheet pack` が書いた並べ方 (JSON)．**`.tsx` を書くならこれが要る**
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
        ExportCommand::Godot { args } => godot(&args),
    }
}

fn godot(args: &GodotArgs) -> Result<()> {
    let text = std::fs::read_to_string(&args.sheet)
        .with_context(|| format!("{} を読めない", args.sheet.display()))?;
    let sheet = SheetDoc::from_json(&text)?;
    let export = godot_styleboxes(&sheet, &args.res_dir)?;

    std::fs::create_dir_all(&args.out_dir)
        .with_context(|| format!("{} を作れない", args.out_dir.display()))?;
    for file in &export.files {
        let path = args.out_dir.join(&file.file_name);
        std::fs::write(&path, &file.text)
            .with_context(|| format!("{} を書き出せない", path.display()))?;
        println!("  {} へ書き出した", path.display());
    }
    if export.palette_swap_warning {
        eprintln!(
            "警告: {} は添字テクスチャなので，そのまま貼ると色にならない．\
             パレットスワップのシェーダが要る",
            sheet.image
        );
    }

    // **書いていないものを黙らない** (D92 の作法)．
    println!(
        "  書いていない: nine_slice を持たない升 ({} 件)",
        export.skipped_cells
    );
    println!("  書いていない: Theme (1 件)");
    Ok(())
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
        // **並べ方は `pxsmith sheet pack` から引く．** 無いなら書かない —
        // ここで «たぶん 8 列» と決めると，シートと `.tsx` が黙って食い違う
        let Some(sheet_path) = &args.sheet else {
            anyhow::bail!(
                ".tsx を書くには --sheet が要る (pxsmith sheet pack が書いた JSON)．\n\
                 並べ方を決めるのは sheet pack であって export ではない"
            );
        };
        let text = std::fs::read_to_string(sheet_path)
            .with_context(|| format!("{} を読めない", sheet_path.display()))?;
        let sheet = SheetDoc::from_json(&text)?;
        if sheet.format == pxsmith_core::sheet::PALETTE_SWAP_FORMAT_VERSION {
            eprintln!("警告: シート画像は添字テクスチャで，Tiled ではそのまま色として表示されない");
        }
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
