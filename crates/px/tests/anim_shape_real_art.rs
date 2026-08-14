//! **M4 の残り 5 件を実素材に掛けて «壊していないか» を測る．**
//!
//! 合成した形では捕まらない壊れ方があるので，実素材を全件通す (M3 の教訓) ．
//!
//! | 道具 | 不変条件 |
//! | --- | --- |
//! | `smear` | **場のままの掃引は union と 1 画素も違わない** (包含定理) ．重心を取り除けば繋がる |
//! | `extrapolate` | **平行移動なら真値と画素単位一致する** |
//! | `squash` | 体積の誤差は丸めの範囲．**画布を広げれば 1 画素も切れない** |
//! | `subpixel` | **パレットの外へ出ない**．接線法はシルエットを動かさない |
//! | `afterimage` | **現在の絵を 1 画素も書き換えない**．無い色は作らない |

use std::path::{Path, PathBuf};

use px_core::afterimage::{AfterimageOptions, afterimage};
use px_core::canvas::{IndexedCanvas, RgbaCanvas};
use px_core::color::Rgba8;
use px_core::deform::{SquashOptions, VolumeRule, squash};
use px_core::frame::{Frame, Layer, LayerMeta, Surface};
use px_core::geom::{Mask, label_mask};
use px_core::math::{IVec2, ivec2};
use px_core::palette::{ChromaCurve, Palette, Ramp};
use px_core::smear::{SmearMethod, SmearOptions, covers_ends, smear_mask};
use px_core::subpixel::{SubpixelMethod, SubpixelOptions, SubpixelReport, SubpixelScope, subpixel};
use px_core::tween::{ExtrapolateKind, TweenAlign, TweenOptions, extrapolate_mask};

fn seeds() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/grid-eval/seeds")
        .canonicalize()
        .expect("種の置き場所がある")
}

fn png_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(seeds())
        .expect("種を読める")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")))
        .collect();
    files.sort();
    files
}

