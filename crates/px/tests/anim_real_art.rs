//! **`px anim` を実素材に掛けて «壊していないか» を測る．**
//!
//! 3 つの道具それぞれに «これが崩れたら道具として使えない» 不変条件がある．
//!
//! | 道具 | 不変条件 |
//! | --- | --- |
//! | `tween` | **両端がキーフレームと画素単位で一致する**．真値のある平行移動を当てる |
//! | `ease` | 設計書 6.11 の表と一致し，`px validate` の表示周期の検査を通る |
//! | `cycle` | **同じ種で同じ絵**．変調が色を作らない |
//!
//! 合成した形では捕まらないので実素材を全件通す (M3 の教訓) ．

use std::path::{Path, PathBuf};

use px_core::anim::{CycleSpec, ModTarget, Wave, cycle, duration_ms, ease};
use px_core::canvas::IndexedCanvas;
use px_core::color::Rgba8;
use px_core::frame::{Frame, Layer, LayerMeta, Surface};
use px_core::geom::Mask;
use px_core::math::{IVec2, ivec2, uvec2};
use px_core::palette::{ChromaCurve, Palette, Ramp};
use px_core::tween::{TweenAlign, TweenOptions, tween_mask, tween_series};

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

/// シルエットを読む．平行移動した相手が切れないだけの余地を付ける．
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

/// **壊れると: 中割りの列の両端がキーフレームと違う絵になる．**
///
/// いちばん気付きにくい壊れ方である — 中割りが «それらしく» 見えていても，
/// キーが 1 画素動いていたら動画としては壊れている．実素材を全件通す．
#[test]
fn the_ends_of_every_series_reproduce_the_key_frames_exactly() {
    let files = png_files();
    let mut checked = 0usize;
    for a_path in files.iter() {
        let Some(a) = silhouette(a_path, 4) else {
            continue;
        };
        let b = shifted(&a, ivec2(3, -2));
        for align in [TweenAlign::None, TweenAlign::Centroid] {
            let opts = TweenOptions { margin: 0, align };
            assert_eq!(
                tween_mask(&a, &b, 0.0, &opts).expect("t=0").mask,
                a,
                "{} の t=0 が元と違う ({})",
                a_path.display(),
                align.as_str()
            );
            assert_eq!(
                tween_mask(&a, &b, 1.0, &opts).expect("t=1").mask,
                b,
                "{} の t=1 が元と違う ({})",
                a_path.display(),
                align.as_str()
            );
            checked += 1;
        }
    }
    assert!(checked > 100, "{checked} 件しか見ていない");
}

/// **壊れると: R11 («SDF 補間が実用にならない») の答えが変わったのに気付けない．**
///
/// 真値のある場面 (平行移動) を作って測る．**«動かさない» を対照に置く** —
/// 中割りがこれに勝てないなら中割りは無価値である．
///
/// 実測 (`px-calib tween`．種 64 枚 x ずらし 4 通り = 256 件) ．
///
/// | ずらし | 場のまま | 重心を取り除く | 動かさない |
/// | --- | --- | --- | --- |
/// | (4, 0) | 0.953 | **1.000** | 0.778 |
/// | (8, 0) | 0.844 | **1.000** | 0.600 |
/// | (6, 6) | 0.828 | **1.000** | 0.493 |
/// | (12, -4) | 0.641 | **1.000** | 0.376 |
#[test]
fn taking_the_centroid_out_lands_on_the_true_middle_on_real_art() {
    let files = png_files();
    let mut cases = 0usize;
    for d in [ivec2(4, 0), ivec2(8, 0), ivec2(6, 6), ivec2(12, -4)] {
        let pad = (d.x.abs().max(d.y.abs()) + 2) as u32;
        for path in files.iter() {
            let Some(a) = silhouette(path, pad) else {
                continue;
            };
            let b = shifted(&a, d);
            let truth = shifted(&a, ivec2(d.x / 2, d.y / 2));
            let out = tween_mask(
                &a,
                &b,
                0.5,
                &TweenOptions {
                    margin: 0,
                    align: TweenAlign::Centroid,
                },
            )
            .expect("中割り");
            assert_eq!(out.shift, d, "{} で取り除いた移動が違う", path.display());
            assert_eq!(out.mask, truth, "{} が真値と違う", path.display());
            // «動かさない» より必ず良い
            assert!(iou(&out.mask, &truth) > iou(&a, &truth) || a == truth);
            cases += 1;
        }
    }
    assert!(cases > 200, "{cases} 件しか見ていない");
}

