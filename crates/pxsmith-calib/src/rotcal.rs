//! **`pxsmith rotate` / `pxsmith scale` の品質を測る** (`pxsmith-calib rotate`)．
//!
//! 実装計画書 M7 の完了条件は «品質が出なければ `--algo` から外す» なので，
//! **測れないと判定できない**．そこで真値のある場面と対照を作る．
//!
//! | 場面 | 真値 | 対照 |
//! | --- | --- | --- |
//! | $\theta$ 回して $-\theta$ 戻す | **元の絵** | `nearest` |
//! | 90 度の倍数 | **厳密**．流儀に依らず一致するはず | — |
//! | 整数倍の拡大 | **厳密** | — |
//!
//! 往復は «回転が形を保つか» をそのまま測れる — 途中の絵に真値は無いが，
//! **戻ってきた絵には在る** (D114 «真値のある場面を作る» と同じ形) ．
//!
//! あわせて**ジャギーの数** (G1 ・G3) を数える — cleanEdge は «斜めの縁を
//! 作り直す» ものなので，そこが減らないなら移植する値が無い．

use anyhow::Result;
use std::path::Path;

use pxsmith_core::canvas::IndexedCanvas;
use pxsmith_core::geom::jaggy::analyze_canvas;
use pxsmith_core::palette::Palette;
use pxsmith_core::resample::{ResampleAlgo, ResampleOptions, rotate, scale};

use crate::animcal::{indexed, name_of, png_files};

/// 1 件ぶん．
pub struct RotRow {
    pub file: String,
    pub algo: &'static str,
    pub degrees: f32,
    /// 往復して元の絵と一致した画素の割合．
    pub round_trip: f32,
    /// 往復してシルエットが一致した割合．
    pub silhouette: f32,
    /// 1 度だけ回した絵のジャギーの数．
    pub jaggies: usize,
    /// **入力に無い添字が出た数．常に 0 でなければならない** (D94)．
    pub created: usize,
}

pub const ROT_HEADER: &str = "file,algo,degrees,round_trip,silhouette,jaggies,created";

pub fn rot_csv(r: &RotRow) -> String {
    format!(
        "{},{},{:.1},{:.4},{:.4},{},{}",
        r.file, r.algo, r.degrees, r.round_trip, r.silhouette, r.jaggies, r.created
    )
}

fn opaque_mask(c: &IndexedCanvas) -> Vec<bool> {
    let t = c.transparent();
    c.pixels().iter().map(|i| t != Some(*i)).collect()
}

/// 中央を合わせて重ねたときに一致する画素の割合．
fn agreement(a: &IndexedCanvas, b: &IndexedCanvas, silhouette_only: bool) -> f32 {
    let (ox, oy) = (
        (b.width() as i32 - a.width() as i32) / 2,
        (b.height() as i32 - a.height() as i32) / 2,
    );
    let (ma, mb) = (opaque_mask(a), opaque_mask(b));
    let (mut same, mut total) = (0usize, 0usize);
    for y in 0..a.height() as i32 {
        for x in 0..a.width() as i32 {
            let k = (y as u32 * a.width() + x as u32) as usize;
            let Some(_) = b.get(x + ox, y + oy) else {
                continue;
            };
            let kb = ((y + oy) as u32 * b.width() + (x + ox) as u32) as usize;
            total += 1;
            let hit = if silhouette_only {
                ma[k] == mb[kb]
            } else {
                a.get(x, y) == b.get(x + ox, y + oy)
            };
            if hit {
                same += 1;
            }
        }
    }
    if total == 0 {
        0.0
    } else {
        same as f32 / total as f32
    }
}

fn created_indices(src: &IndexedCanvas, out: &IndexedCanvas) -> usize {
    let mut seen: Vec<u8> = src.pixels().to_vec();
    seen.sort_unstable();
    seen.dedup();
    out.pixels()
        .iter()
        .filter(|i| seen.binary_search(i).is_err())
        .count()
}

fn one(
    file: &str,
    canvas: &IndexedCanvas,
    palette: &Palette,
    algo: ResampleAlgo,
    degrees: f32,
) -> Option<RotRow> {
    let opts = ResampleOptions { algo, grow: true };
    let (once, _) = rotate(canvas, palette, degrees, &opts).ok()?;
    let (back, _) = rotate(&once, palette, -degrees, &opts).ok()?;
    Some(RotRow {
        file: file.to_string(),
        algo: algo.as_str(),
        degrees,
        round_trip: agreement(canvas, &back, false),
        silhouette: agreement(canvas, &back, true),
        jaggies: analyze_canvas(&once, pxsmith_core::geom::jaggy::DEFAULT_MAX_MOVE)
            .jaggies
            .len(),
        created: created_indices(canvas, &once),
    })
}

