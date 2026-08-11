//! エラーモデル (設計書 3.7)．
//!
//! 構成エラーは即座に停止し，データエラーは [`FailurePolicy`] で扱いを切り替える．

/// `px-core` のエラー．
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CoreError {
    /// パレットのアルファは 2 値のみ (不変条件，設計書 3.2)．
    #[error("パレットのアルファは 0 または 255 のみ許される (添字 {index}，アルファ {alpha})")]
    NonBinaryAlpha { index: usize, alpha: u8 },

    /// 色の添字は `u8` 固定なので 256 色が上限 (D2)．
    #[error("パレットの色数 {0} が上限 256 を超えている")]
    PaletteTooLarge(usize),

    /// 既に明度順へ正規化済みのパレットを再度正規化しようとした．
    #[error("パレットは既に明度順へ正規化されている")]
    AlreadyNormalized,

    /// 正規化していないパレットを逆置換しようとした．
    #[error("パレットは正規化されていないので逆置換できない")]
    NotNormalized,

    /// 置換表の長さがパレットと一致しない．
    #[error("置換表の長さ {actual} がパレットの色数 {expected} と一致しない")]
    BadPermutation { expected: usize, actual: usize },

    /// 画素バッファの長さが `width * height` と一致しない．
    #[error("画素数 {actual} が {width}x{height} = {expected} と一致しない")]
    PixelCountMismatch {
        width: u32,
        height: u32,
        expected: usize,
        actual: usize,
    },

    /// 存在しないレイヤ添字を参照した．
    #[error("レイヤ添字 {index} は範囲外 (レイヤ数 {len})")]
    LayerOutOfRange { index: usize, len: usize },

    /// 存在しないフレーム添字を参照した．
    #[error("フレーム添字 {index} は範囲外 (フレーム数 {len})")]
    FrameOutOfRange { index: usize, len: usize },

    /// インデックスカラー以外の層に対して添字操作を行おうとした．
    #[error("レイヤ '{name}' はインデックスカラーではないのでこの操作を適用できない")]
    NotIndexed { name: String },

    /// パッチの適用先が記録時と異なる．
    #[error("パッチの適用先の状態が記録時と一致しない")]
    PatchMismatch,

    /// L0 テキスト形式の色数上限 (D8)．
    #[error("色キー '{0}' は L0 テキスト形式で使えない (使えるのは 0-9 a-z A-Z と透明の '.')")]
    InvalidColorKey(char),

    /// 16 進色コードの構文誤り．
    #[error("色コード '{0}' を解釈できない (RRGGBB の 6 桁 16 進が必要)")]
    InvalidHexColor(String),
}

/// `px-core` の `Result` 別名．
pub type Result<T> = std::result::Result<T, CoreError>;

/// データエラーの扱い方 (設計書 3.7)．
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum FailurePolicy {
    /// 既定．最初の失敗で全体を停止する．CI 向け．
    #[default]
    FailFast,
    /// 失敗を記録して継続し，終了時に非ゼロを返す．
    Collect,
    /// 警告のみ．終了コードは 0．
    Warn,
}