/// **壊れると: 場をそのまま補間したときの «痩せ» が変わったのに気付けない．**
///
/// 設計書 6.9 のままの式は動きに直交する向きに痩せる — **これは道具の誤りでは
/// なく代数である**ので，起きることを数で固定して報告に回す (D101 と同じ形) ．
#[test]
fn the_plain_field_really_does_thin_on_real_art() {
    let files = png_files();
    let d = ivec2(12, -4);
    let (mut thinner, mut total) = (0usize, 0usize);
    for path in files.iter() {
        let Some(a) = silhouette(path, 14) else {
            continue;
        };
        let b = shifted(&a, d);
        let truth = shifted(&a, ivec2(d.x / 2, d.y / 2));
        let plain = tween_mask(
            &a,
            &b,
            0.5,
            &TweenOptions {
                margin: 0,
                align: TweenAlign::None,
            },
        )
        .expect("中割り");
        if plain.mask.count() < truth.count() {
            thinner += 1;
        }
        total += 1;
    }
    assert!(total > 50);
    let ratio = thinner as f32 / total as f32;
    assert!(ratio > 0.8, "痩せた割合が {ratio:.2} に変わった");
}

/// **壊れると: 中割りが両端のどちらでもない «別の形» になる．**
///
/// $A \cap B \subseteq R \subseteq A \cup B$ は代数から出る (`px_core::tween`) ．
/// 実素材でも破れていないことを確かめる — 破れたら符号の規約が壊れている．
#[test]
fn the_containment_holds_on_real_art() {
    let files = png_files();
    for path in files.iter().take(20) {
        let Some(a) = silhouette(path, 4) else {
            continue;
        };
        let b = shifted(&a, ivec2(3, 3));
        for k in 1..5 {
            let t = k as f32 / 5.0;
            let r = tween_mask(
                &a,
                &b,
                t,
                &TweenOptions {
                    margin: 0,
                    align: TweenAlign::None,
                },
            )
            .expect("中割り")
            .mask;
            for p in r.bounds().iter() {
                if a.get(p) && b.get(p) {
                    assert!(r.get(p), "{} の t={t} で共通部分が落ちた", path.display());
                }
                if !a.get(p) && !b.get(p) {
                    assert!(!r.get(p), "{} の t={t} で和集合の外へ出た", path.display());
                }
            }
        }
    }
}

/// **壊れると: 中割りの列が枚数どおりに出ない，または同じ絵が並ぶ．**
#[test]
fn a_series_gives_distinct_frames_in_order() {
    let path = seeds().join("crawl_urand_fencer.png");
    let a = silhouette(&path, 8).expect("シルエット");
    let b = shifted(&a, ivec2(8, 0));
    let series = tween_series(&a, &b, 5, &TweenOptions::default()).expect("5 枚");
    assert_eq!(series.len(), 5);
    let ts: Vec<f32> = series.iter().map(|s| s.t).collect();
    assert!(ts.windows(2).all(|w| w[0] < w[1]), "t が単調でない");
    // 平行移動なので «少しずつ動く» — 隣どうしが同じ絵にはならない
    for w in series.windows(2) {
        assert_ne!(w[0].mask, w[1].mask, "隣の中割りが同じ絵である");
    }
}

// ------------------------------------------------------------------ ease

fn palette() -> Palette {
    Palette::new(vec![
        Rgba8::TRANSPARENT,
        Rgba8::new(40, 40, 60, 255),
        Rgba8::new(90, 90, 120, 255),
        Rgba8::new(150, 150, 190, 255),
    ])
    .expect("パレット")
}

fn frame_of(canvas: IndexedCanvas) -> Frame {
    let mut f = Frame::new(uvec2(canvas.width(), canvas.height()), palette());
    f.layers.push(Layer::new(
        LayerMeta::named("art"),
        Surface::Indexed(canvas),
    ));
    f
}

fn blob() -> IndexedCanvas {
    let mut c = IndexedCanvas::filled(16, 16, 0).with_transparent(Some(0));
    for y in 4..12 {
        for x in 4..12 {
            c.set(x, y, 2);
        }
    }
    c
}

/// **壊れると: «どの FPS が実機に載るか» が変わったのに気付けない．**
///
/// 自分で付けた表示時間を自分の検査 (`px validate`) に掛ける (D58 と同じ場面) ．
/// **全部通ることを期待して書いたら 8 件落ちた．測ったら道具の誤りではなかった．**
///
/// 60 Hz の実機は 16.74 ms 刻みでしか絵を切り替えられない．24 FPS の 1 コマは
/// 42 ms で，**走査 2.51 回ぶん**にあたる — どちらへ丸めても 8 ms ずれる．
///
/// | 24 FPS | 走査の回数 (gb 59.73 Hz) | 通るか |
/// | --- | --- | --- |
/// | 1 コマ (42 ms) | **2.51** | 落ちる |
/// | 2 コマ (83 ms) | 4.96 | 通る |
/// | 3 コマ (125 ms) | **7.47** | 落ちる |
/// | 4 コマ (167 ms) | 9.97 | 通る |
///
/// **24 FPS は 60 Hz を割り切らないので，奇数コマは載らない** — これは表の誤り
/// でも `ease` の誤りでもなく，そういう事実である．**`px validate` はそれを言う
/// べきなので，落ちる側も «落ちること» を固定する** (D101 の «削減率は報告する
/// だけ» と同じ側の判断) ．
#[test]
fn our_durations_pass_the_frame_period_check_exactly_when_the_fps_divides_the_refresh() {
    use px_core::validate::{Target, validate_frames};
    let fired = |fps: f32, hold: u32, name: &str| {
        let mut frames = vec![frame_of(blob())];
        ease(&mut frames, fps, &[hold]).expect("付けられる");
        assert_eq!(
            frames[0].duration_ms,
            duration_ms(fps, hold).expect("引ける")
        );
        validate_frames(&frames, &Target::builtin(name).expect("組み込み"))
            .violations
            .iter()
            .any(|v| v.constraint == "frame-ms")
    };

    for name in ["gb", "nes", "snes", "gba"] {
        // **30 ・60 FPS は 60 Hz を割り切るのでどのコマ打ちでも通る**
        for fps in [30.0f32, 60.0] {
            for hold in 1..=4u32 {
                assert!(
                    !fired(fps, hold, name),
                    "{name} の {fps} FPS {hold} コマが落ちた"
                );
            }
        }
        // **24 FPS は割り切らない．奇数コマだけが落ちる**
        for hold in [1u32, 3] {
            assert!(
                fired(24.0, hold, name),
                "{name} の 24 FPS {hold} コマが通った"
            );
        }
        for hold in [2u32, 4] {
            assert!(
                !fired(24.0, hold, name),
                "{name} の 24 FPS {hold} コマが落ちた"
            );
        }
    }
}

