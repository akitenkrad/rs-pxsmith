//! 生成のエラー．
//!
//! **落ちた理由は必ず名前を持つ** — 「生成に失敗した」だけでは，鍵が無いのか
//! ・ネットワークが死んでいるのか ・モデルが断ったのかが読めない (D92) ．

use crate::request::GenKind;

pub type Result<T> = std::result::Result<T, GenError>;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum GenError {
    /// 作り直しの上限が 0．
    #[error("作り直しの上限が 0 である (1 以上にすること)")]
    NoAttempts,

    /// 鍵が要る．
    #[error("API 鍵が無い．環境変数 {var} に置くこと — **鍵は依頼にも素性にも書かない**")]
    MissingKey { var: String },

    /// バックエンドが落ちた．
    #[error("バックエンドが応えない: {message}")]
    Backend { message: String },

    /// **モデルが断った** (`stop_reason: refusal`)．
    ///
    /// これは «壊れた» ではない — 応答は正常に返っており，モデルが内容を
    /// 断っただけである．作り直しても同じなので**輪を回さずに落とす**．
    #[error(
        "モデルが生成を断った (分類 {category})．同じ依頼を作り直しても同じなので\
         止める — プロンプトを書き直すこと"
    )]
    Refused { category: String },

    /// 応答の形が想定と違う．
    #[error("応答を読めない: {message}")]
    BadResponse { message: String },

    /// 書いていない種類を頼まれた．
    #[error(
        "{kind:?} は書いていない — 設計書 8.1 «汎用拡散モデルの出力は非一様格子に\
         なることがあり，その場合は棄却する» のとおり成功率がモデル依存で上限が\
         読めないため後回しにした (D156)．経路 C (原画は人・派生は決定論的) を使うこと"
    )]
    NotWritten { kind: GenKind },

    /// L0 として読めない．
    #[error(transparent)]
    Io(#[from] px_io::IoError),

    /// 素性やキャッシュを書けない．
    #[error("{path} を書き出せない: {source}")]
    Write {
        path: String,
        source: std::io::Error,
    },
}
