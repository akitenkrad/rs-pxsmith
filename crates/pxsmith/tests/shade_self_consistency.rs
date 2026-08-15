//! **自己整合性を «本物のシルエット» で見る** (実装計画書 M3)．
//!
//! `pxsmith-lint` 側の同名の試験は合成した形 (円板 ・胴体 ・谷折り ・箱) で見ている．
//! **それでは足りなかった** — CC0 の実物のドット絵から取ったシルエットに掛けたとき
//! だけ，光へ正対した 1 画素が «孤立ピクセル» (ルール 3) になっていた
//! (`crawl_urand_fencer` の (27, 19)) ．合成した形は滑らかすぎて，最上段が
//! 1 画素だけ立つ場面を作れない．
//!
//! ここは**素材を読む側のクレートにしか置けない** (`pxsmith-lint` は `pxsmith-io` に依存しない) ．
//! **種は間引かない．** 落ちていた 2 枚 (`crawl_urand_fencer` ・`kenney_tile_0085`) は
//! 64 枚中 31 番目と 58 番目で，8 枚ごとに拾う形では**どちらも入らなかった** —
//! 間引いた試験は «通った» と言うだけの番人になる．全件でも 1 秒かからない．

use std::path::{Path, PathBuf};

use pxsmith_core::geom::Mask;
use pxsmith_core::palette::ChromaCurve;
use pxsmith_core::ramp::{LightPreset, build_lighting};
use pxsmith_core::shade::{ShadeOptions, shade_to_canvas};
use pxsmith_core::smooth::{SmoothOptions, smooth_canvas};
use pxsmith_core::{Rgba8, canvas::RgbaCanvas};

fn seeds_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/grid-eval/seeds")
}

/// 種をすべて読む．
fn all_seeds() -> Vec<(String, RgbaCanvas)> {
    let dir = seeds_dir();
    let mut files: Vec<PathBuf> = match std::fs::read_dir(&dir) {
        Ok(it) => it
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")))
            .collect(),
        Err(_) => Vec::new(),
    };
    files.sort();
    files
        .into_iter()
        .filter_map(|p| {
            let img = pxsmith_io::png::read_rgba(&p).ok()?;
            Some((p.file_name()?.to_string_lossy().to_string(), img))
        })
        .collect()
}

fn silhouette(img: &RgbaCanvas) -> Mask {
    let mut m = Mask::new(img.width(), img.height());
    for p in m.bounds().iter() {
        if img.get(p.x, p.y).is_some_and(|c| c.a != 0) {
            m.set(p, true);
        }
    }
    m
}

