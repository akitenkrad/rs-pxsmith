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

    /// `px compose` にパーツが 1 つも渡されていない．
    #[error("合成するパーツが 1 つも無い")]
    ComposeNoParts,

    /// フレームを持たないパーツ．
    #[error("パーツ '{part}' にフレームが 1 つも無い")]
    ComposeEmptyPart { part: String },

    /// 同じパーツの中でフレームの大きさが違う．
    #[error(
        "パーツ '{part}' のフレームの大きさが揃っていない ({}x{} と {}x{})",
        first.0, first.1, other.0, other.1
    )]
    ComposePartSizeVaries {
        part: String,
        first: (u32, u32),
        other: (u32, u32),
    },

    /// 指定されたアンカーがパーツに無い．**黙って原点合わせにはしない**．
    #[error("パーツ '{part}' にアンカー '{anchor}' が無い")]
    ComposeAnchorMissing { part: String, anchor: String },

    /// 画素が指している添字がパレットの範囲を超えている．
    ///
    /// **黙って透明にはしない** — 元の絵が既に壊れており，合成で消すと
    /// «合成が消した» ように見える．
    #[error("パーツ '{part}' の画素が添字 {index} を指しているが，パレットは {len} 色しかない")]
    ComposeIndexOutOfPalette { part: String, index: u8, len: usize },

    /// 併合したパレットに元の色が見つからない (内部の不整合)．
    #[error("併合したパレットに色 {color:?} が無い")]
    ComposeColorLost { color: crate::color::Rgba8 },

    /// `${` が閉じていない．
    #[error("'{template}' の ${{ が閉じていない")]
    ComposeBadTemplate { template: String },

    /// 展開しようとした変数が宣言されていない．
    #[error("変数 '{name}' が宣言されていない")]
    ComposeUnknownVar { name: String },

    /// 方向展開に絵が 1 枚も渡されていない．
    #[error("方向展開に渡された絵が 1 枚も無い")]
    DirectionNothingDrawn,

    /// タイルの一辺が 0．
    #[error("タイルの一辺は 1 以上でなければならない")]
    TileSizeZero,

    /// 絵の寸法がタイルの倍数でない．**黙って切り落とさない**．
    #[error("{width}x{height} はタイルの一辺 {tile} の倍数ではない (端数を黙って切り落とさない)")]
    TileSizeMismatch { width: u32, height: u32, tile: u32 },

    /// 格子が指しているタイルが存在しない．
    #[error("タイル添字 {id} が存在しない")]
    TileIdOutOfRange { id: u32 },

    /// autotile のタイルは象限に割れなければならない．
    #[error("autotile のタイルの一辺は正の偶数でなければならない ({tile})")]
    AutotileOddTile { tile: u32 },

    /// 象限の絵が足りない．**どれが足りないかを全部並べる**．
    #[error("象限の絵が {} 通り足りない: {}", missing.len(), missing.join(" ・"))]
    AutotileMissingQuadrants { missing: Vec<String> },

    /// 象限の絵の大きさがタイルの半分でない．
    #[error(
        "象限 '{quadrant}' の絵が {width}x{height} で，タイルの半分 {expected}x{expected} ではない"
    )]
    AutotileQuadrantSize {
        quadrant: String,
        width: u32,
        height: u32,
        expected: u32,
    },

    /// 設計書 4.3 段 3 — 一部のフレームだけが `quadrant` を持つ．
    #[error(
        "フレーム '{name}' の一部だけが quadrant を持っている ({with} / {total})．\
         全象限を明示するか全部省略するかの二択とする (E_QUADRANT_PARTIAL)"
    )]
    QuadrantPartial {
        name: String,
        with: usize,
        total: usize,
    },

    /// 設計書 4.3 段 4 — 象限を明示したのに 4 象限を網羅していない．
    #[error("'{name}' が象限を明示しているが {} が欠けている", missing.join(" ・"))]
    AutotileQuadrantsNotCovered { name: String, missing: Vec<String> },

    /// 知らない状態名．
    #[error("'{name}' は象限の状態ではない (使えるのは {})", known.join(" ・"))]
    AutotileUnknownState { name: String, known: Vec<String> },

    /// autotile に絵が 1 枚も渡されていない．
    #[error("autotile に渡された絵が 1 枚も無い")]
    AutotileNoPieces,

    /// インポータの並びが要求する枚数と合わない．
    #[error("並び '{layout}' は {expected} 枚を要求するが {actual} 枚渡された")]
    ImportWrongCount {
        layout: String,
        expected: usize,
        actual: usize,
    },

    /// 正規 JSON を書けない．
    #[error("タイルセットの JSON を書けない: {message}")]
    TileJsonWrite { message: String },

    /// 正規 JSON を読めない．
    #[error("タイルセットの JSON を読めない: {message}")]
    TileJsonRead { message: String },

    /// `.tsx` の列数が 0．
    #[error("タイルセットの列数は 1 以上でなければならない")]
    ExportBadColumns,

    /// 地図を持たない文書から地図を書こうとした．
    #[error("この文書は map の節を持たないので地図を書けない (terrain だけの文書である)")]
    ExportNoMap,

    /// 版が違う．**黙って読まない** — 欄の意味が変わっている見込みがある．
    #[error("タイルセットの JSON の format が {found} である (扱えるのは {expected})")]
    TileJsonVersion { found: u32, expected: u32 },

    /// 同じ (象限，状態) が食い違う絵を持っている．
    ///
    /// **推測が外れた印である** — 並びが違うか，素材が象限に分解できない．
    #[error(
        "象限 {quadrant} の状態 {state} が mask {mask:#04x} で食い違う\
         (並びが違うか，素材が象限に分解できない)"
    )]
    ImportInconsistent {
        quadrant: String,
        state: String,
        mask: u8,
    },
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
