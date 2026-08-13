//! 出力先アダプタ (設計書 4.4)．
//!
//! 設計書 4.4 は «独自 JSON がエンジン非依存の**正規出力**．Godot / Tiled は
//! **アダプタ**» と定める．ここは [`crate::tilejson::TilesetDoc`] からの変換だけを
//! 持ち，**元の絵は見ない**．
//!
//! > [!warning] **これは «出典どおりに書く» 仕事である** (D92 と同じ性質) ．
//! > 数え上げでも校正でもない — 出力先の仕様が決めていることを写すだけである．
//! > **仕様で対応が取れないものは書かない．**
//!
//! # Tiled は書ける．Godot は仕様を引くまで書かない
//!
//! | 出力先 | 反転の載せ方 | こちらとの対応 |
//! | --- | --- | --- |
//! | **Tiled (`.tmx`)** | GID の上位 3 ビット | **1 対 1** — 書ける |
//! | Godot (terrain set) | peering bit | **未確認** — 引くまで書かない |
//!
//! Tiled は升の GID の上位ビットに反転を載せる．**この対応は 1 対 1 で確かめられる**
//! ので写せる．Godot の terrain set の «peering bit» がこちらの 8 近傍 bitmask と
//! 1 対 1 に対応するとは限らないため，**推測では書かない** — 推測で書いた出力は
//! 使う側が壊れて初めて分かる (設計書 6.8 の «47 枚が静かに壊れる» と同じ) ．

use crate::error::{CoreError, Result};
use crate::sheet::SheetDoc;
use crate::tilejson::{TileRefJson, TilesetDoc};

/// Tiled が GID の上位ビットに載せる反転の旗．
///
/// **出典どおりの値である** — こちらで決めた数ではない．
pub const TILED_FLIP_H: u32 = 0x8000_0000;
pub const TILED_FLIP_V: u32 = 0x4000_0000;
pub const TILED_FLIP_D: u32 = 0x2000_0000;

/// 升 1 つを Tiled の GID にする．
///
/// **添字は 1 から始まる** (Tiled は 0 を «空の升» に使う) ．`first_gid` は
/// タイルセットの先頭 GID で，Tiled の `.tmx` が `<tileset firstgid=...>` で持つ値．
pub fn tiled_gid(cell: TileRefJson, first_gid: u32) -> u32 {
    let mut gid = first_gid + cell.id;
    if cell.flip_x {
        gid |= TILED_FLIP_H;
    }
    if cell.flip_y {
        gid |= TILED_FLIP_V;
    }
    if cell.flip_d {
        gid |= TILED_FLIP_D;
    }
    gid
}

/// `.tsx` (タイルセット) を書く．
///
/// **並べ方はこちらで決めない．** 決めるのは `px sheet pack` で，ここは
/// [`SheetDoc`] から列数 ・升 ・隙間 ・画像の寸法を**引く**だけである
/// (D110 と同じ理由 — 2 か所で決めると必ず食い違う) ．
///
/// 引くついでに**突き合わせる** — シートの升がタイルの一辺と違ったり，
/// タイルの枚数がシートに載っている枚数を超えていたら，黙って書かずに落とす．
pub fn tiled_tsx(doc: &TilesetDoc, sheet: &SheetDoc, name: &str) -> Result<String> {
    if sheet.columns == 0 {
        return Err(CoreError::ExportBadColumns);
    }
    if sheet.cell_w != doc.tile || sheet.cell_h != doc.tile {
        return Err(CoreError::ExportCellMismatch {
            tile: doc.tile,
            cell_w: sheet.cell_w,
            cell_h: sheet.cell_h,
        });
    }
    if doc.tiles > sheet.cells.len() {
        return Err(CoreError::ExportSheetTooSmall {
            tiles: doc.tiles,
            cells: sheet.cells.len(),
        });
    }
    let spacing = if sheet.padding > 0 {
        format!(" spacing=\"{}\"", sheet.padding)
    } else {
        String::new()
    };
    let margin = if sheet.margin > 0 {
        format!(" margin=\"{}\"", sheet.margin)
    } else {
        String::new()
    };
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <tileset version=\"1.10\" name=\"{name}\" tilewidth=\"{t}\" tileheight=\"{t}\" \
         tilecount=\"{count}\" columns=\"{columns}\"{spacing}{margin}>\n \
         <image source=\"{image}\" width=\"{w}\" height=\"{h}\"/>\n\
         </tileset>\n",
        t = doc.tile,
        count = doc.tiles,
        columns = sheet.columns,
        image = sheet.image,
        w = sheet.width,
        h = sheet.height,
    ))
}

