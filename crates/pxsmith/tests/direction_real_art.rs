//! **方向展開を実素材に掛けて «設計書の前提が成り立つか» を測る．**
//!
//! 設計書 4.3 は «自動ミラーで生成したタイルには lint ルール 7 を blocking で
//! 適用する» と定め，実装計画書は «反転 + 陰影再導出» と言う．**どちらも
//! «反転すると矛盾する ・再導出すれば直る» を前提にしている．**
//!
//! ここで固定するのはその 2 つと，反転そのものの性質である．
//! **絵は全件通す** — 間引いた試験は «通った» と言うだけの番人になる (M3 の教訓) ．
//!
//! 光源は 1 つ (斜めの平行光源) に絞ってある．**素材を間引いているのではない** —
//! 鳴るかどうかは «宣言した光源の横成分» だけで決まるので (D96) ，プリセットを
//! 増やしても同じことを 4 回測るだけである (`pxsmith-calib direction` で確かめてある) ．

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use pxsmith_core::canvas::IndexedCanvas;
use pxsmith_core::color::Rgba8;
use pxsmith_core::direction::{
    Direction, ExpandMode, ExpandOptions, ReshadeSpec, expand, mirror_canvas, mirror_frame,
    mirror_is_checkable,
};
use pxsmith_core::frame::{Frame, FrameKind, Layer, LayerMeta, Surface};
use pxsmith_core::geom::Mask;
use pxsmith_core::math::{Vec2, uvec2};
use pxsmith_core::palette::{ChromaCurve, Palette};
use pxsmith_core::ramp::{LightPreset, LightSource, build_lighting};
use pxsmith_core::shade::{ShadeOptions, shade_to_canvas};

/// 斜めの平行光源．**横成分が 0.474 を超えている**ので反転で矛盾が出る．
fn diagonal() -> LightSource {
    LightSource::Directional {
        dir: Vec2 { x: 0.6, y: -0.8 },
    }
}

const BASE: Rgba8 = Rgba8::rgb(0x8a, 0x6a, 0x4a);

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

fn mask_of(canvas: &IndexedCanvas) -> Mask {
    let mut m = Mask::new(canvas.width(), canvas.height());
    for p in canvas.bounds().iter() {
        if !canvas.is_transparent_at(p) {
            m.set(p, true);
        }
    }
    m
}

fn frame_of(canvas: IndexedCanvas, palette: Palette) -> Frame {
    Frame {
        size: uvec2(canvas.width(), canvas.height()),
        layers: vec![Layer::new(
            LayerMeta {
                name: "art".to_string(),
                ..LayerMeta::default()
            },
            Surface::Indexed(canvas),
        )],
        palette,
        duration_ms: 100,
        kind: FrameKind::Key,
    }
}

/// 実素材のシルエットへ陰影を導出した «`pxsmith shade` の出力» を作る．
fn shaded_frame(path: &Path) -> Option<Frame> {
    let img = pxsmith_io::png::read_rgba(path).ok()?;
    let (canvas, _palette) = index_exactly(&img)?;
    let (ramp, model) =
        build_lighting(BASE, LightPreset::Clear, 5, ChromaCurve::PeakMiddle).ok()?;
    let (shaded, shaded_palette) = shade_to_canvas(
        &mask_of(&canvas),
        diagonal(),
        &model,
        &ramp,
        ShadeOptions::default(),
    )
    .ok()?;
    Some(frame_of(shaded, shaded_palette))
}

fn lint_rule_7(frame: &Frame, light: LightSource) -> usize {
    let cfg = pxsmith_lint::LintConfig {
        light: Some(light),
        ..pxsmith_lint::LintConfig::default()
    };
    pxsmith_lint::lint_frame(frame, &cfg)
        .blocking()
        .filter(|v| v.rule == 7)
        .count()
}

/// **壊れると: 反転が色やシルエットを変える．**
///
/// 反転は添字を写すだけなので，色数も画素の集合も変わってはいけない．
/// ここが崩れると «反転が矛盾を作ったのか，反転が絵を壊したのか» を分けられない．
#[test]
fn mirroring_real_art_changes_no_colour_and_undoes_itself() {
    let mut checked = 0usize;
    for path in png_files() {
        let Ok(img) = pxsmith_io::png::read_rgba(&path) else {
            continue;
        };
        let Some((canvas, palette)) = index_exactly(&img) else {
            continue;
        };
        let flipped = mirror_canvas(&canvas);

        // 添字の多重集合が変わらない (色を作らない ・落とさない)
        let (mut a, mut b) = (canvas.pixels().to_vec(), flipped.pixels().to_vec());
        a.sort_unstable();
        b.sort_unstable();
        assert_eq!(a, b, "{} で反転が画素の集合を変えた", path.display());

        // 2 度反転すれば元に戻る
        assert_eq!(
            mirror_canvas(&flipped).pixels(),
            canvas.pixels(),
            "{} で反転が対合になっていない",
            path.display()
        );

        // 反転は左右を入れ替えている (何もしていないのではない)
        let w = canvas.width() as i32;
        for p in canvas.bounds().iter() {
            assert_eq!(
                flipped.get(p.x, p.y),
                canvas.get(w - 1 - p.x, p.y),
                "{} の ({},{}) が写っていない",
                path.display(),
                p.x,
                p.y
            );
        }
        let _ = &palette;
        checked += 1;
    }
    assert!(checked >= 60, "実素材を全件通す (通ったのは {checked} 枚)");
}

