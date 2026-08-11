//! `px-io` のエラー．

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IoError {
    #[error("ファイル入出力に失敗した: {path}")]
    File {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("入出力に失敗した")]
    Io(#[from] std::io::Error),

    #[error(".aseprite の解釈または書き出しに失敗した")]
    Aseprite(#[from] aseprite::AsepriteError),

    #[error(transparent)]
    Core(#[from] px_core::CoreError),

    #[error("{path}:{line}: {message}")]
    Parse {
        path: PathBuf,
        line: usize,
        message: String,
    },

    #[error("パレットの色数 {0} が上限 256 を超えている")]
    PaletteTooLarge(usize),

    #[error(
        "パレット添字 {index} のアルファ {alpha} が 2 値でない．\
         binarize_alpha を有効にすると 128 を境に丸めて読み込める"
    )]
    NonBinaryPaletteAlpha { index: usize, alpha: u8 },

    #[error("フレーム添字 {index} は範囲外 (フレーム数 {len})")]
    FrameOutOfRange { index: usize, len: usize },

    #[error("レイヤ数が射影元と一致しない (保持層 {expected}，作業層 {actual})")]
    LayerCountMismatch { expected: usize, actual: usize },

    #[error("レイヤ '{name}' の種類が射影元と一致しない")]
    SurfaceKindMismatch { name: String },

    #[error("キャンバスの大きさが射影元と一致しない (保持層 {expected:?}，作業層 {actual:?})")]
    SizeMismatch {
        expected: (u32, u32),
        actual: (u32, u32),
    },

    #[error("{0} 色モードはまだ扱えない")]
    UnsupportedColorMode(&'static str),

    #[error("cel の大きさ {w}x{h} が u16 の上限を超えている")]
    CelTooLarge { w: u32, h: u32 },

    #[error(
        "{field} は保持層へ書き戻せない (aseprite-io 0.2 に設定 API が無い)．\
         射影元と同じ値のまま merge_back すること"
    )]
    UnsupportedWriteback { field: &'static str },

    #[error(
        "作業層のパレットが明度順に正規化されたままである．\
         denormalize で元の並びへ戻してから merge_back すること (D50)"
    )]
    NormalizedPaletteWriteback,

    #[error("{path}: L0 形式の制約に反する — {violation}")]
    L0 {
        path: PathBuf,
        violation: crate::l0::Violation,
    },
}

pub type Result<T> = std::result::Result<T, IoError>;
