//! **`pxsmith sheet pack` を実素材に掛けて «壊していないか» を測る．**
//!
//! 梱包は «並べ替えるだけ» の道具である．したがって不変条件は 1 つ —
//! **升の中身が元の絵と 1 画素も違わないこと**である (D94 の合成と同じ性質) ．
//!
//! 添字ではなく**色**で突き合わせる — パレットを束ねるので添字は当然変わる．
//! ここを添字で見ると «付け替え表が正しいか» を検査できない (自明に一致してしまう) ．
//!
//! 合成した形では捕まらないので実素材を全件通す (M3 の教訓) ．

use std::path::{Path, PathBuf};

use pxsmith_core::canvas::IndexedCanvas;
use pxsmith_core::color::Rgba8;
use pxsmith_core::palette::Palette;
use pxsmith_core::sheet::{PackOptions, SheetItem, choose_columns, pack};

fn seeds() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/grid-eval/seeds")
        .canonicalize()
        .expect("種の置き場所がある")
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

/// **1 枚のシートに載るだけの枚数を取る．**
///
/// 実素材 66 枚を全部載せると **417 色**になり，添字が `u8` (D2) なので載らない —
/// **これは道具の誤りではなく，一様な添字シートという形の限界である**．
/// 限界そのものは別の試験で固定し，こちらは «載る範囲で色が変わらないこと» を見る．
fn items_that_fit() -> Vec<SheetItem> {
    fit(items())
}

/// 寸法の違う絵を必ず混ぜたうえで，載る範囲まで取る．
///
/// 種は 16x16 が 32 枚 ・32x32 が 32 枚である．**素直に先頭から取ると片方の
/// 寸法しか入らず**，«升を揃えたぶんの無駄» を測れない (1 度これで測り損ねた) ．
fn items_of_mixed_size() -> Vec<SheetItem> {
    let all = items();
    let mut small: Vec<SheetItem> = all
        .iter()
        .filter(|i| i.canvas.width() == 16)
        .cloned()
        .collect();
    let large: Vec<SheetItem> = all
        .iter()
        .filter(|i| i.canvas.width() != 16)
        .cloned()
        .collect();
    // 大小を交互に並べてから «載る範囲» を取る
    let mut mixed = Vec::new();
    for (a, b) in large.into_iter().zip(small.drain(..)) {
        mixed.push(a);
        mixed.push(b);
    }
    fit(mixed)
}

fn fit(all: Vec<SheetItem>) -> Vec<SheetItem> {
    let mut colours: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for item in all {
        let mut next = colours.clone();
        for v in item.canvas.pixels() {
            if let Some(c) = item.palette.get(*v) {
                next.insert(u32::from_be_bytes([c.r, c.g, c.b, c.a]));
            }
        }
        if next.len() > 256 {
            continue;
        }
        colours = next;
        out.push(item);
    }
    out
}

fn items() -> Vec<SheetItem> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(seeds())
        .expect("種を読める")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")))
        .collect();
    files.sort();
    files
        .iter()
        .filter_map(|p| {
            let img = pxsmith_io::png::read_rgba(p).ok()?;
            let (canvas, palette) = index_exactly(&img)?;
            Some(SheetItem {
                name: p.file_stem()?.to_string_lossy().to_string(),
                canvas,
                palette,
                duration_ms: 100,
            })
        })
        .collect()
}

/// 色で突き合わせる (透明どうしは同じとみなす)．
fn same_colour(a: Option<Rgba8>, b: Option<Rgba8>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) if x.a == 0 && y.a == 0 => true,
        (x, y) => x == y,
    }
}

