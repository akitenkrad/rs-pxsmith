//! `px-core` — pxforge のデータモデルと純関数のアルゴリズム．
//!
//! I/O・ネットワーク・端末に依存しない純関数の集合として保つ (設計書 2.1)．
//! 例外は [`frame::BlendMode`] で，これは `aseprite-io` の型を再輸出したものである
//! (D53)．保持層の型をそのまま通さないと `.aseprite` のバイト一致往復が成立しない．
//!
//! # 構成
//!
//! | モジュール | 内容 | 設計書 |
//! | --- | --- | --- |
//! | [`math`] | 座標・矩形・クリッピング | 3.5 |
//! | [`color`] | RGBA と OKLab，色距離 | 3.2 / 6.6 |
//! | [`canvas`] | インデックス / RGBA キャンバス | 3.2 |
//! | [`palette`] | パレット・ランプ・明度順正規化 | 3.2 |
//! | [`frame`] | 作業層のフレームとレイヤ | 3.1 |
//! | [`geom`] | 幾何基盤 G1・G2・G3・G5 | 2.4 |
//! | [`grid`] | 格子推定と局所格子推定 (G4) | 6.1 |
//! | [`quantize`] | 色数削減とパレット強制 | 6.6 |
//! | [`clean`] | 整形と脱ディザノイズ | 6.3 / 6.14 |
//! | [`ramp`] | ランプ生成と照明モデル | 3.3 / D48 |
//! | [`shade`] | 陰影導出 | 6.2 |
//! | [`aa`] | アンチエイリアス | 6.5 |
//! | [`outline`] | 縁取り | D36 |
//! | [`smooth`] | ジャギー正規化 | 6.4 |
//! | [`compose`] | パーツ合成 ・variants 展開 | 5 章 / D42 |
//! | [`direction`] | 方向展開 (反転 + 陰影再導出) | 4.3 / L2 |
//! | [`tileset`] | タイル分割と同値判定 | 6.7 |
//! | [`autotile`] | 象限合成による 47 枚生成 | 6.8 / 4.3 |
//! | [`tilejson`] | タイルセットの正規出力 (JSON) | 4.4 |
//! | [`export`] | 出力先アダプタ (Tiled) | 4.4 |
//! | [`atmos`] | 空気遠近法と多重スクロールメタ | 4.4 / 5 章 |
//! | [`resample`] | 拡縮と回転 | 5 章 / D18 |
//! | [`cleanedge`] | cleanEdge の移植 (MIT) | D18 |
//! | [`ink`] | 描画インクとブラシ | 3.4 |
//! | [`edit`] | 編集操作とパッチ | 3.6 |
//! | [`error`] | エラーモデル | 3.7 |

pub mod aa;
pub mod afterimage;
pub mod anim;
pub mod atmos;
pub mod autotile;
pub mod canvas;
pub mod clean;
pub mod cleanedge;
pub mod color;
pub mod compose;
pub mod deform;
pub mod direction;
pub mod edit;
pub mod error;
pub mod export;
pub mod frame;
pub mod geom;
pub mod grid;
pub mod ink;
pub mod math;
pub mod outline;
pub mod palette;
pub mod quantize;
pub mod ramp;
pub mod resample;
pub mod shade;
pub mod sheet;
pub mod smear;
pub mod smooth;
pub mod subpixel;
pub mod tilejson;
pub mod tileset;
pub mod tween;
pub mod validate;

pub use aa::{AaAddOptions, AaReport, add_antialiasing};
pub use canvas::{IndexedCanvas, RgbaCanvas};
pub use color::{Oklab, Rgba8};
pub use direction::{
    Direction, ExpandMode, ExpandOptions, ExpandReport, ReshadeSpec, expand, mirror_canvas,
    mirror_frame,
};

pub use anim::{CycleSpec, ModTarget, Wave, cycle, duration_ms, ease, reverse_derive};
pub use autotile::{
    CornerState, Quadrant, QuadrantArt, blob_masks, canonical_mask, corner_state,
    mirror_to_all_quadrants, seam_doubled,
};
pub use compose::{
    Alignment, ComposeOptions, ComposeReport, DelayMode, Part, Placement, compose, expand_template,
    expand_variants,
};
pub use edit::{EditOp, FrameId, LayerId, Patch};
pub use error::{CoreError, FailurePolicy, Result};
pub use frame::{
    BlendMode, Depth, Frame, FrameKind, Layer, LayerMeta, Surface, TileGrid, TileRef, TilesetId,
};
pub use geom::{Chain, Contour, Field, Mask, Region, RegionMap};
pub use grid::{GridError, GridEstimate, GridParams, estimate_grid, local_grid};
pub use ink::{Brush, FillOpts, Ink, PatternMask};
pub use math::{IRect, IVec2, Rect, UVec2, Vec2, clip_pair, ivec2, uvec2, vec2};
pub use outline::{OutlineOptions, OutlineReport, OutlineStyle, outline};
pub use palette::{ChromaCurve, Palette, Ramp};
pub use ramp::{LightPreset, LightSource, LightingModel, RampSpec, generate_ramp};
pub use shade::{
    Lamp, ShadeOptions, Shading, bounce_distance_field, incidence, normal_field, shade, shade_mask,
    shade_to_canvas,
};
pub use sheet::{PackOptions, PackReport, SheetCell, SheetDoc, SheetItem, pack};
pub use smooth::{SmoothOptions, SmoothReport, smooth_canvas, smooth_mask};
pub use tilejson::{TerrainEntry, TileMapJson, TileRefJson, TilesetDoc};
pub use tileset::{DedupeMode, ExtractOptions, ExtractReport, extract, rebuild};
pub use tween::{TweenOptions, Tweened, tween_mask, tween_series};
