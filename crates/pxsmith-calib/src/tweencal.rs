//! **中割りが実用になるかを測る (実装計画書のリスク R11)．**
//!
//! 実装計画書は «SDF 補間が実用にならない → `tween` が無価値» を中リスクに挙げて
//! いる．**「実用になるか」は «真値のある場面» を作らないと測れない**ので，
//! 3 つの口を用意した．
//!
//! | 測るもの | 決まること |
//! | --- | --- |
//! | **平行移動 (真値あり)** | 場をそのまま補間するか，重心を先に取り除くか |
//! | **余白の掃引** | 距離場を測るのに余白が要るのか (要らなかった) |
//! | **別の絵どうし (真値なし)** | トポロジーが変わる割合 ・空になる割合 |
//!
//! 1 つ目が肝である．**平行移動なら真値が分かる** — 12 画素動いた 2 枚の中割りは
//! 6 画素動かした絵でなければならない．真値がある場面を作らずに «それらしい絵が
//! 出た» で判断すると，痩せていることに気付けない．
//!
//! **«動かさない» (元の絵をそのまま出す) を対照に置く．** 中割りがこれに勝てない
//! なら，中割りは無価値である — R11 が言っているのはそういうことである．

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use pxsmith_core::geom::Mask;
use pxsmith_core::math::{IVec2, ivec2};
use pxsmith_core::tween::{TweenAlign, TweenOptions, tween_mask};

/// 真値のある 1 件 (平行移動)．
#[derive(Clone, Debug)]
pub struct Truth {
    pub file: String,
    pub shift: IVec2,
    /// シルエットの «動きに直交する向きの» 差し渡し (痩せの効き方を見るため)．
    pub extent: u32,
    /// 場をそのまま補間したときの真値との IoU．
    pub plain: f32,
    /// 重心を先に取り除いたときの真値との IoU．
    pub centroid: f32,
    /// **対照** — 元の絵をそのまま出したときの真値との IoU．
    pub hold: f32,
}

/// 余白を変えたときに結果がどれだけ動くか．
#[derive(Clone, Debug)]
pub struct MarginRow {
    pub margin: u32,
    /// いちばん大きい余白のときと違う画素の数 (全件合計)．
    pub differing: usize,
    /// 違った絵の枚数．
    pub files: usize,
}

/// 真値の無い 1 件 (別の絵どうし)．
#[derive(Clone, Debug)]
pub struct Pair {
    pub a: String,
    pub b: String,
    pub t: f32,
    pub components: (usize, usize, usize),
    pub holes: (usize, usize, usize),
    pub topology_changed: bool,
    pub area: usize,
    pub empty: bool,
}

pub const TRUTH_HEADER: &str = "file,shift_x,shift_y,extent,plain,centroid,hold";
pub const PAIR_HEADER: &str = "a,b,t,comp_a,comp_b,comp_r,holes_r,topology_changed,area";

pub fn truth_csv(r: &Truth) -> String {
    format!(
        "{},{},{},{},{:.4},{:.4},{:.4}",
        r.file, r.shift.x, r.shift.y, r.extent, r.plain, r.centroid, r.hold
    )
}

pub fn pair_csv(r: &Pair) -> String {
    format!(
        "{},{},{:.2},{},{},{},{},{},{}",
        r.a,
        r.b,
        r.t,
        r.components.0,
        r.components.1,
        r.components.2,
        r.holes.2,
        r.topology_changed,
        r.area
    )
}

/// 既定のずらし量．**動きの大きさを変えて «痩せ» の効き方を見る**．
pub fn default_shifts() -> Vec<IVec2> {
    vec![ivec2(4, 0), ivec2(8, 0), ivec2(6, 6), ivec2(12, -4)]
}