/// **壊れると: 梱包が絵の色を変える．**
///
/// これが成り立っていれば «並べたら色が変わった» は起こりえない．
/// 隙間 ・外周を付けた場合も含めて，升の中身を全画素突き合わせる．
#[test]
fn packing_real_art_changes_no_colour_at_all() {
    let items = items_that_fit();
    assert!(items.len() > 10, "実素材が {} 枚しかない", items.len());

    for (padding, margin, columns) in [(0u32, 0u32, None), (2, 3, None), (0, 0, Some(5u32))] {
        let opts = PackOptions {
            columns,
            padding,
            margin,
        };
        let (sheet, palette, doc, report) = pack(&items, &opts).expect("梱包できる");
        assert_eq!(doc.cells.len(), items.len());
        assert_eq!(report.items, items.len());

        let mut checked = 0usize;
        for (item, cell) in items.iter().zip(&doc.cells) {
            assert_eq!(cell.name, item.name);
            for y in 0..item.canvas.height() as i32 {
                for x in 0..item.canvas.width() as i32 {
                    let want = item.canvas.get(x, y).and_then(|i| item.palette.get(i));
                    let got = sheet
                        .get(cell.x as i32 + x, cell.y as i32 + y)
                        .and_then(|i| palette.get(i));
                    assert!(
                        same_colour(want, got),
                        "{} の ({x},{y}) が {want:?} から {got:?} へ変わった (隙間 {padding} ・外周 {margin})",
                        item.name
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 10_000, "{checked} 画素しか見ていない");
    }
}

/// **壊れると: 升と升のあいだに «前の絵の残り» が漏れる．**
///
/// 隙間を頼んだのに埋まっていなければ，出力先が升をずらして読む．
#[test]
fn the_gap_between_cells_is_transparent() {
    let items = items_that_fit();
    let opts = PackOptions {
        columns: Some(6),
        padding: 2,
        margin: 1,
    };
    let (sheet, palette, doc, _) = pack(&items, &opts).expect("梱包できる");
    let transparent = sheet.transparent().expect("透明がある");

    // 外周の 1 画素と，升のあいだの 2 画素は必ず透明である
    for y in 0..sheet.height() as i32 {
        for x in 0..sheet.width() as i32 {
            let in_cell = doc.cells.iter().any(|c| {
                x >= c.x as i32
                    && y >= c.y as i32
                    && x < (c.x + doc.cell_w) as i32
                    && y < (c.y + doc.cell_h) as i32
            });
            if in_cell {
                continue;
            }
            let v = sheet.get(x, y).expect("画素");
            let colour = palette.get(v).expect("色");
            assert!(
                v == transparent || colour.a == 0,
                "升の外 ({x},{y}) に色が漏れている"
            );
        }
    }
}

/// **壊れると: 同じ入力から違う並びが出る (`.tsx` と食い違う)．**
#[test]
fn packing_is_deterministic_and_the_layout_matches_what_the_rule_says() {
    let items = items_that_fit();
    let opts = PackOptions::default();
    let (a, _, da, _) = pack(&items, &opts).expect("梱包できる");
    let (b, _, db, _) = pack(&items, &opts).expect("梱包できる");
    assert_eq!(da, db, "同じ入力で並びが揺れた");
    assert_eq!(a.pixels(), b.pixels(), "同じ入力で絵が揺れた");
    assert_eq!(da.columns, choose_columns(items.len()));
    assert_eq!(da.rows, (items.len() as u32).div_ceil(da.columns));
    // 書いた文書を自分で読み直せる (寸法と並びの突き合わせも通る)
    let back =
        pxsmith_core::sheet::SheetDoc::from_json(&da.to_json().expect("書ける")).expect("読める");
    assert_eq!(back, da);
}

/// **壊れると: 升を揃えたぶんの無駄が «無い» ことになる．**
///
/// 実素材は寸法がまちまちなので，一様格子は必ず無駄を出す．
/// **無駄が出ること自体は道具の誤りではない** — 黙らずに報告すればよい (D101 と同じ) ．
#[test]
fn the_waste_of_a_uniform_grid_is_reported_not_hidden() {
    let items = items_of_mixed_size();
    let (_, _, doc, report) = pack(&items, &PackOptions::default()).expect("梱包できる");
    let sizes: std::collections::BTreeSet<(u32, u32)> = items
        .iter()
        .map(|i| (i.canvas.width(), i.canvas.height()))
        .collect();
    assert!(sizes.len() > 1, "寸法が 1 通りしかない素材で測っている");
    assert!(
        report.smaller_than_cell > 0,
        "升より小さい絵が 0 枚と報告された"
    );
    assert!(report.waste > 0.0, "無駄が 0 と報告された");
    // 報告と実際が一致していること
    let used: usize = items
        .iter()
        .map(|i| (i.canvas.width() * i.canvas.height()) as usize)
        .sum();
    let total = (doc.width * doc.height) as usize;
    let want = 1.0 - used as f32 / total as f32;
    assert!((report.waste - want).abs() < 1e-6, "報告と実際が食い違う");
}

/// **壊れると: 256 色を超えたシートを黙って作る (減色するか，色が入れ替わる)．**
///
/// 実素材 66 枚を 1 枚に並べると 417 色になる．**1 枚のシートは 1 つのパレットしか
/// 持てない** (添字は `u8`．D2) ので載らない — これは «並べ方» で回避できる話では
/// ないので，**何色になったかを言って落とす**．
#[test]
fn too_many_colours_for_one_sheet_is_refused_with_the_count() {
    let all = items();
    let err = pack(&all, &PackOptions::default()).expect_err("落ちるはず");
    let pxsmith_core::error::CoreError::SheetTooManyColors { colors, items } = err else {
        panic!("別のエラーで落ちた: {err}");
    };
    assert_eq!(items, all.len());
    assert!(colors > 256, "{colors} 色で落ちた");
    // 載る範囲まで減らせば通る
    assert!(pack(&items_that_fit(), &PackOptions::default()).is_ok());
}
