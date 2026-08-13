//! **タイル分割と同値判定を実素材で測る** (`px tileset extract`．設計書 6.7)．
//!
//! 測るのは 2 つで，**性質が違う**．
//!
//! | 測るもの | 判断の性質 |
//! | --- | --- |
//! | 削減率 | **入力で決まる．報告する量であって校正しない** (D92 と同じ側) |
//! | **ルール 7 が掛かるタイルの割合** | **タイルの大きさで決まる．これも数え上げ** |
//!
//! 2 つ目が要点である．ルール 7 は 4 近傍がすべて不透明な画素でしか勾配を測れず，
//! `shading_min_pixels` (既定 64) に届かないタイルは «測れない» を返す．
//! **8x8 のタイルは上限が $6 \times 6 = 36$ 画素なので構造的に 1 枚も検査できない．**
//! 設計書 6.7 の «反転を有効にしたらルール 7 で検出する» は，**タイルが十分大きい
//! ときにだけ成り立つ**．

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use px_core::canvas::IndexedCanvas;
use px_core::palette::Palette;
use px_core::ramp::LightSource;
use px_core::tileset::{DedupeMode, ExtractOptions, extract, mirror_reliant_cells, rebuild};

#[derive(Clone, Debug)]
pub struct Record {
    pub file: String,
    pub tile: u32,
    pub mode: &'static str,
    pub before: usize,
    pub after: usize,
    /// 恒等でない向きで置かれた升の数．**«束ねた枚数» ではない**．
    pub oriented: usize,
    /// **反転に頼って別々の升を再現している升の数** (ルール 7 の相手)．
    pub mirrored: usize,
    /// そのうち勾配を測れた枚数．
    pub measurable: usize,
    /// そのうちルール 7 が鳴った枚数．
    pub fired: usize,
    /// 組み直して元の絵に戻ったか．
    pub rebuilt: bool,
}

pub const HEADER: &str =
    "file,tile,mode,before,after,reduction,oriented,mirrored,measurable,fired,rebuilt";

pub fn to_csv(r: &Record) -> String {
    let reduction = if r.before == 0 {
        0.0
    } else {
        1.0 - r.after as f32 / r.before as f32
    };
    format!(
        "{},{},{},{},{},{:.4},{},{},{},{},{}",
        r.file,
        r.tile,
        r.mode,
        r.before,
        r.after,
        reduction,
        r.oriented,
        r.mirrored,
        r.measurable,
        r.fired,
        r.rebuilt
    )
}

fn png_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("{} を読めない", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")))
        .collect();
    files.sort();
    Ok(files)
}

/// タイル 1 枚を切り出す．
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

/// 1 枚 x 1 タイル寸法 x 1 モードを測る．
fn measure(
    file: &str,
    canvas: &IndexedCanvas,
    palette: &Palette,
    tile: u32,
    mode: DedupeMode,
    light: LightSource,
    cfg: &px_lint::LintConfig,
) -> Option<Record> {
    if !canvas.width().is_multiple_of(tile) || !canvas.height().is_multiple_of(tile) {
        return None;
    }
    let opts = ExtractOptions { tile, mode };
    let (tiles, grid, report) = extract(canvas, &opts).ok()?;

    let back = rebuild(&tiles, &grid, tile, canvas.transparent()).ok()?;
    let rebuilt = back.pixels() == canvas.pixels();

    // **反転で束ねたタイルにだけルール 7 を掛ける** (設計書 6.7)
    let cells = mirror_reliant_cells(&grid);
    let (mut measurable, mut fired) = (0usize, 0usize);
    for (tx, ty) in &cells {
        let t = tile_at(canvas, *tx, *ty, tile);
        // **«鳴らない» と «測れない» を分ける** (D77)
        match px_lint::rules::shading_agreement(&t, palette, light) {
            None => {}
            Some(a) => {
                measurable += 1;
                if a < cfg.min_shading_agreement {
                    fired += 1;
                }
            }
        }
    }

    Some(Record {
        file: file.to_string(),
        tile,
        mode: mode.as_str(),
        before: report.before,
        after: report.after,
        oriented: report.oriented,
        mirrored: cells.len(),
        measurable,
        fired,
        rebuilt,
    })
}

pub const MODES: [DedupeMode; 3] = [DedupeMode::Exact, DedupeMode::Flip, DedupeMode::FlipRotate];

pub fn run(dir: &Path, tiles: &[u32], light: LightSource) -> Result<Vec<Record>> {
    let cfg = px_lint::LintConfig::default();
    let mut out = Vec::new();
    for path in png_files(dir)? {
        let Ok(img) = px_io::png::read_rgba(&path) else {
            continue;
        };
        let Ok((canvas, palette)) = crate::lintcal::index_exactly(&img) else {
            continue;
        };
        let file = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        for &tile in tiles {
            for mode in MODES {
                if let Some(r) = measure(&file, &canvas, &palette, tile, mode, light, &cfg) {
                    out.push(r);
                }
            }
        }
    }
    Ok(out)
}

/// タイル寸法 x モードごとにまとめる．
///
/// 返すのは (タイル寸法，モード，絵の枚数，縮約前，縮約後，削減率，反転で束ねた枚数，
/// 測れた枚数，鳴った枚数，組み直しに失敗した絵の数)．
#[allow(clippy::type_complexity)]
pub fn summarise(
    records: &[Record],
) -> Vec<(
    u32,
    &'static str,
    usize,
    usize,
    usize,
    f32,
    usize,
    usize,
    usize,
    usize,
)> {
    let mut keys: Vec<(u32, &'static str)> = records.iter().map(|r| (r.tile, r.mode)).collect();
    keys.sort_unstable();
    keys.dedup();
    keys.into_iter()
        .map(|(tile, mode)| {
            let g: Vec<&Record> = records
                .iter()
                .filter(|r| r.tile == tile && r.mode == mode)
                .collect();
            let before: usize = g.iter().map(|r| r.before).sum();
            let after: usize = g.iter().map(|r| r.after).sum();
            let reduction = if before == 0 {
                0.0
            } else {
                1.0 - after as f32 / before as f32
            };
            (
                tile,
                mode,
                g.len(),
                before,
                after,
                reduction,
                g.iter().map(|r| r.mirrored).sum(),
                g.iter().map(|r| r.measurable).sum(),
                g.iter().map(|r| r.fired).sum(),
                g.iter().filter(|r| !r.rebuilt).count(),
            )
        })
        .collect()
}
