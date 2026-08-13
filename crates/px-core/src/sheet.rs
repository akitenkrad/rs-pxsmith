//! スプライトシートの梱包 (設計書 5 章 `op = "sheet.pack"`)．
//!
//! **並べ方を決めるのはここである．** `px export tiled` は `.tsx` に列数と画像の
//! 寸法を書くが，それは**ここが決めた並びから引く**もので，利用者に聞き直したり
//! 数値を書き写したりしてはいけない — 2 か所で決めると必ず食い違う (D110 で
//! «正規出力が 2 つあるのは正規出力が無いのと同じ» と書いたのと同じ話) ．
//!
//! # 升は揃える
//!
//! 升の大きさは**全部の絵の最大**に揃える．揃えないと `.tsx` の
//! `tilewidth` / `tileheight` が書けない (Tiled も Godot も一様格子しか受け取らない) ．
//! 絵ごとに切り詰めて詰め込む «非一様な梱包» は**実装していない** — 出力先が
//! 受け取れないものを作っても使えないためで，代わりに**無駄になった面積を
//! 必ず報告する** ([`PackReport::waste`]) ．
//!
//! # 列数は数え上げで決める (校正の対象ではない)
//!
//! 空き升が少なく，かつ正方形に近い並びを選ぶ．$N$ 枚に対して
//! $c \in [\lceil \sqrt N \rceil, 2 \lceil \sqrt N \rceil]$ を掃いて，
//! **空き升が最小 ・同点なら縦横比が 1 に近い方**を採る．
//!
//! | 規則 | $N = 1 \ldots 600$ の空き升 合計 | 最悪 | 実際に出る枚数の例 |
//! | --- | --- | --- | --- |
//! | $\lceil \sqrt N \rceil$ 列 | 4600 | 23 | 47 → 7x7 (空き 2) |
//! | 2 の冪の列 | 7055 | 31 | 47 → 8x6 (空き 1) |
//! | 8 列固定 | 2100 | 7 | 47 → 8x6 (空き 1) |
//! | **掃いて最小 (採用)** | **507** | **6** | 47 → 8x6 (空き 1) ・148 → 15x10 (空き 2) |
//!
//! **これは «数え上げ» であって閾値ではない** (D92 ・D101 と同じ側) ．入力の枚数
//! だけで決まるので校正しない．`--columns` で明示されたらそれに従う．
//!
//! # 隙間と外周は既定で 0
//!
//! 出力先によっては升の間に隙間 (Tiled の `spacing`) や外周の余白 (`margin`) を
//! 要求する — 線形補間のときに隣の升の色が滲むのを避けるためである．
//! **ドット絵は最近傍で表示するので既定では要らない**が，こちらでは «滲むかどうか»
//! を測れない (表示側の話である) ．**既定 0 にして，指定されたら `.tsx` まで
//! そのまま流す** — 数えられないものを勝手に決めない (D92) ．

use serde::{Deserialize, Serialize};

use crate::canvas::IndexedCanvas;
use crate::color::Rgba8;
use crate::error::{CoreError, Result};
use crate::math::ivec2;
use crate::palette::Palette;

pub const FORMAT_VERSION: u32 = 1;

/// 梱包する 1 枚．
#[derive(Clone, Debug)]
pub struct SheetItem {
    pub name: String,
    pub canvas: IndexedCanvas,
    pub palette: Palette,
    /// アニメーションのタイミング (D40)．タイルセットなら 0 でよい．
    pub duration_ms: u32,
}

/// 梱包の設定．
#[derive(Copy, Clone, Debug, Default)]
pub struct PackOptions {
    /// 列数．省略すると [`choose_columns`] が決める．
    pub columns: Option<u32>,
    /// 升と升の間の空き (Tiled の `spacing`)．
    pub padding: u32,
    /// 外周の空き (Tiled の `margin`)．
    pub margin: u32,
}

