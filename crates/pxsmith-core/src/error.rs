//! エラーモデル (設計書 3.7)．
//!
//! 構成エラーは即座に停止し，データエラーは [`FailurePolicy`] で扱いを切り替える．

/// `pxsmith-core` のエラー．
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

    /// `pxsmith compose` にパーツが 1 つも渡されていない．
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

    /// シートの升の大きさがタイルの一辺と違う．
    ///
    /// **引くだけでなく突き合わせる** — 食い違ったまま `.tsx` を書くと，
    /// Tiled が升をずらして読む．
    #[error("シートの升 {cell_w}x{cell_h} がタイルの一辺 {tile} と違う")]
    ExportCellMismatch { tile: u32, cell_w: u32, cell_h: u32 },

    /// シートに載っている枚数より多くのタイルを指している．
    #[error("タイルが {tiles} 枚あるが，シートには {cells} 枚しか載っていない")]
    ExportSheetTooSmall { tiles: usize, cells: usize },

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

    /// シートの JSON を書けない．
    #[error("シートのメタを書き出せない: {message}")]
    SheetWrite { message: String },

    /// 回転の角度が有限でない．
    #[error("回転の角度が {degrees} である")]
    ResampleBadAngle { degrees: f32 },

    /// 拡縮の倍率が正の有限値でない．
    #[error("拡縮の倍率が {factor} である (正の有限値のはず)")]
    ResampleBadFactor { factor: f32 },

    /// 写像が退化している (面積が 0 に潰れる)．
    #[error("写像 {matrix:?} が退化している (行列式が 0 なので絵が線へ潰れる)")]
    ResampleDegenerate { matrix: [f32; 4] },

    /// 投影の段が正の整数でない．
    #[error("投影の段 '{spec}' を読めない (`走り:上がり` の正の整数のはず)")]
    ProjectBadStep { spec: String },

    /// ガイドの刻みか画布が 0 である．
    #[error("ガイドの刻み {cell} ・画布 {width}x{height} に 0 がある")]
    GuideBadSize { cell: u32, width: u32, height: u32 },

    /// 空気遠近法を掛けるフレームが無い．
    #[error("空気遠近法を掛けるフレームが無い")]
    AtmosNoFrames,

    /// 寄せ具合が 0 〜 1 の外にある．
    #[error("寄せ具合が {amount} である (0.0 〜 1.0 のはず)")]
    AtmosAmountOutOfRange { amount: f32 },

    /// 手前ほど濃い霞は空気遠近法ではない．
    #[error(
        "奥へ行くほど霞まない宣言である (前景 {foreground} ・中景 {midground} ・遠景 {background})"
    )]
    AtmosNotMonotone {
        foreground: f32,
        midground: f32,
        background: f32,
    },

    /// 多重スクロールメタを書けない．
    #[error("多重スクロールメタを書けない: {message}")]
    ScrollWrite { message: String },

    /// 多重スクロールメタを読めない．
    #[error("多重スクロールメタを読めない: {message}")]
    ScrollRead { message: String },

    /// 版が違う．**黙って読まない**．
    #[error("多重スクロールメタの format が {found} である (扱えるのは {expected})")]
    ScrollVersion { found: u32, expected: u32 },

    /// 知らない奥行きの綴り．**既定へ倒さない**．
    #[error(
        "多重スクロールメタの depth = '{depth}' を解釈できない (foreground / midground / background)"
    )]
    ScrollUnknownDepth { depth: String },

    /// シートの JSON を読めない．
    #[error("シートのメタを読めない: {message}")]
    SheetRead { message: String },

    /// 版が違う．**黙って読まない** — 欄の意味が変わっている見込みがある．
    #[error("シートのメタの format が {found} である (扱えるのは {expected})")]
    SheetVersion { found: u32, expected: u32 },

    /// 書いてある寸法と並びが食い違う．
    ///
    /// **`.tsx` は列数と寸法の両方を書く**ので，食い違ったまま流すと使う側が
    /// 升をずらして読む．
    #[error("シートの寸法 {width}x{height} が並びから出る {expected_w}x{expected_h} と違う")]
    SheetSizeMismatch {
        width: u32,
        height: u32,
        expected_w: u32,
        expected_h: u32,
    },

    /// 升の数より多くの絵が載っている．
    #[error("シートに {cells} 枚あるが，並びが持てるのは {capacity} 枚である")]
    SheetTooManyCells { cells: usize, capacity: usize },

    /// シートに載る色が 256 を超えた．
    ///
    /// **1 枚のシートは 1 つのパレットしか持てない** (添字は `u8`．D2) ．
    /// 黙って減色すると «並べたら色が変わった» になるので落とす．
    #[error(
        "{items} 枚を並べると {colors} 色になり，1 枚のシートに載る 256 色を超える．\n\
         先に pxsmith quantize で色数を揃えるか，シートを分けること"
    )]
    SheetTooManyColors { colors: usize, items: usize },

    /// 梱包する絵が 1 枚も無い．
    #[error("梱包する絵が 1 枚も無い")]
    SheetNoItems,

    /// 画素が指している添字がパレットに無い．**黙って透明にしない** (D93 と同じ)．
    #[error("{name} の添字 {index} がパレット ({len} 色) の外を指している")]
    SheetIndexOutOfPalette { name: String, index: u8, len: usize },

    /// 画素が指している添字がパレットに無い．
    #[error("画素が添字 {index} を指しているが，パレットは {len} 色しかない")]
    PaletteIndexMissing { index: u8, len: usize },

    /// FPS が正でない．
    #[error("FPS は正の有限値でなければならない ({fps})")]
    AnimBadFps { fps: f32 },

    /// コマ打ちが 0．
    #[error("コマ打ちは 1 以上でなければならない")]
    AnimBadHold,

    /// フレームが 1 枚も無い．
    #[error("表示時間を付けるフレームが 1 枚も無い")]
    AnimNoFrames,

    /// コマ打ちの数がフレーム数と合わない．**足りないぶんを黙って埋めない**．
    #[error("コマ打ちが {holds} 個だがフレームは {frames} 枚ある (1 個か，枚数ちょうどにすること)")]
    AnimHoldCountMismatch { holds: usize, frames: usize },

    /// 周期アニメのフレームが 3 枚に満たない (D44)．
    #[error(
        "周期アニメのフレームは {min} 枚以上でなければならない ({frames} 枚)．\n\
         2 枚では軌跡が表現できず，正弦波では振幅がいくつでも 1 画素も動かない"
    )]
    AnimTooFewFrames { frames: u32, min: u32 },

    /// 回転は書いていない．
    ///
    /// **推測で書かない** — 回転は `pxsmith rotate` (設計書 6.13) の仕事であり，
    /// ここで別に書くと回転の実装が 2 つになる (D110 と同じ形の誤り) ．
    #[error(
        "回転の変調は書いていない (pxsmith rotate が未実装のため)．\n\
         16 通りのうち Rotate の 4 通りだけが未実装である"
    )]
    AnimRotateNotWritten,

    /// ランプが宣言されていない．
    #[error(
        "ランプの変調にはランプの宣言が要る (--ramp)．\n\
         絵だけから «どの色がどのランプの何段目か» は決まらない"
    )]
    AnimNoRamp,

    /// 中割りの `t` が $[0, 1]$ の外．**外挿は `pxsmith anim extrapolate` の仕事**である．
    #[error("中割りの t は 0 以上 1 以下でなければならない ({t})")]
    TweenBadT { t: f32 },

    /// 中割りの入力が空．補間する形が無い．
    #[error("中割りの入力に透明でない画素が 1 つも無い")]
    TweenEmptyMask,

    /// 中割りの枚数が 0．
    #[error("中割りの枚数は 1 以上でなければならない")]
    TweenNoSteps,

    /// 中割りの結果が共通の画布の外へ出た．
    ///
    /// $R \subseteq A \cup B$ は代数から出るので，**これが起きたら符号の規約か
    /// 余白の切り方が壊れている**．黙って切らずに落とす．
    #[error("中割りの結果が画布の外 ({x},{y}) へ出た (包含関係が破れている)")]
    TweenEscapedCanvas { x: i32, y: i32 },

    /// おばけの標本が 0．**両端だけを静かに返さない**．
    #[error("掃引の標本は 1 以上でなければならない (0 では両端しか出ない)")]
    SmearNoSamples,

    /// 外挿の振り幅が負か有限でない．**向きは `--kind` で決める**．
    #[error("外挿の振り幅は 0 以上の有限値でなければならない ({amount})．向きは --kind で決める")]
    ExtrapolateBadAmount { amount: f32 },

    /// 潰しの倍率が 0 以下．
    #[error("潰しの量は -1 より大きくなければならない ({amount}．縦の倍率が 0 以下になる)")]
    SquashBadAmount { amount: f32 },

    /// 潰す絵が無い．
    #[error("潰す画布に不透明な画素が 1 つも無い")]
    SquashEmptyCanvas,

    /// 残像を作るフレームが足りない．
    #[error("残像には 2 枚以上のフレームが要る ({frames} 枚)")]
    AfterimageTooFewFrames { frames: usize },

    /// 残像の長さが 0．
    #[error("残像の長さは 1 コマ以上でなければならない")]
    AfterimageNoTrail,

    /// サブピクセルの移動率が $[0, 1]$ の外．
    #[error("サブピクセルの移動率は 0 以上 1 以下でなければならない ({fraction})")]
    SubpixelBadFraction { fraction: f32 },

    /// 高速法の向きが 0．**接線を見ないので呼ぶ側が決める**．
    #[error("高速法は形の向きを見ないので，動かす向きを指定しなければならない (--direction)")]
    SubpixelNoDirection,
}

/// `pxsmith-core` の `Result` 別名．
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
