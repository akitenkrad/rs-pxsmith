//! スプライトシートの梱包 (設計書 5 章 `op = "sheet.pack"`)．
//!
//! **並べ方を決めるのはここである．** `pxsmith export tiled` は `.tsx` に列数と画像の
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

use crate::canvas::{IndexedCanvas, RgbaCanvas};
use crate::color::Rgba8;
use crate::error::{CoreError, Result};
use crate::math::ivec2;
use crate::palette::Palette;

pub const FORMAT_VERSION: u32 = 1;
pub const PALETTE_SWAP_FORMAT_VERSION: u32 = 2;

/// 梱包する 1 枚．
#[derive(Clone, Debug)]
pub struct SheetItem {
    pub name: String,
    pub canvas: IndexedCanvas,
    pub palette: Palette,
    /// アニメーションのタイミング (D40)．タイルセットなら 0 でよい．
    pub duration_ms: u32,
}

/// パレットスワップ用に梱包する 1 枚と，そのパレット行の表示名．
#[derive(Clone, Debug)]
pub struct PaletteSwapItem {
    pub item: SheetItem,
    /// 元のパレットファイル名の語幹．無ければ行番号から名前を作る．
    pub palette_name: Option<String>,
    /// L0 が参照する `.hex` など，アトラスへそのまま載せる元パレット．
    /// 無ければフレームのパレットを使う．
    pub atlas_palette: Option<Palette>,
}

