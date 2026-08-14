//! `px tileset extract` — タイル分割と重複除去 (設計書 6.7)．
//!
//! 設計書 6.7 は «縮約前後のタイル数と削減率を必ず報告する» ・«反転を有効にした
//! 場合，陰影を持つ素材では lint ルール 7 (blocking) で検出する» の 2 つを定める．
//! **どちらもここで満たす** — `px-core` は切って束ねるだけで，判定は `px-lint` を
//! 呼べるこちらが持つ (`px direction` と同じ形) ．
//!
//! > [!warning] **ルール 7 が掛かるのはタイルが十分大きいときだけである．**
//! > 4 近傍がすべて不透明な画素でしか勾配を測れないので，`shading_min_pixels`
//! > (既定 64) に届かないタイルは «測れない» を返す — **8x8 は上限が
//! > $6 \times 6 = 36$ 画素なので構造的に 1 枚も検査できない**．
//! > 検査できなかった枚数を必ず併記する (D92 の作法) ．

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use px_core::autotile::{
    ImportLayout, Piece, Quadrant, build, import_quadrants, resolve, seam_doubled,
};
use px_core::canvas::IndexedCanvas;
use px_core::frame::{Frame, Layer, LayerMeta, Surface};
use px_core::math::uvec2;
use px_core::palette::Palette;
use px_core::ramp::LightSource;
use px_core::tilejson::TilesetDoc;
use px_core::tileset::{DedupeMode, ExtractOptions, extract, mirror_reliant_cells};
use px_io::l0::L0Document;

use crate::color_cmds::load_indexed;
use crate::shape_cmds::parse_light;

#[derive(Subcommand)]
pub enum TilesetCommand {
    /// 絵をタイルへ切り，同値なものを束ねる (設計書 6.7)
    Extract {
        #[command(flatten)]
        args: ExtractArgs,
    },
    /// 象限から 47 枚の autotile を組む (設計書 6.8 ・4.3)
    Autotile {
        #[command(flatten)]
        args: AutotileArgs,
    },
    /// 既存のシートから象限を取り出して L0 を書く (設計書 6.8 のインポータ)
    Import {
        #[command(flatten)]
        args: ImportArgs,
    },
}

#[derive(Args, Clone)]
pub struct ImportArgs {
    /// タイルを 1 枚 1 フレームで持つ `.aseprite`
    /// (`px tileset extract` や `px tileset autotile` の出力)
    pub input: PathBuf,
    /// 書き出す L0 (`.px.toml`)
    pub output: PathBuf,
    /// **並びを明示すること** — `quadrants-5` / `quadrants-20` / `blob-47`．
    ///
    /// 設計書 6.8 は «レイアウトの明示を必須とする» と定める．
    /// **自動推測はしない** — 外れると 47 枚すべてが静かに壊れる
    #[arg(long = "from-template", required = true)]
    pub template: String,
    /// タイルの一辺 (象限はその半分)
    #[arg(long, default_value_t = 16)]
    pub tile: u32,
    /// L0 が参照するパレット (`.hex`)．無ければ作る
    #[arg(long)]
    pub palette: PathBuf,
    /// L0 の `[meta] name`
    #[arg(long, default_value = "imported")]
    pub name: String,
}

#[derive(Args, Clone)]
pub struct AutotileArgs {
    /// 象限の絵を並べた L0 (`kind = "autotile_quadrants"`)．
    ///
    /// フレーム名は `corner_convex` / `edge_h` / `edge_v` / `inner` /
    /// `corner_concave` の 5 種．`quadrant` を書けば象限ごとに描き分けられる
    /// (**全象限を明示するか全部省略するかの二択**．設計書 4.3)
    pub input: PathBuf,
    /// 47 枚を並べた `.aseprite` の出力先
    pub output: PathBuf,
    /// タイルの一辺．**象限はその半分になる**．省略時は L0 の `tile_size`
    #[arg(long)]
    pub tile: Option<u32>,
    /// bitmask との対応表を JSON で書き出す先 (設計書 4.4 の «正規 JSON»)
    #[arg(long)]
    pub map: Option<PathBuf>,
    /// 宣言する光源 (`dir:X,Y`)．**自動ミラーを使うなら宣言すること** (設計書 4.3)
    #[arg(long)]
    pub light: Option<String>,
    /// ルール 7 が鳴っても非ゼロで終わらない
    #[arg(long)]
    pub allow_inconsistent: bool,
}

