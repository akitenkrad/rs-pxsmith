//! **`px outline` を本物のドット絵に掛けて «壊していないか» を見る．**
//!
//! `px-core` 側の試験は合成した四角で «描けること» を見ている．ここで見るのは逆で，
//! **良い絵 (CC0 の実物) に 5 分類を掛けたときに何も壊れないこと**である．
//!
//! | 何が壊れると落ちるか | |
//! | --- | --- |
//! | シルエットを触る | 透明の位置が変わる (既定は内側に描く) |
//! | 縁取りが lint に落ちる絵を作る | blocking が増える |
//! | 掛け直すと色が沈む ・増える | 2 度目で塗る |
//! | 色を増やしすぎる | 1 枚あたりの追加色が上限を超える |
//!
//! 全件の集計は `px-calib outline` にある (4 分類 x 61 枚で 2 度目 0 ・blocking 増 0)．

use std::path::{Path, PathBuf};

use px_core::canvas::IndexedCanvas;
use px_core::outline::{OutlineOptions, OutlineStyle, outline};
use px_core::palette::Palette;

fn seeds_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/grid-eval/seeds")
}

fn index_exactly(img: &px_core::canvas::RgbaCanvas) -> Option<(IndexedCanvas, Palette)> {
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
    let cfg = px_lint::LintConfig::default();
    let mut report = px_lint::rules::lint_palette(palette, &cfg);
    report.extend(px_lint::lint_canvas(canvas, palette, &cfg));
    report.blocking().count()
}

/// **5 分類のどれを掛けても壊れない．**
#[test]
fn outlining_real_pixel_art_breaks_nothing() {
    let mut files: Vec<PathBuf> = match std::fs::read_dir(seeds_dir()) {
        Ok(it) => it
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")))
            .collect(),
        Err(_) => Vec::new(),
    };
    files.sort();
    assert!(!files.is_empty(), "種が 1 枚も無い");

    let styles = [
        OutlineStyle::Black,
        OutlineStyle::Tinted,
        OutlineStyle::Contrast,
        OutlineStyle::Shaded,
    ];
    let mut failures = Vec::new();
    let (mut checked, mut painted_total) = (0usize, 0usize);

    for path in &files {
        let Ok(img) = px_io::png::read_rgba(path) else {
            continue;
        };
        let Some((canvas, palette)) = index_exactly(&img) else {
            continue;
        };
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let transparent = canvas.transparent().unwrap_or(255);
        let silhouette = canvas.mask_of(transparent);
        let before_blocking = blocking(&canvas, &palette);

        for style in styles {
            checked += 1;
            let opts = OutlineOptions {
                style,
                ..OutlineOptions::default()
            };
            let (mut c, mut p) = (canvas.clone(), palette.clone());
            let report = outline(&mut c, &mut p, &opts).expect("縁取りを描けない");
            painted_total += report.painted;
            let tag = format!("{name} / {}", style.as_str());

            // **シルエットを触らない** (既定は内側に描く)
            if c.mask_of(transparent) != silhouette {
                failures.push(format!("{tag}: シルエットが動いた"));
            }
            // **lint の blocking を増やさない**
            let after = blocking(&c, &p);
            if after > before_blocking {
                failures.push(format!("{tag}: blocking が {before_blocking} → {after}"));
            }
            // **色を増やしすぎない**
            if report.added_colors > opts.max_new_colors {
                failures.push(format!(
                    "{tag}: 色を {} 増やした (上限 {})",
                    report.added_colors, opts.max_new_colors
                ));
            }
            // **掛け直しても動かない**
            let (mut c2, mut p2) = (c.clone(), p.clone());
            let again = outline(&mut c2, &mut p2, &opts).expect("縁取りを描けない");
            if again.painted > 0 {
                failures.push(format!("{tag}: 2 度目で {} 画素塗った", again.painted));
            }
        }
    }

    eprintln!("{checked} 通りで {painted_total} 画素を描いた");
    assert!(
        failures.is_empty(),
        "縁取りが絵を壊した ({} 件)\n{}",
        failures.len(),
        failures.join("\n")
    );
}