/// 基のパレットに対応させる色違い．
#[derive(Clone, Debug)]
pub struct PaletteVariant {
    pub name: String,
    pub base: Palette,
    pub palette: Palette,
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
    /// format 2 でこの升が参照する基のパレット行．
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<u32>,
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

/// パレットアトラスの 1 行．
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaletteRow {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant_of: Option<u32>,
}

/// 添字テクスチャと対になるパレットアトラス．
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaletteAtlas {
    pub image: String,
    pub width: u32,
    pub rows: Vec<PaletteRow>,
}

/// シートの正規メタ．**`pxsmith export tiled` はここからしか読まない．**
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palettes: Option<PaletteAtlas>,
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
        if !matches!(doc.format, FORMAT_VERSION | PALETTE_SWAP_FORMAT_VERSION) {
            return Err(CoreError::SheetVersion { found: doc.format });
        }
        match doc.format {
            FORMAT_VERSION => {
                if doc.palettes.is_some() {
                    return Err(CoreError::SheetPaletteMetadata {
                        message: "format 1 に palettes がある".to_string(),
                    });
                }
                if doc.cells.iter().any(|cell| cell.palette.is_some()) {
                    return Err(CoreError::SheetPaletteMetadata {
                        message: "format 1 の升に palette がある".to_string(),
                    });
                }
            }
            PALETTE_SWAP_FORMAT_VERSION => validate_palette_metadata(&doc)?,
            _ => unreachable!("版は直前で検査した"),
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

fn validate_palette_metadata(doc: &SheetDoc) -> Result<()> {
    let atlas = doc
        .palettes
        .as_ref()
        .ok_or_else(|| CoreError::SheetPaletteMetadata {
            message: "format 2 に palettes が無い".to_string(),
        })?;
    if atlas.width == 0 {
        return Err(CoreError::SheetPaletteMetadata {
            message: "パレットアトラスの幅が 0".to_string(),
        });
    }
    if atlas.rows.is_empty() {
        return Err(CoreError::SheetPaletteMetadata {
            message: "パレットアトラスに行が無い".to_string(),
        });
    }
    let mut names = std::collections::BTreeSet::new();
    for row in &atlas.rows {
        if !names.insert(&row.name) {
            return Err(CoreError::SheetPaletteMetadata {
                message: format!("行名 '{}' が重複している", row.name),
            });
        }
    }
    for (row_index, row) in atlas.rows.iter().enumerate() {
        let Some(base) = row.variant_of else {
            continue;
        };
        let Some(base_row) = atlas.rows.get(base as usize) else {
            return Err(CoreError::SheetPaletteMetadata {
                message: format!("行 {row_index} の variant_of = {base} が範囲外"),
            });
        };
        if base_row.variant_of.is_some() {
            return Err(CoreError::SheetPaletteMetadata {
                message: format!("行 {row_index} の variant_of = {base} が色違い行を指す"),
            });
        }
    }
    for cell in &doc.cells {
        let row_index = cell
            .palette
            .ok_or_else(|| CoreError::SheetPaletteMetadata {
                message: format!("升 {} に palette が無い", cell.index),
            })?;
        let Some(row) = atlas.rows.get(row_index as usize) else {
            return Err(CoreError::SheetPaletteMetadata {
                message: format!("升 {} の palette = {row_index} が範囲外", cell.index),
            });
        };
        if row.variant_of.is_some() {
            return Err(CoreError::SheetPaletteMetadata {
                message: format!("升 {} が色違い行 {row_index} を指す", cell.index),
            });
        }
    }
    Ok(())
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
            palette: None,
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
        palettes: None,
    };
    Ok((sheet, palette, doc, report))
}

/// 絵ごとの添字を変えずに，RGBA8 の添字テクスチャとパレットアトラスへ梱包する．
///
/// 添字テクスチャをグレースケール PNG にしないのは，ゲームエンジンの取り込み時に
/// 色空間や画素形式が変換され，添字が別の値になる余地を減らすためである．
pub fn pack_palette_swap(
    items: &[PaletteSwapItem],
    variants: &[PaletteVariant],
    opts: &PackOptions,
) -> Result<(RgbaCanvas, RgbaCanvas, SheetDoc, PackReport)> {
    if items.is_empty() {
        return Err(CoreError::SheetNoItems);
    }
    if let Some(0) = opts.columns {
        return Err(CoreError::ExportBadColumns);
    }

    let cell_w = items
        .iter()
        .map(|i| i.item.canvas.width())
        .max()
        .unwrap_or(1);
    let cell_h = items
        .iter()
        .map(|i| i.item.canvas.height())
        .max()
        .unwrap_or(1);
    let columns = opts.columns.unwrap_or_else(|| choose_columns(items.len()));
    let rows = (items.len() as u32).div_ceil(columns);
    let width = opts.margin * 2 + columns * cell_w + columns.saturating_sub(1) * opts.padding;
    let height = opts.margin * 2 + rows * cell_h + rows.saturating_sub(1) * opts.padding;

    let mut palettes: Vec<Palette> = Vec::new();
    let mut palette_rows: Vec<PaletteRow> = Vec::new();
    let mut item_rows = Vec::with_capacity(items.len());
    for input in items {
        let source_palette = input.atlas_palette.as_ref().unwrap_or(&input.item.palette);
        let row = palettes
            .iter()
            .position(|palette| palette.entries() == source_palette.entries())
            .unwrap_or_else(|| {
                let row = palettes.len();
                palettes.push(source_palette.clone());
                palette_rows.push(PaletteRow {
                    name: input
                        .palette_name
                        .clone()
                        .unwrap_or_else(|| format!("palette{row}")),
                    variant_of: None,
                });
                row
            });
        item_rows.push(row as u32);
    }

    let mut row_names = std::collections::BTreeSet::new();
    for row in &palette_rows {
        if !row_names.insert(row.name.clone()) {
            return Err(CoreError::SheetPaletteNameDuplicate {
                name: row.name.clone(),
            });
        }
    }
    for variant in variants {
        let base_row = palettes
            .iter()
            .position(|palette| palette.entries() == variant.base.entries())
            .ok_or_else(|| CoreError::SheetVariantBaseMissing {
                name: variant.name.clone(),
            })?;
        if variant.palette.len() != variant.base.len() {
            return Err(CoreError::SheetVariantLengthMismatch {
                name: variant.name.clone(),
                expected: variant.base.len(),
                actual: variant.palette.len(),
            });
        }
        if let Some(index) = variant
            .base
            .entries()
            .iter()
            .zip(variant.palette.entries())
            .position(|(base, changed)| (base.a == 0) != (changed.a == 0))
        {
            return Err(CoreError::SheetVariantTransparencyMismatch {
                name: variant.name.clone(),
                index,
            });
        }
        if !row_names.insert(variant.name.clone()) {
            return Err(CoreError::SheetPaletteNameDuplicate {
                name: variant.name.clone(),
            });
        }
        palettes.push(variant.palette.clone());
        palette_rows.push(PaletteRow {
            name: variant.name.clone(),
            variant_of: Some(base_row as u32),
        });
    }

    let atlas_width = palettes.iter().map(Palette::len).max().unwrap_or(0) as u32;
    let mut atlas = RgbaCanvas::filled(atlas_width, palette_rows.len() as u32, Rgba8::TRANSPARENT);
    for (y, palette) in palettes.iter().enumerate() {
        for (x, color) in palette.entries().iter().enumerate() {
            atlas.set(x as i32, y as i32, *color);
        }
    }

    let mut indices = RgbaCanvas::filled(width, height, Rgba8::TRANSPARENT);
    let mut cells = Vec::with_capacity(items.len());
    let mut smaller = 0usize;
    for (index, input) in items.iter().enumerate() {
        let item = &input.item;
        let col = index as u32 % columns;
        let row = index as u32 / columns;
        let x = opts.margin + col * (cell_w + opts.padding);
        let y = opts.margin + row * (cell_h + opts.padding);
        for sy in 0..item.canvas.height() as i32 {
            for sx in 0..item.canvas.width() as i32 {
                let source_index = item.canvas.get(sx, sy).expect("走査範囲はキャンバスの内側");
                let (_, transparent) = source_color(item, source_index)?;
                if !transparent {
                    indices.set(
                        x as i32 + sx,
                        y as i32 + sy,
                        Rgba8::new(source_index, 0, 0, 255),
                    );
                }
            }
        }
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
            palette: Some(item_rows[index]),
        });
    }

    let used: usize = items
        .iter()
        .map(|i| (i.item.canvas.width() * i.item.canvas.height()) as usize)
        .sum();
    let total = (width * height) as usize;
    let report = PackReport {
        items: items.len(),
        empty_cells: (columns * rows) as usize - items.len(),
        waste: 1.0 - used as f32 / total.max(1) as f32,
        colors: atlas_width as usize,
        smaller_than_cell: smaller,
    };
    let doc = SheetDoc {
        // format 1 の読み手が添字テクスチャを通常の色画像として黙って扱わないよう，
        // パレットスワップ出力だけ版を上げる．
        format: PALETTE_SWAP_FORMAT_VERSION,
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
        palettes: Some(PaletteAtlas {
            image: String::new(),
            width: atlas_width,
            rows: palette_rows,
        }),
    };
    Ok((indices, atlas, doc, report))
}

