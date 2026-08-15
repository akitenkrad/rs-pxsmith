//! レシピ — 式評価 ・依存グラフ ・差分ビルド (設計書 4.2 ・6.15)．
//!
//! **レシピはスクリプト言語ではない** (D25) ．制限式言語だけを持つことで，
//! ステップキーが前段の出力を実体化した直後に逐次確定でき，差分ビルドと
//! 決定論性が保てる — この «なぜ制限するか» が M5 全体の背骨である．

pub mod cache;
pub mod error;
pub mod expr;
pub mod graph;
pub mod key;
pub mod recipe;
pub mod value;

pub use cache::Cache;
pub use error::{RecipeError, Result};
pub use graph::Graph;
pub use key::{StepKey, Versions};
pub use recipe::{Recipe, ResolvedStep, Step};
pub use value::Value;
