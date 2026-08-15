//! `pxsmith-view` — ターミナルプレビュー (設計書 2.1)．
//!
//! **確認専用であることを構造で示す**ためにクレートを分けてある．ここに絵を
//! 変える処理は置かない．
//!
//! | モジュール | 内容 | 設計書 |
//! | --- | --- | --- |
//! | [`render`] | キャンバスを画像へ起こす | — |
//! | [`term`] | 端末の能力判定と表示 | M1 (R1・R18) |
//! | [`diff`] | 2 枚の差分 | 5 章 `pxsmith diff` |
//! | [`onion`] | オニオンスキン (輪郭のみ) | 5 章 / D52 |
//!
//! # `terminal` feature
//!
//! **端末へ実際に描く 2 関数だけが既定機能の後ろにある．** [`term::detect`] と
//! [`term::show`] は `viuer` を使い，`viuer` は `ansi_colours`
//! (LGPL-3.0-or-later) を引き込む唯一の経路である．`--no-default-features` で
//! 建てると LGPL の連鎖が 0 件になり，[`render`] ・[`diff`] ・[`onion`] と
//! [`TerminalKind`] はそのまま使える．

pub mod diff;
pub mod onion;
pub mod render;
pub mod term;

pub use diff::{Diff, PixelChange};
pub use onion::{OnionOptions, OnionReport, onion_image};
pub use render::{RenderOptions, to_rgba_image};
pub use term::TerminalKind;
#[cfg(feature = "terminal")]
pub use term::{detect, show};

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ViewError {
    #[cfg(feature = "terminal")]
    #[error("端末への表示に失敗した")]
    Display(#[from] viuer::ViuError),

    #[error("画像の構築に失敗した")]
    Image(#[from] image::ImageError),

    #[error("大きさが違う画像は比較できない ({a:?} と {b:?})")]
    SizeMismatch { a: (u32, u32), b: (u32, u32) },

    #[error(transparent)]
    Core(#[from] pxsmith_core::CoreError),
}

pub type Result<T> = std::result::Result<T, ViewError>;