fn name_of(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
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

/// PNG のシルエット (透明でない画素) を読む．
///
/// **余地を持たせて読む** — 平行移動した相手を同じ画布に載せるので，元の寸法の
/// ままだと動かした先が切れて «真値» が作れない．
fn silhouette(path: &Path, pad: u32) -> Result<Mask> {
    let img = pxsmith_io::png::read_rgba(path)?;
    let mut m = Mask::new(img.width() + pad * 2, img.height() + pad * 2);
    for y in 0..img.height() as i32 {
        for x in 0..img.width() as i32 {
            if img.get(x, y).is_some_and(|c| c.a != 0) {
                m.set(ivec2(x + pad as i32, y + pad as i32), true);
            }
        }
    }
    Ok(m)
}

fn shifted(m: &Mask, d: IVec2) -> Mask {
    let mut out = Mask::new(m.width(), m.height());
    for p in m.iter_set() {
        out.set(p + d, true);
    }
    out
}

fn iou(x: &Mask, y: &Mask) -> f32 {
    let inter = x.iter_set().filter(|p| y.get(*p)).count();
    let union = x.count() + y.count() - inter;
    if union == 0 {
        return 1.0;
    }
    inter as f32 / union as f32
}

/// 動きに直交する向きの差し渡し．
fn extent_across(m: &Mask, d: IVec2) -> u32 {
    let Some(bb) = m.bbox() else { return 0 };
    // 動きが横向きなら縦の差し渡し，縦向きなら横の差し渡しを見る
    if d.x.abs() >= d.y.abs() { bb.h } else { bb.w }
}

/// **真値のある場面で測る．** $B$ は $A$ を偶数画素だけ動かしたもので，
/// $t = 1/2$ の真値は «半分だけ動かした $A$» である．
pub fn truths(dir: &Path, shifts: &[IVec2]) -> Result<Vec<Truth>> {
    let mut out = Vec::new();
    for path in png_files(dir)? {
        for d in shifts {
            // 動かした先が切れないだけの余地を取る
            let pad = (d.x.abs().max(d.y.abs()) + 2) as u32;
            let Ok(a) = silhouette(&path, pad) else {
                continue;
            };
            if a.is_empty() {
                continue;
            }
            let b = shifted(&a, *d);
            let truth = shifted(&a, ivec2(d.x / 2, d.y / 2));

            let plain = TweenOptions {
                align: TweenAlign::None,
                ..TweenOptions::default()
            };
            let centroid = TweenOptions {
                align: TweenAlign::Centroid,
                ..TweenOptions::default()
            };
            let (Ok(p), Ok(c)) = (
                tween_mask(&a, &b, 0.5, &plain),
                tween_mask(&a, &b, 0.5, &centroid),
            ) else {
                continue;
            };
            out.push(Truth {
                file: name_of(&path),
                shift: *d,
                extent: extent_across(&a, *d),
                plain: iou(&p.mask, &truth),
                centroid: iou(&c.mask, &truth),
                hold: iou(&a, &truth),
            });
        }
    }
    Ok(out)
}

/// **余白をいくつ取れば結果が動かなくなるかを測る．**
///
/// [`pxsmith_core::geom::signed_distance`] は画像の外を «背景» として扱うので，縁の
/// 近くでは外側の距離が頭打ちになる (D91 が `--ao` で踏んだのと同じ穴) ．
/// いちばん大きい余白を基準にして，そこから何画素ずれるかを数える．
pub fn margin_sweep(
    dir: &Path,
    margins: &[u32],
    reference: u32,
    align: TweenAlign,
    pad: u32,
) -> Result<Vec<MarginRow>> {
    let files = png_files(dir)?;
    let shift = ivec2(8, 0);
    let mut rows: Vec<MarginRow> = margins
        .iter()
        .map(|m| MarginRow {
            margin: *m,
            differing: 0,
            files: 0,
        })
        .collect();

    for path in &files {
        let Ok(a) = silhouette(path, pad) else {
            continue;
        };
        if a.is_empty() {
            continue;
        }
        let b = shifted(&a, shift);
        let opts = |m: u32| TweenOptions { margin: m, align };
        let Ok(base) = tween_mask(&a, &b, 0.5, &opts(reference)) else {
            continue;
        };
        for row in rows.iter_mut() {
            let Ok(got) = tween_mask(&a, &b, 0.5, &opts(row.margin)) else {
                continue;
            };
            let diff = got
                .mask
                .bounds()
                .iter()
                .filter(|p| got.mask.get(*p) != base.mask.get(*p))
                .count();
            row.differing += diff;
            if diff > 0 {
                row.files += 1;
            }
        }
    }
    Ok(rows)
}

/// **真値の無い場面 (別の絵どうし) でトポロジーを数える．**
///
/// 設計書 6.9 は «トポロジー変化は扱えない» と言う．扱えないことを黙るのでは
/// なく，**どれくらいの割合で起きるかを数える**．
pub fn pairs(dir: &Path, names: &[String], ts: &[f32]) -> Result<Vec<Pair>> {
    let mut out = Vec::new();
    let opts = TweenOptions::default();
    for i in 0..names.len() {
        for j in (i + 1)..names.len() {
            let (pa, pb) = (
                dir.join(format!("{}.png", names[i])),
                dir.join(format!("{}.png", names[j])),
            );
            let (Ok(a), Ok(b)) = (silhouette(&pa, 2), silhouette(&pb, 2)) else {
                continue;
            };
            if a.is_empty() || b.is_empty() {
                continue;
            }
            for t in ts {
                let Ok(r) = tween_mask(&a, &b, *t, &opts) else {
                    continue;
                };
                out.push(Pair {
                    a: names[i].clone(),
                    b: names[j].clone(),
                    t: *t,
                    components: r.components,
                    holes: r.holes,
                    topology_changed: r.topology_changed(),
                    area: r.mask.count(),
                    empty: r.mask.is_empty(),
                });
            }
        }
    }
    Ok(out)
}

fn median(mut v: Vec<f32>) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f32::total_cmp);
    v[v.len() / 2]
}