fn silhouette(path: &Path, pad: u32) -> Option<Mask> {
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

/// **その場の量子化を挟まずに指標へ落とす**．256 色を超える絵は飛ばす．
fn index_exactly(img: &RgbaCanvas) -> Option<(IndexedCanvas, Palette)> {
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

fn indexed(path: &Path) -> Option<(IndexedCanvas, Palette)> {
    index_exactly(&px_io::png::read_rgba(path).ok()?)
}

fn shifted(m: &Mask, d: IVec2) -> Mask {
    let mut out = Mask::new(m.width(), m.height());
    for p in m.iter_set() {
        out.set(p + d, true);
    }
    out
}

// ------------------------------------------------------------------ おばけ

/// **壊れると: «掃引は union より良い» という誤った前提で重心合わせを外す．**
///
/// 6.9 の包含定理から $\bigcup_t R_t = A \cup B$ が出る．代数なので実素材でも
/// 1 画素の違いも出てはいけない — **出たら包含定理か掃引のどちらかが壊れている**．
#[test]
fn the_plain_sweep_never_differs_from_the_union_on_real_art() {
    let mut checked = 0usize;
    for path in png_files() {
        for dx in [8i32, 24] {
            let Some(a) = silhouette(&path, dx.unsigned_abs() + 2) else {
                continue;
            };
            let b = shifted(&a, ivec2(dx, 0));
            let plain = smear_mask(
                &a,
                &b,
                &SmearOptions {
                    align: TweenAlign::None,
                    ..Default::default()
                },
            )
            .expect("掃引");
            let union = smear_mask(
                &a,
                &b,
                &SmearOptions {
                    method: SmearMethod::Union,
                    ..Default::default()
                },
            )
            .expect("和集合");
            let differing = plain
                .mask
                .bounds()
                .iter()
                .filter(|p| plain.mask.get(*p) != union.mask.get(*p))
                .count();
            assert_eq!(
                differing,
                0,
                "{} のずらし {dx} で {differing} 画素違う",
                path.display()
            );
            checked += 1;
        }
    }
    assert!(checked >= 100, "実素材を通せていない ({checked} 件)");
}

/// **壊れると: 速い動きでおばけが 2 塊のままになる (道具の存在理由が消える)．**
///
/// **両端を含むこと**も同時に見る — 含まなければ符号の規約が逆である．
#[test]
fn the_aligned_sweep_connects_where_the_union_falls_apart() {
    let (mut split_union, mut split_aligned, mut checked) = (0usize, 0usize, 0usize);
    // **1 枚おきに取る．** 掃引は 1 件あたり 33 回の距離場計算になるので，全件だと
    // 試験だけで数分掛かる．**間引いたことをここに書いておく** (黙って減らさない)
    for path in png_files().into_iter().step_by(2) {
        let dx = 32i32;
        let Some(a) = silhouette(&path, dx.unsigned_abs() + 2) else {
            continue;
        };
        // 元の絵が既に千切れているものは «繋がるか» を問えない
        if label_mask(&a, false).len() != 1 {
            continue;
        }
        let b = shifted(&a, ivec2(dx, 0));
        let union = smear_mask(
            &a,
            &b,
            &SmearOptions {
                method: SmearMethod::Union,
                ..Default::default()
            },
        )
        .expect("和集合");
        let aligned = smear_mask(&a, &b, &SmearOptions::default()).expect("掃引");
        assert!(
            covers_ends(&a, &b, &aligned.mask),
            "{} で両端を含んでいない",
            path.display()
        );
        if union.components.2 > 1 {
            split_union += 1;
        }
        if !aligned.connects() {
            split_aligned += 1;
        }
        checked += 1;
    }
    assert!(checked >= 25, "実素材を通せていない ({checked} 件)");
    assert!(
        split_union > checked / 2,
        "ずらし 32 で union が繋がってしまう — 試験が意味を失っている ({split_union} / {checked})"
    );
    assert_eq!(
        split_aligned, 0,
        "重心を取り除いても {split_aligned} 枚が繋がらない"
    );
}

// -------------------------------------------------------------------- 外挿

/// **壊れると: 予備動作 / オーバーシュートが «それらしいが違う» 絵になる．**
///
/// 平行移動には真値がある — $t$ 倍だけ動かした絵である．**画素単位で一致する
/// ことを求める** (D114 が中割りで確かめたのと同じ強さ) ．
#[test]
fn extrapolating_a_translation_lands_on_the_truth_exactly() {
    let mut checked = 0usize;
    for path in png_files() {
        for dx in [8i32, 16] {
            for (kind, amount) in [
                (ExtrapolateKind::Anticipation, 0.5f32),
                (ExtrapolateKind::Overshoot, 0.5),
                (ExtrapolateKind::Overshoot, 1.0),
            ] {
                let t = kind.t_for(amount);
                let pad = (dx as f32 * t).abs().ceil() as u32 + dx.unsigned_abs() + 2;
                let Some(a) = silhouette(&path, pad) else {
                    continue;
                };
                let b = shifted(&a, ivec2(dx, 0));
                let truth = shifted(&a, ivec2((dx as f32 * t).round() as i32, 0));
                let got =
                    extrapolate_mask(&a, &b, kind, amount, &TweenOptions::default()).expect("外挿");
                assert_eq!(got.clipped, 0, "{} で画布の外へ出た", path.display());
                let differing = got
                    .mask
                    .bounds()
                    .iter()
                    .filter(|p| got.mask.get(*p) != truth.get(*p))
                    .count();
                assert_eq!(
                    differing,
                    0,
                    "{} の {} {amount} で {differing} 画素違う",
                    path.display(),
                    kind.as_str()
                );
                checked += 1;
            }
        }
    }
    assert!(checked >= 300, "実素材を通せていない ({checked} 件)");
}

// -------------------------------------------------------------------- 潰し

/// **壊れると: 潰しが画布の外で絵を捨て，«体積を保った» と報告する．**
///
/// 実素材は縁に接している絵が多いので (D93: 38 枚中 33 枚) ，広げないと必ず切れる．
#[test]
fn growing_the_canvas_is_what_keeps_the_squash_from_cutting_the_art() {
    let (mut cut_tight, mut cut_grown, mut checked) = (0usize, 0usize, 0usize);
    for path in png_files() {
        let Some((canvas, _)) = indexed(&path) else {
            continue;
        };
        for amount in [-0.5f32, -0.25, 0.25, 0.5] {
            let tight = squash(
                &canvas,
                amount,
                &SquashOptions {
                    grow: false,
                    ..Default::default()
                },
            )
            .expect("潰し");
            let grown = squash(&canvas, amount, &SquashOptions::default()).expect("潰し");
            if tight.1.clipped > 0 {
                cut_tight += 1;
            }
            if grown.1.clipped > 0 {
                cut_grown += 1;
            }
            checked += 1;
        }
    }
    assert!(checked >= 100, "実素材を通せていない ({checked} 件)");
    assert!(
        cut_tight > checked * 3 / 4,
        "広げない側が切れていない — 試験が意味を失っている ({cut_tight} / {checked})"
    );
    assert_eq!(cut_grown, 0, "広げても {cut_grown} 通りで切れた");
}

/// **壊れると: 体積保存の «決め方» を変えても同じだと思い込む．**
///
/// 幅を体積の式から引く方 (`Derived`) が，倍率をそのまま当てる方より良い．
/// **どちらも 0 にはならない** — 画素が整数だからである．
#[test]
fn the_derived_rule_beats_the_independent_one_on_real_art() {
    let (mut derived, mut independent, mut checked) = (0.0f64, 0.0f64, 0usize);
    let (mut worst_derived, mut added) = (0.0f32, 0usize);
    for path in png_files() {
        let Some((canvas, palette)) = indexed(&path) else {
            continue;
        };
        for amount in [-0.5f32, -0.25, 0.25, 0.5] {
            for rule in [VolumeRule::Derived, VolumeRule::Independent] {
                let (out, r) = squash(
                    &canvas,
                    amount,
                    &SquashOptions {
                        rule,
                        ..Default::default()
                    },
                )
                .expect("潰し");
                // **拡縮は最近傍なので，パレットの外の色は出しようがない** (D94)
                assert!(
                    !SubpixelReport::escapes_palette(&out, &palette),
                    "{} でパレットの外の添字が出た",
                    path.display()
                );
                if r.colors.1 > r.colors.0 {
                    added += 1;
                }
                match rule {
                    VolumeRule::Derived => {
                        derived += r.volume_error() as f64;
                        worst_derived = worst_derived.max(r.volume_error());
                    }
                    VolumeRule::Independent => independent += r.volume_error() as f64,
                }
            }
            checked += 1;
        }
    }
    assert!(checked >= 100, "実素材を通せていない ({checked} 件)");
    assert_eq!(added, 0, "最近傍の拡縮が添字を増やした ({added} 通り)");
    assert!(
        derived < independent,
        "体積の式から引く方が良くない (derived {derived:.3} 対 independent {independent:.3})"
    );
    assert!(
        worst_derived > 0.0,
        "誤差 0 — 画素が整数である以上ありえないので，測り方が壊れている"
    );
}

// ------------------------------------------------------------ サブピクセル

/// **壊れると: サブピクセルがパレットの外の色を出す (ルール 1 のパレット逸脱)．**
///
/// > [!note] «使った添字の数が増えないこと» は不変条件ではない．
/// > 中間色を置くとは «パレットの中の，まだ使っていない色を使い始める» ことで
/// > あって，予備の色があるパレットでは増えるのが正しい．
#[test]
fn no_subpixel_method_ever_leaves_the_palette() {
    let mut checked = 0usize;
    for path in png_files() {
        let Some((canvas, palette)) = indexed(&path) else {
            continue;
        };
        for method in [SubpixelMethod::Tangent, SubpixelMethod::Fast] {
            let (out, _) = subpixel(
                &canvas,
                &palette,
                &SubpixelOptions {
                    method,
                    ..Default::default()
                },
            )
            .expect("生成");
            assert!(
                !SubpixelReport::escapes_palette(&out, &palette),
                "{} の {} でパレットの外の添字が出た",
                path.display(),
                method.as_str()
            );
            checked += 1;
        }
    }
    assert!(checked >= 100, "実素材を通せていない ({checked} 件)");
}

/// **壊れると: サブピクセルが形を動かし，«半画素の錯覚» ではなく本当の移動になる．**
///
/// 接線法は色を渡すだけなのでシルエットが動いてはいけない．**高速法は動く** —
/// 200% 拡大して 1 画素ずらす以上どうしようもないので，そちらは
/// «動くこと» の方を固定する (D39 の但し書き) ．
#[test]
fn the_tangent_method_never_moves_the_silhouette_but_the_fast_one_does() {
    let moved = |a: &IndexedCanvas, b: &IndexedCanvas| -> usize {
        let mut n = 0usize;
        for y in 0..a.height() as i32 {
            for x in 0..a.width() as i32 {
                let p = ivec2(x, y);
                if a.is_transparent_at(p) != b.is_transparent_at(p) {
                    n += 1;
                }
            }
        }
        n
    };
    let (mut tangent_moved, mut fast_moved, mut checked) = (0usize, 0usize, 0usize);
    for path in png_files() {
        let Some((canvas, palette)) = indexed(&path) else {
            continue;
        };
        for (method, scope) in [
            (SubpixelMethod::Tangent, SubpixelScope::Silhouette),
            (SubpixelMethod::Tangent, SubpixelScope::Colours),
        ] {
            let (out, _) = subpixel(
                &canvas,
                &palette,
                &SubpixelOptions {
                    method,
                    scope,
                    ..Default::default()
                },
            )
            .expect("生成");
            tangent_moved += moved(&canvas, &out);
        }
        let (out, _) = subpixel(
            &canvas,
            &palette,
            &SubpixelOptions {
                method: SubpixelMethod::Fast,
                ..Default::default()
            },
        )
        .expect("生成");
        if moved(&canvas, &out) > 0 {
            fast_moved += 1;
        }
        checked += 1;
    }
    assert!(checked >= 50, "実素材を通せていない ({checked} 件)");
    assert_eq!(
        tangent_moved, 0,
        "接線法がシルエットを {tangent_moved} 画素動かした"
    );
    assert!(
        fast_moved > 0,
        "高速法がシルエットを動かしていない — «動く» ことを試験で固定できていない"
    );
}

// -------------------------------------------------------------------- 残像

/// **壊れると: 残像が主体を書き換える，または無い色を作る．**
///
/// 実素材のシルエットを 3 コマ動かして測る．**動きが小さいと 1 画素も見えない**
/// のは失敗ではなく結果なので，そこも一緒に固定する．
#[test]
fn the_trail_stays_behind_the_subject_and_invents_nothing() {
    use px_core::ramp::{LightPreset, build_lighting};
    use px_core::shade::{ShadeOptions, shade_to_canvas};

    let base = Rgba8::new(0x8a, 0x6a, 0x4a, 255);
    let (shade_palette, model) =
        build_lighting(base, LightPreset::Clear, 5, ChromaCurve::PeakMiddle).expect("ランプ");
    let source = LightPreset::default_source(LightPreset::Clear);

    let (mut invisible_slow, mut visible_fast, mut checked) = (0usize, 0usize, 0usize);
    for path in png_files() {
        for dx in [1i32, 8] {
            let Some(a) = silhouette(&path, dx.unsigned_abs() * 3 + 2) else {
                continue;
            };
            let mut frames: Vec<Frame> = Vec::new();
            for k in 0..3i32 {
                let m = shifted(&a, ivec2(dx * k, 0));
                let Ok((canvas, palette)) =
                    shade_to_canvas(&m, source, &model, &shade_palette, ShadeOptions::default())
                else {
                    break;
                };
                let mut f = Frame::new(m.size(), palette);
                f.layers.push(Layer::new(
                    LayerMeta::named("art"),
                    Surface::Indexed(canvas),
                ));
                frames.push(f);
            }
            if frames.len() < 3 {
                continue;
            }
            let ramp: Ramp = model.key.clone();
            // **長さ 1 で測る．** 3 コマの列では長さ 2 以上だと «前の前» まで
            // 拾ってしまい，1 画素の動きでも見えてしまう (px-calib で測った表)
            let (out, r) = afterimage(
                &frames,
                &ramp,
                &AfterimageOptions {
                    trail: 1,
                    ..Default::default()
                },
            )
            .expect("残像");

            for (before, after) in frames.iter().zip(&out) {
                let (b, c) = (
                    before.layers[0].surface.as_indexed().expect("指標"),
                    after.layers[0].surface.as_indexed().expect("指標"),
                );
                // **主体は 1 画素も変わらない**
                for y in 0..b.height() as i32 {
                    for x in 0..b.width() as i32 {
                        let p = ivec2(x, y);
                        if !b.is_transparent_at(p) {
                            assert_eq!(
                                b.get_at(p),
                                c.get_at(p),
                                "{} ({x},{y}) の主体が変わった",
                                path.display()
                            );
                        }
                    }
                }
                // **パレットの外の色は作らない**
                assert!(
                    !SubpixelReport::escapes_palette(c, &after.palette),
                    "{} でパレットの外の添字が出た",
                    path.display()
                );
            }
            if dx == 1 && r.invisible() {
                invisible_slow += 1;
            }
            if dx == 8 && !r.invisible() {
                visible_fast += 1;
            }
            checked += 1;
        }
    }
    assert!(checked >= 100, "実素材を通せていない ({checked} 件)");
    assert!(
        invisible_slow > 0,
        "1 画素の動きで «見えない» が 1 件も出ない — 報告が意味を失っている"
    );
    assert!(visible_fast > 0, "8 画素動かしても残像が 1 枚も見えない");
}