/// **`pxsmith shade` の出力が `pxsmith lint` の blocking を 1 件も出さない．**
///
/// 落ちたら**どちらが正しいかを先に見る** — ルール 13 は代理指標なので `shade` が
/// 正だが (D58) ，それ以外のルールは «出力に本当に欠陥があるのか» を確かめてから
/// 決める．実際ルール 3 は «ランプの隣の段» を除外する形で直した (陰影の最終段を
/// 迷子と呼んでいた) ．
#[test]
fn shading_real_silhouettes_never_produces_a_blocking_violation() {
    let seeds = all_seeds();
    assert!(
        !seeds.is_empty(),
        "種が 1 枚も読めない ({} を確かめる)",
        seeds_dir().display()
    );

    let cfg = pxsmith_lint::LintConfig::default();
    let mut failures = Vec::new();
    let mut checked = 0usize;
    for (name, img) in &seeds {
        let mask = silhouette(img);
        if mask.is_empty() {
            continue;
        }
        for preset in LightPreset::ALL {
            let (palette, model) =
                build_lighting(Rgba8::rgb(138, 106, 74), preset, 5, ChromaCurve::PeakMiddle)
                    .expect("ランプを作れない");
            let (canvas, palette) = shade_to_canvas(
                &mask,
                preset.default_source(),
                &model,
                &palette,
                ShadeOptions::default(),
            )
            .expect("陰影を付けられない");

            let mut report = pxsmith_lint::rules::lint_palette(&palette, &cfg);
            report.extend(pxsmith_lint::lint_canvas(&canvas, &palette, &cfg));
            checked += 1;
            for v in report.blocking() {
                failures.push(format!(
                    "{name} / {} — ルール {} {}: {}",
                    preset.as_str(),
                    v.rule,
                    v.name,
                    v.message
                ));
            }
        }
    }
    eprintln!(
        "{checked} 通りを検査した ({} 枚 x 5 プリセット)",
        seeds.len()
    );
    assert!(
        failures.is_empty(),
        "陰影の出力が自分の lint に落ちた ({} 件)\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// **`pxsmith shade` → `pxsmith smooth` → `pxsmith lint` が通る．**
///
/// 整形は陰影の段の境目を動かすので，**陰影を付けた絵こそ整形の相手**である．
/// 直した結果が lint に落ちるなら，どちらかの規則が間違っている．
#[test]
fn smoothing_a_shaded_sprite_still_passes_the_lint() {
    let seeds = all_seeds();
    let cfg = pxsmith_lint::LintConfig::default();
    let opts = SmoothOptions::default();
    let mut failures = Vec::new();
    let mut moved_total = 0usize;

    for (name, img) in &seeds {
        let mask = silhouette(img);
        if mask.is_empty() {
            continue;
        }
        let (palette, model) = build_lighting(
            Rgba8::rgb(138, 106, 74),
            LightPreset::Clear,
            5,
            ChromaCurve::PeakMiddle,
        )
        .expect("ランプを作れない");
        let (canvas, palette) = shade_to_canvas(
            &mask,
            LightPreset::Clear.default_source(),
            &model,
            &palette,
            ShadeOptions::default(),
        )
        .expect("陰影を付けられない");

        let mut smoothed = canvas.clone();
        let report = smooth_canvas(&mut smoothed, &opts);
        moved_total += report.moved;

        let mut lint = pxsmith_lint::rules::lint_palette(&palette, &cfg);
        lint.extend(pxsmith_lint::lint_canvas(&smoothed, &palette, &cfg));
        for v in lint.blocking() {
            failures.push(format!(
                "{name} — ルール {} {}: {}",
                v.rule, v.name, v.message
            ));
        }
    }
    eprintln!("{} 枚で {moved_total} 画素を動かした", seeds.len());
    assert!(
        failures.is_empty(),
        "陰影 → 整形 の出力が lint に落ちた ({} 件)\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// **`pxsmith shade` の出力を «その光源で» 検査すると，ルール 7 に落ちない．**
///
/// ルール 7 (反転同値の陰影不整合) は光源が宣言されたときだけ働く．宣言する側が
/// まさに `pxsmith shade` なので，**自分の出力が自分の検査を通ること**が最低条件である．
/// 併せて左右反転が捕まることも見る — 片方だけでは «常に鳴らない» ルールでも通る．
///
/// > [!warning] **点光源のプリセット (`night`) は対象外である．**
/// > 点 ・線 ・面の光源では $\ell$ が画素ごとに違うので，絵全体で平均した勾配と
/// > 突き合わせても意味を持たない．実測では `night` の 64 枚すべてで一致度が
/// > $-0.54$ 〜 $-1.0$ になり，平行光源の 4 プリセット (最小 0.714) とまったく
/// > 別の分布になった．ルール 7 は平行光源にだけ掛かる．
#[test]
fn shading_agrees_with_its_own_light_and_a_mirror_does_not() {
    let seeds = all_seeds();
    assert!(!seeds.is_empty(), "種が 1 枚も無い");

    let base = Rgba8::rgb(0x8a, 0x6a, 0x4a);
    let (mut checked, mut upright_fired, mut mirrors_caught, mut mirrors) = (0, 0, 0, 0);

    for (_name, img) in &seeds {
        let mask = silhouette(img);
        if mask.count() < 64 {
            continue;
        }
        for preset in LightPreset::ALL {
            let source = preset.default_source();
            // 平行光源のプリセットだけを見る (上の警告)
            if !matches!(source, pxsmith_core::ramp::LightSource::Directional { .. }) {
                continue;
            }
            let Ok((palette, model)) = build_lighting(base, preset, 5, ChromaCurve::PeakMiddle)
            else {
                continue;
            };
            let Ok((canvas, palette)) =
                shade_to_canvas(&mask, source, &model, &palette, ShadeOptions::default())
            else {
                continue;
            };
            let cfg = pxsmith_lint::LintConfig {
                light: Some(source),
                ..pxsmith_lint::LintConfig::default()
            };

            checked += 1;
            if pxsmith_lint::lint_canvas(&canvas, &palette, &cfg)
                .violations
                .iter()
                .any(|v| v.rule == 7)
            {
                upright_fired += 1;
            }

            // **絵だけを左右反転する** (光源の宣言はそのまま) ．自動ミラーの失敗
            let mut flipped = canvas.clone();
            for p in canvas.bounds().iter() {
                let q = pxsmith_core::math::ivec2(canvas.width() as i32 - 1 - p.x, p.y);
                if let Some(i) = canvas.get_at(q) {
                    flipped.set_at(p, i);
                }
            }
            mirrors += 1;
            if pxsmith_lint::lint_canvas(&flipped, &palette, &cfg)
                .violations
                .iter()
                .any(|v| v.rule == 7)
            {
                mirrors_caught += 1;
            }
        }
    }

    eprintln!("{checked} 通りを検査 ・反転 {mirrors} 通り");
    assert!(checked > 0, "1 通りも検査していない");
    assert_eq!(
        upright_fired, 0,
        "pxsmith shade の出力が自分の光源でルール 7 に落ちた ({upright_fired} / {checked})"
    );
    assert_eq!(
        mirrors_caught, mirrors,
        "左右反転を取り逃した ({mirrors_caught} / {mirrors})"
    );
}

/// **`pxsmith shade` の出力がルール 14 (AA 過多) に落ちない．**
///
/// 陰影の «段» は端の 2 色の間にあり，端より狭く使われる — ルール 14 の «中間色» の
/// 定義にそのまま当てはまる．**閾値を良い絵だけで決めると自分の出力が鳴る**
/// (実測で 38.8%) ．D58 ・D77 と同じ作法で `pxsmith shade` の出力を第 3 群として扱い，
/// その上に閾値を置いてある．
///
/// > [!warning] この試験は blocking を数える上の試験では捕まらない —
/// > ルール 14 は advisory である．**端から端まで CLI で通して初めて見つかった.**
#[test]
fn shading_does_not_trip_the_too_much_antialiasing_rule() {
    let seeds = all_seeds();
    assert!(!seeds.is_empty(), "種が 1 枚も無い");

    let base = Rgba8::rgb(0x8a, 0x6a, 0x4a);
    let cfg = pxsmith_lint::LintConfig::default();
    let (mut checked, mut fired, mut worst) = (0usize, 0usize, 0.0f32);

    for (name, img) in &seeds {
        let mask = silhouette(img);
        if mask.count() < 64 {
            continue;
        }
        for preset in LightPreset::ALL {
            let Ok((palette, model)) = build_lighting(base, preset, 5, ChromaCurve::PeakMiddle)
            else {
                continue;
            };
            let Ok((canvas, palette)) = shade_to_canvas(
                &mask,
                preset.default_source(),
                &model,
                &palette,
                ShadeOptions::default(),
            ) else {
                continue;
            };
            checked += 1;
            let opaque = canvas
                .pixels()
                .iter()
                .filter(|i| canvas.transparent() != Some(**i))
                .count()
                .max(1);
            let ratio = pxsmith_core::aa::intermediate_pixels(
                &canvas,
                &palette,
                cfg.intermediate_tolerance,
            ) as f32
                / opaque as f32;
            worst = worst.max(ratio);
            if pxsmith_lint::lint_canvas(&canvas, &palette, &cfg)
                .violations
                .iter()
                .any(|v| v.rule == 14)
            {
                fired += 1;
                eprintln!("{name} / {}: 中間色 {:.1}%", preset.as_str(), ratio * 100.0);
            }
        }
    }

    eprintln!("{checked} 通り ・中間色の最大 {:.1}%", worst * 100.0);
    assert!(checked > 0, "1 通りも検査していない");
    assert_eq!(
        fired, 0,
        "pxsmith shade の出力が自分の AA 過多検査に落ちた ({fired} / {checked})"
    );
}