/// 添字テクスチャとパレットアトラスを通常の RGBA 画像へ復元する．
///
/// `variant` を指定したときは，その行が `variant_of` で指す基の升だけを差し替える．
/// 他の升は各自の基の行を使うため，複数のパレットを混ぜたシートでも形を保てる．
pub fn resolve(
    indices: &RgbaCanvas,
    palette_atlas: &RgbaCanvas,
    doc: &SheetDoc,
    variant: Option<&str>,
) -> Result<RgbaCanvas> {
    if doc.format != PALETTE_SWAP_FORMAT_VERSION {
        return Err(CoreError::SheetResolveRequiresFormat2 { found: doc.format });
    }
    validate_palette_metadata(doc)?;
    let atlas_meta = doc
        .palettes
        .as_ref()
        .expect("validate_palette_metadata が存在を検査した");
    if (indices.width(), indices.height()) != (doc.width, doc.height) {
        return Err(CoreError::SheetResolveImageSize {
            image: "添字テクスチャ",
            width: indices.width(),
            height: indices.height(),
            expected_w: doc.width,
            expected_h: doc.height,
        });
    }
    let atlas_height = atlas_meta.rows.len() as u32;
    if (palette_atlas.width(), palette_atlas.height()) != (atlas_meta.width, atlas_height) {
        return Err(CoreError::SheetResolveImageSize {
            image: "パレットアトラス",
            width: palette_atlas.width(),
            height: palette_atlas.height(),
            expected_w: atlas_meta.width,
            expected_h: atlas_height,
        });
    }
    let selected = variant
        .map(|name| {
            atlas_meta
                .rows
                .iter()
                .enumerate()
                .find_map(|(row, metadata)| {
                    (metadata.name == name)
                        .then_some(metadata.variant_of.map(|base| (row as u32, base)))
                        .flatten()
                })
                .ok_or_else(|| CoreError::SheetResolveVariantMissing {
                    name: name.to_string(),
                })
        })
        .transpose()?;

    let mut out = RgbaCanvas::filled(doc.width, doc.height, Rgba8::TRANSPARENT);
    for cell in &doc.cells {
        let base_row = cell
            .palette
            .expect("validate_palette_metadata が升の行を検査した");
        let row = match selected {
            Some((variant_row, variant_base)) if variant_base == base_row => variant_row,
            _ => base_row,
        };
        for dy in 0..cell.h as i32 {
            for dx in 0..cell.w as i32 {
                let x = cell.x as i32 + dx;
                let y = cell.y as i32 + dy;
                let Some(encoded) = indices.get(x, y) else {
                    continue;
                };
                if encoded.a == 0 {
                    continue;
                }
                if encoded.r as u32 >= atlas_meta.width {
                    return Err(CoreError::SheetResolveIndexOutOfPalette {
                        cell: cell.name.clone(),
                        index: encoded.r,
                        width: atlas_meta.width,
                    });
                }
                let color = palette_atlas
                    .get(encoded.r as i32, row as i32)
                    .expect("アトラス寸法と行・添字を検査した");
                out.set(x, y, color);
            }
        }
    }
    Ok(out)
}

