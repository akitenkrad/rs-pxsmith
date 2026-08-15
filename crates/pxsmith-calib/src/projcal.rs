//! **投影変換の主張を測る** (`pxsmith-calib project`)．
//!
//! 設計書 6.13 は短い節だが，主張が 3 つ入っている．**閾値を決める前に，主張が
//! 本当かを測る** (M4 で 10 回 ・M7 で 3 回外れた) ．
//!
//! | 主張 | 測り方 |
//! | --- | --- |
//! | «45 度回転 → 高さ 1/2» **または** «幅 0.866 → −30 度歪み» | 2 通りで写して**突き合わせる** |
//! | 等角は 2:1 で 26.57 度 (正確な 30 度は引けないため代用) | **段の走りを数える** (真値のある場面) |
//! | 投影は下地であって完成品ではない (1.3) | **自分の出力を自分の検査に掛ける** |
//!
//! 1 つ目は «同じもの» なら突き合わせて一致するはずである — 一致しなければ
//! «または» が誤りで，**適用先が違う 2 つの変換**ということになる．

use anyhow::Result;
use std::path::Path;

use pxsmith_core::canvas::IndexedCanvas;
use pxsmith_core::geom::jaggy::analyze_canvas;
use pxsmith_core::palette::Palette;
use pxsmith_core::project::ProjectOptions;
use pxsmith_core::project::{Facing, Projection, SourcePlane, Step, as_written, matrix, project};
use pxsmith_core::resample::{ResampleAlgo, ResampleOptions, affine};

use crate::animcal::{indexed, name_of, png_files};

fn opaque_mask(c: &IndexedCanvas) -> Vec<bool> {
    let t = c.transparent();
    c.pixels().iter().map(|i| t != Some(*i)).collect()
}

/// 中央を合わせて重ねたときにシルエットが一致する割合．
fn silhouette_agreement(a: &IndexedCanvas, b: &IndexedCanvas) -> f32 {
    let (w, h) = (a.width().max(b.width()), a.height().max(b.height()));
    let (ax, ay) = (
        (w as i32 - a.width() as i32) / 2,
        (h as i32 - a.height() as i32) / 2,
    );
    let (bx, by) = (
        (w as i32 - b.width() as i32) / 2,
        (h as i32 - b.height() as i32) / 2,
    );
    let (ma, mb) = (opaque_mask(a), opaque_mask(b));
    let (mut same, mut total) = (0usize, 0usize);
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let ga = a
                .get(x - ax, y - ay)
                .map(|_| ma[((y - ay) as u32 * a.width() + (x - ax) as u32) as usize])
                .unwrap_or(false);
            let gb = b
                .get(x - bx, y - by)
                .map(|_| mb[((y - by) as u32 * b.width() + (x - bx) as u32) as usize])
                .unwrap_or(false);
            total += 1;
            if ga == gb {
                same += 1;
            }
        }
    }
    same as f32 / total.max(1) as f32
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

fn blocking_of(canvas: &IndexedCanvas, palette: &Palette) -> usize {
    let cfg = pxsmith_lint::LintConfig::default();
    let mut r = pxsmith_lint::rules::lint_palette(palette, &cfg);
    r.extend(pxsmith_lint::lint_canvas(canvas, palette, &cfg));
    r.blocking().count()
}

// ---------------------------------------------------------------- 主張 1

/// 設計書 6.13 の 2 手順を突き合わせた結果．
pub struct TwoProceduresRow {
    pub file: String,
    /// «45 度回転 → 高さ 1/2» の出力の寸法．
    pub halve_size: (u32, u32),
    /// «幅 0.866 → −30 度歪み» の出力の寸法．
    pub shear_size: (u32, u32),
    /// 中央を合わせたときのシルエット一致率．
    pub agreement: f32,
}

/// **2 手順が同じ変換なのかを突き合わせる．**
pub fn two_procedures(dir: &Path) -> Result<(Vec<TwoProceduresRow>, usize)> {
    let opts = ResampleOptions::default();
    let (mut rows, mut skipped) = (Vec::new(), 0usize);
    for path in png_files(dir)? {
        let Some((canvas, palette)) = indexed(&path) else {
            skipped += 1;
            continue;
        };
        let Ok((a, _)) = affine(&canvas, &palette, as_written::rotate_then_halve(), &opts) else {
            skipped += 1;
            continue;
        };
        let Ok((b, _)) = affine(&canvas, &palette, as_written::squash_then_shear(), &opts) else {
            skipped += 1;
            continue;
        };
        rows.push(TwoProceduresRow {
            file: name_of(&path),
            halve_size: (a.width(), a.height()),
            shear_size: (b.width(), b.height()),
            agreement: silhouette_agreement(&a, &b),
        });
    }
    Ok((rows, skipped))
}

