//! `px-io` — `.aseprite` / パレットファイルの入出力と保持層．
//!
//! | モジュール | 内容 | 設計書 |
//! | --- | --- | --- |
//! | [`document`] | 保持層 `Document` と作業層への射影 | 3.1 |
//! | [`hex`] | `.hex` パレット (正規形) と変換入力 | 4.5 |
//! | [`l0`] | L0 テキストビットマップ | 4.1 |
//! | [`png`] | PNG の入出力 | 4.4 |
//! | [`atomic`] | 原子的な出力 | 3.7 |

pub mod atomic;
pub mod document;
pub mod error;
pub mod hex;
pub mod l0;
pub mod png;

pub use atomic::AtomicOutput;
pub use document::{Document, ProjectOptions};
pub use error::{IoError, Result};
pub use l0::L0Document;

/// `aseprite-io` を再輸出する．保持層の型を第 2 の真実にしないため (D53)．
pub use aseprite;
/// 作業層の型は `px-core` のものをそのまま使う．
pub use px_core;
pub use px_core::{Frame, FrameId, Palette};