#[derive(Args, Clone)]
pub struct ExtractArgs {
    /// 切る絵．**PNG は受け取らない** — その場で添字化すると
    /// «こちらが選んだ量子化» をタイルの同値判定へ持ち込むことになる (D92 と同じ理由)
    pub input: PathBuf,
    /// タイルを並べた `.aseprite` の出力先
    pub output: PathBuf,
    /// タイルの一辺．**絵の寸法がこの倍数でなければ誤りとする** (黙って切らない)
    #[arg(long, default_value_t = 16)]
    pub tile: u32,
    /// 同値とみなす範囲．**既定は完全一致のみ** (設計書 6.7)
    #[arg(long, default_value = "exact")]
    pub dedupe: String,
    /// 格子の対応表を JSON で書き出す先
    #[arg(long)]
    pub map: Option<PathBuf>,
    /// 宣言する光源 (`dir:X,Y`)．**無ければルール 7 は掛からない** (D89)
    #[arg(long)]
    pub light: Option<String>,
    /// ルール 7 が鳴っても非ゼロで終わらない
    #[arg(long)]
    pub allow_inconsistent: bool,
}

pub fn tileset(command: TilesetCommand) -> Result<()> {
    match command {
        TilesetCommand::Extract { args } => extract_cmd(&args),
        TilesetCommand::Autotile { args } => autotile_cmd(&args),
        TilesetCommand::Import { args } => import_cmd(&args),
    }
}

/// シートから象限を取り出して L0 を書く．
fn import_cmd(args: &ImportArgs) -> Result<()> {
    let layout = ImportLayout::parse(&args.template).with_context(|| {
        format!(
            "--from-template は quadrants-5 / quadrants-20 / blob-47 ('{}')",
            args.template
        )
    })?;

    let frames = crate::load_frames(&args.input)?;
    let palette = frames
        .first()
        .context("フレームが 1 つも無い")?
        .palette
        .clone();
    let pieces: Vec<px_core::canvas::IndexedCanvas> = frames
        .iter()
        .map(|f| {
            f.layers
                .iter()
                .find_map(|l| l.surface.as_indexed())
                .cloned()
                .context("インデックスカラーのレイヤが無い")
        })
        .collect::<Result<_>>()?;

    let (art, report) = import_quadrants(&pieces, layout, args.tile)?;
    println!(
        "{} — 並び '{}' から象限 {} 通りを取り出した",
        args.input.display(),
        layout.as_str(),
        report.pieces
    );
    if report.mirrored {
        println!(
            "  自動ミラーで 4 象限へ広げた\n\
             (**陰影やディザを持つ素材には使えない** — 光源が裏返り (設計書 4.3)，\n\
             ディザの位相が壊れる (D105)．その場合は quadrants-20 で 20 枚渡すこと)"
        );
    }
    if report.cross_checked > 0 {
        println!(
            "  同じ象限 ・状態が {} 組で重なり，すべて一致した (推測ではなく突き合わせ)",
            report.cross_checked
        );
    }

    // パレットを先に確定させる
    if !args.palette.exists() {
        px_io::hex::write(&args.palette, &palette)
            .with_context(|| format!("{} を書き出せない", args.palette.display()))?;
        println!("  パレットを作成: {}", args.palette.display());
    }

    let doc = build_l0(&art, &palette, args, layout)?;
    doc.write(&args.output)
        .with_context(|| format!("{} を書き出せない", args.output.display()))?;
    println!("  {} へ書き出した", args.output.display());
    Ok(())
}