/// 実素材を掃く．
pub fn build(dir: &Path, angles: &[f32]) -> Result<(Vec<RotRow>, usize)> {
    let mut out = Vec::new();
    let mut skipped = 0usize;
    for path in png_files(dir)? {
        let file = name_of(&path);
        let Some((canvas, palette)) = indexed(&path) else {
            skipped += 1;
            continue;
        };
        for algo in ResampleAlgo::ALL {
            for &deg in angles {
                match one(&file, &canvas, &palette, algo, deg) {
                    Some(r) => out.push(r),
                    None => skipped += 1,
                }
            }
        }
    }
    Ok((out, skipped))
}

/// 流儀 x 角度でまとめる．
#[allow(clippy::type_complexity)]
pub fn summarise(rows: &[RotRow]) -> Vec<(&'static str, f32, usize, f32, f32, f32, usize)> {
    let mut keys: Vec<(&'static str, i32)> = rows
        .iter()
        .map(|r| (r.algo, (r.degrees * 10.0) as i32))
        .collect();
    keys.sort_unstable();
    keys.dedup();
    keys.into_iter()
        .map(|(algo, deg10)| {
            let set: Vec<&RotRow> = rows
                .iter()
                .filter(|r| r.algo == algo && (r.degrees * 10.0) as i32 == deg10)
                .collect();
            let mean = |f: &dyn Fn(&RotRow) -> f32| -> f32 {
                set.iter().map(|r| f(r)).sum::<f32>() / set.len().max(1) as f32
            };
            let jag: usize = set.iter().map(|r| r.jaggies).sum();
            (
                algo,
                deg10 as f32 / 10.0,
                set.len(),
                mean(&|r| r.round_trip),
                mean(&|r| r.silhouette),
                jag as f32 / set.len().max(1) as f32,
                set.iter().map(|r| r.created).sum(),
            )
        })
        .collect()
}

/// **拡大を伴う場面で測る．**
///
/// cleanEdge は «拡大しながら縁を作り直す» ものなので，等倍で測ると本来の
/// 場面を外している見込みがある．4 倍に拡大したときと，拡大してから回した
/// ときのジャギーを流儀ごとに数える．
///
/// 返り値は (流儀, 場面, 件数, ジャギーの平均, 作った色の合計)．
#[allow(clippy::type_complexity)]
pub fn upscaled(dir: &Path) -> Result<Vec<(&'static str, &'static str, usize, f32, usize)>> {
    let mut out = Vec::new();
    for algo in ResampleAlgo::ALL {
        for (scene, angle) in [("4 倍拡大", 0.0f32), ("4 倍 + 30 度", 30.0)] {
            let opts = ResampleOptions { algo, grow: true };
            let (mut jag, mut created, mut n) = (0usize, 0usize, 0usize);
            for path in png_files(dir)? {
                let Some((canvas, palette)) = indexed(&path) else {
                    continue;
                };
                let Ok((big, _)) = scale(&canvas, &palette, 4.0, &opts) else {
                    continue;
                };
                let result = if angle == 0.0 {
                    big
                } else {
                    match rotate(&big, &palette, angle, &opts) {
                        Ok((r, _)) => r,
                        Err(_) => continue,
                    }
                };
                n += 1;
                jag += analyze_canvas(&result, pxsmith_core::geom::jaggy::DEFAULT_MAX_MOVE)
                    .jaggies
                    .len();
                created += created_indices(&canvas, &result);
            }
            out.push((
                algo.as_str(),
                scene,
                n,
                jag as f32 / n.max(1) as f32,
                created,
            ));
        }
    }
    Ok(out)
}

/// 整数倍の拡大が厳密かを確かめる (**流儀に依らず一致するはず**)．
pub fn integer_scale_is_exact(dir: &Path) -> Result<Vec<(&'static str, usize, usize)>> {
    let mut out = Vec::new();
    for algo in ResampleAlgo::ALL {
        let (mut ok, mut total) = (0usize, 0usize);
        for path in png_files(dir)? {
            let Some((canvas, palette)) = indexed(&path) else {
                continue;
            };
            let opts = ResampleOptions { algo, grow: true };
            let Ok((big, _)) = scale(&canvas, &palette, 3.0, &opts) else {
                continue;
            };
            total += 1;
            let exact = (0..canvas.height() as i32).all(|y| {
                (0..canvas.width() as i32).all(|x| {
                    (0..3).all(|dy| {
                        (0..3).all(|dx| big.get(x * 3 + dx, y * 3 + dy) == canvas.get(x, y))
                    })
                })
            });
            if exact {
                ok += 1;
            }
        }
        out.push((algo.as_str(), ok, total));
    }
    Ok(out)
}
