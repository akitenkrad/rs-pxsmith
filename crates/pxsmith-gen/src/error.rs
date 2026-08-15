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
    ///
    /// > [!note] **分類だけでは何が引っ掛かったか読めない** (D171)．
    /// > `stop_details` には `category` と並んで `explanation` が入る．
    /// > 分類は開いた集合 (`cyber` ・`bio` ・`reasoning_extraction` …) なので，
    /// > 名前だけ見ても直しようがない — **説明の方が唯一の手掛かりである**．
    /// > どちらも欠けうるので，無いときは無いと言う (D77 ・D104)．
    #[error(
        "モデルが生成を断った (分類 {category}{})．同じ依頼を作り直しても同じなので\
         止める — プロンプトを書き直すこと",
        .explanation.as_deref().map(|e| format!("・{e}")).unwrap_or_default()
    )]
    Refused {
        category: String,
        /// `stop_details.explanation`．**無いこともある**．
        explanation: Option<String>,
    },

    /// 応答の形が想定と違う．
    #[error("応答を読めない: {message}")]
    BadResponse { message: String },

    /// **出力の上限に当たって途中で切れた** (`stop_reason: max_tokens`)．
    ///
    /// これは «壊れた応答» ではない — 上限を要求したのはこちらである．
    /// `max_tokens` は**思考と本文の合算**に掛かるので，寸法が大きいか
    /// 思考が深いと当たる．構造化出力は途中で切れると JSON にならないので，
    /// **`stop_reason` を見ないと «JSON になっていない» と誤診する**
    /// (D104 «鳴らなかった» と «検査していない» を分ける．と同じ形)．
    #[error(
        "出力が上限 ({max_tokens} トークン) に当たって途中で切れた．\
         上限は思考と本文の合算に掛かる — 寸法を小さくするか思考を浅くすること"
    )]
    Truncated { max_tokens: u32 },

    /// 書いていない種類を頼まれた．
    #[error(
        "{kind:?} は書いていない — 設計書 8.1 «汎用拡散モデルの出力は非一様格子に\
         なることがあり，その場合は棄却する» のとおり成功率がモデル依存で上限が\
         読めないため後回しにした (D156)．経路 C (原画は人・派生は決定論的) を使うこと"
    )]
    NotWritten { kind: GenKind },

    /// L0 として読めない．
    #[error(transparent)]
    Io(#[from] pxsmith_io::IoError),

    /// 素性やキャッシュを書けない．
    #[error("{path} を書き出せない: {source}")]
    Write {
        path: String,
        source: std::io::Error,
    },
}