/// 象限を L0 の `autotile_quadrants` にする．
///
/// **`quadrants-5` は象限を書かない** (全省略 = 自動ミラー．設計書 4.3 段 2) ．
/// それ以外は**全象限を明示する** (段 1) — 段 3 の混在は作らない．
fn build_l0(
    art: &px_core::autotile::QuadrantArt,
    palette: &Palette,
    args: &ImportArgs,
    layout: ImportLayout,
) -> Result<L0Document> {
    // 使っている添字を集めて文字を割り当てる
    let mut used: Vec<u8> = art
        .values()
        .flat_map(|c| c.pixels().iter().copied())
        .collect();
    used.sort_unstable();
    used.dedup();
    if used.len() > px_io::l0::COLOR_KEYS.len() {
        bail!(
            "{} 色を使っているが L0 の上限は {} 色である",
            used.len(),
            px_io::l0::COLOR_KEYS.len()
        );
    }
    // **キャンバスの «透明添字» をそのまま信じない．**
    // `.aseprite` の透明添字は «透明として扱う添字» の宣言であって，パレットの
    // その色が本当にアルファ 0 とは限らない — 添字 0 が実色 (`2b2b3f`) の素材で
    // 往復すると **12006 画素が変わった**．パレットのアルファで決める
    let transparent = art
        .values()
        .next()
        .and_then(|c| c.transparent())
        .filter(|i| palette.get(*i).is_some_and(|c| c.a == 0));

    let mut key_of: BTreeMap<u8, char> = BTreeMap::new();
    let mut map: BTreeMap<String, px_io::l0::RawColorKey> = BTreeMap::new();
    let mut keys = px_io::l0::COLOR_KEYS.chars();
    for i in &used {
        if transparent == Some(*i) {
            key_of.insert(*i, '.');
            map.insert(
                ".".to_string(),
                px_io::l0::RawColorKey::Name("transparent".into()),
            );
            continue;
        }
        let c = keys.next().context("色キーが足りない")?;
        key_of.insert(*i, c);
        map.insert(c.to_string(), px_io::l0::RawColorKey::Index(*i));
    }

    let mirrored = layout == ImportLayout::Quadrants5;
    let mut frames: Vec<px_io::l0::L0Frame> = Vec::new();
    for state in px_core::autotile::STATES {
        if mirrored {
            // 全省略 — 自動ミラーに任せる (段 2)
            let c = &art[&(Quadrant::NW, state)];
            frames.push(l0_frame(state.as_str(), None, c, &key_of));
        } else {
            for q in px_core::autotile::QUADRANTS {
                let c = &art[&(q, state)];
                frames.push(l0_frame(state.as_str(), Some(q), c, &key_of));
            }
        }
    }

    let relative = args
        .palette
        .file_name()
        .map(PathBuf::from)
        .unwrap_or_else(|| args.palette.clone());
    Ok(L0Document {
        meta: px_io::l0::L0Meta {
            format: 1,
            name: args.name.clone(),
            kind: Some("autotile_quadrants".to_string()),
            tile_size: Some(args.tile),
            ..px_io::l0::L0Meta::default()
        },
        palette: px_io::l0::L0PaletteSpec {
            reference: relative,
            map,
        },
        frames,
    })
}

fn l0_frame(
    name: &str,
    quadrant: Option<Quadrant>,
    canvas: &px_core::canvas::IndexedCanvas,
    key_of: &BTreeMap<u8, char>,
) -> px_io::l0::L0Frame {
    let mut data = String::new();
    for y in 0..canvas.height() as i32 {
        for x in 0..canvas.width() as i32 {
            let i = canvas.get(x, y).unwrap_or(0);
            data.push(key_of.get(&i).copied().unwrap_or('.'));
        }
        data.push('\n');
    }
    px_io::l0::L0Frame {
        name: name.to_string(),
        kind: "key".to_string(),
        duration_ms: 100,
        quadrant: quadrant.map(|q| match q {
            Quadrant::NW => px_io::l0::Quadrant::NW,
            Quadrant::NE => px_io::l0::Quadrant::NE,
            Quadrant::SW => px_io::l0::Quadrant::SW,
            Quadrant::SE => px_io::l0::Quadrant::SE,
        }),
        data,
    }
}