// ---------------------------------------------------------------- 主張 2

/// 段の刻み — 走りの長さがいくつ現れるか．
pub struct StairRow {
    pub label: &'static str,
    pub slope: f32,
    pub degrees: f32,
    /// 現れた走りの長さ (端は除く)．**1 種類なら格子に乗っている**．
    pub runs: Vec<u32>,
}

/// 傾き $s$ の段を長さ `len` まで刻んだときの走りの長さを数える．
///
/// **真値のある場面である** — 段が揃った線とは «走りの長さが 1 種類» の線で
/// あって，それ以外の定義は要らない (D102 と同じ数え上げの側) ．
fn stair_runs(slope: f32, len: u32) -> Vec<u32> {
    let mut runs = Vec::new();
    let (mut last_y, mut run) = ((0.0f32).round() as i32, 0u32);
    for x in 0..=len {
        let y = (slope * x as f32).round() as i32;
        if y == last_y {
            run += 1;
        } else {
            runs.push(run);
            last_y = y;
            run = 1;
        }
    }
    // 両端は切れているので落とす
    if runs.len() > 2 {
        runs.remove(0);
        runs.pop();
    }
    runs.sort_unstable();
    runs.dedup();
    runs
}

/// **2:1 と 30 度を並べる．**
pub fn stairs(len: u32) -> Vec<StairRow> {
    let tan30 = (30.0f32).to_radians().tan();
    [
        ("2:1 (表が言う代用)", 0.5f32),
        ("1:1", 1.0),
        ("tan 30 度 (手順が使う)", tan30),
    ]
    .into_iter()
    .map(|(label, slope)| StairRow {
        label,
        slope,
        degrees: slope.atan().to_degrees(),
        runs: stair_runs(slope, len),
    })
    .collect()
}

// ---------------------------------------------------------------- 主張 3

/// 投影 1 通りぶんの実測．
pub struct ProjRow {
    pub file: String,
    pub projection: &'static str,
    pub plane: &'static str,
    pub degrees: f32,
    pub keeps_vertical: bool,
    pub area_ratio: f32,
    pub size: ((u32, u32), (u32, u32)),
    /// 広げたときに切れた画素．**0 のはず**．
    pub clipped: usize,
    /// 広げなかったときに切れた画素．
    pub clipped_no_grow: usize,
    /// **作った色．常に 0 でなければならない** (D94)．
    pub created: usize,
    /// 元の絵の blocking．
    pub blocking_before: usize,
    /// 投影した絵の blocking．
    pub blocking_after: usize,
    /// ジャギーの数．
    pub jaggies: usize,
}

pub const PROJ_HEADER: &str = "file,projection,plane,degrees,keeps_vertical,area_ratio,\
in_w,in_h,out_w,out_h,clipped,clipped_no_grow,created,blocking_before,blocking_after,jaggies";

pub fn proj_csv(r: &ProjRow) -> String {
    format!(
        "{},{},{},{:.2},{},{:.4},{},{},{},{},{},{},{},{},{},{}",
        r.file,
        r.projection,
        r.plane,
        r.degrees,
        r.keeps_vertical,
        r.area_ratio,
        r.size.0.0,
        r.size.0.1,
        r.size.1.0,
        r.size.1.1,
        r.clipped,
        r.clipped_no_grow,
        r.created,
        r.blocking_before,
        r.blocking_after,
        r.jaggies
    )
}

fn one(
    file: &str,
    canvas: &IndexedCanvas,
    palette: &Palette,
    projection: Projection,
    plane: SourcePlane,
) -> Option<ProjRow> {
    let base = ProjectOptions {
        projection,
        plane,
        facing: Facing::Right,
        step: None,
        resample: ResampleOptions {
            algo: ResampleAlgo::Nearest,
            grow: true,
        },
    };
    let (out, r) = project(canvas, palette, &base).ok()?;

    let tight = ProjectOptions {
        resample: ResampleOptions {
            algo: ResampleAlgo::Nearest,
            grow: false,
        },
        ..base
    };
    let clipped_no_grow = project(canvas, palette, &tight)
        .map(|(_, r)| r.resample.clipped)
        .unwrap_or(0);

    Some(ProjRow {
        file: file.to_string(),
        projection: r.projection,
        plane: r.plane,
        degrees: r.degrees,
        keeps_vertical: r.keeps_vertical,
        area_ratio: r.area_ratio,
        size: r.resample.size,
        clipped: r.resample.clipped,
        clipped_no_grow,
        created: created_indices(canvas, &out),
        blocking_before: blocking_of(canvas, palette),
        blocking_after: blocking_of(&out, palette),
        jaggies: analyze_canvas(&out, pxsmith_core::geom::jaggy::DEFAULT_MAX_MOVE)
            .jaggies
            .len(),
    })
}

