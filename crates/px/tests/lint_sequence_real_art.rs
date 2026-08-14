//! **道具が作った列が，自分のフレーム間検査に落ちないこと** (設計書 7.3 の 22 〜 27)．
//!
//! `px shade` の自己整合性試験 (D58) と同じ性質の検査である．落ちた場合の直し方も
//! 同じ — **道具を正としてルールの適用範囲を直す**．逆向きの修正は禁じる．
//!
//! ここで通す列は 3 通り．
//!
//! | 列 | 作り方 | 掛かるべきでないルール |
//! | --- | --- | --- |
//! | 剛体の平行移動 | 実素材を整数画素ずつずらす | **全部** (何も壊れていない) |
//! | `px anim tween` | 中割り 3 枚 | 23 ・24 ・25 (22 は列しだい) |
//! | `px anim squash` | 潰しと伸ばし | **23 ・25** — 変形はルール 27 の持ち場である |
//!
//! **飛ばした件も数える** (D128) ．種 64 枚のうち通るのは **35 枚**である —
//! 残りは «透明の宣言が無い» (全面が絵のタイル．26 枚) と «256 色を超えて添字に
//! できない» (3 枚) ．列の検査はシルエットの出入りを見るので，**透明が無い絵は
//! そもそも動かしようがない**．«全件通った» と «飛ばしたので通った» を分ける．

use std::path::{Path, PathBuf};

use px_core::canvas::{IndexedCanvas, RgbaCanvas};
use px_core::color::Rgba8;
use px_core::deform::{SquashOptions, squash};
use px_core::frame::{Frame, FrameKind, Layer, LayerMeta, Surface};
use px_core::geom::Mask;
use px_core::math::{IRect, IVec2, ivec2, uvec2};
use px_core::palette::Palette;
use px_core::tween::{TweenAlign, TweenOptions, tween_series};
use px_lint::rules::LintConfig;
use px_lint::{Report, lint_sequence};

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
        .map(|i| i as u8)?;
    let pixels: Vec<u8> = img
        .pixels()
        .iter()
        .map(|c| match c.a {
            0 => transparent,
            _ => palette.nearest(*c, 1.0).unwrap_or(0),
        })
        .collect();
    let canvas = IndexedCanvas::from_pixels(img.width(), img.height(), pixels)
        .ok()?
        .with_transparent(Some(transparent));
    Some((canvas, palette))
}

fn pad(canvas: &IndexedCanvas, margin: u32) -> IndexedCanvas {
    let fill = canvas.transparent().unwrap_or(0);
    canvas.crop(
        IRect {
            x: -(margin as i32),
            y: -(margin as i32),
            w: canvas.width() + margin * 2,
            h: canvas.height() + margin * 2,
        },
        fill,
    )
}

fn shift(canvas: &IndexedCanvas, d: IVec2) -> IndexedCanvas {
    let fill = canvas.transparent().unwrap_or(0);
    let mut out = IndexedCanvas::filled(canvas.width(), canvas.height(), fill);
    out.set_transparent(canvas.transparent());
    for y in 0..canvas.height() as i32 {
        for x in 0..canvas.width() as i32 {
            if let Some(i) = canvas.get(x, y) {
                out.set(x + d.x, y + d.y, i);
            }
        }
    }
    out
}

fn frame_of(canvas: IndexedCanvas, palette: &Palette, kind: FrameKind) -> Frame {
    let mut f = Frame::new(uvec2(canvas.width(), canvas.height()), palette.clone());
    f.kind = kind;
    f.layers.push(Layer::new(
        LayerMeta::named("art"),
        Surface::Indexed(canvas),
    ));
    f
}

fn mask_of(canvas: &IndexedCanvas, palette: &Palette) -> Mask {
    let mut m = Mask::new(canvas.width(), canvas.height());
    for y in 0..canvas.height() as i32 {
        for x in 0..canvas.width() as i32 {
            let Some(i) = canvas.get(x, y) else { continue };
            if canvas.transparent() == Some(i) || palette.get(i).is_some_and(|c| c.a == 0) {
                continue;
            }
            m.set(ivec2(x, y), true);
        }
    }
    m
}

