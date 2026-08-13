//! タイルセットの正規出力 (設計書 4.4)．
//!
//! 設計書 4.4 は «**独自 JSON (bitmask → tile index + flip flags) がエンジン非依存の
//! 正規出力．常に生成**» とし，Godot / Tiled は**そこからのアダプタ**と定める．
//!
//! # 2 つのスキーマに分かれていたのを 1 つに寄せた
//!
//! 実装を進める間に `px tileset autotile --map` は «bitmask → 添字» を，
//! `px tileset extract --map` は «升 → 添字 + 反転» を，**それぞれ別の形で**
//! 書いていた．**正規出力が 2 つあるのは正規出力が無いのと同じ**なので，
//! 1 つの型にまとめた．
//!
//! | 節 | 何を持つか | 誰が書くか |
//! | --- | --- | --- |
//! | `terrain` | **bitmask → タイル** (設計書 4.4 の正規形) | `px tileset autotile` |
//! | `map` | **升 → タイル** (絵をタイルへ分解した結果) | `px tileset extract` |
//!
//! どちらも省略できる．両方を持つ文書もありうる (組んだタイルセットと，それを
//! 使って組んだ画面) ．
//!
//! # 書けないものは書かない
//!
//! **これは数え上げでも校正でもなく «出典どおりに書く» 仕事である** (D92 と同じ性質) ．
//! 出力先の仕様に無いものを埋めない — たとえば Godot の terrain set の «peering bit»
//! はこちらの 8 近傍 bitmask と 1 対 1 に対応するとは限らないので，
//! **アダプタは仕様を引いてから書くこと**．

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::frame::{TileGrid, TileRef};

pub const FORMAT_VERSION: u32 = 1;

/// タイルへの参照．`TileRef` と同じ内容を JSON の名前で持つ．
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TileRefJson {
    pub id: u32,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flip_x: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flip_y: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flip_d: bool,
}

impl From<TileRef> for TileRefJson {
    fn from(r: TileRef) -> Self {
        Self {
            id: r.id,
            flip_x: r.flip_x,
            flip_y: r.flip_y,
            flip_d: r.flip_d,
        }
    }
}

impl From<TileRefJson> for TileRef {
    fn from(r: TileRefJson) -> Self {
        Self {
            id: r.id,
            flip_x: r.flip_x,
            flip_y: r.flip_y,
            flip_d: r.flip_d,
        }
    }
}

/// 8 近傍 bitmask と，そこへ置くタイル (設計書 4.4 の «bitmask → tile index + flip flags»)．
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerrainEntry {
    pub mask: u8,
    #[serde(flatten)]
    pub tile: TileRefJson,
}

/// 升の並び．
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TileMapJson {
    pub cols: u32,
    pub rows: u32,
    /// **行優先**で並べる (設計書 6.15 規則 1．並びを決めておかないと差分が揺れる)．
    pub cells: Vec<TileRefJson>,
}

/// タイルセットの正規出力．
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TilesetDoc {
    pub format: u32,
    /// タイルの一辺．
    pub tile: u32,
    /// タイルの枚数．
    pub tiles: usize,
    /// bitmask → タイル．**昇順**である．
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub terrain: Vec<TerrainEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub map: Option<TileMapJson>,
}

impl TilesetDoc {
    pub fn new(tile: u32, tiles: usize) -> Self {
        Self {
            format: FORMAT_VERSION,
            tile,
            tiles,
            terrain: Vec::new(),
            map: None,
        }
    }

    /// `px tileset autotile` の出力を載せる．
    pub fn with_terrain(mut self, entries: impl IntoIterator<Item = (u8, TileRef)>) -> Self {
        self.terrain = entries
            .into_iter()
            .map(|(mask, tile)| TerrainEntry {
                mask,
                tile: tile.into(),
            })
            .collect();
        self.terrain.sort_by_key(|e| e.mask);
        self
    }