/// 実素材を掃く．
pub fn build(dir: &Path) -> Result<(Vec<ProjRow>, usize)> {
    let (mut rows, mut skipped) = (Vec::new(), 0usize);
    for path in png_files(dir)? {
        let file = name_of(&path);
        let Some((canvas, palette)) = indexed(&path) else {
            skipped += 1;
            continue;
        };
        for projection in Projection::ALL {
            for plane in SourcePlane::ALL {
                match one(&file, &canvas, &palette, projection, plane) {
                    Some(r) => rows.push(r),
                    None => skipped += 1,
                }
            }
        }
    }
    Ok((rows, skipped))
}

/// 投影 x 面でまとめる．
///
/// 返り値は (投影, 面, 件数, 角度, 垂直を保った率, 面積比, 切れ(広げた),
/// 切れ(広げない), 作った色, blocking 増, ジャギー平均)．
#[allow(clippy::type_complexity)]
pub fn summarise(
    rows: &[ProjRow],
) -> Vec<(
    &'static str,
    &'static str,
    usize,
    f32,
    f32,
    f32,
    usize,
    usize,
    usize,
    f32,
    f32,
)> {
    let mut keys: Vec<(&'static str, &'static str)> =
        rows.iter().map(|r| (r.projection, r.plane)).collect();
    keys.sort_unstable();
    keys.dedup();
    keys.into_iter()
        .map(|(projection, plane)| {
            let set: Vec<&ProjRow> = rows
                .iter()
                .filter(|r| r.projection == projection && r.plane == plane)
                .collect();
            let n = set.len().max(1);
            (
                projection,
                plane,
                set.len(),
                set.iter().map(|r| r.degrees).sum::<f32>() / n as f32,
                set.iter().filter(|r| r.keeps_vertical).count() as f32 / n as f32,
                set.iter().map(|r| r.area_ratio).sum::<f32>() / n as f32,
                set.iter().map(|r| r.clipped).sum(),
                set.iter().map(|r| r.clipped_no_grow).sum(),
                set.iter().map(|r| r.created).sum(),
                set.iter()
                    .map(|r| r.blocking_after as f32 - r.blocking_before as f32)
                    .sum::<f32>()
                    / n as f32,
                set.iter().map(|r| r.jaggies as f32).sum::<f32>() / n as f32,
            )
        })
        .collect()
}

/// **格子に乗る段と乗らない段を，実素材のジャギーで比べる．**
///
/// 段の刻みは合成の線で数え上げれば決まる (上の [`stairs`]) が，**実素材でも
/// 差が出るか**は別の問いである．横から見た絵として写し，2:1 と «幅 0.866 →
/// −30 度» のジャギーを並べる．
///
/// 返り値は (名前, 件数, ジャギー平均, 作った色の合計)．
pub fn grid_vs_thirty(dir: &Path) -> Result<Vec<(&'static str, usize, f32, usize)>> {
    let opts = ResampleOptions::default();
    let cases: [(&'static str, [f32; 4]); 2] = [
        (
            "2:1 (採った側)",
            matrix(
                Projection::Iso,
                SourcePlane::Side,
                Facing::Right,
                Step::TWO_TO_ONE,
            ),
        ),
        ("幅 0.866 → −30 度", as_written::squash_then_shear()),
    ];
    let mut out = Vec::new();
    for (label, m) in cases {
        let (mut n, mut jag, mut created) = (0usize, 0f32, 0usize);
        for path in png_files(dir)? {
            let Some((canvas, palette)) = indexed(&path) else {
                continue;
            };
            let Ok((result, _)) = affine(&canvas, &palette, m, &opts) else {
                continue;
            };
            n += 1;
            jag += analyze_canvas(&result, pxsmith_core::geom::jaggy::DEFAULT_MAX_MOVE)
                .jaggies
                .len() as f32;
            created += created_indices(&canvas, &result);
        }
        out.push((label, n, jag / n.max(1) as f32, created));
    }
    Ok(out)
}
