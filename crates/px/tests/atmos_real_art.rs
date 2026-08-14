//! **`px atmos` を実素材に掛けて «壊していないか» を測る．**
//!
//! 空気遠近法は «並べ替えるだけ» の道具である (D94 の合成 ・D112 の梱包と同じ
//! 性質) ．したがって不変条件を実素材で固定する — 合成した形では捕まらない
//! (M3 の教訓 ・D81 の «端から端まで») ．
//!
//! | 不変条件 | 壊れると |
//! | --- | --- |
//! | パレットが 1 項目も変わらない | 霞ませたつもりで色が増える (D94) |
//! | 透明な画素の集合が 1 画素も動かない | シルエットが霞んで背景が透ける |
//! | 明度の幅が広がらない | «明暗差を落とす» と言いながら上げている |
//! | 奥へ行くほど空に近づく | 逆の空気遠近法になる |
//! | `moved + no_step == colors` | 数え落としがある (D128) |
//!
//! **飛ばした件も数える** — 256 色を超えて添字にできない絵があるので，
//! «全件通った» と «飛ばしたので通った» を分ける (D128) ．

use std::path::{Path, PathBuf};

use px_core::atmos::{AtmosOptions, AtmosReport, HazeTable, atmos};
use px_core::canvas::{IndexedCanvas, RgbaCanvas};
use px_core::color::{Rgba8, distance_sq, oklab_of};
use px_core::frame::{Depth, Frame, Layer, LayerMeta, Surface};
use px_core::math::uvec2;
use px_core::palette::Palette;

/// 測る空の色．`palettes/sweetie-16.hex` (CC0) から取った晴天 ・曇天 ・夕方．
const SKIES: [&str; 3] = ["41a6f6", "f4f4f4", "ef7d57"];

fn seeds() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/grid-eval/seeds")
        .canonicalize()
        .expect("種の置き場所がある")
}

fn png_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("読める")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")))
        .collect();
    v.sort();
    v
}

/// **その場の量子化を挟まない** — 使っている色をそのままパレットにする．
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

fn frame_of(canvas: IndexedCanvas, palette: Palette) -> Frame {
    let mut f = Frame::new(uvec2(canvas.width(), canvas.height()), palette);
    f.layers.push(Layer::new(
        LayerMeta::named("art"),
        Surface::Indexed(canvas),
    ));
    f
}

fn opaque_mask(f: &Frame) -> Vec<bool> {
    let c = f.layers[0].surface.as_indexed().expect("添字の画布");
    let transparent = c.transparent();
    c.pixels()
        .iter()
        .map(|i| transparent != Some(*i) && f.palette.get(*i).is_some_and(|c| c.a != 0))
        .collect()
}

/// 使っている色の «空からの距離» の平均 (画素で重み付け)．
fn mean_distance_to_sky(f: &Frame, sky: Rgba8) -> f32 {
    let c = f.layers[0].surface.as_indexed().expect("添字の画布");
    let lab_sky = oklab_of(sky);
    let (mut sum, mut n) = (0.0f32, 0usize);
    for &i in c.pixels() {
        if c.transparent() == Some(i) || f.palette.get(i).is_some_and(|c| c.a == 0) {
            continue;
        }
        if let Some(lab) = f.palette.lab_of(i) {
            sum += distance_sq(lab, lab_sky, 1.0).sqrt();
            n += 1;
        }
    }
    if n == 0 { 0.0 } else { sum / n as f32 }
}