/// ずらし量ごとに «場のまま ・重心を取り除く ・動かさない» の IoU 中央値を並べる．
pub fn summarise_truths(rows: &[Truth]) -> Vec<(IVec2, usize, f32, f32, f32, usize)> {
    let mut keys: Vec<(i32, i32)> = rows.iter().map(|r| (r.shift.x, r.shift.y)).collect();
    keys.sort_unstable();
    keys.dedup();
    keys.into_iter()
        .map(|(x, y)| {
            let g: Vec<&Truth> = rows
                .iter()
                .filter(|r| r.shift.x == x && r.shift.y == y)
                .collect();
            let better = g.iter().filter(|r| r.centroid > r.plain).count();
            (
                ivec2(x, y),
                g.len(),
                median(g.iter().map(|r| r.plain).collect()),
                median(g.iter().map(|r| r.centroid).collect()),
                median(g.iter().map(|r| r.hold).collect()),
                better,
            )
        })
        .collect()
}

/// «動かさない» に勝てた件数 (件数，場のまま，重心)．
pub fn beats_hold(rows: &[Truth]) -> (usize, usize, usize) {
    (
        rows.len(),
        rows.iter().filter(|r| r.plain > r.hold).count(),
        rows.iter().filter(|r| r.centroid > r.hold).count(),
    )
}

/// トポロジーの結果をまとめる (件数 ・変わった ・空になった ・成分が増えた)．
pub fn summarise_pairs(rows: &[Pair]) -> (usize, usize, usize, usize) {
    (
        rows.len(),
        rows.iter().filter(|r| r.topology_changed).count(),
        rows.iter().filter(|r| r.empty).count(),
        rows.iter()
            .filter(|r| r.components.2 > r.components.0.max(r.components.1))
            .count(),
    )
}

/// 既定の «別の絵» — Dungeon Crawl の生き物 (CC0)．実際に中割りしたくなる素材である．
pub const DEFAULT_PAIRS: &[&str] = &[
    "crawl_goblin",
    "crawl_deformed_orc",
    "crawl_saint_roka",
    "crawl_urand_fencer",
    "crawl_salamander",
    "crawl_elephant",
];