/// L0 の `autotile_quadrants` を読んで 47 枚を組む．
fn autotile_cmd(args: &AutotileArgs) -> Result<()> {
    let doc = L0Document::read(&args.input)
        .with_context(|| format!("{} を L0 として読めない", args.input.display()))?;
    if doc.meta.kind.as_deref() != Some("autotile_quadrants") {
        bail!(
            "{} の [meta] kind が \"autotile_quadrants\" ではない (象限として読まない)",
            args.input.display()
        );
    }
    let tile = args
        .tile
        .or(doc.meta.tile_size)
        .context("タイルの一辺が分からない ([meta] tile_size か --tile で与えること)")?;

    let frames = doc
        .to_frames(&args.input)
        .with_context(|| format!("{} を作業層へ変換できない", args.input.display()))?;
    let palette = frames
        .first()
        .context("フレームが 1 つも無い")?
        .palette
        .clone();

    let pieces: Vec<Piece> = doc
        .frames
        .iter()
        .zip(&frames)
        .map(|(raw, frame)| {
            let art = frame
                .layers
                .first()
                .and_then(|l| l.surface.as_indexed())
                .context("インデックスカラーのレイヤが無い")?
                .clone();
            Ok(Piece {
                name: raw.name.clone(),
                quadrant: raw.quadrant.map(|q| match q {
                    px_io::l0::Quadrant::NW => Quadrant::NW,
                    px_io::l0::Quadrant::NE => Quadrant::NE,
                    px_io::l0::Quadrant::SW => Quadrant::SW,
                    px_io::l0::Quadrant::SE => Quadrant::SE,
                }),
                art,
            })
        })
        .collect::<Result<_>>()?;

    if let Some(phases) = doc.meta.dither_phase
        && phases > 1
    {
        eprintln!(
            "注意: [meta] dither_phase = {phases} は使わない．\n\
             設計書 4.3 の «偶数幅だと同一タイルの反復でディザが連結する» は測ると逆で，\n\
             偶数幅の反復は継ぎ目が合う (連結するのは奇数幅と «鏡像を隣に置いたとき») ．\n\
             位相バリアントを交互に置くと合っている継ぎ目を壊す (同色の隣接 0 → 16)．\n\
             代わりに象限の継ぎ目を検査する (D105 ・D106)"
        );
    }

    let (art, used_mirror) = resolve(&pieces)?;
    let (tiles, count) = build(&art, tile)?;
    println!(
        "{} — {} 画素のタイルを {} 枚 (象限は {} 画素)",
        args.input.display(),
        tile,
        count,
        tile / 2
    );
    println!(
        "  象限の絵 {} 枚{}",
        pieces.len(),
        if used_mirror {
            " (**自動ミラーで 4 象限へ広げた**)"
        } else {
            " (全象限を明示している．自動ミラーは使っていない)"
        }
    );

    // 設計書 4.3 — **自動ミラーで生成したタイルにルール 7 を blocking で適用する**．
    // **組んだタイルに掛ける** — 象限はタイルの半分なので，16 画素のタイルの象限は
    // 8x8 になり，ルール 7 は勾配を測れる画素が上限 36 で下限 64 に届かない (D100)
    let light: Option<LightSource> = match &args.light {
        Some(spec) => Some(parse_light(spec)?),
        None => None,
    };
    let cfg = px_lint::LintConfig {
        light,
        ..px_lint::LintConfig::default()
    };
    let (mut measurable, mut fired, mut too_small) = (0usize, 0usize, 0usize);
    if used_mirror && let Some(declared) = light {
        for (mask, canvas) in &tiles {
            // **«鳴らない» と «測れない» を分け，«測れない» の理由も分ける** (D77)
            let Some(a) = px_lint::rules::shading_agreement(canvas, &palette, declared) else {
                if px_lint::rules::shading_sample_count(canvas, &palette)
                    < cfg.shading_min_pixels as usize
                {
                    too_small += 1;
                }
                continue;
            };
            measurable += 1;
            if a < cfg.min_shading_agreement {
                fired += 1;
                println!("    mask {mask:#04x} の陰影が光源と合っていない (一致度 {a:.2})");
            }
        }
    }

    // **検査していないことを黙らない** (D92 の作法)
    if used_mirror {
        match light {
            None => println!(
                "  検査していない: 自動ミラーを使ったが光源を宣言していない\n\
                 (陰影を持つ素材なら --light で宣言すること．設計書 4.3)"
            ),
            Some(_) if measurable < tiles.len() => println!(
                "  検査していない: {} 枚は勾配を測れなかった (標本不足 {} / **対称** {})\n\
                 (象限を鏡像で組むので対称なタイルができる — 左右 ・上下 ・180 度回転のどれか．\n\
                 対称な絵は勾配が打ち消し合うので «向きが無い» — 見逃しではない)",
                tiles.len() - measurable,
                too_small,
                tiles.len() - measurable - too_small
            ),
            Some(_) => println!("  ルール 7 を掛けた ({measurable} 枚)"),
        }
    }

    // **ディザの位相** — 自動ミラーは継ぎ目の列を複製するので，1 画素の縞が
    // 2 画素になる (D105) ．設計書 4.3 の «偶数幅だと反復で連結する» は測ると
    // 逆だったので，**位相バリアントの交互配置は採らない** (D106) ．
    // タイル間ではなくタイルの内側の問題なので，2 種類作っても直らない
    let damaged: Vec<(u8, usize)> = tiles
        .iter()
        .map(|(m, t)| (*m, seam_doubled(t)))
        .filter(|(_, n)| *n > 0)
        .collect();
    if !damaged.is_empty() {
        let total: usize = damaged.iter().map(|(_, n)| n).sum();
        println!(
            "  **象限の継ぎ目でディザの位相が崩れている** — {} 枚 ・{} 箇所\n\
             (反転は継ぎ目の列を複製するので 1 画素の縞が 2 画素になる．\n\
             位相をずらした 2 バリアントを交互に置いても直らない — 問題はタイルの内側にある．\n\
             ディザを持つ素材では 4 象限を描くこと)",
            damaged.len(),
            total
        );
    }

    let out_frames: Vec<Frame> = tiles
        .iter()
        .map(|(_, t)| Frame {
            size: uvec2(t.width(), t.height()),
            layers: vec![Layer::new(
                LayerMeta::named("tile"),
                Surface::Indexed(t.clone()),
            )],
            palette: palette.clone(),
            duration_ms: 100,
            kind: px_core::frame::FrameKind::Key,
        })
        .collect();
    crate::save_frames(&args.output, &out_frames, "tiles")?;
    println!(
        "  {} へ {} 枚を書き出した",
        args.output.display(),
        tiles.len()
    );

    if let Some(path) = &args.map {
        // **正規出力は 1 つである** (設計書 4.4 «bitmask → tile index + flip flags»)．
        // 組んだ 47 枚はそれぞれ別のタイルなので，反転の旗は立たない
        let doc = TilesetDoc::new(tile, tiles.len()).with_terrain(tiles.iter().enumerate().map(
            |(i, (mask, _))| {
                (
                    *mask,
                    px_core::frame::TileRef {
                        id: i as u32,
                        ..px_core::frame::TileRef::default()
                    },
                )
            },
        ));
        std::fs::write(path, doc.to_json()?)
            .with_context(|| format!("{} を書き出せない", path.display()))?;
        println!("  {} へ正規 JSON を書き出した", path.display());
    }

    if fired > 0 && !args.allow_inconsistent {
        bail!(
            "自動ミラーで組んだ {fired} 枚が宣言した光源と矛盾している．\
             象限を全象限ぶん描くか，陰影を持たない素材にすること\
             (承知の上なら --allow-inconsistent)"
        );
    }
    Ok(())
}

