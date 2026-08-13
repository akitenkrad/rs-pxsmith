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
//! | [`ink`] | 描画インクとブラシ | 3.4 |
//! | [`edit`] | 編集操作とパッチ | 3.6 |
//! | [`error`] | エラーモデル | 3.7 |

pub mod canvas;
pub mod clean;
pub mod color;
pub mod edit;
pub mod error;
pub mod frame;
pub mod geom;
pub mod grid;
pub mod ink;
pub mod math;
pub mod palette;
pub mod quantize;
pub mod ramp;
pub mod shade;

pub use canvas::{IndexedCanvas, RgbaCanvas};
pub use color::{Oklab, Rgba8};
pub use edit::{EditOp, FrameId, LayerId, Patch};
pub use error::{CoreError, FailurePolicy, Result};
pub use frame::{
    BlendMode, Depth, Frame, FrameKind, Layer, LayerMeta, Surface, TileGrid, TileRef, TilesetId,
};
pub use geom::{Chain, Contour, Field, Mask, Region, RegionMap};
pub use grid::{GridError, GridEstimate, GridParams, estimate_grid, local_grid};
pub use ink::{Brush, FillOpts, Ink, PatternMask};
pub use math::{IRect, IVec2, Rect, UVec2, Vec2, clip_pair, ivec2, uvec2, vec2};
pub use palette::{ChromaCurve, Palette, Ramp};
pub use ramp::{LightPreset, LightSource, LightingModel, RampSpec, generate_ramp};
pub use shade::{Lamp, Shading, incidence, shade};
