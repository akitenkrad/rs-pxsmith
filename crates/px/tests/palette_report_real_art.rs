//! **書籍の «主な色を 2 〜 3 色に収める» を実素材で数える** (設計書 5 章 ・G5)．
//!
//! 参考書籍の第四章 «色のデザイン» (PAGE:104) の勧めである．**閾値を決める前に
//! 主張を測る** — «主な色» が実際の絵で何色なのかを数えないと，
//! `px palette report` が出す数字を読む基準が無い．
//!
//! **数えるのは «絵の何割を何色が占めているか»** である．書籍の «2 〜 3 色» は
//! 面積の大半を担う色の数と読める — 12 色使っていても，そのうち 3 色で 8 割を
//! 占めるなら «主な色は 3 色» である．
//!
//! **飛ばした件も数える** (D128)．

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use px_core::canvas::{IndexedCanvas, RgbaCanvas};
use px_core::color::Rgba8;
use px_core::palette::Palette;
use px_core::palreport::report;

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

fn median(mut v: Vec<usize>) -> usize {
    if v.is_empty() {
        return 0;
    }
    v.sort_unstable();
    v[v.len() / 2]
}

/// **書籍の «主な色は 2 〜 3 色» が実素材で成り立つかを数える．**
///
/// 成り立たなくても道具の誤りではない — 数えた結果をそのまま記録する
/// (D101 «削減率は入力で決まるので報告するだけ» と同じ側)．
#[test]
fn how_many_colours_carry_a_real_sprite() {
    let (mut n, mut skipped) = (0usize, 0usize);
    let (mut used, mut c50, mut c80, mut c90) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut hist80: BTreeMap<usize, usize> = BTreeMap::new();

    for path in png_files(&seeds()) {
        let Ok(img) = px_io::png::read_rgba(&path) else {
            skipped += 1;
            continue;
        };
        let Some((canvas, palette)) = index_exactly(&img) else {
            skipped += 1;
            continue;
        };
        let r = report(&canvas, &palette, 3);
        if r.opaque == 0 {
            skipped += 1;
            continue;
        }
        n += 1;
        used.push(r.used);
        c50.push(r.cover(50));
        c80.push(r.cover(80));
        c90.push(r.cover(90));
        *hist80.entry(r.cover(80)).or_default() += 1;

        // **積み上げは単調** — 覆う割合を上げれば要る色数は減らない
        assert!(
            r.cover(50) <= r.cover(80) && r.cover(80) <= r.cover(90),
            "{path:?}: 覆う色数が単調でない ({} / {} / {})",
            r.cover(50),
            r.cover(80),
            r.cover(90)
        );
        assert!(
            r.cover(95) <= r.used,
            "{path:?}: 使った色数より多くの色が要ると出た"
        );
    }

    assert!(n >= 60, "測れた素材が足りない: {n} (飛ばした {skipped})");
    println!(
        "実素材 {n} 枚 (飛ばした {skipped}) — 使った色数の中央 {} ・\
         50% を覆う色数の中央 {} ・80% の中央 {} ・90% の中央 {}",
        median(used),
        median(c50),
        median(c80.clone()),
        median(c90)
    );
    let within = c80.iter().filter(|k| **k <= 3).count();
    println!(
        "  書籍の «主な色は 2 〜 3 色» を 80% の基準で読むと {within} / {n} 枚 \
         ({:.1}%) が収まっている",
        within as f32 / n as f32 * 100.0
    );
    let spread: Vec<String> = hist80
        .iter()
        .map(|(colours, count)| format!("{colours} 色 = {count} 枚"))
        .collect();
    println!("  80% を覆う色数の分布: {}", spread.join(" ・"));
}

/// **壊れると: 散らばった色を «主な色» と読み違える．**
///
/// 同じ添字をぜんぶ足した量と 1 つながりの塊として最大の量は違うので，
/// **実素材でも塊の方が必ず小さいか等しい**．
#[test]
fn the_largest_region_never_exceeds_the_total_area_of_its_colour() {
    let mut n = 0usize;
    let mut scattered = 0usize;
    for path in png_files(&seeds()) {
        let Ok(img) = px_io::png::read_rgba(&path) else {
            continue;
        };
        let Some((canvas, palette)) = index_exactly(&img) else {
            continue;
        };
        let r = report(&canvas, &palette, 3);
        if r.opaque == 0 {
            continue;
        }
        n += 1;
        for c in &r.by_area {
            assert!(
                c.largest_region <= c.area,
                "{path:?}: 添字 {} の塊 {} が合計 {} を超えた",
                c.index,
                c.largest_region,
                c.area
            );
            assert!(c.regions > 0, "面積があるのに領域が 0");
            // 塊が合計の半分に満たない色は «撒いた色» である
            if c.largest_region * 2 < c.area {
                scattered += 1;
            }
        }
    }
    assert!(n >= 60);
    println!("実素材 {n} 枚 — 塊が合計の半分に満たない «撒いた色» が {scattered} 件");
}
