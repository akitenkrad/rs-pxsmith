//! **`px smooth` を本物のドット絵に掛けて «壊していないか» を見る．**
//!
//! `px-core` 側の試験は階段と円で «直ること» を見ている．ここで見るのは逆で，
//! **良い絵 (CC0 の実物) に掛けたときに何も壊れないこと**である．
//!
//! | 何が壊れると落ちるか | |
//! | --- | --- |
//! | 直した先で別のジャギーを作る | 残りが元より増える |
//! | 収束しない (直しては壊すを繰り返す) | 2 回目で画素が動く |
//! | 画素を «足して» 形を変える | 面積の変化が動かした数を超える |
//! | 整形が lint に落ちる絵を作る | blocking が増える |
//!
//! 全件の集計は `px-calib jaggy --apply` にある (61 枚で 104 画素 ・blocking の増加 0) ．

use std::path::{Path, PathBuf};

use px_core::canvas::IndexedCanvas;
use px_core::palette::Palette;
use px_core::smooth::{SmoothOptions, smooth_canvas};

fn seeds_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/grid-eval/seeds")
}

/// PNG をそのまま添字にする (`px-calib` と同じ作法．色を減らさない)．
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

fn opaque_area(canvas: &IndexedCanvas) -> i64 {
    canvas
        .pixels()
        .iter()
        .filter(|i| canvas.transparent() != Some(**i))
        .count() as i64
}

fn blocking(canvas: &IndexedCanvas, palette: &Palette) -> usize {
    let cfg = px_lint::LintConfig::default();
    let mut report = px_lint::rules::lint_palette(palette, &cfg);
    report.extend(px_lint::lint_canvas(canvas, palette, &cfg));
    report.blocking().count()
}

fn seeds() -> Vec<(String, IndexedCanvas, Palette)> {
    let mut files: Vec<PathBuf> = match std::fs::read_dir(seeds_dir()) {
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
            let img = px_io::png::read_rgba(&p).ok()?;
            let (canvas, palette) = index_exactly(&img)?;
            Some((
                p.file_name()?.to_string_lossy().to_string(),
                canvas,
                palette,
            ))
        })
        .collect()
}

/// **良い絵に掛けても壊れない．** 4 つの壊れ方をまとめて見る．
#[test]
fn smoothing_real_pixel_art_breaks_nothing() {
    let seeds = seeds();
    assert!(
        !seeds.is_empty(),
        "種が 1 枚も読めない ({} を確かめる)",
        seeds_dir().display()
    );

    let opts = SmoothOptions::default();
    let mut failures = Vec::new();
    let (mut moved_total, mut touched) = (0usize, 0usize);
    for (name, canvas, palette) in &seeds {
        let before_area = opaque_area(canvas);
        let before_blocking = blocking(canvas, palette);

        let mut smoothed = canvas.clone();
        let report = smooth_canvas(&mut smoothed, &opts);
        moved_total += report.moved;
        if report.moved > 0 {
            touched += 1;
        }

        // **収束している** — もう一度掛けて 1 画素も動かない
        let again = smooth_canvas(&mut smoothed.clone(), &opts);
        if again.moved > 0 {
            failures.push(format!("{name}: 2 回目で {} 画素動いた", again.moved));
        }
        // **足し引きは動かした画素の数まで**
        let delta = (opaque_area(&smoothed) - before_area).abs();
        if delta > report.moved as i64 {
            failures.push(format!(
                "{name}: 面積が {delta} 変わったのに動かしたのは {} 画素",
                report.moved
            ));
        }
        // **lint の blocking を増やさない**
        let after_blocking = blocking(&smoothed, palette);
        if after_blocking > before_blocking {
            failures.push(format!(
                "{name}: blocking が {before_blocking} → {after_blocking} に増えた"
            ));
        }
        // **直した先で増やさない**
        let before_jaggies = px_core::geom::jaggy::analyze_canvas(canvas, opts.max_move)
            .jaggies
            .len();
        if report.remaining > before_jaggies {
            failures.push(format!(
                "{name}: ジャギーが {before_jaggies} → {} に増えた",
                report.remaining
            ));
        }
    }
    eprintln!(
        "{} 枚中 {touched} 枚で {moved_total} 画素を動かした",
        seeds.len()
    );
    assert!(
        failures.is_empty(),
        "整形が絵を壊した ({} 件)\n{}",
        failures.len(),
        failures.join("\n")
    );
}