/// 素材の添字が指す色と，シートにおける透明性を返す．
///
/// 透明の定義は **パレット色の alpha が 0** の 1 つだけである．
/// `IndexedCanvas::transparent` は他の道具が使う付帯情報だが，CLI のレイヤ平坦化は
/// 透明色が無い場合にも受け皿として添字 0 を設定する．それを画素の意味として
/// 読むと，不透明な添字 0 が消えて従来の `map_of` と食い違う．
fn source_color(item: &SheetItem, index: u8) -> Result<(Rgba8, bool)> {
    let color = item
        .palette
        .get(index)
        .ok_or(CoreError::SheetIndexOutOfPalette {
            name: item.name.clone(),
            index,
            len: item.palette.len(),
        })?;
    Ok((color, color.a == 0))
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
        let (color, transparent_color) = source_color(item, i as u8)?;
        if transparent_color {
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
        let old = r#"{"format":3,"image":"s.png","width":8,"height":8,
            "columns":1,"rows":1,"cell_w":8,"cell_h":8,"cells":[]}"#;
        assert!(matches!(
            SheetDoc::from_json(old),
            Err(CoreError::SheetVersion { found: 3, .. })
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

    /// **壊れると: 絵ごとの色数は上限内なのに，併合後の色数だけを理由に梱包できない．**
    #[test]
    fn palette_swap_packs_more_than_256_colours_without_merging_indices() {
        let make = |name: &str, offset: u8| {
            let colors: Vec<Rgba8> = (0..200u16)
                .map(|i| Rgba8::rgb(i as u8, offset, (i / 2) as u8))
                .collect();
            let palette = Palette::new(colors).expect("各パレットは 256 色以内");
            let pixels: Vec<u8> = (0..200u16).map(|i| i as u8).collect();
            PaletteSwapItem {
                item: SheetItem {
                    name: name.to_string(),
                    canvas: IndexedCanvas::from_pixels(200, 1, pixels).expect("画素数"),
                    palette,
                    duration_ms: 0,
                },
                palette_name: Some(name.to_string()),
                atlas_palette: None,
            }
        };
        let items = vec![make("first", 10), make("second", 20)];

        assert!(matches!(
            pack(
                &items.iter().map(|i| i.item.clone()).collect::<Vec<_>>(),
                &PackOptions::default()
            ),
            Err(CoreError::SheetTooManyColors { .. })
        ));
        let (indices, atlas, doc, _) =
            pack_palette_swap(&items, &[], &PackOptions::default()).expect("添字なら梱包できる");

        assert_eq!((indices.width(), indices.height()), (400, 1));
        assert_eq!((atlas.width(), atlas.height()), (200, 2));
        assert_eq!(indices.get(199, 0), Some(Rgba8::new(199, 0, 0, 255)));
        assert_eq!(indices.get(200, 0), Some(Rgba8::new(0, 0, 0, 255)));
        assert_eq!(doc.format, 2);
    }

    /// **壊れると: 透明色が添字として残るか，空き升へ不透明な添字 0 が入る．**
    #[test]
    fn palette_swap_keeps_indices_but_makes_all_kinds_of_empty_pixels_transparent() {
        let palette = Palette::new(vec![
            Rgba8::rgb(1, 2, 3),
            Rgba8::rgb(4, 5, 6),
            Rgba8::TRANSPARENT,
        ])
        .expect("パレット");
        let make = |name: &str| PaletteSwapItem {
            item: SheetItem {
                name: name.to_string(),
                canvas: IndexedCanvas::from_pixels(3, 1, vec![0, 1, 2])
                    .expect("画素")
                    .with_transparent(Some(2)),
                palette: palette.clone(),
                duration_ms: 0,
            },
            palette_name: Some("shared".to_string()),
            atlas_palette: None,
        };
        let opts = PackOptions {
            columns: Some(2),
            padding: 1,
            margin: 1,
        };
        let (indices, _, doc, _) =
            pack_palette_swap(&[make("a"), make("b"), make("c")], &[], &opts).expect("梱包");

        let first = &doc.cells[0];
        assert_eq!(
            indices.get(first.x as i32, first.y as i32),
            Some(Rgba8::new(0, 0, 0, 255))
        );
        assert_eq!(
            indices.get(first.x as i32 + 1, first.y as i32),
            Some(Rgba8::new(1, 0, 0, 255))
        );
        assert_eq!(
            indices.get(first.x as i32 + 2, first.y as i32),
            Some(Rgba8::TRANSPARENT),
            "パレット側で透明な色も透明にする"
        );
        assert_eq!(indices.get(0, 0), Some(Rgba8::TRANSPARENT), "外周");
        assert_eq!(
            indices.get((first.x + first.w) as i32, first.y as i32),
            Some(Rgba8::TRANSPARENT),
            "升間の隙間"
        );
        assert_eq!(
            indices.get(
                doc.cells[2].x as i32 + doc.cell_w as i32 + 1,
                doc.cells[2].y as i32
            ),
            Some(Rgba8::TRANSPARENT),
            "空き升"
        );
        assert_eq!(doc.palettes.as_ref().expect("アトラス").rows.len(), 1);
        assert!(doc.cells.iter().all(|cell| cell.palette == Some(0)));
    }

    /// **壊れると: 透明キーを使わない L0 の不透明な添字 0 が消える．**
    ///
    /// CLI のレイヤ平坦化は，透明添字が無いフレームにも `Some(0)` を持たせる．
    /// シートの透明定義はパレット色の alpha だけなので，その付帯情報によらず添字 0 は残る．
    #[test]
    fn opaque_index_zero_survives_with_or_without_a_canvas_transparent_marker() {
        let palette = Palette::new(vec![Rgba8::rgb(38, 43, 68)]).expect("パレット");
        let make = |name: &str, transparent| PaletteSwapItem {
            item: SheetItem {
                name: name.to_string(),
                canvas: IndexedCanvas::filled(1, 1, 0).with_transparent(transparent),
                palette: palette.clone(),
                duration_ms: 0,
            },
            palette_name: Some(name.to_string()),
            atlas_palette: None,
        };
        let swap_items = vec![make("marked", Some(0)), make("unmarked", None)];
        let plain_items: Vec<SheetItem> =
            swap_items.iter().map(|input| input.item.clone()).collect();
        let opts = PackOptions {
            columns: Some(2),
            ..PackOptions::default()
        };

        let (legacy_indices, legacy_palette, _, _) = pack(&plain_items, &opts).expect("従来梱包");
        let expected = resolve_legacy_sheet(&legacy_indices, &legacy_palette);
        let (indices, atlas, doc, _) =
            pack_palette_swap(&swap_items, &[], &opts).expect("パレットスワップ梱包");

        for cell in &doc.cells {
            assert_eq!(
                indices.get(cell.x as i32, cell.y as i32),
                Some(Rgba8::new(0, 0, 0, 255)),
                "{} の不透明な添字 0 が消えた",
                cell.name
            );
        }
        let resolved = resolve(&indices, &atlas, &doc, None).expect("復元");
        assert_eq!(resolved, expected);
    }

    fn swap_item_with_palette(name: &str, palette: Palette) -> PaletteSwapItem {
        PaletteSwapItem {
            item: SheetItem {
                name: name.to_string(),
                canvas: IndexedCanvas::filled(1, 1, 1).with_transparent(Some(0)),
                palette,
                duration_ms: 0,
            },
            palette_name: Some(name.to_string()),
            atlas_palette: None,
        }
    }

    /// **壊れると: 色違いが別の形・別の基へ黙って結び付く．**
    #[test]
    fn invalid_palette_variants_are_rejected_for_each_reason() {
        let base = Palette::new(vec![Rgba8::TRANSPARENT, Rgba8::rgb(1, 2, 3)]).expect("基パレット");
        let item = swap_item_with_palette("base", base.clone());

        let missing_base = PaletteVariant {
            name: "missing".to_string(),
            base: Palette::new(vec![Rgba8::TRANSPARENT, Rgba8::rgb(9, 9, 9)]).expect("別の基"),
            palette: Palette::new(vec![Rgba8::TRANSPARENT, Rgba8::rgb(4, 5, 6)]).expect("色違い"),
        };
        assert!(matches!(
            pack_palette_swap(
                std::slice::from_ref(&item),
                &[missing_base],
                &PackOptions::default()
            ),
            Err(CoreError::SheetVariantBaseMissing { .. })
        ));

        let short = PaletteVariant {
            name: "short".to_string(),
            base: base.clone(),
            palette: Palette::new(vec![Rgba8::TRANSPARENT]).expect("短い色違い"),
        };
        assert!(matches!(
            pack_palette_swap(
                std::slice::from_ref(&item),
                &[short],
                &PackOptions::default()
            ),
            Err(CoreError::SheetVariantLengthMismatch { .. })
        ));

        let shape_change = PaletteVariant {
            name: "shape".to_string(),
            base: base.clone(),
            palette: Palette::new(vec![Rgba8::rgb(4, 5, 6), Rgba8::TRANSPARENT])
                .expect("透明位置が違う色違い"),
        };
        assert!(matches!(
            pack_palette_swap(
                std::slice::from_ref(&item),
                &[shape_change],
                &PackOptions::default()
            ),
            Err(CoreError::SheetVariantTransparencyMismatch { .. })
        ));

        let duplicate = PaletteVariant {
            name: "base".to_string(),
            base,
            palette: Palette::new(vec![Rgba8::TRANSPARENT, Rgba8::rgb(4, 5, 6)]).expect("色違い"),
        };
        assert!(matches!(
            pack_palette_swap(
                std::slice::from_ref(&item),
                &[duplicate],
                &PackOptions::default()
            ),
            Err(CoreError::SheetPaletteNameDuplicate { .. })
        ));
    }

    /// **壊れると: format 1 の読み手が添字テクスチャを色画像として黙って扱うか，**
    /// **壊れた行参照がシェーダ側まで流れる．**
    #[test]
    fn format_two_round_trips_and_rejects_inconsistent_palette_metadata() {
        let base = Palette::new(vec![Rgba8::TRANSPARENT, Rgba8::rgb(1, 2, 3)]).expect("基パレット");
        let variant = PaletteVariant {
            name: "red".to_string(),
            base: base.clone(),
            palette: Palette::new(vec![Rgba8::TRANSPARENT, Rgba8::rgb(200, 10, 10)])
                .expect("色違い"),
        };
        let (_, _, mut doc, _) = pack_palette_swap(
            &[swap_item_with_palette("base", base)],
            &[variant],
            &PackOptions::default(),
        )
        .expect("梱包");
        doc.image = "sheet.png".to_string();
        doc.palettes.as_mut().expect("アトラス").image = "sheet.palettes.png".to_string();
        let text = doc.to_json().expect("書ける");
        assert_eq!(SheetDoc::from_json(&text).expect("読める"), doc);

        let mut value: serde_json::Value = serde_json::from_str(&text).expect("JSON");
        value["format"] = 1.into();
        assert!(
            SheetDoc::from_json(&value.to_string()).is_err(),
            "format 1 に palettes"
        );

        let mut value: serde_json::Value = serde_json::from_str(&text).expect("JSON");
        value.as_object_mut().expect("object").remove("palettes");
        assert!(
            SheetDoc::from_json(&value.to_string()).is_err(),
            "format 2 に palettes が無い"
        );

        let mut value: serde_json::Value = serde_json::from_str(&text).expect("JSON");
        value["cells"][0]["palette"] = 2.into();
        assert!(
            SheetDoc::from_json(&value.to_string()).is_err(),
            "升が範囲外の行を指す"
        );

        let mut value: serde_json::Value = serde_json::from_str(&text).expect("JSON");
        value["cells"][0]["palette"] = 1.into();
        assert!(
            SheetDoc::from_json(&value.to_string()).is_err(),
            "升が色違い行を指す"
        );

        let mut value: serde_json::Value = serde_json::from_str(&text).expect("JSON");
        value["palettes"]["rows"][1]["variant_of"] = 1.into();
        assert!(
            SheetDoc::from_json(&value.to_string()).is_err(),
            "色違いが色違い行を指す"
        );

        let mut value: serde_json::Value = serde_json::from_str(&text).expect("JSON");
        value["palettes"]["width"] = 0.into();
        assert!(
            SheetDoc::from_json(&value.to_string()).is_err(),
            "幅 0 のアトラス"
        );
    }

    fn resolve_legacy_sheet(canvas: &IndexedCanvas, palette: &Palette) -> RgbaCanvas {
        let mut out = RgbaCanvas::filled(canvas.width(), canvas.height(), Rgba8::TRANSPARENT);
        for y in 0..canvas.height() as i32 {
            for x in 0..canvas.width() as i32 {
                let index = canvas.get(x, y).expect("範囲内");
                if canvas.transparent() != Some(index) {
                    out.set(x, y, palette.get(index).unwrap_or(Rgba8::TRANSPARENT));
                }
            }
        }
        out
    }

    /// **壊れると: シェーダ相当の復元が従来の色シートと 1 画素でも食い違う．**
    #[test]
    fn resolving_palette_swap_output_matches_the_legacy_colour_sheet_pixel_for_pixel() {
        let red =
            Palette::new(vec![Rgba8::TRANSPARENT, Rgba8::rgb(180, 20, 30)]).expect("赤パレット");
        let green =
            Palette::new(vec![Rgba8::TRANSPARENT, Rgba8::rgb(20, 180, 30)]).expect("緑パレット");
        let canvas = || {
            IndexedCanvas::from_pixels(2, 2, vec![0, 1, 1, 0])
                .expect("画素")
                .with_transparent(Some(0))
        };
        let plain_items = vec![
            SheetItem {
                name: "red".to_string(),
                canvas: canvas(),
                palette: red.clone(),
                duration_ms: 0,
            },
            SheetItem {
                name: "green".to_string(),
                canvas: canvas(),
                palette: green.clone(),
                duration_ms: 0,
            },
        ];
        let swap_items = vec![
            PaletteSwapItem {
                item: plain_items[0].clone(),
                palette_name: Some("red-base".to_string()),
                atlas_palette: None,
            },
            PaletteSwapItem {
                item: plain_items[1].clone(),
                palette_name: Some("green-base".to_string()),
                atlas_palette: None,
            },
        ];
        let variant = PaletteVariant {
            name: "blue".to_string(),
            base: red,
            palette: Palette::new(vec![Rgba8::TRANSPARENT, Rgba8::rgb(20, 30, 220)])
                .expect("青パレット"),
        };
        let opts = PackOptions {
            columns: Some(2),
            padding: 1,
            margin: 1,
        };
        let (legacy_indices, legacy_palette, _, _) = pack(&plain_items, &opts).expect("従来梱包");
        let expected = resolve_legacy_sheet(&legacy_indices, &legacy_palette);
        let (indices, atlas, doc, _) =
            pack_palette_swap(&swap_items, &[variant], &opts).expect("パレットスワップ梱包");

        let resolved = resolve(&indices, &atlas, &doc, None).expect("基の色で復元");
        assert_eq!(resolved, expected);

        let changed = resolve(&indices, &atlas, &doc, Some("blue")).expect("色違いで復元");
        let red_cell = &doc.cells[0];
        let green_cell = &doc.cells[1];
        assert_eq!(
            changed.get(red_cell.x as i32 + 1, red_cell.y as i32),
            Some(Rgba8::rgb(20, 30, 220))
        );
        assert_eq!(
            changed.get(green_cell.x as i32 + 1, green_cell.y as i32),
            Some(Rgba8::rgb(20, 180, 30)),
            "色違いの基でない升は変えない"
        );
        assert_eq!(
            changed
                .pixels()
                .iter()
                .map(|color| color.a == 0)
                .collect::<Vec<_>>(),
            expected
                .pixels()
                .iter()
                .map(|color| color.a == 0)
                .collect::<Vec<_>>(),
            "透明の形は変えない"
        );
    }
}
