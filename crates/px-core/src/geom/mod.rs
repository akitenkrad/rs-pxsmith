//! 幾何基盤 (設計書 2.4)．
//!
//! **複数の機能が共有する土台**であり，これを最初に実装する (中心となる設計判断 6)．
//! 個別に実装すると輪郭追跡を 5 回書くことになる．
//!
//! | 基盤 | 提供するもの | 利用する機能 |
//! | --- | --- | --- |
//! | G1 [`contour`] | 境界の追跡，単調区間への分割，接線方向 | `smooth` / `aa` / `outline` / `anim subpixel` / lint 4, 8, 20, 23, 25 |
//! | G2 [`distance`] | 符号付き距離場，勾配 (疑似法線)，曲率符号 | `shade` / `aa` / `--ao` / `anim tween` / `anim smear` / lint 7, 13, 22 |
//! | G3 [`runs`] | チェーンのラン長列，谷検出 | `smooth` / lint 8, 12 |
//! | G4 局所格子推定 | 窓ごとの $\hat{s}$ とばらつき | `conform` / lint 2, 9 (M2 で実装) |
//! | G5 [`regions`] | 同色連結成分，面積，周囲長，隣接関係 | `denoise-dither` / `palette report` / lint 10, 11, 16, 17, 19, 20, 21, 24 |
//!
//! [`jaggy`] は基盤そのものではなく，**G1 と G3 を繋いで API の妥当性を確かめる
//! ための試作**である (R19)．基盤単体のテストでは API が使えるかどうかが分からない．

pub mod contour;
pub mod distance;
pub mod jaggy;
pub mod mask;
pub mod regions;
pub mod runs;
pub mod topology;

pub use contour::{Chain, Contour, split_monotone, trace_color_boundaries, trace_contours};
pub use distance::{curvature_field, normal_and_curvature, ridge_mask, signed_distance};
pub use jaggy::turn_runs;
pub use jaggy::{Jaggy, JaggyReport, analyze_canvas, analyze_mask};
pub use mask::{Field, Mask};
pub use regions::{Region, RegionId, RegionMap, label_mask, label_regions};
pub use runs::{
    banding, is_digital_straight, is_digital_straight_span, is_unimodal, jaggy_valleys,
    run_lengths, run_pixels, run_valleys,
};
pub use topology::{components, euler_characteristic, holes};
