//! `px-macro` — L0 テキストビットマップを Rust に埋め込む `pixels!` (設計書 4.1)．
//!
//! # なぜ proc-macro か
//!
//! 2 つの性質を**コンパイル時**に保証するためである．
//!
//! 1. **行長の不一致がコンパイルエラーになる**．食い違った行そのものに下線が引かれる．
//! 2. **パレットを変えると再コンパイルされる**．展開結果に `include_str!` を含めて
//!    cargo に依存を追跡させるので，`.hex` を書き換えたら添字の範囲検査がやり直される．
//!
//! 実行時に読む作りでは，どちらも「動かしてみるまで分からない」ままになる．
//!
//! # 2 つの書き方
//!
//! どちらも `Vec<px_core::Frame>` へ展開される．L0 は 1 ファイル = 1 レイヤ分の
//! **フレーム列**なので (D9)，返り値も列で揃えてある．
//!
//! ```ignore
//! // 1. L0 ファイルを読む
//! let frames = pixels!("sprites/hero_body.px.toml");
//!
//! // 2. その場に書く
//! let frames = pixels! {
//!     palette = "palettes/sweetie-16.hex",
//!     layer = "body",
//!     duration_ms = 83,
//!     map = { '.' = transparent, 'k' = 0, 'h' = 12 },
//!     rows = [
//!         "..kk..",
//!         ".khhk.",
//!         "..kk..",
//!     ],
//! };
//! ```
//!
//! パスはいずれも呼び出し側クレートの `CARGO_MANIFEST_DIR` からの相対で解決する．
//!
//! # 前提
//!
//! 展開結果は `px_core` の型を名指しするので，**呼び出し側が `px-core` に依存している
//! 必要がある**．proc-macro クレートは自身の依存を再輸出できないためである．

use proc_macro::TokenStream;

mod expand;
mod input;

/// L0 テキストビットマップを `Vec<px_core::Frame>` へ展開する．
///
/// 詳しい書き方はクレートの説明を参照．
#[proc_macro]
pub fn pixels(item: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(item as input::Input);
    match expand::expand(&input) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}