/// **壊れると: 設計書 4.3 の «自動ミラーにルール 7 を blocking» が空振りする．**
///
/// 横成分のある光源を宣言した `pxsmith shade` の出力は，反転すると**必ず**矛盾する．
/// ここが 1 枚でも通ると，反転したタイルが黙って出荷されうる．
#[test]
fn mirroring_shaded_art_always_contradicts_a_declared_diagonal_light() {
    let (mut checked, mut caught) = (0usize, 0usize);
    for path in png_files() {
        let Some(frame) = shaded_frame(&path) else {
            continue;
        };
        // 元の絵は鳴らない
        assert_eq!(
            lint_rule_7(&frame, diagonal()),
            0,
            "{} は反転する前から鳴っている — 測り方が壊れている",
            path.display()
        );
        let flipped = mirror_frame(&frame).expect("反転できる");
        checked += 1;
        caught += usize::from(lint_rule_7(&flipped, diagonal()) > 0);
    }
    assert!(checked >= 60, "実素材を全件通す (通ったのは {checked} 枚)");
    assert_eq!(
        caught, checked,
        "反転した {checked} 枚のうち {caught} 枚しか捕まえていない"
    );
}

/// **壊れると: 実装計画書の «反転 + 陰影再導出» が直っていない．**
///
/// 再導出したものは 1 枚も鳴ってはいけない．
#[test]
fn reshading_after_a_mirror_clears_rule_7_on_every_picture() {
    let mut checked = 0usize;
    for path in png_files() {
        let Some(frame) = shaded_frame(&path) else {
            continue;
        };
        let mut drawn = BTreeMap::new();
        drawn.insert(Direction::E, vec![frame]);
        let opts = ExpandOptions {
            mode: ExpandMode::Reshade(Box::new(ReshadeSpec {
                base: BASE,
                preset: LightPreset::Clear,
                steps: 5,
                curve: ChromaCurve::PeakMiddle,
                shade: ShadeOptions::default(),
            })),
        };
        let (all, report) = expand(&drawn, &opts).expect("展開できる");
        assert_eq!(report.generated.len(), 1);
        assert_eq!(report.generated[0].direction, Direction::W);

        for frame in &all[&Direction::W] {
            assert_eq!(
                lint_rule_7(frame, LightPreset::Clear.default_source()),
                0,
                "{} は再導出しても鳴っている",
                path.display()
            );
        }
        checked += 1;
    }
    assert!(checked >= 60, "実素材を全件通す (通ったのは {checked} 枚)");
}

/// **壊れると: 矛盾が起きない光源まで «検査した» ことにして，
/// 見逃していないものを見逃しとして数える．**
///
/// 反転で裏返るのは $x$ 成分だけなので，真上からの光では矛盾が起きない．
/// 境目は閾値から代数的に決まる (校正の対象ではない) ．
#[test]
fn a_light_without_a_horizontal_component_makes_mirroring_consistent() {
    let straight = LightSource::Directional {
        dir: Vec2 { x: 0.0, y: -1.0 },
    };
    let threshold = pxsmith_lint::LintConfig::default().min_shading_agreement;
    assert!(!mirror_is_checkable(straight, threshold));
    assert!(mirror_is_checkable(diagonal(), threshold));

    // 実素材で «鳴らないこと» まで確かめる — 代数が絵の上でも成り立つか
    let mut checked = 0usize;
    for path in png_files() {
        let Ok(img) = pxsmith_io::png::read_rgba(&path) else {
            continue;
        };
        let Some((canvas, _)) = index_exactly(&img) else {
            continue;
        };
        let Ok((ramp, model)) =
            build_lighting(BASE, LightPreset::Clear, 5, ChromaCurve::PeakMiddle)
        else {
            continue;
        };
        let Ok((shaded, palette)) = shade_to_canvas(
            &mask_of(&canvas),
            straight,
            &model,
            &ramp,
            ShadeOptions::default(),
        ) else {
            continue;
        };
        let frame = frame_of(shaded, palette);
        let flipped = mirror_frame(&frame).expect("反転できる");
        assert_eq!(
            lint_rule_7(&flipped, straight),
            0,
            "{} で真上からの光なのに反転が矛盾と判定された",
            path.display()
        );
        checked += 1;
    }
    assert!(checked >= 60, "実素材を全件通す (通ったのは {checked} 枚)");
}