/// シートに載った 1 枚の位置．
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SheetCell {
    pub index: u32,
    pub name: String,
    pub x: u32,
    pub y: u32,
    /// 元の絵の寸法 (升より小さいことがある)．
    pub w: u32,
    pub h: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub duration_ms: u32,
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

/// シートの正規メタ．**`px export tiled` はここからしか読まない．**
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SheetDoc {
    pub format: u32,
    /// シート画像のファイル名 (`.tsx` の `<image source>` になる)．
    pub image: String,
    pub width: u32,
    pub height: u32,
    pub columns: u32,
    pub rows: u32,
    /// 升の寸法．
    pub cell_w: u32,
    pub cell_h: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub padding: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub margin: u32,
    pub cells: Vec<SheetCell>,
}

impl SheetDoc {
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).map_err(|e| CoreError::SheetWrite {
            message: e.to_string(),
        })
    }

    pub fn from_json(text: &str) -> Result<Self> {
        let doc: Self = serde_json::from_str(text).map_err(|e| CoreError::SheetRead {
            message: e.to_string(),
        })?;
        if doc.format != FORMAT_VERSION {
            return Err(CoreError::SheetVersion {
                found: doc.format,
                expected: FORMAT_VERSION,
            });
        }
        // **書いてある並びと画像の寸法が食い違わないこと．**
        // 食い違ったまま `.tsx` を書くと，使う側が升をずらして読む
        let want_w =
            doc.margin * 2 + doc.columns * doc.cell_w + doc.columns.saturating_sub(1) * doc.padding;
        let want_h =
            doc.margin * 2 + doc.rows * doc.cell_h + doc.rows.saturating_sub(1) * doc.padding;
        if doc.width != want_w || doc.height != want_h {
            return Err(CoreError::SheetSizeMismatch {
                width: doc.width,
                height: doc.height,
                expected_w: want_w,
                expected_h: want_h,
            });
        }
        if doc.cells.len() > (doc.columns * doc.rows) as usize {
            return Err(CoreError::SheetTooManyCells {
                cells: doc.cells.len(),
                capacity: (doc.columns * doc.rows) as usize,
            });
        }
        Ok(doc)
    }
}

/// 梱包の結果 (報告用)．
#[derive(Clone, Debug)]
pub struct PackReport {
    pub items: usize,
    /// 空いている升の数．
    pub empty_cells: usize,
    /// **無駄になった面積の割合** — 升を揃えたぶんと空き升のぶんを合わせたもの．
    pub waste: f32,
    pub colors: usize,
    /// 升より小さい絵の枚数 (揃えたぶんの無駄がどこから来たか)．
    pub smaller_than_cell: usize,
}

/// 枚数から列数を決める (モジュールの説明の «掃いて最小») ．
pub fn choose_columns(n: usize) -> u32 {
    if n <= 1 {
        return 1;
    }
    let lo = (n as f64).sqrt().ceil() as u32;
    let hi = lo * 2;
    (lo..=hi)
        .min_by(|a, b| {
            let cost = |c: u32| {
                let rows = (n as u32).div_ceil(c);
                let empty = c * rows - n as u32;
                let aspect = (c as f32 / rows as f32 - 1.0).abs();
                (empty, (aspect * 1000.0) as u32)
            };
            cost(*a).cmp(&cost(*b))
        })
        .unwrap_or(lo)
}

/// 一様格子へ梱包する．
pub fn pack(
    items: &[SheetItem],
    opts: &PackOptions,
) -> Result<(IndexedCanvas, Palette, SheetDoc, PackReport)> {
    if items.is_empty() {
        return Err(CoreError::SheetNoItems);
    }
    if let Some(0) = opts.columns {
        return Err(CoreError::ExportBadColumns);
    }

    // **升は全部の絵の最大に揃える** (出力先が一様格子しか受け取らないため)
    let cell_w = items.iter().map(|i| i.canvas.width()).max().unwrap_or(1);
    let cell_h = items.iter().map(|i| i.canvas.height()).max().unwrap_or(1);

    let columns = opts.columns.unwrap_or_else(|| choose_columns(items.len()));
    let rows = (items.len() as u32).div_ceil(columns);
    let width = opts.margin * 2 + columns * cell_w + columns.saturating_sub(1) * opts.padding;
    let height = opts.margin * 2 + rows * cell_h + rows.saturating_sub(1) * opts.padding;

    // 使っている色だけを束ねる (D93 の合成と同じ作法 — 未使用色で 256 を使い切らない)
    let sources: Vec<(&IndexedCanvas, &Palette)> =
        items.iter().map(|i| (&i.canvas, &i.palette)).collect();
    // **1 枚のシートは 1 つのパレットしか持てない** (添字は `u8`．D2) ．
    // 実素材を並べると普通に超えるので (種 66 枚で 417 色) ，**何色になったかを
    // 言って落とす** — 黙って減色すると «並べたら色が変わった» になる
    let mut palette = Palette::extract_from(sources.iter().copied()).map_err(|e| match e {
        CoreError::PaletteTooLarge(colors) => CoreError::SheetTooManyColors {
            colors,
            items: items.len(),
        },
        other => other,
    })?;
    let transparent = match palette.entries().iter().position(|c| c.a == 0) {
        Some(i) => i as u8,
        None => palette.push(Rgba8::TRANSPARENT)?,
    };

    let mut sheet =
        IndexedCanvas::filled(width, height, transparent).with_transparent(Some(transparent));
    let mut cells = Vec::with_capacity(items.len());
    let mut smaller = 0usize;

    for (index, item) in items.iter().enumerate() {
        let col = index as u32 % columns;
        let row = index as u32 / columns;
        let x = opts.margin + col * (cell_w + opts.padding);
        let y = opts.margin + row * (cell_h + opts.padding);

        let mut moved = item.canvas.clone();
        moved.remap(&map_of(item, &palette, transparent)?)?;
        moved.set_transparent(Some(transparent));
        sheet.blit(&moved, ivec2(x as i32, y as i32), false);

        if item.canvas.width() < cell_w || item.canvas.height() < cell_h {
            smaller += 1;
        }
        cells.push(SheetCell {
            index: index as u32,
            name: item.name.clone(),
            x,
            y,
            w: item.canvas.width(),
            h: item.canvas.height(),
            duration_ms: item.duration_ms,
        });
    }

    let used: usize = items
        .iter()
        .map(|i| (i.canvas.width() * i.canvas.height()) as usize)
        .sum();
    let total = (width * height) as usize;
    let report = PackReport {
        items: items.len(),
        empty_cells: (columns * rows) as usize - items.len(),
        waste: 1.0 - used as f32 / total.max(1) as f32,
        colors: palette.len(),
        smaller_than_cell: smaller,
    };

    let doc = SheetDoc {
        format: FORMAT_VERSION,
        image: String::new(),
        width,
        height,
        columns,
        rows,
        cell_w,
        cell_h,
        padding: opts.padding,
        margin: opts.margin,
        cells,
    };
    Ok((sheet, palette, doc, report))
}

