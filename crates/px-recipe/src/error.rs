//! レシピのエラー．
//!
//! **構成エラーは即座に停止する** (設計書 3.7 の表 — «レシピの構文誤り，
//! 存在しないパレット参照» は «即座に全体を停止») ．したがってここには
//! [`crate::FailurePolicy`] にあたる «続行» の道は無い．
//!
//! エラー文は**何が間違っているかだけでなく，何なら通るか**を出す．
//! レシピは人が書くものなので，«変数が無い» だけでは直せない．

/// レシピの読み込み ・評価 ・実行のエラー．
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RecipeError {
    // -------------------------------------------------------------- 式
    #[error("式が空である")]
    ExprEmpty,

    #[error("式に使えない文字がある: '{ch}'")]
    ExprBadChar { ch: char },

    #[error("数として読めない: '{text}'")]
    ExprBadNumber { text: String },

    #[error("文字列の引用符が閉じていない")]
    ExprUnterminatedString,

    #[error("式が途中で終わっている")]
    ExprUnexpectedEnd,

    #[error("ここに来られない字句がある: {text}")]
    ExprUnexpectedToken { text: String },

    #[error("{what} が必要である")]
    ExprExpected { what: String },

    #[error("式の後ろに余りがある: '{text}'")]
    ExprTrailing { text: String },

    /// **書いていない演算子**．設計書 4.2 の «許すもの» に無い．
    #[error(
        "{name} は書いていない — 設計書 4.2 の «許すもの» (変数参照 ・算術 ・\n\
         文字列連結 ・比較 ・三項演算子 ・直積展開) に入っていないためである．\n\
         組込み述語と三項演算子で書けないか確かめること"
    )]
    ExprOperatorNotWritten { name: String },

    #[error("変数 '{name}' が無い (ある変数: {known})")]
    ExprUnknownVariable { name: String, known: String },

    #[error(
        "'{name}' という関数は無い — レシピに関数は定義できない (D25)．\n\
         呼べるのは組込み述語だけである: {known}"
    )]
    ExprUnknownFunction { name: String, known: String },

    #[error("{op} に {got} は渡せない")]
    ExprBadType { op: String, got: String },

    #[error("{op} を {left} と {right} には当てられない")]
    ExprBadOperands {
        op: String,
        left: String,
        right: String,
    },

    #[error("0 で割っている")]
    ExprDivideByZero,

    #[error("'${{' が閉じていない: '{text}'")]
    ExprUnclosedInterpolation { text: String },

    #[error(
        "列を文字列の途中に埋め込めない: '{text}'\n\
         直積展開に渡すなら，文字列全体をちょうど 1 つの ${{...}} にすること"
    )]
    ExprListInString { text: String },

    // ------------------------------------------------------------ 述語
    #[error("述語 {name} は引数を {want} 個取る ({got} 個渡された)")]
    PredicateArity {
        name: String,
        want: usize,
        got: usize,
    },

    #[error("述語 {name} の対象 '{target}' が分からない (input ・previous ・名前)")]
    PredicateBadTarget { name: String, target: String },

    #[error("述語 {name} の対象 '{target}' がまだ実体化していない")]
    PredicateNotMaterialised { name: String, target: String },

    #[error("述語 {name} が {path} を読めない: {source}")]
    PredicateRead {
        name: String,
        path: String,
        source: std::io::Error,
    },

    // ---------------------------------------------------------- レシピ
    #[error("{path} を読めない: {source}")]
    RecipeRead {
        path: String,
        source: std::io::Error,
    },

    #[error("{path} を TOML として読めない: {source}")]
    RecipeParse {
        path: String,
        source: Box<toml::de::Error>,
    },

    /// **版が違う文書は黙って読まない** (D110 と同じ作法)．
    #[error("レシピのスキーマ版 {got} は読めない (このツールが読めるのは {want})")]
    RecipeVersion { got: u32, want: u32 },

    #[error("[[step]] が 1 つも無い")]
    RecipeNoSteps,

    #[error(
        "ステップ {at} の op '{op}' に当たるコマンドが無い．\n\
         op は px のサブコマンドと 1 対 1 である (設計書 4.2)．使えるのは:\n{known}"
    )]
    RecipeUnknownOp {
        at: usize,
        op: String,
        known: String,
    },

    #[error("ステップ {at} ({op}) の '{key}' に {got} は書けない")]
    RecipeBadField {
        at: usize,
        op: String,
        key: String,
        got: String,
    },

    #[error("ステップ {at} ({op}) の for_each の '{key}' が列でない ({got})")]
    RecipeForEachNotList {
        at: usize,
        op: String,
        key: String,
        got: String,
    },

    #[error("[vars] の '{name}' が入れ子の列になっている — 列の入れ子は書けない")]
    RecipeNestedList { name: String },

    // ------------------------------------------------------------ 依存
    #[error(
        "ステップの依存が循環している: {cycle}\n\
         レシピは有向非巡回グラフでなければならない"
    )]
    GraphCycle { cycle: String },

    #[error("ステップ {at} ({op}) の入力 '{path}' を作るステップが無く，ファイルも無い")]
    GraphMissingInput { at: usize, op: String, path: String },

    #[error("出力 '{path}' を 2 つのステップ ({first} と {second}) が書こうとしている")]
    GraphDuplicateOutput {
        path: String,
        first: usize,
        second: usize,
    },

    // ------------------------------------------------------ キャッシュ
    #[error("{path} を書けない: {source}")]
    CacheWrite {
        path: String,
        source: std::io::Error,
    },

    #[error("{path} を読めない: {source}")]
    CacheRead {
        path: String,
        source: std::io::Error,
    },

    /// **`op = "gen"` は既定でキャッシュ参照のみ** (D31)．
    #[error(
        "ステップ {at} の op = \"gen\" はキャッシュに無い．\n\
         レシピからの生成は既定で «キャッシュ参照のみ» である (D31)．\n\
         生成してよいなら --allow-generate を付けること"
    )]
    GenNotCached { at: usize },

    #[error("op = \"gen\" は書いていない (M6 の仕事である)")]
    GenNotWritten,
}

pub type Result<T> = std::result::Result<T, RecipeError>;