fn extract_cmd(args: &ExtractArgs) -> Result<()> {
    let mode = DedupeMode::parse(&args.dedupe)
        .with_context(|| format!("--dedupe は exact / flip / flip-rotate ('{}')", args.dedupe))?;
    let light: Option<LightSource> = match &args.light {
        Some(spec) => Some(parse_light(spec)?),
        None => None,
    };

    let (canvas, palette) = load_indexed(&args.input)?;
    let opts = ExtractOptions {
        tile: args.tile,
        mode,
    };
    let (tiles, grid, report) = extract(&canvas, &opts)?;

    // 設計書 6.7 — **縮約前後のタイル数と削減率を必ず報告する**
    println!(
        "{} — {} 画素のタイル ・{}\n  縮約前 {} 枚 → 縮約後 {} 枚 (削減率 {:.1}%)",
        args.input.display(),
        args.tile,
        mode.as_str(),
        report.before,
        report.after,
        report.reduction() * 100.0
    );

    // 設計書 6.7 — **反転を有効にしたら陰影の矛盾をルール 7 で検出する**
    let cells = mirror_reliant_cells(&grid);
    if !cells.is_empty() {
        println!("  反転に頼って再現している升 {} 個", cells.len());
    }

    let cfg = px_lint::LintConfig {
        light,
        ..px_lint::LintConfig::default()
    };
    let (mut measurable, mut fired, mut too_small) = (0usize, 0usize, 0usize);
    if let Some(declared) = light {
        for (tx, ty) in &cells {
            let t = tile_at(&canvas, *tx, *ty, args.tile);
            // **«鳴らない» と «測れない» を分け，«測れない» の理由も分ける** (D77)
            let Some(a) = px_lint::rules::shading_agreement(&t, &palette, declared) else {
                if px_lint::rules::shading_sample_count(&t, &palette)
                    < cfg.shading_min_pixels as usize
                {
                    too_small += 1;
                }
                continue;
            };
            measurable += 1;
            if a < cfg.min_shading_agreement {
                fired += 1;
                println!("    ({tx},{ty}) 陰影が光源と合っていない (一致度 {a:.2})");
            }
        }
    }

    // **検査していないことを黙らない** (D92 の作法)
    let unmeasured = cells.len() - measurable;
    match light {
        None if !cells.is_empty() => println!(
            "  検査していない: 光源を宣言していないのでルール 7 を掛けていない (--light で宣言する)"
        ),
        Some(_) if unmeasured > 0 => println!(
            "  検査していない: {unmeasured} 個は勾配を測れなかった (小さい {} / 対称 {})\n\
             (標本不足は 4 近傍が揃う画素が {} 未満のもの — 8x8 は上限 36 画素で構造的に届かない．\n\
             対称な絵は勾配が打ち消し合うので «向きが無い» — 見逃しではない)",
            too_small,
            unmeasured - too_small,
            cfg.shading_min_pixels
        ),
        _ => {}
    }

    // タイルを 1 枚 1 フレームで書き出す (**走査順で最初に現れた順**．決定論的)
    let frames: Vec<Frame> = tiles
        .iter()
        .map(|t| Frame {
            size: uvec2(t.width(), t.height()),
            layers: vec![Layer::new(
                LayerMeta::named("tile"),
                Surface::Indexed(t.clone()),
            )],
            palette: palette.clone(),
            duration_ms: 100,
            kind: px_core::frame::FrameKind::Key,
        })
        .collect();
    crate::save_frames(&args.output, &frames, "tiles")?;
    println!(
        "  {} へ {} 枚を書き出した",
        args.output.display(),
        tiles.len()
    );

    if let Some(path) = &args.map {
        // **正規出力は 1 つである** (設計書 4.4)．autotile と同じ型で書く
        let doc = TilesetDoc::new(args.tile, tiles.len()).with_map(&grid);
        std::fs::write(path, doc.to_json()?)
            .with_context(|| format!("{} を書き出せない", path.display()))?;
        println!("  {} へ正規 JSON を書き出した", path.display());
    }

    if fired > 0 && !args.allow_inconsistent {
        bail!(
            "反転で再現している升のうち {fired} 個が宣言した光源と矛盾している．\
             --dedupe exact にするか，その升を手で描くこと (承知の上なら --allow-inconsistent)"
        );
    }
    Ok(())
}

