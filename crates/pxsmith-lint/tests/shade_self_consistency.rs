//! **自己整合性 — `pxsmith shade` の出力は `pxsmith lint` を通らなければならない**
//! (設計書 6.2 の注記 ・実装計画書 M3) ．
//!
//! `shade` は距離場の勾配から陰影を作り，lint のいくつかのルールは距離場や領域から
//! «おかしさ» を測る．**自作の出力が自作の検査に引っかかりうる**ので，回帰試験で
//! 押さえる．
//!
//! > [!warning] **落ちたら `shade` を正として lint を直す** (D58) ．
//! > 逆向きの修正 (lint に合わせて `shade` を変える) は禁じる．`shade` は
//! > 3 ランプ ・光源 5 型 ・分岐別正規化という理論をそのまま実装したものであり，
//! > 検出側は代理指標にすぎない．
//!
//! # 何が壊れると落ちるか
//!
//! | 壊れ方 | 鳴るルール |
//! | --- | --- |
//! | ランプの添字がパレットの外を指す (透明色を先頭に足すなど) | 1 パレット逸脱 |
//! | 段の境目に 1 画素の帯ができる (量子化の縁) | 3 孤立ピクセル |
//! | 3 ランプの彩度が明度に対し単調 | 5 彩度カーブ異常 |
//! | 段の境目がディザに見える | 10 ディザの塊化 |
//! | シルエットの外が «格子» に見える | 9 ミクセル |
//! | 明るさが «縁からの距離» で決まってしまう | **13 pillow shading** |
//!
//! **ルール 13 が本題である** (設計書 6.2 の注記が名指ししている) ．advisory なので
//! `has_blocking` では捕まらない — 別に数える．

use pxsmith_core::Rgba8;
use pxsmith_core::canvas::{IndexedCanvas, RgbaCanvas};
use pxsmith_core::geom::Mask;
use pxsmith_core::math::{ivec2, vec2};
use pxsmith_core::palette::{ChromaCurve, Palette};
use pxsmith_core::ramp::{LightPreset, LightSource, build_lighting};
use pxsmith_core::shade::{ShadeOptions, shade_to_canvas};
use pxsmith_lint::Severity;

/// 円板 — **中心に稜線 (medial axis) ができる**形．3 段処理の相手である．
fn disc(size: u32, radius: f32) -> Mask {
    let mut m = Mask::new(size, size);
    let c = (size as f32 - 1.0) / 2.0;
    for p in m.bounds().iter() {
        let (dx, dy) = (p.x as f32 - c, p.y as f32 - c);
        if dx * dx + dy * dy <= radius * radius {
            m.set(p, true);
        }
    }
    m
}

/// 縦長の胴体 — キャラクタのシルエットに近い形 (稜線が線分になる)．
fn capsule(w: u32, h: u32) -> Mask {
    let mut m = Mask::new(w, h);
    let r = (w as f32 - 2.0) / 2.0;
    let cx = (w as f32 - 1.0) / 2.0;
    let (top, bottom) = (r, h as f32 - 1.0 - r);
    for p in m.bounds().iter() {
        let (x, y) = (p.x as f32, p.y as f32);
        let cy = y.clamp(top, bottom);
        let (dx, dy) = (x - cx, y - cy);
        if dx * dx + dy * dy <= r * r {
            m.set(p, true);
        }
    }
    m
}

/// 谷折りのある形 — 環境遮蔽が乗る相手．
fn notch(size: u32) -> Mask {
    let mut m = Mask::new(size, size);
    for p in m.bounds().iter() {
        let cut = p.x >= size as i32 / 2 && p.y >= size as i32 / 2;
        if !cut && p.x > 0 && p.y > 0 && p.x < size as i32 - 1 && p.y < size as i32 - 1 {
            m.set(p, true);
        }
    }
    m
}

/// 四角い箱 — 面が平らなので **段の境目が直線になる**．
fn box_shape(size: u32) -> Mask {
    let mut m = Mask::new(size, size);
    for p in m.bounds().iter() {
        if p.x > 1 && p.y > 1 && p.x < size as i32 - 2 && p.y < size as i32 - 2 {
            m.set(p, true);
        }
    }
    m
}

fn shapes() -> Vec<(&'static str, Mask)> {
    vec![
        ("円板", disc(21, 9.0)),
        ("胴体", capsule(14, 22)),
        ("谷折り", notch(16)),
        ("箱", box_shape(18)),
    ]
}

