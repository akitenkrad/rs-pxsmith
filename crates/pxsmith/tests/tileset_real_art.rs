//! **タイル分割と同値判定を実素材で固定する** (設計書 6.7)．
//!
//! 押さえるのは 2 つ．
//!
//! 1. **切って束ねて組み直すと元の絵に戻る** — 3 モードすべてで，実素材を全件通す．
//!    ここが崩れると縮約は使えない (旗の向きを取り違えると静かに絵が変わる) ．
//! 2. **反転に頼った升にルール 7 が掛かる** — 設計書 6.7 が定めている検出である．
//!
//! > [!warning] **2 は実素材では 1 度も起きない．**
//! > `pxsmith-calib tileset` で測ると，種の 61 枚では 16 画素で反転に頼る升が **0 個**，
//! > 8 画素では 30 個あるが**ルール 7 の測定下限 (64 画素) に構造的に届かない**．
//! > 種は 1 枚 1 スプライトであって «タイルを並べた画面» ではないためである．
//! > **素材が検査を働かせられないので，働くことは組んだ場合で確かめる** —
//! > «実素材で鳴らなかった» を «検査が効いている» と読み替えない．

use std::path::{Path, PathBuf};

use pxsmith_core::canvas::IndexedCanvas;
use pxsmith_core::color::Rgba8;
use pxsmith_core::geom::Mask;
use pxsmith_core::math::Vec2;
use pxsmith_core::palette::{ChromaCurve, Palette};
use pxsmith_core::ramp::{LightPreset, LightSource, build_lighting};
use pxsmith_core::shade::{ShadeOptions, shade_to_canvas};
use pxsmith_core::tileset::{DedupeMode, ExtractOptions, extract, mirror_reliant_cells, rebuild};

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

