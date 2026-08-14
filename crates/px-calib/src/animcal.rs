//! **M4 の残り 5 件 (おばけ ・外挿 ・潰し ・サブピクセル) の主張を測る口．**
//!
//! 設計書の主張は M4 だけで 5 回外れているので，**閾値や既定値を決める前に
//! «その主張は本当か» を測る**．ここで測るのは次の 4 つである．
//!
//! | 測るもの | 設計書の主張 | 真値の作り方 |
//! | --- | --- | --- |
//! | おばけ | «union は 2 塊が残るだけで繋がらない» | **繋がったかは数えられる** (成分数) |
//! | 刻み幅 | «$\Delta t \lVert 変位 \rVert \lesssim 1$ でないと数珠状» | 標本を掃いて成分数 |
//! | 外挿 | «変位を符号反転 / 超過させて外挿» | **平行移動なら真値がある** ($t$ 倍動かした絵) |
//! | 潰し | «体積保存 ($h \times w$ 一定)» | 丸めの残りを数える |
//! | サブピクセル | «$f$ を丸めると 2 値に潰れる» «高速法はパレット強制で滲みが消える» | 効いた画素を数える |
//!
//! **«効かない» と «測れない» を分ける** (D77 ・D104) ．サブピクセルは «透明との
//! 間に中間色が無い» という構造的な理由で効かない画素があるので，そこは
//! «候補にならなかった» として別に数える．

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use px_core::canvas::{IndexedCanvas, RgbaCanvas};
use px_core::color::Rgba8;
use px_core::deform::{SquashOptions, VolumeRule, squash};
use px_core::geom::Mask;
use px_core::math::{IVec2, ivec2};
use px_core::palette::Palette;
use px_core::smear::{SmearMethod, SmearOptions, smear_mask};
use px_core::subpixel::{
    SubpixelMethod, SubpixelOptions, SubpixelScope, pairs_with_intermediate, subpixel,
};
use px_core::tween::{ExtrapolateKind, TweenAlign, TweenOptions, extrapolate_mask};

// ------------------------------------------------------------------ 読み込み

pub fn png_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("{} を読めない", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")))
        .collect();
    files.sort();
    Ok(files)
}

pub fn name_of(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// シルエットを余地付きで読む．
pub fn silhouette(path: &Path, pad: u32) -> Option<Mask> {
    let img = px_io::png::read_rgba(path).ok()?;
    let mut m = Mask::new(img.width() + pad * 2, img.height() + pad * 2);
    for y in 0..img.height() as i32 {
        for x in 0..img.width() as i32 {
            if img.get(x, y).is_some_and(|c| c.a != 0) {
                m.set(ivec2(x + pad as i32, y + pad as i32), true);
            }
        }
    }
    (!m.is_empty()).then_some(m)
}

/// **その場の量子化を挟まずに指標へ落とす** — 使っている色をそのままパレットにする．
pub fn index_exactly(img: &RgbaCanvas) -> Option<(IndexedCanvas, Palette)> {
    let mut colors: Vec<Rgba8> = img.pixels().to_vec();
    colors.sort_unstable_by_key(|c| c.sort_key());
    colors.dedup();
    if colors.len() > 256 {
        return None;
    }
    let palette = Palette::new(colors).ok()?;
    let transparent = palette
        .entries()
        .iter()
        .position(|c| c.a == 0)
        .map(|i| i as u8);
    let pixels: Vec<u8> = img
        .pixels()
        .iter()
        .map(|c| match (c.a, transparent) {
            (0, Some(i)) => i,
            _ => palette.nearest(*c, 1.0).unwrap_or(0),
        })
        .collect();
    let canvas = IndexedCanvas::from_pixels(img.width(), img.height(), pixels)
        .ok()?
        .with_transparent(transparent);
    Some((canvas, palette))
}

pub fn indexed(path: &Path) -> Option<(IndexedCanvas, Palette)> {
    index_exactly(&px_io::png::read_rgba(path).ok()?)
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

fn median(mut v: Vec<f32>) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f32::total_cmp);
    v[v.len() / 2]
}

// ---------------------------------------------------------------- おばけ