    /// `px tileset extract` の出力を載せる．
    pub fn with_map(mut self, grid: &TileGrid) -> Self {
        self.map = Some(TileMapJson {
            cols: grid.width(),
            rows: grid.height(),
            cells: grid.tiles().iter().map(|r| (*r).into()).collect(),
        });
        self
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).map_err(|e| CoreError::TileJsonWrite {
            message: e.to_string(),
        })
    }

    pub fn from_json(text: &str) -> Result<Self> {
        let doc: Self = serde_json::from_str(text).map_err(|e| CoreError::TileJsonRead {
            message: e.to_string(),
        })?;
        if doc.format != FORMAT_VERSION {
            return Err(CoreError::TileJsonVersion {
                found: doc.format,
                expected: FORMAT_VERSION,
            });
        }
        // **書いてある枚数と，参照されている添字が食い違わないこと．**
        // 食い違ったまま出力先へ流すと，エンジン側で «存在しないタイル» になる
        let max = doc
            .terrain
            .iter()
            .map(|e| e.tile.id)
            .chain(doc.map.iter().flat_map(|m| m.cells.iter().map(|c| c.id)))
            .max();
        if let Some(max) = max
            && max as usize >= doc.tiles
        {
            return Err(CoreError::TileIdOutOfRange { id: max });
        }
        if let Some(m) = &doc.map
            && m.cells.len() != (m.cols * m.rows) as usize
        {
            return Err(CoreError::PixelCountMismatch {
                width: m.cols,
                height: m.rows,
                expected: (m.cols * m.rows) as usize,
                actual: m.cells.len(),
            });
        }
        Ok(doc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> TileGrid {
        TileGrid::from_tiles(
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
                    ..TileRef::default()
                },
            ],
        )
        .expect("2 枚")
    }

    /// **壊れると: 書いた JSON を自分で読み直せない．**
    /// アダプタは正規 JSON からしか読まないので，ここが崩れると出力先が全部死ぬ．
    #[test]
    fn the_canonical_document_round_trips() {
        let doc = TilesetDoc::new(16, 2)
            .with_terrain([
                (
                    3,
                    TileRef {
                        id: 1,
                        flip_y: true,
                        ..TileRef::default()
                    },
                ),
                (0, TileRef::default()),
            ])
            .with_map(&grid());
        let text = doc.to_json().expect("書ける");
        let back = TilesetDoc::from_json(&text).expect("読める");
        assert_eq!(back, doc);
        // bitmask は昇順に並ぶ
        assert_eq!(back.terrain[0].mask, 0);
        assert_eq!(back.terrain[1].mask, 3);
    }

    /// **壊れると: 存在しないタイルを指す文書が出力先へ流れ，エンジン側で壊れる．**
    #[test]
    fn a_tile_id_beyond_the_count_is_an_error() {
        let doc = TilesetDoc::new(16, 1).with_terrain([(
            0,
            TileRef {
                id: 5,
                ..TileRef::default()
            },
        )]);
        let text = doc.to_json().expect("書ける");
        assert!(matches!(
            TilesetDoc::from_json(&text),
            Err(CoreError::TileIdOutOfRange { id: 5 })
        ));
    }

    /// **壊れると: 升の数と cols x rows が食い違う文書を受け取る．**
    #[test]
    fn a_map_whose_cells_do_not_fill_the_grid_is_an_error() {
        let text = r#"{"format":1,"tile":16,"tiles":1,
            "map":{"cols":2,"rows":2,"cells":[{"id":0}]}}"#;
        assert!(matches!(
            TilesetDoc::from_json(text),
            Err(CoreError::PixelCountMismatch { .. })
        ));
    }

    /// **壊れると: 別の版の文書を黙って読み，欄の意味が変わったまま流れる．**
    #[test]
    fn a_document_from_another_format_version_is_an_error() {
        let text = r#"{"format":2,"tile":16,"tiles":0}"#;
        assert!(matches!(
            TilesetDoc::from_json(text),
            Err(CoreError::TileJsonVersion { found: 2, .. })
        ));
    }

    /// **壊れると: 旗の立っていないタイルにも `false` が並び，
    /// 出力が読みにくくなるうえ差分が膨らむ．**
    #[test]
    fn flags_that_are_not_set_are_left_out() {
        let doc = TilesetDoc::new(8, 1).with_terrain([(0, TileRef::default())]);
        let text = doc.to_json().expect("書ける");
        assert!(!text.contains("flip_x"), "立っていない旗は書かない");
        assert!(text.contains("\"mask\": 0"));
    }
}