fn index_exactly(img: &pxsmith_core::canvas::RgbaCanvas) -> Option<(IndexedCanvas, Palette)> {
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

const MODES: [DedupeMode; 3] = [DedupeMode::Exact, DedupeMode::Flip, DedupeMode::FlipRotate];

/// **壊れると: 縮約した絵が静かに変わる．**
///
/// 旗の向き (正規形へ写す変換か，正規形から戻す変換か) を取り違えると，
/// 対称でないタイルだけがずれる — 目で見て気付ける保証は無い．
#[test]
fn extracting_and_rebuilding_reproduces_every_picture_in_every_mode() {
    let mut checked = 0usize;
    for path in png_files() {
        let Ok(img) = pxsmith_io::png::read_rgba(&path) else {
            continue;
        };
        let Some((canvas, _)) = index_exactly(&img) else {
            continue;
        };
        for tile in [8u32, 16] {
            if !canvas.width().is_multiple_of(tile) || !canvas.height().is_multiple_of(tile) {
                continue;
            }
            for mode in MODES {
                let opts = ExtractOptions { tile, mode };
                let (tiles, grid, report) = extract(&canvas, &opts).expect("切れる");
                assert_eq!(
                    report.before,
                    (canvas.width() / tile * canvas.height() / tile) as usize
                );
                assert!(report.after <= report.before);
                let back = rebuild(&tiles, &grid, tile, canvas.transparent()).expect("組み直せる");
                assert_eq!(
                    back.pixels(),
                    canvas.pixels(),
                    "{} を {} 画素 ・{} で切ると絵が戻らない",
                    path.display(),
                    tile,
                    mode.as_str()
                );
                checked += 1;
            }
        }
    }
    assert!(
        checked >= 180,
        "実素材を全件通す (通ったのは {checked} 通り)"
    );
}

/// **壊れると: 同じ入力で束ね方が変わり，差分ビルドの鍵が揺れる**
/// (設計書 6.7 «同点なら変換 ID の最小値» ・6.15 規則 1) ．
#[test]
fn extracting_twice_gives_the_same_tiles_and_the_same_flags() {
    for path in png_files().into_iter().take(20) {
        let Ok(img) = pxsmith_io::png::read_rgba(&path) else {
            continue;
        };
        let Some((canvas, _)) = index_exactly(&img) else {
            continue;
        };
        let opts = ExtractOptions {
            tile: 8,
            mode: DedupeMode::FlipRotate,
        };
        let (ta, ga, ra) = extract(&canvas, &opts).expect("切れる");
        let (tb, gb, rb) = extract(&canvas, &opts).expect("切れる");
        assert_eq!(ra.after, rb.after);
        assert_eq!(ra.oriented, rb.oriented);
        assert_eq!(ra.mirror_reliant, rb.mirror_reliant);
        assert_eq!(ga.tiles(), gb.tiles(), "{} で旗が変わった", path.display());
        for (a, b) in ta.iter().zip(&tb) {
            assert_eq!(a.pixels(), b.pixels());
        }
    }
}

/// **壊れると: «恒等でない向きで置かれた升» を «反転に頼っている» と数え，
/// 陰影の矛盾が起きていない升にルール 7 が掛かる．**
///
/// 実素材で数えると，種の 61 枚には 16 画素で反転に頼る升が 1 つも無い —
/// 一方で正規形の向きが元と違う升は普通にある．**この 2 つを混ぜると，
/// 何も起きていない絵に blocking が出る．**
#[test]
fn real_art_has_oriented_tiles_but_they_are_not_mirror_reliant() {
    let (mut oriented, mut reliant, mut checked) = (0usize, 0usize, 0usize);
    for path in png_files() {
        let Ok(img) = pxsmith_io::png::read_rgba(&path) else {
            continue;
        };
        let Some((canvas, _)) = index_exactly(&img) else {
            continue;
        };
        if !canvas.width().is_multiple_of(16) || !canvas.height().is_multiple_of(16) {
            continue;
        }
        let opts = ExtractOptions {
            tile: 16,
            mode: DedupeMode::Flip,
        };
        let (_, grid, report) = extract(&canvas, &opts).expect("切れる");
        oriented += report.oriented;
        reliant += report.mirror_reliant;
        assert_eq!(report.mirror_reliant, mirror_reliant_cells(&grid).len());
        checked += 1;
    }
    assert!(checked >= 60, "実素材を全件通す (通ったのは {checked} 枚)");
    assert!(
        oriented > 0,
        "正規形の向きが元と違う升があるはずである (この試験の前提)"
    );
    assert_eq!(
        reliant, 0,
        "種は 1 枚 1 スプライトなので反転に頼る升は無いはずである"
    );
}

/// **壊れると: 設計書 6.7 の «反転を有効にしたらルール 7 で検出する» が働かない．**
///
/// **実素材ではこの場面を作れない**ので組んで確かめる — 陰影のあるタイルと
/// その左右反転を並べた絵は，反転で束ねると «反転に頼った升» になり，
/// 宣言した光源と矛盾する．
#[test]
fn a_shaded_tile_next_to_its_mirror_is_caught_by_rule_7() {
    let light = LightSource::Directional {
        dir: Vec2 { x: 0.6, y: -0.8 },
    };
    // 16x16 の «丘» — 4 近傍が揃う画素を 64 以上持たせる
    let mut mask = Mask::new(16, 16);
    for p in mask.bounds().iter() {
        let (dx, dy) = (p.x as f32 - 7.5, p.y as f32 - 7.5);
        if (dx * dx + dy * dy).sqrt() <= 7.0 {
            mask.set(p, true);
        }
    }
    let (ramp, model) = build_lighting(
        Rgba8::rgb(0x8a, 0x6a, 0x4a),
        LightPreset::Clear,
        5,
        ChromaCurve::PeakMiddle,
    )
    .expect("ランプを作れる");
    let (tile, palette) = shade_to_canvas(&mask, light, &model, &ramp, ShadeOptions::default())
        .expect("陰影を導出できる");

    // タイルとその左右反転を横に並べる
    let mut pixels = Vec::with_capacity(32 * 16);
    for y in 0..16i32 {
        for x in 0..16i32 {
            pixels.push(tile.get(x, y).expect("範囲内"));
        }
        for x in 0..16i32 {
            pixels.push(tile.get(15 - x, y).expect("範囲内"));
        }
    }
    let canvas = IndexedCanvas::from_pixels(32, 16, pixels)
        .expect("画素数が合う")
        .with_transparent(tile.transparent());

    // 完全一致では束ねられない — 2 枚のまま
    let exact = extract(
        &canvas,
        &ExtractOptions {
            tile: 16,
            mode: DedupeMode::Exact,
        },
    )
    .expect("切れる")
    .2;
    assert_eq!(exact.after, 2);
    assert_eq!(exact.mirror_reliant, 0);

    // 反転で束ねると 1 枚になり，**反転に頼った升が 1 つ出る**
    let (_, grid, flip) = extract(
        &canvas,
        &ExtractOptions {
            tile: 16,
            mode: DedupeMode::Flip,
        },
    )
    .expect("切れる");
    assert_eq!(flip.after, 1);
    // **2 升とも数える．** 正規形はバイト列最小なので，どちらの升とも違う向き
    // (上下反転側) になりうる — そのとき両方が反転で再現されることになる．
    // そもそも «どちらが元でどちらが鏡像か» は絵からは決まらないので，
    // **対を丸ごと報告して利用者に見せる**のが正しい
    assert_eq!(flip.mirror_reliant, 2);

    // 対のうち**ちょうど一方**が光源と矛盾する — 描いたとおりの側は合っている
    let cells = mirror_reliant_cells(&grid);
    assert_eq!(cells.len(), 2);
    let threshold = pxsmith_lint::LintConfig::default().min_shading_agreement;
    let mut fired = 0usize;
    for (tx, ty) in &cells {
        let mut cell = Vec::with_capacity(256);
        for y in 0..16u32 {
            for x in 0..16u32 {
                cell.push(
                    canvas
                        .get((tx * 16 + x) as i32, (ty * 16 + y) as i32)
                        .expect("範囲内"),
                );
            }
        }
        let cell = IndexedCanvas::from_pixels(16, 16, cell)
            .expect("画素数が合う")
            .with_transparent(canvas.transparent());
        let agreement = pxsmith_lint::rules::shading_agreement(&cell, &palette, light)
            .expect("16x16 なら勾配を測れる (8x8 では測れない)");
        if agreement < threshold {
            fired += 1;
        }
    }
    assert_eq!(
        fired, 1,
        "鏡像の対なのだから，光源と合うのは片方だけのはずである"
    );
}

/// **壊れると: 8x8 のタイルで «鳴らなかった» を «検査した» と読み違える．**
///
/// ルール 7 は 4 近傍がすべて不透明な画素でしか勾配を測れないので，
/// 8x8 は上限が $6 \times 6 = 36$ 画素で下限 64 に**構造的に届かない**．
/// **«測れない» が返ることを固定する** — 0.0 のような値が返ってはいけない．
#[test]
fn an_eight_pixel_tile_can_never_be_measured_by_rule_7() {
    let light = LightSource::Directional {
        dir: Vec2 { x: 0.6, y: -0.8 },
    };
    // 全面不透明の 8x8 — 測れる画素が最大になる形
    let mut mask = Mask::new(8, 8);
    for p in mask.bounds().iter() {
        mask.set(p, true);
    }
    let (ramp, model) = build_lighting(
        Rgba8::rgb(0x8a, 0x6a, 0x4a),
        LightPreset::Clear,
        5,
        ChromaCurve::PeakMiddle,
    )
    .expect("ランプを作れる");
    let (tile, palette) = shade_to_canvas(&mask, light, &model, &ramp, ShadeOptions::default())
        .expect("陰影を導出できる");

    assert!(
        pxsmith_lint::rules::shading_agreement(&tile, &palette, light).is_none(),
        "8x8 で勾配が測れてしまった — 下限の意味が変わっている"
    );
}
