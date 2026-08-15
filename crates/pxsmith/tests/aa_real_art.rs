//! **`pxsmith aa` を本物のドット絵に掛けて «壊していないか» を見る．**
//!
//! `pxsmith-core` 側の試験は合成した 2 面の絵で «付くこと» を見ている．ここで見るのは
//! 逆で，**良い絵 (CC0 の実物) に掛けたときに何も壊れないこと**である．
//!
//! | 何が壊れると落ちるか | |
//! | --- | --- |
//! | シルエットを触る (D34) | 透明の位置が変わる |
//! | AA が lint に落ちる絵を作る | blocking が増える |
//! | 置きすぎる (ルール 14 の相手) | 不透明画素に占める割合が上限を超える |
//! | 色を増やしすぎる | 1 枚あたりの追加色が上限を超える |
//!
//! **冪等性は要求しない** — AA は輪郭の形を変えるので 2 度目には新しい角ができる
//! (`AaAddOptions::min_run` の説明) ．全件の集計は `pxsmith-calib aa` にある．

use std::path::{Path, PathBuf};

use pxsmith_core::aa::{AaAddOptions, add_antialiasing};
use pxsmith_core::canvas::IndexedCanvas;
use pxsmith_core::palette::Palette;

fn seeds_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/grid-eval/seeds")
}

fn index_exactly(img: &pxsmith_core::canvas::RgbaCanvas) -> Option<(IndexedCanvas, Palette)> {
    let mut colors: Vec<_> = img.pixels().to_vec();
    colors.sort_unstable_by_key(|c| c.sort_key());
    colors.dedup();
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

fn blocking(canvas: &IndexedCanvas, palette: &Palette) -> usize {
    let cfg = pxsmith_lint::LintConfig::default();
    let mut report = pxsmith_lint::rules::lint_palette(palette, &cfg);
    report.extend(pxsmith_lint::lint_canvas(canvas, palette, &cfg));
    report.blocking().count()
}

/// **良い絵に掛けても壊れない．**
#[test]
fn adding_antialiasing_to_real_pixel_art_breaks_nothing() {
    let mut files: Vec<PathBuf> = match std::fs::read_dir(seeds_dir()) {
        Ok(it) => it
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")))
            .collect(),
        Err(_) => Vec::new(),
    };
    files.sort();
    assert!(
        !files.is_empty(),
        "種が 1 枚も無い ({} を確かめる)",
        seeds_dir().display()
    );

    let opts = AaAddOptions::default();
    let mut failures = Vec::new();
    let (mut checked, mut painted_total, mut opaque_total) = (0usize, 0usize, 0usize);

    for path in &files {
        let Ok(img) = pxsmith_io::png::read_rgba(path) else {
            continue;
        };
        let Some((canvas, palette)) = index_exactly(&img) else {
            continue;
        };
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        checked += 1;

        let transparent = canvas.transparent();
        let silhouette = canvas.mask_of(transparent.unwrap_or(255));
        let opaque = canvas
            .pixels()
            .iter()
            .filter(|i| transparent != Some(**i))
            .count();
        opaque_total += opaque;
        let before_blocking = blocking(&canvas, &palette);

        let (mut c, mut p) = (canvas.clone(), palette.clone());
        let report = add_antialiasing(&mut c, &mut p, &opts).expect("AA を付けられない");
        painted_total += report.painted;

        // **シルエットを触らない** (D34 — 外郭は既定で対象外)
        if c.mask_of(transparent.unwrap_or(255)) != silhouette {
            failures.push(format!("{name}: シルエットが動いた"));
        }
        // **lint の blocking を増やさない**
        let after = blocking(&c, &p);
        if after > before_blocking {
            failures.push(format!("{name}: blocking が {before_blocking} → {after}"));
        }
        // **置きすぎない** (ルール 14 «AA 過多» の相手にならない)
        if opaque > 0 && report.painted * 100 > opaque * 15 {
            failures.push(format!(
                "{name}: 不透明 {opaque} 画素中 {} 画素に置いた (15% 超)",
                report.painted
            ));
        }
        // **色を増やしすぎない**
        if report.added_colors > 8 {
            failures.push(format!("{name}: 色を {} 増やした", report.added_colors));
        }
    }

    eprintln!(
        "{checked} 枚 (不透明 {opaque_total} 画素) に {painted_total} 画素置いた ({:.1}%)",
        painted_total as f32 / opaque_total as f32 * 100.0
    );
    assert!(
        failures.is_empty(),
        "AA が絵を壊した ({} 件)\n{}",
        failures.len(),
        failures.join("\n")
    );
}