/// 添字をパレットで解決する (`pxsmith lint` が PNG を読むときと同じ形にするため)．
///
/// `pxsmith-io` は `pxsmith-lint` の依存ではないので `png::resolve` は使えない．
fn resolve(canvas: &IndexedCanvas, palette: &Palette) -> RgbaCanvas {
    let mut out = RgbaCanvas::filled(canvas.width(), canvas.height(), Rgba8::TRANSPARENT);
    for p in canvas.bounds().iter() {
        let Some(index) = canvas.get_at(p) else {
            continue;
        };
        let color = if canvas.transparent() == Some(index) {
            Rgba8::TRANSPARENT
        } else {
            palette.get(index).unwrap_or(Rgba8::TRANSPARENT)
        };
        out.set(p.x, p.y, color);
    }
    out
}

/// 1 件ぶんの陰影付けと検査．`pxsmith shade` → `pxsmith lint` と同じ経路を通す．
fn lint_shaded(
    mask: &Mask,
    preset: LightPreset,
    source: LightSource,
    opts: ShadeOptions,
) -> pxsmith_lint::Report {
    let (palette, model) =
        build_lighting(Rgba8::rgb(120, 90, 70), preset, 5, ChromaCurve::PeakMiddle)
            .expect("ランプを作れない");
    let (canvas, palette) =
        shade_to_canvas(mask, source, &model, &palette, opts).expect("陰影を付けられない");

    let cfg = pxsmith_lint::LintConfig::default();
    let mut report = pxsmith_lint::rules::lint_palette(&palette, &cfg);
    report.extend(pxsmith_lint::lint_canvas(&canvas, &palette, &cfg));
    // PNG で受け取ったときに掛かる格子系のルール (2 ・9) も同じ絵で見る
    report.extend(pxsmith_lint::rules::lint_grid(
        &resolve(&canvas, &palette),
        &cfg,
    ));
    report.sorted()
}