fn tile_at(canvas: &IndexedCanvas, tx: u32, ty: u32, n: u32) -> IndexedCanvas {
    let mut pixels = Vec::with_capacity((n * n) as usize);
    for y in 0..n {
        for x in 0..n {
            pixels.push(
                canvas
                    .get((tx * n + x) as i32, (ty * n + y) as i32)
                    .unwrap_or(0),
            );
        }
    }
    IndexedCanvas::from_pixels(n, n, pixels)
        .expect("画素数が合う")
        .with_transparent(canvas.transparent())
}

#[cfg(test)]
mod tests {
    use super::*;
    use px_core::autotile::{CornerState, QUADRANTS, STATES};
    use px_core::canvas::IndexedCanvas;
    use px_core::color::Rgba8;

    /// **壊れると: 添字 0 が実色の素材で，L0 へ書くとき «透明» と宣言してしまう．**
    ///
    /// `.aseprite` の透明添字は «透明として扱う添字» の宣言であって，パレットの
    /// その色が本当にアルファ 0 とは限らない．信じたまま書いたら
    /// **L0 往復で 12006 画素が変わった** — 単体試験では出ず，
    /// `autotile` → `import` → `autotile` と CLI で通して初めて分かった (D81 の教訓)．
    #[test]
    fn an_opaque_index_zero_is_not_written_as_transparent() {
        // 添字 0 が不透明な色のパレット
        let palette = Palette::new(vec![
            Rgba8::rgb(0x2b, 0x2b, 0x3f),
            Rgba8::rgb(0x6f, 0x8f, 0x5f),
        ])
        .expect("2 色");

        let mut art = px_core::autotile::QuadrantArt::new();
        for q in QUADRANTS {
            for state in STATES {
                // **透明添字として 0 を宣言してある** — ここが罠である
                let c = IndexedCanvas::filled(4, 4, 0).with_transparent(Some(0));
                art.insert((q, state), c);
            }
        }

        let args = ImportArgs {
            input: PathBuf::from("in.aseprite"),
            output: PathBuf::from("out.px.toml"),
            template: "quadrants-20".to_string(),
            tile: 8,
            palette: PathBuf::from("p.hex"),
            name: "t".to_string(),
        };
        let doc = build_l0(&art, &palette, &args, ImportLayout::Quadrants20).expect("書ける");

        // 添字 0 は不透明なので «transparent» にしてはいけない
        for (key, value) in &doc.palette.map {
            if let px_io::l0::RawColorKey::Name(name) = value {
                panic!("キー '{key}' が '{name}' として書かれている (0 は不透明である)");
            }
        }
        assert!(!doc.palette.map.contains_key("."));
    }

