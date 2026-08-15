//! `pxsmith-gen` — 生成 AI 連携 (設計書 8 章)．
//!
//! | モジュール | 内容 | 設計書 |
//! | --- | --- | --- |
//! | [`request`] | 依頼 ・制約 ・素性 | 8.3 |
//! | [`repair`] | [`Generator`] と検証付き生成ループ | 8.2 |
//! | [`anthropic`] | 外部 API のバックエンド | 8.3 / D156 |
//! | [`session`] | 場所づくりと素性の書き出し | 8.3 |
//! | [`error`] | 落ちた理由 | 3.7 |
//!
//! # 採ったのは経路 B だけである (D156)
//!
//! 設計書 8.2 の 3 経路のうち，**LLM に L0 を書かせる経路 B** を書いた．
//! 経路 A (画像生成) は 8.1 が «汎用拡散モデルの出力は非一様格子になることが
//! あり，その場合は棄却する» と決めているので**成功率がモデル依存で上限が
//! 読めない** — D92 の作法で «書いていない» と報告する ([`error::GenError::NotWritten`]) ．
//! 経路 C (原画は人 ・派生は決定論的) は M0 〜 M7 がすでにその形である．
//!
//! # **決定論はこの層には無い．キャッシュが担う** (D157)
//!
//! 設計書 8.3 は «モデル ・プロンプト ・seed を manifest に残す» と言うが，
//! **採ったバックエンドには seed が無い** — 標本の温度も種も受け付けない．
//! だから «同じ依頼 → 同じ絵» は成り立たない．
//!
//! これは道具の欠陥ではなく，**D31 «`op = "gen"` は既定でキャッシュ参照のみ»
//! の理由そのもの**である．6.15 の «`RAYON_NUM_THREADS` を変えてもバイト一致»
//! が生成を含む木でも成り立つのは，生成物がキャッシュに貯まりリポジトリへ
//! コミットされる (設計書 4.1 の表) からであって，モデルが決定論的だからでは
//! ない．**素性には seed の代わりに «なぜ無いか» を書く**
//! ([`request::DETERMINISM_NOTE`]) ．
//!
//! # 検査器を先に持っていたから L0 を選んだ
//!
//! 8.2 の擬似コードの `ParseAndValidate` は L0 の読み取り，`Lint` は 27 ルール，
//! `ToPromptHint` はその報告文である．**3 つとも M1 〜 M7 で書き終えている** —
//! 出力形式の選択 (D156) はこの一致で決まった．

pub mod anthropic;
pub mod error;
pub mod repair;
pub mod request;
pub mod session;

pub use error::{GenError, Result};
pub use repair::{Attempt, Generator, Report, Verified, generate_with_repair};
pub use request::{
    Backend, Constraints, DETERMINISM_NOTE, Effort, GenKind, GenRequest, Provenance,
};
pub use session::Session;