/// `.tmx` (地図) を書く．
///
/// **`map` の節が要る** — `terrain` しか持たない文書には地図が無い．
pub fn tiled_tmx(doc: &TilesetDoc, tileset_source: &str, first_gid: u32) -> Result<String> {
    let map = doc.map.as_ref().ok_or(CoreError::ExportNoMap)?;
    // **区切りは行末に置く** — 行頭に置くと `\n,2,2` となり Tiled が読めない．
    // 最後の升の後ろにだけ区切りを付けない
    let mut csv = String::new();
    let last = map.cells.len().saturating_sub(1);
    for y in 0..map.rows {
        for x in 0..map.cols {
            let at = (y * map.cols + x) as usize;
            csv.push_str(&tiled_gid(map.cells[at], first_gid).to_string());
            if at != last {
                csv.push(',');
            }
        }
        csv.push('\n');
    }
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <map version=\"1.10\" orientation=\"orthogonal\" renderorder=\"right-down\" \
         width=\"{cols}\" height=\"{rows}\" tilewidth=\"{t}\" tileheight=\"{t}\" infinite=\"0\">\n \
         <tileset firstgid=\"{first_gid}\" source=\"{tileset_source}\"/>\n \
         <layer id=\"1\" name=\"tiles\" width=\"{cols}\" height=\"{rows}\">\n  \
         <data encoding=\"csv\">\n{csv}</data>\n \
         </layer>\n\
         </map>\n",
        cols = map.cols,
        rows = map.rows,
        t = doc.tile,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{TileGrid, TileRef};

    /// 試験用のシート — `px sheet pack` が実際に作るのと同じ並びにする．
    fn packed(n: usize, cell: u32, padding: u32, margin: u32) -> SheetDoc {
        use crate::color::Rgba8;
        use crate::palette::Palette;
        use crate::sheet::{PackOptions, SheetItem, pack};
        let palette =
            Palette::new(vec![Rgba8::TRANSPARENT, Rgba8::new(9, 9, 9, 255)]).expect("パレット");
        let items: Vec<SheetItem> = (0..n)
            .map(|i| SheetItem {
                name: format!("t{i}"),
                canvas: crate::canvas::IndexedCanvas::filled(cell, cell, 1)
                    .with_transparent(Some(0)),
                palette: palette.clone(),
                duration_ms: 0,
            })
            .collect();
        let opts = PackOptions {
            columns: None,
            padding,
            margin,
        };
        let (_, _, mut sheet, _) = pack(&items, &opts).expect("梱包");
        sheet.image = "sheet.png".to_string();
        sheet
    }

    fn doc() -> TilesetDoc {
        let grid = TileGrid::from_tiles(
            2,
            1,
            vec![
                TileRef {
                    id: 0,
                    ..TileRef::default()
                },
                TileRef {
                    id: 1,
                    flip_x: true,
                    flip_d: true,
                    ..TileRef::default()
                },
            ],
        )
        .expect("2 枚");
        TilesetDoc::new(16, 2).with_map(&grid)
    }

    /// **壊れると: 反転が Tiled で別の向きになる，または «空の升» になる．**
    ///
    /// 上位 3 ビットの値は Tiled の仕様であって，こちらで決めた数ではない．
    /// 添字が 1 から始まるのも仕様である (0 は空の升) ．
    #[test]
    fn a_cell_becomes_a_gid_with_the_flip_bits_of_the_spec() {
        let plain = TileRefJson {
            id: 0,
            ..TileRefJson::default()
        };
        assert_eq!(tiled_gid(plain, 1), 1, "添字 0 は GID 1 (0 は空の升)");

        let flipped = TileRefJson {
            id: 2,
            flip_x: true,
            flip_y: true,
            flip_d: true,
        };
        let gid = tiled_gid(flipped, 1);
        assert_eq!(gid & 0x1fff_ffff, 3, "下位は添字 + firstgid");
        assert_eq!(gid & TILED_FLIP_H, TILED_FLIP_H);
        assert_eq!(gid & TILED_FLIP_V, TILED_FLIP_V);
        assert_eq!(gid & TILED_FLIP_D, TILED_FLIP_D);
    }

    /// **壊れると: 升の並びが行優先でなくなり，地図が転置される．**
    #[test]
    fn the_map_is_written_row_by_row() {
        let text = tiled_tmx(&doc(), "t.tsx", 1).expect("書ける");
        assert!(text.contains("width=\"2\" height=\"1\""));
        // 1 枚目は素のまま，2 枚目は反転の旗が立つ
        let expected = format!("1,{}\n", 0x8000_0000u32 | 0x2000_0000u32 | 2);
        assert!(text.contains(&expected), "{text}");
        // **区切りが行頭に落ちていないこと** — Tiled は行末に置く
        assert!(!text.contains("\n,"), "区切りが行頭にある: {text}");
    }

    /// **壊れると: 地図を持たない文書から «空の地図» を書き，使う側で気付けない．**
    #[test]
    fn a_document_without_a_map_cannot_become_a_tmx() {
        let terrain_only = TilesetDoc::new(16, 1);
        assert!(matches!(
            tiled_tmx(&terrain_only, "t.tsx", 1),
            Err(CoreError::ExportNoMap)
        ));
    }

    /// **壊れると: `.tsx` の並びが，シートの実際の並びと食い違う．**
    ///
    /// 数値は `px sheet pack` が決めたものを**引く**のであって，こちらで
    /// 計算し直さない (D110 と同じ理由) ．
    #[test]
    fn the_tsx_takes_its_layout_from_the_pack_instead_of_computing_it() {
        let d = TilesetDoc::new(16, 47);
        let sheet = packed(47, 16, 0, 0);
        let text = tiled_tsx(&d, &sheet, "grass").expect("書ける");
        // 47 枚は 8x6 に並ぶ (px_core::sheet::choose_columns)
        assert!(text.contains("columns=\"8\""), "{text}");
        assert!(text.contains("width=\"128\" height=\"96\""), "{text}");
        assert!(text.contains("tilecount=\"47\""));
        assert!(text.contains("source=\"sheet.png\""), "{text}");
        // 隙間と外周を頼んだらそのまま流れる
        let spaced = packed(4, 16, 2, 3);
        let text = tiled_tsx(&TilesetDoc::new(16, 4), &spaced, "g").expect("書ける");
        assert!(text.contains("spacing=\"2\""), "{text}");
        assert!(text.contains("margin=\"3\""), "{text}");
    }

    /// **壊れると: 升の大きさが違うシートを指す `.tsx` を書き，Tiled が升をずらす．**
    ///
    /// 引くだけでなく**突き合わせる** — 食い違いは «使う側が壊れて初めて分かる»
    /// たぐいの誤りなので，書く前に落とす (D111 と同じ判断) ．
    #[test]
    fn a_sheet_whose_cells_do_not_match_the_tile_size_is_an_error() {
        let d = TilesetDoc::new(16, 4);
        assert!(matches!(
            tiled_tsx(&d, &packed(4, 8, 0, 0), "g"),
            Err(CoreError::ExportCellMismatch { .. })
        ));
        // シートに載っている枚数より多いタイルを指す文書も落とす
        assert!(matches!(
            tiled_tsx(&TilesetDoc::new(16, 9), &packed(4, 16, 0, 0), "g"),
            Err(CoreError::ExportSheetTooSmall { .. })
        ));
    }
}
