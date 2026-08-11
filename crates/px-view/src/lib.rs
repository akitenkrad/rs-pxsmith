//! `px-view` — ターミナルプレビュー (設計書 2.1)．
//!
//! **確認専用であることを構造で示す**ためにクレートを分けてある．ここに絵を
//! 変える処理は置かない．
//!
//! | モジュール | 内容 | 設計書 |
//! | --- | --- | --- |
//! | [`render`] | キャンバスを画像へ起こす | — |
//! | [`term`] | 端末の能力判定と表示 | M1 (R1・R18) |
//! | [`diff`] | 2 枚の差分 | 5 章 `px diff` |

pub mod diff;
pub mod render;
pub mod term;

pub use diff::{Diff, PixelChange};
pub use render::{RenderOptions, to_rgba_image};
pub use term::{TerminalKind, detect, show};

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ViewError {
    #[error("端末への表示に失敗した")]
    Display(#[from] viuer::ViuError),

    #[error("画像の構築に失敗した")]
    Image(#[from] image::ImageError),

    #[error("大きさが違う画像は比較できない ({a:?} と {b:?})")]
    SizeMismatch { a: (u32, u32), b: (u32, u32) },

    #[error(transparent)]
    Core(#[from] px_core::CoreError),
}

pub type Result<T> = std::result::Result<T, ViewError>;