/// 1 つのずらしについて，3 通りの作り方を並べた結果．
#[derive(Clone, Debug)]
pub struct SmearRow {
    pub file: String,
    pub shift: i32,
    /// シルエットの «動く向きの» 差し渡し (これに対して動きが大きいと切れる)．
    pub extent: u32,
    /// 成分数 (union ・場のままの掃引 ・重心を取り除いた掃引)．
    pub components: (usize, usize, usize),
    /// 面積 (同上)．
    pub areas: (usize, usize, usize),
}

pub const SMEAR_HEADER: &str =
    "file,shift,extent,comp_union,comp_plain,comp_aligned,area_union,area_plain,area_aligned";

pub fn smear_csv(r: &SmearRow) -> String {
    format!(
        "{},{},{},{},{},{},{},{},{}",
        r.file,
        r.shift,
        r.extent,
        r.components.0,
        r.components.1,
        r.components.2,
        r.areas.0,
        r.areas.1,
        r.areas.2
    )
}

/// **3 通りを同じ絵に当てて «繋がったか» を数える．**
pub fn smear_rows(dir: &Path, shifts: &[i32]) -> Result<Vec<SmearRow>> {
    let mut out = Vec::new();
    for path in png_files(dir)? {
        for &dx in shifts {
            let pad = (dx.unsigned_abs()) + 2;
            let Some(a) = silhouette(&path, pad) else {
                continue;
            };
            let b = shifted(&a, ivec2(dx, 0));
            let run = |method: SmearMethod, align: TweenAlign| {
                smear_mask(
                    &a,
                    &b,
                    &SmearOptions {
                        method,
                        align,
                        samples: None,
                    },
                )
                .ok()
            };
            let (Some(u), Some(p), Some(c)) = (
                run(SmearMethod::Union, TweenAlign::None),
                run(SmearMethod::Sweep, TweenAlign::None),
                run(SmearMethod::Sweep, TweenAlign::Centroid),
            ) else {
                continue;
            };
            let extent = a.bbox().map(|b| b.w).unwrap_or(0);
            out.push(SmearRow {
                file: name_of(&path),
                shift: dx,
                extent,
                components: (u.components.2, p.components.2, c.components.2),
                areas: (u.mask.count(), p.mask.count(), c.mask.count()),
            });
        }
    }
    Ok(out)
}

/// ずらしごとに «繋がった件数» を数える．
pub fn summarise_smear(rows: &[SmearRow]) -> Vec<(i32, usize, usize, usize, usize)> {
    let mut keys: Vec<i32> = rows.iter().map(|r| r.shift).collect();
    keys.sort_unstable();
    keys.dedup();
    keys.into_iter()
        .map(|s| {
            let g: Vec<&SmearRow> = rows.iter().filter(|r| r.shift == s).collect();
            (
                s,
                g.len(),
                g.iter().filter(|r| r.components.0 == 1).count(),
                g.iter().filter(|r| r.components.1 == 1).count(),
                g.iter().filter(|r| r.components.2 == 1).count(),
            )
        })
        .collect()
}

/// 標本の数を掃いて «数珠状» になるかを見る．
///
/// 返すのは (標本の数，切れた枚数，枚数)．`None` は «変位から決める» 側．
pub fn sample_sweep(
    dir: &Path,
    shift: i32,
    samples: &[Option<u32>],
) -> Result<Vec<(Option<u32>, usize, usize)>> {
    let mut rows: Vec<(Option<u32>, usize, usize)> =
        samples.iter().map(|s| (*s, 0usize, 0usize)).collect();
    for path in png_files(dir)? {
        let pad = shift.unsigned_abs() + 2;
        let Some(a) = silhouette(&path, pad) else {
            continue;
        };
        let b = shifted(&a, ivec2(shift, 0));
        for row in rows.iter_mut() {
            let Ok(r) = smear_mask(
                &a,
                &b,
                &SmearOptions {
                    method: SmearMethod::Sweep,
                    align: TweenAlign::Centroid,
                    samples: row.0,
                },
            ) else {
                continue;
            };
            row.2 += 1;
            if r.components.2 > 1 {
                row.1 += 1;
            }
        }
    }
    Ok(rows)
}

// ------------------------------------------------------------------ 外挿

/// 外挿 1 件．**平行移動なら真値がある** — $t$ 倍だけ動かした絵である．
#[derive(Clone, Debug)]
pub struct ExtrapolateRow {
    pub file: String,
    pub shift: i32,
    pub kind: &'static str,
    pub amount: f32,
    /// 真値との IoU (場のまま)．
    pub plain: f32,
    /// 真値との IoU (重心を取り除く)．
    pub centroid: f32,
    /// **対照** — 端のキーフレームをそのまま出したときの IoU．
    pub hold: f32,
    /// 画布の外へ出て切れた画素 (重心側)．
    pub clipped: usize,
    /// トポロジーが両端のどちらとも違うか (重心側)．
    pub topology_changed: bool,
}