// ----------------------------------------------------------------- cycle

/// **壊れると: 同じ種で違う絵が出る (設計書 6.12 が禁じている)．**
///
/// **書いた 12 通りすべて**を実素材で通す．
#[test]
fn every_written_combination_is_deterministic_on_real_art() {
    let files = png_files();
    let ramp = Ramp::new(vec![1, 2, 3, 4, 5], ChromaCurve::PeakMiddle);
    let mut cases = 0usize;
    for path in files.iter().take(8) {
        let img = px_io::png::read_rgba(path).expect("読める");
        let Some((canvas, pal)) = index_exactly(&img) else {
            continue;
        };
        let mut src = Frame::new(uvec2(img.width(), img.height()), pal);
        src.layers.push(Layer::new(
            LayerMeta::named("art"),
            Surface::Indexed(canvas),
        ));
        for target in [ModTarget::Ramp, ModTarget::Offset, ModTarget::Mask] {
            for wave in Wave::ALL {
                let spec = CycleSpec {
                    target,
                    wave: *wave,
                    frames: 4,
                    amplitude: 2.0,
                    seed: 99,
                    ..CycleSpec::default()
                };
                let (a, _) = cycle(&src, &spec, Some(&ramp)).expect("作れる");
                let (b, _) = cycle(&src, &spec, Some(&ramp)).expect("作れる");
                for (x, y) in a.iter().zip(&b) {
                    assert_eq!(
                        x.layers[0].surface.as_indexed(),
                        y.layers[0].surface.as_indexed(),
                        "{} の {}x{} が揺れた",
                        path.display(),
                        target.as_str(),
                        wave.as_str()
                    );
                }
                cases += 1;
            }
        }
    }
    // 256 色を超える種は添字にできないので飛ばす — 枚数ではなく «通り数» で見る
    assert!(cases >= 72, "{cases} 通りしか見ていない");
}

/// **壊れると: 変調が色を作る．**
///
/// 3 つの変調はどれも «既にある添字を置き直す» 操作なので，出てくる添字は
/// 元の絵とランプの範囲に収まる — 合成の不変条件 (D94) と同じ性質である．
#[test]
fn modulation_never_invents_a_colour_on_real_art() {
    let files = png_files();
    let ramp = Ramp::new(vec![1, 2, 3, 4, 5], ChromaCurve::PeakMiddle);
    for path in files.iter().take(12) {
        let img = px_io::png::read_rgba(path).expect("読める");
        let Some((canvas, pal)) = index_exactly(&img) else {
            continue;
        };
        let before: std::collections::BTreeSet<u8> = canvas.pixels().iter().copied().collect();
        let mut src = Frame::new(uvec2(img.width(), img.height()), pal);
        src.layers.push(Layer::new(
            LayerMeta::named("art"),
            Surface::Indexed(canvas),
        ));
        for target in [ModTarget::Ramp, ModTarget::Offset, ModTarget::Mask] {
            let spec = CycleSpec {
                target,
                wave: Wave::Sine,
                frames: 5,
                amplitude: 2.0,
                seed: 3,
                ..CycleSpec::default()
            };
            let (frames, _) = cycle(&src, &spec, Some(&ramp)).expect("作れる");
            for f in &frames {
                for v in f.layers[0].surface.as_indexed().expect("添字").pixels() {
                    assert!(
                        before.contains(v) || ramp.entries().contains(v),
                        "{} の {} が添字 {v} を作った",
                        path.display(),
                        target.as_str()
                    );
                }
            }
        }
    }
}

fn index_exactly(img: &px_core::canvas::RgbaCanvas) -> Option<(IndexedCanvas, Palette)> {
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