#[test]
fn atmos_keeps_every_invariant_on_real_art() {
    let files = png_files(&seeds());
    assert!(files.len() >= 60, "実素材が足りない: {}", files.len());

    let (mut checked, mut skipped_not_indexable, mut skipped_empty) = (0usize, 0usize, 0usize);
    let mut ineffective = 0usize;

    for path in &files {
        let Ok(img) = px_io::png::read_rgba(path) else {
            skipped_not_indexable += 1;
            continue;
        };
        let Some((canvas, palette)) = index_exactly(&img) else {
            skipped_not_indexable += 1;
            continue;
        };
        let before = frame_of(canvas, palette.clone());
        if opaque_mask(&before).iter().all(|b| !b) {
            skipped_empty += 1;
            continue;
        }

        for hex in SKIES {
            let sky = Rgba8::from_hex_str(hex).expect("空の色");
            let haze = HazeTable {
                foreground: 0.0,
                midground: 0.3,
                background: 0.6,
            };
            let opts = AtmosOptions {
                sky,
                haze,
                ..Default::default()
            };

            let mut previous = mean_distance_to_sky(&before, sky);
            let mut reports: Vec<AtmosReport> = Vec::new();

            for depth in [Depth::Foreground, Depth::Midground, Depth::Background] {
                let (out, report) =
                    atmos(std::slice::from_ref(&before), depth, &opts).expect("掛かる");
                let after = &out[0];

                // 1. 色を作らない
                assert_eq!(
                    after.palette.entries(),
                    palette.entries(),
                    "{}: パレットが変わった",
                    path.display()
                );

                // 2. シルエットが動かない
                assert_eq!(
                    opaque_mask(after),
                    opaque_mask(&before),
                    "{}: 透明な画素の集合が動いた",
                    path.display()
                );

                // 3. 明暗差は広がらない
                assert!(
                    report.spread.1 <= report.spread.0 + 1e-5,
                    "{}: 明度の幅が広がった {:?}",
                    path.display(),
                    report.spread
                );

                // 4. 奥へ行くほど空に近づく
                let now = mean_distance_to_sky(after, sky);
                assert!(
                    now <= previous + 1e-5,
                    "{}: {} で空から遠ざかった ({previous} -> {now})",
                    path.display(),
                    depth.as_str()
                );
                previous = now;

                // 5. 数え落としが無い
                if report.amount > 0.0 {
                    assert_eq!(
                        report.moved + report.no_step,
                        report.colors,
                        "{}: 数えていない色がある",
                        path.display()
                    );
                } else {
                    assert_eq!(report.moved, 0);
                    assert_eq!(
                        report.no_step, 0,
                        "«霞ませなかった» を «段が無い» と数えない"
                    );
                }

                if report.ineffective() && report.amount > 0.0 {
                    ineffective += 1;
                }
                reports.push(report);
            }

            // 手前は基準なので 1 画素も動かない
            assert_eq!(reports[0].pixels, 0, "{}: 手前が霞んだ", path.display());
        }
        checked += 1;
    }

    println!(
        "実素材 {checked} 枚 x 空 {} 色を通した (飛ばした: 添字にできない {skipped_not_indexable} ・不透明画素が無い {skipped_empty}) ．\
         1 色も動かなかった (絵, 空, 段) は {ineffective} 件",
        SKIES.len()
    );
    assert!(checked >= 60, "通した枚数が少なすぎる: {checked}");
}

/// **壊れると: «霞ませた» と言いながら元の色から離れた色を選ぶ．**
///
/// 選んだ先は必ず «元の色と空を結ぶ線» から許容以内にある — これが
/// «寄せる» の定義そのものである (制約なしの最近傍は実素材で 59.0% 外れる)．
#[test]
fn every_replacement_stays_on_the_line_towards_the_sky() {
    let files = png_files(&seeds());
    let tolerance = AtmosOptions::DEFAULT_TOLERANCE;
    let mut checked = 0usize;

    for path in &files {
        let Ok(img) = px_io::png::read_rgba(path) else {
            continue;
        };
        let Some((canvas, palette)) = index_exactly(&img) else {
            continue;
        };
        let before = frame_of(canvas, palette.clone());
        let sky = Rgba8::from_hex_str(SKIES[0]).expect("空の色");
        let lab_sky = oklab_of(sky);
        let opts = AtmosOptions {
            sky,
            haze: HazeTable {
                foreground: 0.0,
                midground: 0.3,
                background: 0.6,
            },
            ..Default::default()
        };
        let (out, _) =
            atmos(std::slice::from_ref(&before), Depth::Background, &opts).expect("掛かる");

        let a = before.layers[0].surface.as_indexed().expect("添字の画布");
        let b = out[0].layers[0].surface.as_indexed().expect("添字の画布");
        for (x, y) in a.pixels().iter().zip(b.pixels()) {
            if x == y {
                continue;
            }
            let (Some(lx), Some(ly)) = (palette.lab_of(*x), palette.lab_of(*y)) else {
                panic!("パレットに無い添字")
            };
            let span = distance_sq(lx, lab_sky, 1.0).sqrt();
            let da = distance_sq(ly, lx, 1.0).sqrt();
            let db = distance_sq(ly, lab_sky, 1.0).sqrt();
            assert!(
                db < span,
                "{}: 添字 {x} -> {y} が空から遠ざかっている",
                path.display()
            );
            assert!(
                da + db - span <= tolerance + 1e-5,
                "{}: 添字 {x} -> {y} が線から {:.4} 外れている (許容 {tolerance})",
                path.display(),
                da + db - span
            );
        }
        checked += 1;
    }
    assert!(checked >= 60, "通した枚数が少なすぎる: {checked}");
}