    /// **壊れると: 本物の透明が «色» として書かれ，L0 が背景を塗り潰す．**
    #[test]
    fn a_genuinely_transparent_index_is_written_as_transparent() {
        let palette =
            Palette::new(vec![Rgba8::TRANSPARENT, Rgba8::rgb(0x6f, 0x8f, 0x5f)]).expect("2 色");
        let mut art = px_core::autotile::QuadrantArt::new();
        for q in QUADRANTS {
            for state in STATES {
                let mut c = IndexedCanvas::filled(4, 4, 1).with_transparent(Some(0));
                c.set(0, 0, 0);
                art.insert((q, state), c);
            }
        }
        let args = ImportArgs {
            input: PathBuf::from("in.aseprite"),
            output: PathBuf::from("out.px.toml"),
            template: "quadrants-20".to_string(),
            tile: 8,
            palette: PathBuf::from("p.hex"),
            name: "t".to_string(),
        };
        let doc = build_l0(&art, &palette, &args, ImportLayout::Quadrants20).expect("書ける");
        assert!(matches!(
            doc.palette.map.get("."),
            Some(px_io::l0::RawColorKey::Name(n)) if n == "transparent"
        ));
    }

    /// **壊れると: `quadrants-5` が象限を書いてしまい，設計書 4.3 段 2
    /// (全省略 = 自動ミラー) にならない．**
    #[test]
    fn the_five_piece_layout_writes_no_quadrant_at_all() {
        let palette = Palette::new(vec![Rgba8::rgb(1, 2, 3)]).expect("1 色");
        let mut art = px_core::autotile::QuadrantArt::new();
        for q in QUADRANTS {
            for state in STATES {
                art.insert((q, state), IndexedCanvas::filled(4, 4, 0));
            }
        }
        let args = ImportArgs {
            input: PathBuf::from("in.aseprite"),
            output: PathBuf::from("out.px.toml"),
            template: "quadrants-5".to_string(),
            tile: 8,
            palette: PathBuf::from("p.hex"),
            name: "t".to_string(),
        };
        let doc = build_l0(&art, &palette, &args, ImportLayout::Quadrants5).expect("書ける");
        assert_eq!(doc.frames.len(), 5);
        assert!(
            doc.frames.iter().all(|f| f.quadrant.is_none()),
            "全省略でないと段 2 (自動ミラー) にならない"
        );

        // 20 枚の並びは全象限を明示する (段 1)
        let doc = build_l0(&art, &palette, &args, ImportLayout::Quadrants20).expect("書ける");
        assert_eq!(doc.frames.len(), 20);
        assert!(doc.frames.iter().all(|f| f.quadrant.is_some()));
        assert_eq!(
            doc.frames[0].name,
            CornerState::Convex.as_str(),
            "状態の名前で書く"
        );
    }
}