fn blocking_ids(report: &Report) -> Vec<u8> {
    let mut v: Vec<u8> = report
        .violations
        .iter()
        .filter(|x| x.is_blocking())
        .map(|x| x.rule)
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// **剛体の平行移動は «正しい動き» の定義そのものである** (書籍が線揺れの直し方を
/// «選択ツールでパーツをスライドさせる» とする)．**1 件も鳴ってはいけない**．
#[test]
fn a_rigid_translation_of_real_art_fires_no_sequence_rule() {
    let cfg = LintConfig::default();
    let (mut checked, mut skipped) = (0usize, 0usize);

    for path in png_files(&seeds()) {
        let Ok(img) = px_io::png::read_rgba(&path) else {
            skipped += 1;
            continue;
        };
        let Some((canvas, palette)) = index_exactly(&img) else {
            skipped += 1;
            continue;
        };
        let base = pad(&canvas, 12);
        if mask_of(&base, &palette).is_empty() {
            skipped += 1;
            continue;
        }
        for step in [1i32, 2, 3] {
            let frames: Vec<Frame> = (0..5)
                .map(|t| frame_of(shift(&base, ivec2(step * t, 0)), &palette, FrameKind::Key))
                .collect();
            let (report, _) = lint_sequence(&frames, &cfg);
            assert!(
                report.violations.is_empty(),
                "{} をずらし {step} で動かしただけで鳴った: {:?}",
                path.display(),
                report.violations
            );
        }
        checked += 1;
    }
    println!("実素材 {checked} 枚 x ずらし 3 通り (飛ばした {skipped} 枚)");
    assert!(
        checked >= 30,
        "通した枚数が少なすぎる: {checked} (透明の宣言がある種は 35 枚)"
    );
}

/// **`px anim tween` の出力が自分の検査に落ちないこと．**
///
/// 中割りのトポロジー (ルール 22) は**保証されない** — 設計書 6.9 のとおりで
/// あり，D114 が実測している．したがってここが見るのは 23 ・24 ・25 である．
#[test]
fn the_tween_tool_does_not_trip_its_own_sequence_lint() {
    let cfg = LintConfig::default();
    let (mut checked, mut skipped, mut topology) = (0usize, 0usize, 0usize);

    for path in png_files(&seeds()) {
        let Ok(img) = px_io::png::read_rgba(&path) else {
            skipped += 1;
            continue;
        };
        let Some((canvas, palette)) = index_exactly(&img) else {
            skipped += 1;
            continue;
        };
        let base = pad(&canvas, 12);
        let transparent = base.transparent().expect("透明の宣言がある");
        let index = base
            .pixels()
            .iter()
            .copied()
            .find(|i| *i != transparent)
            .expect("不透明な画素がある");
        let a = mask_of(&base, &palette);
        let b = mask_of(&shift(&base, ivec2(6, 0)), &palette);
        if a.is_empty() || b.is_empty() {
            skipped += 1;
            continue;
        }
        let Ok(mid) = tween_series(
            &a,
            &b,
            3,
            &TweenOptions {
                align: TweenAlign::Centroid,
                ..Default::default()
            },
        ) else {
            skipped += 1;
            continue;
        };

        let paint = |m: &Mask| {
            let mut c = IndexedCanvas::filled(m.width(), m.height(), transparent);
            c.set_transparent(Some(transparent));
            for p in m.iter_set() {
                c.set(p.x, p.y, index);
            }
            c
        };
        let mut frames = vec![frame_of(paint(&a), &palette, FrameKind::Key)];
        for t in &mid {
            frames.push(frame_of(paint(&t.mask), &palette, FrameKind::Inbetween));
        }
        frames.push(frame_of(paint(&b), &palette, FrameKind::Key));

        let (report, cov) = lint_sequence(&frames, &cfg);
        assert!(cov.inbetweens > 0, "中割りの印が残っていない");
        let fired = blocking_ids(&report);
        // **22 だけは «道具が保証しない» ものなので数えて別に扱う** (設計書 6.9)
        if fired.contains(&22) {
            topology += 1;
        }
        let unexpected: Vec<u8> = fired.into_iter().filter(|id| *id != 22).collect();
        assert!(
            unexpected.is_empty(),
            "{} の中割りが自分の検査に落ちた: ルール {unexpected:?}",
            path.display()
        );
        checked += 1;
    }
    println!(
        "実素材 {checked} 枚 (飛ばした {skipped} 枚)．\
         うちトポロジーが変わった列 {topology} 本 — 設計書 6.9 のとおり保証されない"
    );
    assert!(
        checked >= 30,
        "通した枚数が少なすぎる: {checked} (透明の宣言がある種は 35 枚)"
    );
}

/// **`px anim squash` の出力が «揺れる線» と «孤立列» に落ちないこと．**
///
/// 潰しと伸ばしは**設計上 非単調**なので，設計書 7.3 の «軌跡に対して非単調» を
/// そのまま読むと必ず鳴る — 書籍が教える技法を禁じてしまう．適用範囲を
/// «外接矩形の寸法が変わっていないコマ» に絞ってあることをここで固定する．
#[test]
fn the_squash_tool_does_not_trip_the_wobble_or_orphan_rules() {
    let cfg = LintConfig::default();
    let (mut checked, mut skipped) = (0usize, 0usize);

    for path in png_files(&seeds()) {
        let Ok(img) = px_io::png::read_rgba(&path) else {
            skipped += 1;
            continue;
        };
        let Some((canvas, palette)) = index_exactly(&img) else {
            skipped += 1;
            continue;
        };
        let base = pad(&canvas, 8);
        if mask_of(&base, &palette).is_empty() {
            skipped += 1;
            continue;
        }

        let mut frames = Vec::new();
        let mut size = (0u32, 0u32);
        for amount in [0.0f32, -0.1, -0.2, -0.1, 0.0] {
            let Ok((c, _)) = squash(&base, amount, &SquashOptions::default()) else {
                break;
            };
            size = (size.0.max(c.width()), size.1.max(c.height()));
            frames.push(c);
        }
        if frames.len() < 5 {
            skipped += 1;
            continue;
        }
        let frames: Vec<Frame> = frames
            .into_iter()
            .map(|c| {
                let fill = c.transparent().unwrap_or(0);
                let grown = c.crop(
                    IRect {
                        x: 0,
                        y: 0,
                        w: size.0,
                        h: size.1,
                    },
                    fill,
                );
                frame_of(grown, &palette, FrameKind::Key)
            })
            .collect();

        let (report, _) = lint_sequence(&frames, &cfg);
        let fired = blocking_ids(&report);
        assert!(
            fired.is_empty(),
            "{} の潰しが blocking に落ちた: ルール {fired:?}．\
             変形はルール 27 (advisory) の持ち場である",
            path.display()
        );
        checked += 1;
    }
    println!("実素材 {checked} 枚 (飛ばした {skipped} 枚)");
    assert!(
        checked >= 30,
        "通した枚数が少なすぎる: {checked} (透明の宣言がある種は 35 枚)"
    );
}