fn describe(report: &pxsmith_lint::Report) -> String {
    report
        .violations
        .iter()
        .map(|v| {
            format!(
                "    [{}] ルール {} {}: {}",
                if v.severity == Severity::Blocking {
                    "blocking"
                } else {
                    "advisory"
                },
                v.rule,
                v.name,
                v.message
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// **`pxsmith shade` の出力に blocking 違反が 1 件も出ない．**
///
/// 形 4 種 x 光源プリセット 5 種を掛ける．落ちたときは `shade` ではなく
/// **ルールの側を直す** (D58) ．
#[test]
fn shading_a_silhouette_never_produces_a_blocking_violation() {
    let mut failures = Vec::new();
    for (name, mask) in shapes() {
        for preset in LightPreset::ALL {
            let report = lint_shaded(
                &mask,
                preset,
                preset.default_source(),
                ShadeOptions::default(),
            );
            eprintln!(
                "{name} / {} — blocking {} ・advisory {}\n{}",
                preset.as_str(),
                report.blocking().count(),
                report.advisory().count(),
                describe(&report)
            );
            if report.has_blocking() {
                failures.push(format!(
                    "{name} / {}\n{}",
                    preset.as_str(),
                    describe(&report)
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "陰影の出力が自分の lint に落ちた ({} 件)．\
         **`shade` を正としてルールを直すこと** (D58)\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// **環境遮蔽 (`--ao`) を掛けても通る．** 遮蔽色は 3 ランプの外の 1 色なので，
/// 谷折りの内側に**その色だけの小さい領域**ができうる (ルール 3 の相手) ．
#[test]
fn ambient_occlusion_does_not_break_self_consistency() {
    let opts = ShadeOptions {
        ambient_occlusion: Some(0.25),
        ..ShadeOptions::default()
    };
    let mut failures = Vec::new();
    for (name, mask) in shapes() {
        let report = lint_shaded(
            &mask,
            LightPreset::Clear,
            LightPreset::Clear.default_source(),
            opts,
        );
        eprintln!(
            "{name} / --ao — blocking {} ・advisory {}\n{}",
            report.blocking().count(),
            report.advisory().count(),
            describe(&report)
        );
        if report.has_blocking() {
            failures.push(format!("{name}\n{}", describe(&report)));
        }
    }
    assert!(
        failures.is_empty(),
        "--ao で lint に落ちた\n{}",
        failures.join("\n")
    );
}

/// **光源 5 型のどれでも通る．** 型ごとに照度の式が違うので，段の出方も違う．
#[test]
fn every_kind_of_light_source_passes_the_lint() {
    let sources = [
        (
            "directional",
            LightSource::Directional {
                dir: vec2(0.6, 0.8),
            },
        ),
        (
            "point",
            LightSource::Point {
                pos: vec2(-4.0, -6.0),
                intensity: 60.0,
            },
        ),
        (
            "line",
            LightSource::Line {
                a: vec2(-8.0, -8.0),
                b: vec2(8.0, -8.0),
                intensity: 1.0,
            },
        ),
        (
            "area",
            LightSource::Area {
                rect: pxsmith_core::math::Rect {
                    x: -8.0,
                    y: -10.0,
                    w: 16.0,
                    h: 4.0,
                },
                intensity: 1.0,
            },
        ),
        ("ambient", LightSource::Ambient),
    ];
    let mask = disc(21, 9.0);
    let mut failures = Vec::new();
    for (name, source) in sources {
        let report = lint_shaded(&mask, LightPreset::Clear, source, ShadeOptions::default());
        eprintln!(
            "円板 / {name} — blocking {} ・advisory {}\n{}",
            report.blocking().count(),
            report.advisory().count(),
            describe(&report)
        );
        if report.has_blocking() {
            failures.push(format!("{name}\n{}", describe(&report)));
        }
    }
    assert!(
        failures.is_empty(),
        "光源型によって lint に落ちる\n{}",
        failures.join("\n")
    );
}

/// **`pxsmith shade` の出力がルール 13 (pillow shading) に鳴らない．**
///
/// 設計書 6.2 が名指しした自己整合性の相手である．advisory なので blocking の試験では
/// 捕まらず，ここで別に見る．
///
/// 相関の実測は `pxsmith-calib pillow` にある — 良い絵 61 枚 ・負例 6 枚 ・
/// **`pxsmith shade` の出力 320 通り**を測り，`pxsmith shade` は全件が**負の相関** (最大
/// $-0.020$) だった．閾値 0.85 に対し十分離れている．
///
/// **落ちたらルール 13 の側を直す** (D58) ．
#[test]
fn shading_is_never_mistaken_for_pillow_shading() {
    let cfg = pxsmith_lint::LintConfig::default();
    let mut worst = f32::NEG_INFINITY;
    let mut failures = Vec::new();
    for (name, mask) in shapes() {
        for preset in LightPreset::ALL {
            let (palette, model) =
                build_lighting(Rgba8::rgb(120, 90, 70), preset, 5, ChromaCurve::PeakMiddle)
                    .expect("ランプを作れない");
            let (canvas, palette) = shade_to_canvas(
                &mask,
                preset.default_source(),
                &model,
                &palette,
                ShadeOptions::default(),
            )
            .expect("陰影を付けられない");
            let Some(rho) = pxsmith_lint::rules::pillow_correlation(&canvas, &palette) else {
                continue;
            };
            eprintln!("{name} / {} — rho {rho:+.3}", preset.as_str());
            worst = worst.max(rho);
            if rho > cfg.max_pillow_correlation {
                failures.push(format!("{name} / {} — rho {rho:+.3}", preset.as_str()));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "陰影が pillow shading と判定された ({} 件．上限 {:.2})．\
         **`shade` を正としてルール 13 を直すこと** (D58)\n{}",
        failures.len(),
        cfg.max_pillow_correlation,
        failures.join("\n")
    );
    // **余裕も見る．** 閾値のすぐ下に張り付いているなら «たまたま通った» だけである
    assert!(
        worst < cfg.max_pillow_correlation - 0.2,
        "上限 {:.2} に対し最悪の相関が {worst:+.3} と近い — 余裕が無い",
        cfg.max_pillow_correlation
    );
}

/// シルエットの外は透明のまま残る (陰影は形を変えない)．
#[test]
fn shading_never_paints_outside_the_silhouette() {
    let mask = disc(21, 9.0);
    let (palette, model) = build_lighting(
        Rgba8::rgb(120, 90, 70),
        LightPreset::Clear,
        5,
        ChromaCurve::PeakMiddle,
    )
    .expect("ランプを作れない");
    let (canvas, _) = shade_to_canvas(
        &mask,
        LightPreset::Clear.default_source(),
        &model,
        &palette,
        ShadeOptions::default(),
    )
    .expect("陰影を付けられない");
    for p in mask.bounds().iter() {
        assert_eq!(
            !canvas.is_transparent_at(p),
            mask.get(p),
            "{p:?} でシルエットと食い違う"
        );
    }
    let _ = ivec2(0, 0);
}