/// 1 枚の添字を併合後の添字へ写す表．
///
/// **写すのは «画素が実際に指している添字» だけ** — 素材のパレットには使っていない
/// 色が普通に入っており，全項目を写そうとすると «併合先に無い» と言って落ちる
/// (D93 で 1 度落ちた) ．
fn map_of(item: &SheetItem, to: &Palette, transparent: u8) -> Result<Vec<u8>> {
    let mut used = [false; 256];
    for v in item.canvas.pixels() {
        used[*v as usize] = true;
    }
    let mut map = vec![transparent; 256];
    for (i, u) in used.iter().enumerate() {
        if !u {
            continue;
        }
        let color = item
            .palette
            .get(i as u8)
            .ok_or(CoreError::SheetIndexOutOfPalette {
                name: item.name.clone(),
                index: i as u8,
                len: item.palette.len(),
            })?;
        if color.a == 0 {
            map[i] = transparent;
            continue;
        }
        let found = to
            .entries()
            .iter()
            .position(|d| *d == color)
            .ok_or(CoreError::ComposeColorLost { color })?;
        map[i] = found as u8;
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str, w: u32, h: u32, fill: u8) -> SheetItem {
        let palette = Palette::new(vec![
            Rgba8::TRANSPARENT,
            Rgba8::new(200, 30, 30, 255),
            Rgba8::new(30, 200, 30, 255),
        ])
        .expect("パレット");
        SheetItem {
            name: name.to_string(),
            canvas: IndexedCanvas::filled(w, h, fill).with_transparent(Some(0)),
            palette,
            duration_ms: 100,
        }
    }

    /// **壊れると: `.tsx` が書く列数と，実際にシートに並んだ列数が食い違う．**
    ///
    /// 並べ方を決めるのは `sheet pack` である — 列数は数え上げなので試験で固定する．
    #[test]
    fn the_column_count_is_the_one_that_leaves_the_fewest_empty_cells() {
        // 47 枚 (autotile の正規形) は 8x6 で空き 1．7x7 だと空き 2 になる
        assert_eq!(choose_columns(47), 8);
        assert_eq!(choose_columns(16), 4);
        assert_eq!(choose_columns(64), 8);
        assert_eq!(choose_columns(1), 1);
        for n in 1..600usize {
            let c = choose_columns(n);
            let rows = (n as u32).div_ceil(c);
            assert!((c * rows) as usize - n <= 6, "{n} 枚で空き升が多すぎる");
        }
    }

    /// **壊れると: シートの寸法と並びが食い違い，使う側が升をずらして読む．**
    #[test]
    fn the_sheet_size_follows_from_the_layout() {
        let items: Vec<SheetItem> = (0..5).map(|i| item(&format!("t{i}"), 8, 8, 1)).collect();
        let (sheet, _, doc, report) = pack(&items, &PackOptions::default()).expect("梱包");
        assert_eq!((doc.columns, doc.rows), (5, 1), "{doc:?}");
        assert_eq!((doc.width, doc.height), (40, 8));
        assert_eq!((sheet.width(), sheet.height()), (40, 8));
        assert_eq!(report.empty_cells, 0);
        // 書いた文書を自分で読み直せること (寸法の検査も通ること)
        let back = SheetDoc::from_json(&doc.to_json().expect("書ける")).expect("読める");
        assert_eq!(back, doc);
    }

    /// **壊れると: 隙間と外周が寸法に反映されず，`.tsx` の升がずれる．**
    #[test]
    fn padding_and_margin_grow_the_sheet_by_exactly_what_was_asked() {
        let items: Vec<SheetItem> = (0..4).map(|i| item(&format!("t{i}"), 8, 8, 1)).collect();
        let opts = PackOptions {
            columns: Some(2),
            padding: 2,
            margin: 3,
        };
        let (_, _, doc, _) = pack(&items, &opts).expect("梱包");
        // 3 + 8 + 2 + 8 + 3 = 24
        assert_eq!((doc.width, doc.height), (24, 24));
        assert_eq!(doc.cells[1].x, 3 + 8 + 2);
        assert_eq!(doc.cells[2].y, 3 + 8 + 2);
        assert!(SheetDoc::from_json(&doc.to_json().expect("書ける")).is_ok());
    }

    /// **壊れると: 寸法の違う絵を混ぜたときに «升より大きい» 絵が切れる．**
    ///
    /// 升は最大に揃える．揃えたぶんの無駄は黙らずに報告する．
    #[test]
    fn cells_are_sized_to_the_largest_picture_and_the_waste_is_reported() {
        let items = vec![item("small", 4, 4, 1), item("big", 16, 8, 2)];
        let (sheet, _, doc, report) = pack(&items, &PackOptions::default()).expect("梱包");
        assert_eq!((doc.cell_w, doc.cell_h), (16, 8));
        assert_eq!(report.smaller_than_cell, 1);
        assert!(report.waste > 0.0, "無駄が 0 と報告された");
        // 大きい方が 1 画素も切れていない
        assert_eq!(sheet.width(), 32);
        assert_eq!(doc.cells[1].w, 16);
    }

    /// **壊れると: 併合したパレットで添字がずれ，シートの色が入れ替わる．**
    #[test]
    fn indices_are_remapped_into_the_merged_palette() {
        let mut a = item("a", 4, 4, 1);
        let mut b = item("b", 4, 4, 1);
        // b のパレットは色の並びが違う — 添字 1 が緑である
        b.palette = Palette::new(vec![
            Rgba8::TRANSPARENT,
            Rgba8::new(30, 200, 30, 255),
            Rgba8::new(200, 30, 30, 255),
        ])
        .expect("パレット");
        a.canvas = IndexedCanvas::filled(4, 4, 1).with_transparent(Some(0));
        let (sheet, palette, doc, _) = pack(&[a, b], &PackOptions::default()).expect("梱包");
        let at = |x: i32, y: i32| palette.get(sheet.get(x, y).expect("画素")).expect("色");
        let left = at(doc.cells[0].x as i32, 0);
        let right = at(doc.cells[1].x as i32, 0);
        assert_eq!(left, Rgba8::new(200, 30, 30, 255), "1 枚目が赤でない");
        assert_eq!(right, Rgba8::new(30, 200, 30, 255), "2 枚目が緑でない");
    }

    /// **壊れると: 並びと寸法が食い違う文書を «正しい» と言って読む．**
    #[test]
    fn a_document_whose_size_does_not_match_its_layout_is_an_error() {
        let text = r#"{"format":1,"image":"s.png","width":99,"height":8,
            "columns":5,"rows":1,"cell_w":8,"cell_h":8,"cells":[]}"#;
        assert!(matches!(
            SheetDoc::from_json(text),
            Err(CoreError::SheetSizeMismatch { .. })
        ));
        let old = r#"{"format":2,"image":"s.png","width":8,"height":8,
            "columns":1,"rows":1,"cell_w":8,"cell_h":8,"cells":[]}"#;
        assert!(matches!(
            SheetDoc::from_json(old),
            Err(CoreError::SheetVersion { found: 2, .. })
        ));
    }

    /// **壊れると: 空の入力から «空のシート» を書き，使う側で気付けない．**
    #[test]
    fn packing_nothing_is_an_error() {
        assert!(matches!(
            pack(&[], &PackOptions::default()),
            Err(CoreError::SheetNoItems)
        ));
    }
}