pub const EXTRAPOLATE_HEADER: &str =
    "file,shift,kind,amount,plain,centroid,hold,clipped,topology_changed";

pub fn extrapolate_csv(r: &ExtrapolateRow) -> String {
    format!(
        "{},{},{},{:.2},{:.4},{:.4},{:.4},{},{}",
        r.file,
        r.shift,
        r.kind,
        r.amount,
        r.plain,
        r.centroid,
        r.hold,
        r.clipped,
        r.topology_changed
    )
}

/// **真値のある場面で外挿を測る．**
///
/// $A$ を $d$ 動かした $B$ に対し，$t$ の外挿の真値は «$A$ を $t d$ 動かした絵» で
/// ある．予備動作 ($t < 0$) では真値が $A$ の手前に出るので，**余地を両側に取る**．
pub fn extrapolate_rows(
    dir: &Path,
    shifts: &[i32],
    amounts: &[f32],
) -> Result<Vec<ExtrapolateRow>> {
    let mut out = Vec::new();
    for path in png_files(dir)? {
        for &dx in shifts {
            for &amount in amounts {
                for kind in [ExtrapolateKind::Anticipation, ExtrapolateKind::Overshoot] {
                    let t = kind.t_for(amount);
                    // 真値が切れないだけの余地を両側に取る
                    let reach = (dx as f32 * t).abs().ceil() as u32 + dx.unsigned_abs() + 2;
                    let Some(a) = silhouette(&path, reach) else {
                        continue;
                    };
                    let b = shifted(&a, ivec2(dx, 0));
                    let truth = shifted(&a, ivec2((dx as f32 * t).round() as i32, 0));
                    let run = |align: TweenAlign| {
                        extrapolate_mask(&a, &b, kind, amount, &TweenOptions { margin: 0, align })
                            .ok()
                    };
                    let (Some(p), Some(c)) = (run(TweenAlign::None), run(TweenAlign::Centroid))
                    else {
                        continue;
                    };
                    let hold = match kind {
                        ExtrapolateKind::Anticipation => iou(&a, &truth),
                        ExtrapolateKind::Overshoot => iou(&b, &truth),
                    };
                    out.push(ExtrapolateRow {
                        file: name_of(&path),
                        shift: dx,
                        kind: kind.as_str(),
                        amount,
                        plain: iou(&p.mask, &truth),
                        centroid: iou(&c.mask, &truth),
                        hold,
                        clipped: c.clipped,
                        topology_changed: {
                            let chi = |(cc, hh): (usize, usize)| cc as i64 - hh as i64;
                            let r = chi((c.components.2, c.holes.2));
                            r != chi((c.components.0, c.holes.0))
                                && r != chi((c.components.1, c.holes.1))
                        },
                    });
                }
            }
        }
    }
    Ok(out)
}

/// (種類, 振り幅) ごとに中央値をまとめる．
pub fn summarise_extrapolate(
    rows: &[ExtrapolateRow],
) -> Vec<(&'static str, f32, usize, f32, f32, f32, usize)> {
    let mut keys: Vec<(&'static str, String)> = rows
        .iter()
        .map(|r| (r.kind, format!("{:.2}", r.amount)))
        .collect();
    keys.sort();
    keys.dedup();
    keys.into_iter()
        .map(|(kind, amount)| {
            let g: Vec<&ExtrapolateRow> = rows
                .iter()
                .filter(|r| r.kind == kind && format!("{:.2}", r.amount) == amount)
                .collect();
            (
                kind,
                amount.parse::<f32>().unwrap_or(0.0),
                g.len(),
                median(g.iter().map(|r| r.plain).collect()),
                median(g.iter().map(|r| r.centroid).collect()),
                median(g.iter().map(|r| r.hold).collect()),
                g.iter().filter(|r| r.clipped > 0).count(),
            )
        })
        .collect()
}

// -------------------------------------------------------------------- 潰し

/// 潰し 1 件．
#[derive(Clone, Debug)]
pub struct SquashRow {
    pub file: String,
    pub rule: &'static str,
    pub amount: f32,
    /// 外接矩形の面積 (前, 後)．
    pub bbox_volume: (u32, u32),
    /// 不透明な画素の数 (前, 後)．
    pub pixels: (usize, usize),
    pub volume_error: f32,
    pub pixel_error: f32,
    /// 拡縮そのものが動かした量 (切れたぶんを除く)．
    pub resample_error: f32,
    pub grew: bool,
    pub clipped: usize,
    /// 増えた添字の数 (**0 でなければならない**)．
    pub added_colors: i64,
}

pub const SQUASH_HEADER: &str = "file,rule,grow,amount,bbox_before,bbox_after,px_before,px_after,volume_error,pixel_error,resample_error,clipped,added_colors";

pub fn squash_csv(r: &SquashRow) -> String {
    format!(
        "{},{},{},{:.2},{},{},{},{},{:.4},{:.4},{:.4},{},{}",
        r.file,
        r.rule,
        r.grew,
        r.amount,
        r.bbox_volume.0,
        r.bbox_volume.1,
        r.pixels.0,
        r.pixels.1,
        r.volume_error,
        r.pixel_error,
        r.resample_error,
        r.clipped,
        r.added_colors
    )
}

pub fn squash_rows(dir: &Path, amounts: &[f32]) -> Result<Vec<SquashRow>> {
    let mut out = Vec::new();
    for path in png_files(dir)? {
        let Some((canvas, _)) = indexed(&path) else {
            continue;
        };
        for &amount in amounts {
            for rule in [VolumeRule::Independent, VolumeRule::Derived] {
                for grow in [false, true] {
                    let Ok((_, r)) = squash(
                        &canvas,
                        amount,
                        &SquashOptions {
                            rule,
                            grow,
                            ..Default::default()
                        },
                    ) else {
                        continue;
                    };
                    out.push(SquashRow {
                        file: name_of(&path),
                        rule: rule.as_str(),
                        grew: grow,
                        amount,
                        bbox_volume: r.bbox_volume,
                        pixels: r.pixels,
                        volume_error: r.volume_error(),
                        pixel_error: r.pixel_error(),
                        resample_error: r.resample_error(),
                        clipped: r.clipped,
                        added_colors: r.colors.1 as i64 - r.colors.0 as i64,
                    });
                }
            }
        }
    }
    Ok(out)
}

/// 規則ごとに (件数, 体積誤差の中央値, 最大, 画素数誤差の中央値, 切れた件数, 色が増えた件数)．
#[allow(clippy::type_complexity)]
pub fn summarise_squash(
    rows: &[SquashRow],
) -> Vec<(&'static str, bool, usize, f32, f32, f32, f32, usize, usize)> {
    let mut keys: Vec<(&'static str, bool)> = rows.iter().map(|r| (r.rule, r.grew)).collect();
    keys.sort_unstable();
    keys.dedup();
    keys.into_iter()
        .map(|(rule, grew)| {
            let g: Vec<&SquashRow> = rows
                .iter()
                .filter(|r| r.rule == rule && r.grew == grew)
                .collect();
            let mut worst = 0.0f32;
            for r in &g {
                worst = worst.max(r.volume_error);
            }
            (
                rule,
                grew,
                g.len(),
                median(g.iter().map(|r| r.volume_error).collect()),
                worst,
                median(g.iter().map(|r| r.pixel_error).collect()),
                median(g.iter().map(|r| r.resample_error).collect()),
                g.iter().filter(|r| r.clipped > 0).count(),
                g.iter().filter(|r| r.added_colors > 0).count(),
            )
        })
        .collect()
}

// ------------------------------------------------------------ サブピクセル

/// サブピクセル 1 件．
#[derive(Clone, Debug)]
pub struct SubpixelRow {
    pub file: String,
    pub method: &'static str,
    pub scope: &'static str,
    pub fraction: f32,
    pub chains: usize,
    /// 接線方向に色が変わっていた画素の対 (**効きうる画素**)．
    pub candidates: usize,
    pub changed: usize,
    /// 中間色がパレットに無くて動かせなかった対．
    pub no_colour: usize,
    /// シルエットが動いた画素の数．
    pub silhouette_moved: usize,
    pub added_colors: i64,
    /// パレットの色の組のうち «間の色» がある割合．
    pub intermediate_rate: f32,
    /// lint の blocking が増えた数 (**道具が絵を壊していないか**)．
    pub blocking_delta: i64,
}

pub const SUBPIXEL_HEADER: &str = "file,method,scope,fraction,chains,candidates,changed,no_colour,silhouette_moved,added_colors,intermediate_rate,blocking_delta";

pub fn subpixel_csv(r: &SubpixelRow) -> String {
    format!(
        "{},{},{},{:.2},{},{},{},{},{},{},{:.4},{}",
        r.file,
        r.method,
        r.scope,
        r.fraction,
        r.chains,
        r.candidates,
        r.changed,
        r.no_colour,
        r.silhouette_moved,
        r.added_colors,
        r.intermediate_rate,
        r.blocking_delta
    )
}

/// blocking の件数 (パレット検査 + 画布検査)．
fn blocking_of(canvas: &IndexedCanvas, palette: &Palette) -> usize {
    let cfg = px_lint::LintConfig::default();
    let mut r = px_lint::rules::lint_palette(palette, &cfg);
    r.extend(px_lint::lint_canvas(canvas, palette, &cfg));
    r.blocking().count()
}

pub fn subpixel_rows(dir: &Path, fractions: &[f32]) -> Result<Vec<SubpixelRow>> {
    let mut out = Vec::new();
    for path in png_files(dir)? {
        let Some((canvas, palette)) = indexed(&path) else {
            continue;
        };
        let (have, total) = pairs_with_intermediate(&palette, 0.04);
        let before_blocking = blocking_of(&canvas, &palette) as i64;
        let rate = if total == 0 {
            0.0
        } else {
            have as f32 / total as f32
        };
        for &fraction in fractions {
            let mut cases: Vec<(SubpixelMethod, SubpixelScope)> = vec![
                (SubpixelMethod::Tangent, SubpixelScope::Silhouette),
                (SubpixelMethod::Tangent, SubpixelScope::Colours),
            ];
            // 高速法は移動率を持たないので 1 度だけ測る
            if (fraction - 0.5).abs() < 1e-6 {
                cases.push((SubpixelMethod::Fast, SubpixelScope::Colours));
            }
            for (method, scope) in cases {
                let Ok((got, r)) = subpixel(
                    &canvas,
                    &palette,
                    &SubpixelOptions {
                        fraction,
                        method,
                        scope,
                        ..Default::default()
                    },
                ) else {
                    continue;
                };
                out.push(SubpixelRow {
                    file: name_of(&path),
                    method: method.as_str(),
                    scope: scope.as_str(),
                    fraction,
                    chains: r.chains,
                    candidates: r.candidates,
                    changed: r.changed,
                    no_colour: r.no_colour,
                    // **道具が数えた値をそのまま使う** — 取り方が 2 か所に
                    // あってはいけない (D110)
                    silhouette_moved: r.silhouette_moved,
                    added_colors: r.colors.1 as i64 - r.colors.0 as i64,
                    intermediate_rate: rate,
                    blocking_delta: blocking_of(&got, &palette) as i64 - before_blocking,
                });
            }
        }
    }
    Ok(out)
}

/// (方法, 範囲, 移動率) ごとに数える．
///
/// 返すのは (方法, 範囲, 移動率, 件数, 動いた枚数, 動いた画素の中央値,
/// 中間色が無くて諦めた対の合計, シルエットが動いた枚数, 色が増えた枚数)．
#[allow(clippy::type_complexity)]
pub fn summarise_subpixel(
    rows: &[SubpixelRow],
) -> Vec<(
    &'static str,
    &'static str,
    f32,
    usize,
    usize,
    f32,
    usize,
    usize,
    usize,
    usize,
)> {
    let mut keys: Vec<(&'static str, &'static str, String)> = rows
        .iter()
        .map(|r| (r.method, r.scope, format!("{:.2}", r.fraction)))
        .collect();
    keys.sort();
    keys.dedup();
    keys.into_iter()
        .map(|(method, scope, fraction)| {
            let g: Vec<&SubpixelRow> = rows
                .iter()
                .filter(|r| {
                    r.method == method
                        && r.scope == scope
                        && format!("{:.2}", r.fraction) == fraction
                })
                .collect();
            (
                method,
                scope,
                fraction.parse::<f32>().unwrap_or(0.0),
                g.len(),
                g.iter().filter(|r| r.changed > 0).count(),
                median(g.iter().map(|r| r.changed as f32).collect()),
                g.iter().map(|r| r.no_colour).sum(),
                g.iter().filter(|r| r.silhouette_moved > 0).count(),
                g.iter().filter(|r| r.added_colors > 0).count(),
                g.iter().filter(|r| r.blocking_delta > 0).count(),
            )
        })
        .collect()
}

/// パレットに «間の色» がある組の割合 (絵ごと)．D83 の 81.3% と同じ量．
pub fn intermediate_rates(rows: &[SubpixelRow]) -> (f32, f32, f32) {
    let mut v: Vec<f32> = rows.iter().map(|r| r.intermediate_rate).collect();
    v.sort_by(f32::total_cmp);
    if v.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    (v[0], v[v.len() / 2], v[v.len() - 1])
}

/// **移動率が結果を変えるかを測る．**
///
/// $f$ は «ランプ上の移動段数と中間色数» へ写るはずである (設計書 6.10) ．
/// **写した先に色が無ければ，$f$ を変えても結果は動かない** — «実装が $f$ を
/// 見ていない» のか «パレットに間の色が 1 色しか無い» のかを分けるために，
/// 出力そのものを突き合わせる．
///
/// 返すのは (枚数, 0.25 と 0.75 で違った枚数, 違った画素の中央値, 間の色が 2 色以上ある組の割合)．
pub fn fraction_sensitivity(dir: &Path) -> Result<(usize, usize, f32, f32)> {
    let (mut files, mut differing) = (0usize, 0usize);
    let mut diffs: Vec<f32> = Vec::new();
    let (mut multi, mut total) = (0usize, 0usize);
    for path in png_files(dir)? {
        let Some((canvas, palette)) = indexed(&path) else {
            continue;
        };
        let run = |f: f32| {
            subpixel(
                &canvas,
                &palette,
                &SubpixelOptions {
                    fraction: f,
                    method: SubpixelMethod::Tangent,
                    scope: SubpixelScope::Colours,
                    ..Default::default()
                },
            )
            .ok()
            .map(|(c, _)| c)
        };
        let (Some(low), Some(high)) = (run(0.25), run(0.75)) else {
            continue;
        };
        files += 1;
        let n = low
            .pixels()
            .iter()
            .zip(high.pixels())
            .filter(|(a, b)| a != b)
            .count();
        diffs.push(n as f32);
        if n > 0 {
            differing += 1;
        }
        let (m, t) = pairs_with_two_intermediates(&palette, 0.04);
        multi += m;
        total += t;
    }
    let rate = if total == 0 {
        0.0
    } else {
        multi as f32 / total as f32
    };
    Ok((files, differing, median(diffs), rate))
}

/// 間に **2 色以上** ある組の数と，間に色がある組の数．
fn pairs_with_two_intermediates(palette: &Palette, tolerance: f32) -> (usize, usize) {
    use px_core::color::{Oklab, distance_sq};
    let n = palette.len().min(256);
    let (mut multi, mut have) = (0usize, 0usize);
    for a in 0..n {
        for b in (a + 1)..n {
            let (a, b) = (a as u8, b as u8);
            let (Some(x), Some(y)) = (palette.lab_of(a), palette.lab_of(b)) else {
                continue;
            };
            if palette.get(a).is_none_or(|c| c.a == 0) || palette.get(b).is_none_or(|c| c.a == 0) {
                continue;
            }
            let span = distance_sq(x, y, 1.0).sqrt();
            if span <= f32::EPSILON {
                continue;
            }
            let mut between = 0usize;
            for (i, lab) in palette.lab().iter().enumerate() {
                let i = i as u8;
                if i == a || i == b || palette.get(i).is_none_or(|c| c.a == 0) {
                    continue;
                }
                let (da, db) = (
                    distance_sq(*lab, x, 1.0).sqrt(),
                    distance_sq(*lab, y, 1.0).sqrt(),
                );
                if da + db - span <= tolerance && da < span && db < span {
                    between += 1;
                }
            }
            let _: Oklab = x;
            if between >= 1 {
                have += 1;
            }
            if between >= 2 {
                multi += 1;
            }
        }
    }
    (multi, have)
}

// ------------------------------------------------------------------ 残像

/// 残像 1 件．
#[derive(Clone, Debug)]
pub struct AfterimageRow {
    pub file: String,
    pub shift: i32,
    pub trail: u32,
    /// 実際に見えた画素．
    pub drawn: usize,
    /// 現在の絵に隠れた画素．
    pub covered: usize,
    /// ランプの端に着いて置けなかった画素．
    pub saturated: usize,
    /// ランプに無い添字だった画素．
    pub not_in_ramp: usize,
}

pub const AFTERIMAGE_HEADER: &str = "file,shift,trail,drawn,covered,saturated,not_in_ramp";

pub fn afterimage_csv(r: &AfterimageRow) -> String {
    format!(
        "{},{},{},{},{},{},{}",
        r.file, r.shift, r.trail, r.drawn, r.covered, r.saturated, r.not_in_ramp
    )
}

/// **残像が «見える» のはどれくらい動いたときかを測る．**
///
/// ランプの宣言が要るので，**`px shade` に描かせた絵を使う** — 実素材の
/// シルエットを取り，宣言した光源で陰影を付ければ «どの色がどのランプの
/// 何段目か» が分かっている状態を作れる (D77 が第 3 群として `px shade` の
/// 出力を測ったのと同じやり方) ．
pub fn afterimage_rows(dir: &Path, shifts: &[i32], trails: &[u32]) -> Result<Vec<AfterimageRow>> {
    use px_core::afterimage::{AfterimageOptions, TrailEdge, afterimage};
    use px_core::frame::{Frame, Layer, LayerMeta, Surface};
    use px_core::palette::ChromaCurve;
    use px_core::ramp::{LightPreset, build_lighting};
    use px_core::shade::{ShadeOptions, shade_to_canvas};

    let base = Rgba8::new(0x8a, 0x6a, 0x4a, 255);
    let (shade_palette, model) =
        build_lighting(base, LightPreset::Clear, 5, ChromaCurve::PeakMiddle)?;
    let source = LightPreset::default_source(LightPreset::Clear);

    let mut out = Vec::new();
    for path in png_files(dir)? {
        for &dx in shifts {
            let Some(a) = silhouette(&path, dx.unsigned_abs() * 3 + 2) else {
                continue;
            };
            // 3 コマの列を «同じ形を dx ずつ動かす» で作る
            let mut frames: Vec<Frame> = Vec::new();
            let mut ok = true;
            for k in 0..3i32 {
                let m = shifted(&a, ivec2(dx * k, 0));
                let Ok((canvas, palette)) =
                    shade_to_canvas(&m, source, &model, &shade_palette, ShadeOptions::default())
                else {
                    ok = false;
                    break;
                };
                let mut f = Frame::new(m.size(), palette);
                f.layers.push(Layer::new(
                    LayerMeta::named("art"),
                    Surface::Indexed(canvas),
                ));
                frames.push(f);
            }
            if !ok || frames.len() < 3 {
                continue;
            }
            for &trail in trails {
                let Ok((_, r)) = afterimage(
                    &frames,
                    &model.key,
                    &AfterimageOptions {
                        trail,
                        step: 1,
                        edge: TrailEdge::None,
                    },
                ) else {
                    continue;
                };
                out.push(AfterimageRow {
                    file: name_of(&path),
                    shift: dx,
                    trail,
                    drawn: r.drawn,
                    covered: r.covered,
                    saturated: r.saturated,
                    not_in_ramp: r.not_in_ramp,
                });
            }
        }
    }
    Ok(out)
}

/// (ずらし, 長さ) ごとに (件数, 見えた枚数, 見えた画素の中央, 隠れた画素の中央, 端に着いた画素の中央)．
#[allow(clippy::type_complexity)]
pub fn summarise_afterimage(
    rows: &[AfterimageRow],
) -> Vec<(i32, u32, usize, usize, f32, f32, f32)> {
    let mut keys: Vec<(i32, u32)> = rows.iter().map(|r| (r.shift, r.trail)).collect();
    keys.sort_unstable();
    keys.dedup();
    keys.into_iter()
        .map(|(shift, trail)| {
            let g: Vec<&AfterimageRow> = rows
                .iter()
                .filter(|r| r.shift == shift && r.trail == trail)
                .collect();
            (
                shift,
                trail,
                g.len(),
                g.iter().filter(|r| r.drawn > 0).count(),
                median(g.iter().map(|r| r.drawn as f32).collect()),
                median(g.iter().map(|r| r.covered as f32).collect()),
                median(g.iter().map(|r| r.saturated as f32).collect()),
            )
        })
        .collect()
}
