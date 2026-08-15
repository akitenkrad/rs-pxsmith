//! `pxsmith-calib` — 閾値を決めるための校正ツール (D59)．
//!
//! **出荷物ではない．** `pyo3` + `maturin` で Python から呼ぶ案を採らず，校正を Rust に
//! 閉じるための道具である．指標は CSV へ書き出し，作図は任意のツールに任せる．
//!
//! ```sh
//! # 1. 合成 500 件を作る (正解つき)
//! cargo run -p pxsmith-calib --release -- gen
//! # 2. 閾値を掃引する (検証セット 300 件)
//! cargo run -p pxsmith-calib --release -- sweep
//! # 3. 指標を出して運転点を選ぶ
//! cargo run -p pxsmith-calib --release -- report
//! ```
//!
//! `sweep` は件数 x パラメータ組の回数だけ格子推定を回すので，**必ず `--release` で
//! 実行する**．

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

mod animcal;
mod aocal;
mod atmoscal;
mod bands;
mod composecal;
mod confidence;
mod dataset;
mod degrade;
mod diagnose;
mod dircal;
mod exactgrid;
mod ingest;
mod jaggycal;
mod jaggyseam;
mod jaggytruth;
mod lintcal;
mod lintgen;
mod metrics;
mod mixel;
mod pillow;
mod projcal;
mod real;
mod recon;
mod recover;
mod render;
mod replay;
mod rng;
mod rotcal;
mod scene;
mod seqcal;
mod shapecal;
mod sprite;
mod sweep;
mod tilecal;
mod tweencal;

use dataset::Split;
use metrics::Summary;
use sweep::{ParamGrid, Row};

/// 既定の作業場所．
const DEFAULT_DIR: &str = "grid-eval";

#[derive(Parser)]
#[command(
    name = "pxsmith-calib",
    version,
    about = "格子推定の閾値を決めるための校正ツール"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

// 掃引の口は «掛け替えられる軸» をそのまま旗にしているので，`Sweep` と `Replay` だけ
// 大きくなる．1 回しか作らない値なので，箱に入れて間接参照を増やす意味が無い
#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
enum Command {
    /// 合成の評価データセットを作る
    Gen {
        /// 出力先
        #[arg(long, default_value = DEFAULT_DIR)]
        dir: PathBuf,
        /// 元絵と位相を決める種 (決定論性のため必須)
        #[arg(long, default_value_t = 0)]
        seed: u64,
        #[arg(long, default_value_t = dataset::TOTAL)]
        count: u32,
        /// 元絵にする**実物のドット絵**の置き場所．省略すると合成の元絵を使う
        #[arg(long, default_value = "testdata/grid-eval/seeds")]
        seeds: Option<PathBuf>,
    },
    /// 閾値を掃引して 1 行 1 件の CSV を書く
    Sweep {
        #[arg(long, default_value = DEFAULT_DIR)]
        dir: PathBuf,
        /// 出力する CSV．既定は <dir>/sweep.csv
        #[arg(long)]
        out: Option<PathBuf>,
        /// 対象の分割．`all` で両方
        #[arg(long, default_value = "validation")]
        split: String,
        #[arg(long, default_value_t = 16)]
        max_scale: u32,
        /// セル内平均分散の許容 (複数指定可)
        #[arg(long, num_args = 1..)]
        epsilon: Vec<f32>,
        /// 再構成の画素色差の許容 (複数指定可)
        #[arg(long, num_args = 1..)]
        delta: Vec<f32>,
        /// 再構成の不一致画素率の許容 (複数指定可)
        #[arg(long, num_args = 1..)]
        tau: Vec<f32>,
        /// 位相ずれ検査の帯の数．0 で検査を外す
        #[arg(long, default_value_t = 4)]
        phase_bands: usize,
        /// 帯どうしの位相のずれの許容 ($s$ に対する割合．複数指定可)
        #[arg(long, num_args = 1..)]
        phase_tolerance: Vec<f32>,
        /// 帯のずれの許容の下限 (画素．複数指定可)
        #[arg(long, num_args = 1..)]
        phase_tolerance_floor: Vec<f32>,
        /// 帯ごとの位相**曲線**の食い違いの許容 (複数指定可)．1.0 以上で検査を外す
        #[arg(long, num_args = 1..)]
        phase_agreement: Vec<f32>,
        /// 半セルずらしたときの崩れ方の下限 (複数指定可)．1.0 以下で検査を外す
        #[arg(long, num_args = 1..)]
        phase_contrast_min: Vec<f32>,
        /// 測れない候補も素通しする (既定は棄却)．掃引 1 回につき 1 通り
        #[arg(long)]
        allow_unmeasurable: bool,
        /// 帯ごとの位相を**副画素**で求める．掃引 1 回につき 1 通り
        #[arg(long)]
        phase_subpixel: bool,
        /// 信頼度の下限を $\hat{s}$ で割らない (既定は割る)．掃引 1 回につき 1 通り
        #[arg(long)]
        uniform_confidence: bool,
        /// 位相ずれ検査に要る帯あたりのセル数
        #[arg(long, default_value_t = 2)]
        phase_min_cells: usize,
        /// $\varepsilon$ を**画像分散に対する割合**として扱う (絶対値ではなく)．
        /// 省略すると既定値に従う — **旗の有無で既定を潰さない**
        #[arg(long, num_args = 0..=1, default_missing_value = "true")]
        normalize_epsilon: Option<bool>,
        /// 境界の当てはめに使う差分の階数．**0 でこの関門を外して比べられる**
        #[arg(long, default_value_t = pxsmith_core::grid::GridParams::default().edge_fit_order)]
        edge_fit_order: u32,
        /// 曲線を肩代わりする残差 (D73)．**負の値でこの肩代わりを外して比べられる**
        #[arg(long)]
        edge_fit_curve_residual: Option<f32>,
    },
    /// 再構成誤差を帯ごとに測る (掃引の行き止まりを抜けられるかの実測)
    Bands {
        #[arg(long, default_value = DEFAULT_DIR)]
        dir: PathBuf,
        /// 出力する CSV．既定は <dir>/bands.csv
        #[arg(long)]
        out: Option<PathBuf>,
        /// 対象の分割．`all` で両方
        #[arg(long, default_value = "validation")]
        split: String,
        /// 掃引で完全一致率が最大だった水準
        #[arg(long, default_value_t = 0.02)]
        epsilon: f32,
        #[arg(long, default_value_t = 0.15)]
        delta: f32,
        #[arg(long, default_value_t = 0.02)]
        tau: f32,
    },
    /// 信頼度の各項を測る (なぜ誤答の方が自信を持つのか)
    Confidence {
        #[arg(long, default_value = DEFAULT_DIR)]
        dir: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, default_value = "validation")]
        split: String,
        /// 掃引で最良だった組
        #[arg(long, default_value_t = 0.01)]
        epsilon: f32,
        #[arg(long, default_value_t = 0.05)]
        delta: f32,
        #[arg(long, default_value_t = 0.3)]
        tau: f32,
    },
    /// ドット絵風の画像を実データ枠の正例へ仕立てる
    Ingest {
        /// 入力画像 (生成 AI の出力など)
        images: Vec<PathBuf>,
        #[arg(long, default_value = "testdata/grid-eval/real")]
        dir: PathBuf,
        /// 拡大倍率．省略すると 2〜12 を巡回して条件を散らす
        #[arg(long)]
        scale: Option<u32>,
        /// 位相をずらすために切り落とす画素数 `DX,DY`．省略すると倍率から散らす
        #[arg(long)]
        crop: Option<String>,
        /// 区分
        #[arg(long, default_value = "ai-output")]
        category: String,
        /// 目録に書くライセンス
        #[arg(long, default_value = "CC0 (自作)")]
        license: String,
        /// 縮小後に許す一辺の上限．背景画は 160x90 などになるので広げる
        #[arg(long, default_value_t = ingest::NATIVE_MAX)]
        native_max: u32,
        /// 見かけの周期を測らずにこの値で縮小する．**格子が無い画像からも正例を作れる**
        /// (正解は拡大倍率の方なので循環しない)
        #[arg(long)]
        force_period: Option<usize>,
        /// 非整数倍で拡大されて配られている素材から**元絵を厳密に復元**してから拡大する．
        /// 中身が本物のドット絵である正例を作れる
        #[arg(long, conflicts_with = "force_period")]
        recover_native: bool,
        /// **入力がすでに元絵である** (CC0 素材の 16x16 ・32x32 タイル等)．縮小しない
        #[arg(long, conflicts_with_all = ["force_period", "recover_native"])]
        native: bool,
        /// 拡大時に合成データと同じ劣化 (補間 ・JPEG) を通す．非整数倍リサイズは掛けない
        #[arg(long)]
        degrade: bool,
        /// 「周期が読めない」で拒否したものを**負例**として取り込む (棄却が正解)
        #[arg(long)]
        keep_refused: bool,
        /// 負例として保存するときの一辺 (中央を切り出す．再標本化はしない)
        #[arg(long, default_value_t = ingest::NEGATIVE_SIDE)]
        negative_side: u32,
    },
    /// Tiled の地図 (.tmx) を元絵の解像度で描き出す (画面を組んだ正例の素材)
    Scene {
        /// 入力する .tmx
        maps: Vec<PathBuf>,
        /// 出力先ディレクトリ
        #[arg(long, default_value = "scenes")]
        out: PathBuf,
        /// 描くタイル数の上限 `W,H`．**推定の費用は面積で効く**ので大きい地図は切る
        #[arg(long, default_value = "30,20")]
        max_tiles: String,
    },
    /// 実データ枠の素材を自作レンダで作る (区分 `render`)
    Render {
        #[arg(long, default_value = "testdata/grid-eval/real")]
        dir: PathBuf,
        #[arg(long, default_value_t = 25)]
        count: u32,
        #[arg(long, default_value_t = 7)]
        seed: u64,
    },
    /// 実データ (合成でない入力) を推定して 1 件ずつ並べる
    Real {
        /// 素材と目録の置き場所
        #[arg(long, default_value = "testdata/grid-eval/real")]
        dir: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
        /// 閾値．**省略すると既定値**で回す (合成データで校正した組)
        #[arg(long)]
        epsilon: Option<f32>,
        #[arg(long)]
        delta: Option<f32>,
        #[arg(long)]
        tau: Option<f32>,
        #[arg(long)]
        min_confidence: Option<f32>,
        /// $\varepsilon$ を画像分散に対する割合として扱う．省略すると既定値に従う
        #[arg(long, num_args = 0..=1, default_missing_value = "true")]
        normalize_epsilon: Option<bool>,
        /// 帯どうしの位相のずれの許容 ($s$ に対する割合)．**実データ枠で閾値を
        /// 選ぶためではなく，どの変更がどう効いたかを分けて見るための口である**
        #[arg(long)]
        phase_tolerance: Option<f32>,
        /// 帯ごとの位相曲線の食い違いの許容．1.0 以上でこの検査を外す
        #[arg(long)]
        phase_agreement: Option<f32>,
        /// 半セルずらしたときの崩れ方の下限．1.0 以下でこの検査を外す
        #[arg(long)]
        phase_contrast_min: Option<f32>,
        /// 帯ずれの許容の下限 (画素)．割合の許容が小さい $s$ で 1 画素を割るのを防ぐ
        #[arg(long)]
        phase_tolerance_floor: Option<f32>,
        /// 測れない候補も素通しする (既定は棄却)
        #[arg(long, num_args = 0..=1, default_missing_value = "true")]
        allow_unmeasurable: Option<bool>,
        /// 境界の当てはめに使う差分の階数．**0 でこの関門を外す**
        #[arg(long)]
        edge_fit_order: Option<u32>,
        /// 当てはめた間隔のずれの許容
        #[arg(long)]
        edge_fit_slope: Option<f32>,
        /// 肩代わりに要る境界の本数 (軸ごと)
        #[arg(long)]
        edge_fit_min_count: Option<usize>,
        /// 曲線を肩代わりする残差 (D73)．**負の値でこの肩代わりを外す**
        #[arg(long)]
        edge_fit_curve_residual: Option<f32>,
        /// **境界の峰の非極大抑制の半径** ($s$ に対する割合．D173)
        #[arg(long)]
        peak_suppression: Option<f32>,
        /// **峰とみなすエネルギーの下限** (平均に対する倍率．D173)
        #[arg(long)]
        peak_floor: Option<f32>,
    },
    /// 再構成検査の統計を測り直す (内側と境界を分けたら真の s を見分けられるか)
    Recon {
        #[arg(long, default_value = DEFAULT_DIR)]
        dir: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, default_value = "validation")]
        split: String,
        /// 整数の格子が無い件 (非整数倍リサイズ) も測る．**位相ずれ検査の相手**である
        #[arg(long)]
        include_resized: bool,
        /// **実データの目録を測る** (`--dir testdata/grid-eval/real` のように指す)．
        /// 負例に何が起きたかは判定行からしか読めなかった — 漏れを追う口である
        #[arg(long)]
        real: bool,
        /// **境界の峰の非極大抑制の半径** ($s$ に対する割合．D173)
        #[arg(long, default_value_t = pxsmith_core::grid::GridParams::default().peak_suppression)]
        peak_suppression: f32,
        /// **峰とみなすエネルギーの下限** (平均に対する倍率．D173)
        #[arg(long, default_value_t = pxsmith_core::grid::GridParams::default().peak_floor)]
        peak_floor: f32,
    },
    /// **掃引を回さずに関門を掛け替える** — recon の CSV から estimate_grid を再現する
    Replay {
        #[arg(long, default_value = DEFAULT_DIR)]
        dir: PathBuf,
        /// 読む CSV．既定は <dir>/recon.csv (**--include-resized で取ったもの**)
        #[arg(long)]
        csv: Option<PathBuf>,
        #[arg(long, default_value = "validation")]
        split: String,
        /// 画像から estimate_grid を回し，再現とどれだけ食い違うかを数える
        #[arg(long)]
        verify: bool,
        /// 落ちた関門を候補ごとに数える (真の $s$ を何が落としているか)
        #[arg(long)]
        gates: bool,
        #[arg(long, num_args = 1.., default_values_t = [pxsmith_core::grid::GridParams::default().epsilon])]
        epsilon: Vec<f32>,
        #[arg(long, num_args = 1.., default_values_t = [pxsmith_core::grid::GridParams::default().tau])]
        tau: Vec<f32>,
        #[arg(long, num_args = 1.., default_values_t = [pxsmith_core::grid::GridParams::default().phase_tolerance])]
        phase_tolerance: Vec<f32>,
        #[arg(long, num_args = 1.., default_values_t = [pxsmith_core::grid::GridParams::default().phase_agreement])]
        phase_agreement: Vec<f32>,
        #[arg(long, num_args = 1.., default_values_t = [pxsmith_core::grid::GridParams::default().phase_contrast_min])]
        phase_contrast_min: Vec<f32>,
        #[arg(long, num_args = 1.., default_values_t = [pxsmith_core::grid::GridParams::default().min_confidence])]
        min_confidence: Vec<f32>,
        /// 境界の当てはめを使う階数 (1 か 2)．0 でこの関門を外す
        #[arg(long, num_args = 1.., default_values_t = [pxsmith_core::grid::GridParams::default().edge_fit_order])]
        edge_order: Vec<u32>,
        /// 境界の当てはめの掛け方 (`and` = 位相に足す ・`or` = 位相を肩代わりする)
        #[arg(long, num_args = 1.., default_values_t = [String::from("or-drift")])]
        edge_mode: Vec<String>,
        /// 当てはめた間隔のずれの許容 (複数指定可)
        #[arg(long, num_args = 1.., default_values_t = [pxsmith_core::grid::GridParams::default().edge_fit_slope])]
        edge_slope: Vec<f32>,
        /// 残差 RMS ($s$ で正規化) の許容 (複数指定可)
        #[arg(long, num_args = 1.., default_values_t = [pxsmith_core::grid::GridParams::default().edge_fit_residual])]
        edge_residual: Vec<f32>,
        /// 拾えた境界の本数の下限 (複数指定可)
        #[arg(long, num_args = 1.., default_values_t = [pxsmith_core::grid::GridParams::default().edge_fit_min_count])]
        edge_min_count: Vec<usize>,
        /// 期待される本数に対する割合の下限 (複数指定可)
        #[arg(long, num_args = 1.., default_values_t = [0.0f32])]
        edge_min_coverage: Vec<f32>,
        /// 境界が拾えない候補に肩代わりを**させない** (`and` では棄却する)．
        /// 既定は真 — `--edge-require-measurable false` で外せる
        #[arg(long, num_args = 0..=1, default_missing_value = "true", default_value_t = true)]
        edge_require_measurable: bool,
        /// 境界の当てはめが**帯ずれ以外に**肩代わりする関門
        /// (`none` ・`eps` ・`recon` ・`contrast` ・`curve` を `+` でつなぐ．複数指定可)
        #[arg(long, num_args = 1.., default_values_t = [String::from("none")])]
        edge_rescue: Vec<String>,
        /// **曲線を肩代わりするときだけの傾きの許容** (既定は帯ずれと同じ値)．
        /// 曲線は «棄却を引き受ける» 量なので，緩いまま手放すと誤受理が戻る
        #[arg(long, num_args = 1.., default_values_t = [pxsmith_core::grid::GridParams::default().edge_fit_slope])]
        edge_curve_slope: Vec<f32>,
        /// 同上 (残差 RMS)
        #[arg(long, num_args = 1.., default_values_t = [pxsmith_core::grid::GridParams::default().edge_fit_residual])]
        edge_curve_residual: Vec<f32>,
        /// 同上 (境界の本数の下限)
        #[arg(long, num_args = 1.., default_values_t = [pxsmith_core::grid::GridParams::default().edge_fit_min_count])]
        edge_curve_min_count: Vec<usize>,
        /// **当てはまりが酷い候補を落とす床** (残差の上限．複数指定可)．
        /// **負の値で «落とさない»** (`--edge-drop-residual=-1` と書く)．
        /// 肩代わりは残したまま «落とす側» にだけ働く
        #[arg(long, num_args = 1.., allow_negative_numbers = true, default_values_t = [-1.0f32])]
        edge_drop_residual: Vec<f32>,
        /// 床を «どちらかの軸が酷ければ» 落とす形にする (既定は «両軸とも酷いときだけ»)
        #[arg(long)]
        edge_drop_any_axis: bool,
        /// 曲線の正規化 — 分母を $(A - M) + \lambda A$ にする ($\lambda = 0$ が現行)
        #[arg(long, num_args = 1.., default_values_t = [0.0f32])]
        curve_lambda: Vec<f32>,
        /// **当てはまりの測り方** (`rms` = 現行 ・`median` ・`folded`)
        #[arg(long, num_args = 1.., default_values_t = [String::from("rms")])]
        edge_stat: Vec<String>,
        /// 曲線を軸ごとにまとめる形 (`mean` = 現行 ・`max`)
        #[arg(long, num_args = 1.., default_values_t = [String::from("mean")])]
        curve_axis: Vec<String>,
        /// 上位何組を出すか
        #[arg(long, default_value_t = 20)]
        top: usize,
    },
    /// **正解の格子を与えて**縮小し，元絵が戻るかを測る (当てる価値があるのか)
    Recover {
        #[arg(long, default_value = DEFAULT_DIR)]
        dir: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, default_value = "validation")]
        split: String,
        /// 元絵に使った実物のドット絵の置き場所 (gen と同じものを渡す)
        #[arg(long, default_value = "testdata/grid-eval/seeds")]
        seeds: Option<PathBuf>,
    },
    /// 実データの誤棄却を 1 件ずつ解剖する (どの関門が真のスケールを落としたか)
    Diagnose {
        #[arg(long, default_value = "testdata/grid-eval/real")]
        dir: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// lint の閾値を測る (良い絵に現行の閾値を掛けて何が鳴るかを見る)
    Lint {
        /// 検査する PNG の置き場所
        #[arg(long, default_value = "testdata/grid-eval/seeds")]
        dir: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
        /// 正解つきの評価データセットとして扱い，**格子の有無で分けて**数える．
        /// ルール 2 は «崩れている件» で鳴らなければ意味が無い
        #[arg(long)]
        dataset: bool,
        /// ルール 2 の «格子を名乗っているか» の閾値 (複数指定可．掃引する)
        #[arg(long, num_args = 1..)]
        grid_like_ratio: Vec<f32>,
        /// ルール 11 の明度差の下限 (複数指定可．掃引する)
        #[arg(long, num_args = 1..)]
        min_lightness_delta: Vec<f32>,
        /// ルール 6 の «同一色相とみなす色相差» (度．複数指定可．掃引する)
        #[arg(long, num_args = 1..)]
        shadow_hue: Vec<f32>,
        /// ルール 4 の «縁取りの色に許す内側の割合» (複数指定可．掃引する)．
        /// **正例と負例を同時に出す**
        #[arg(long, num_args = 1..)]
        outline_interior: Vec<f32>,
        /// 掃引で並べて見る負例の置き場所
        #[arg(long, default_value = "testdata/lint-cases/negative")]
        negative: PathBuf,
    },
    /// **ルール 13 (pillow shading) の閾値を測る** — 良い絵 ・負例 ・`pxsmith shade` の出力
    Pillow {
        /// 良い絵 (正例)
        #[arg(long, default_value = "testdata/grid-eval/seeds")]
        seeds: PathBuf,
        /// 負例の置き場所 (`lint-gen` が作る `pillow-*.png` だけを見る)
        #[arg(long, default_value = "testdata/lint-cases/negative")]
        negative: PathBuf,
        /// `pxsmith shade` のランプ段数
        #[arg(long, default_value_t = 5)]
        steps: u8,
        /// 掃く閾値
        #[arg(long, num_args = 1..)]
        threshold: Vec<f32>,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// **ジャギー検出を良い絵に掛けて数える** (`pxsmith smooth` が動かす前に測る)
    Jaggy {
        #[arg(long, default_value = "testdata/grid-eval/seeds")]
        dir: PathBuf,
        /// 移動上限 (画素)
        #[arg(long, default_value_t = jaggycal::DEFAULT_MOVE)]
        max_move: u32,
        #[arg(long)]
        out: Option<PathBuf>,
        /// 件数の多い絵を何枚並べるか
        #[arg(long, default_value_t = 10)]
        top: usize,
        /// **`pxsmith smooth` を実際に掛けて，直った結果を測る**
        #[arg(long)]
        apply: bool,
    },
    /// **局所格子推定の窓を «真値のある場面» で測る** — 付録 C 要調査事項 #4
    Mixel {
        #[arg(long, default_value = "testdata/grid-eval/seeds")]
        dir: PathBuf,
        /// 種の組を何通り作るか (1 通りにつき場面 27 枚)
        #[arg(long, default_value_t = 6)]
        sheets: usize,
        /// 一致率の閾値 (lint ルール 9 ・`pxsmith conform --uniformity` の既定)
        #[arg(long, default_value_t = 0.8)]
        uniformity: f32,
        #[arg(long)]
        out: Option<PathBuf>,
        /// **場面を PNG で書き出す** — `pxsmith lint` ・`pxsmith conform` に食わせて端から端まで通す
        #[arg(long)]
        dump: Option<PathBuf>,
    },
    /// **ルール 9 を «厳密なブロック判定» に替えたときの上限を測る** — D37 に触る前の判断材料
    MixelExact {
        #[arg(long, default_value = "testdata/grid-eval/seeds")]
        dir: PathBuf,
        /// 種の組を何通り作るか
        #[arg(long, default_value_t = 6)]
        sheets: usize,
        /// 見る升の上限
        #[arg(long, default_value_t = exactgrid::MAX_K)]
        max_k: u32,
    },
    /// **ジャギー検出に «真値のある場面» を掛ける** — 付録 C 要調査事項 #1 を閉じる
    JaggyTruth {
        /// 移動上限 (画素)
        #[arg(long, default_value_t = jaggycal::DEFAULT_MOVE)]
        max_move: u32,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// **円板の谷を «継ぎ目» として許したときの上限を測る** — 認識器を書く前の判断材料
    JaggySeam {
        /// 移動上限 (画素)
        #[arg(long, default_value_t = jaggycal::DEFAULT_MOVE)]
        max_move: u32,
        /// 実素材 (省略すると清書と負例だけ測る)
        #[arg(long, default_value = "testdata/grid-eval/seeds")]
        dir: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// **`pxsmith aa` を良い絵に掛けて壊れないか測る**
    Aa {
        #[arg(long, default_value = "testdata/grid-eval/seeds")]
        dir: PathBuf,
        /// 外郭にも付ける (D34 の既定は内部境界のみ)
        #[arg(long)]
        outline: bool,
    },
    /// **`pxsmith outline` を良い絵に掛けて壊れないか測る** (5 分類すべて)
    Outline {
        #[arg(long, default_value = "testdata/grid-eval/seeds")]
        dir: PathBuf,
        /// 外側に描く (既定は内側)
        #[arg(long)]
        outer: bool,
    },
    /// **環境遮蔽 (`pxsmith shade --ao`) の閾値を測る** — 凸な形 ・凹んだ形 ・実素材の 3 群
    Ao {
        #[arg(long, default_value = "testdata/grid-eval/seeds")]
        seeds: PathBuf,
        /// 掃く閾値
        #[arg(long, num_args = 1.., default_values_t = [0.10f32, 0.15, 0.20, 0.25, 0.30, 0.40, 0.60])]
        threshold: Vec<f32>,
        /// 距離場を均す回数 (**閾値と組で掃く**)
        #[arg(long, num_args = 1.., default_values_t = [pxsmith_core::shade::DEFAULT_AO_SMOOTH_PASSES])]
        passes: Vec<usize>,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// **ディザの位相を測る** — 設計書 4.3 / D45 の «偶数幅だと反復で連結する» を確かめる
    Dither {
        /// 測るタイルの一辺
        #[arg(long, num_args = 1.., default_values_t = [8u32, 15, 16])]
        tile: Vec<u32>,
    },
    /// **タイル分割と同値判定を測る** — 3 モードの削減率と，ルール 7 が掛かるタイルの割合
    Tileset {
        #[arg(long, default_value = "testdata/grid-eval/seeds")]
        seeds: PathBuf,
        /// 切るタイルの一辺
        #[arg(long, num_args = 1.., default_values_t = [8u32, 16])]
        tile: Vec<u32>,
        /// 宣言する光源 (ルール 7 用)
        #[arg(long, default_value = "dir:-0.6,0.8")]
        light: String,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// **方向展開 (反転 + 陰影再導出) を測る** — 実素材でルール 7 が鳴るか ・再導出が何を書き換えるか
    Direction {
        #[arg(long, default_value = "testdata/grid-eval/seeds")]
        seeds: PathBuf,
        #[arg(long, default_value_t = 5)]
        steps: u8,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// **`pxsmith compose` の «置き方» を測る** — 余白 ・併合したパレット ・実際に合成した結果
    Compose {
        #[arg(long, default_value = "testdata/grid-eval/seeds")]
        seeds: PathBuf,
        /// 先頭のパーツにする絵 (省略時は Dungeon Crawl の生き物 12 枚)
        #[arg(long, num_args = 1..)]
        base: Vec<String>,
        /// 重ねるパーツにする絵 (省略時は被り物と衣服 3 枚)
        #[arg(long, num_args = 1..)]
        equip: Vec<String>,
        /// 先頭のパーツの画布に切り揃える側も測る
        #[arg(long)]
        clip: bool,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// **ルール 7 (反転同値の陰影不整合) の閾値を測る** — `pxsmith shade` の出力と，その左右反転
    Flip {
        #[arg(long, default_value = "testdata/grid-eval/seeds")]
        seeds: PathBuf,
        #[arg(long, default_value_t = 5)]
        steps: u8,
    },
    /// **おばけが繋がるかを測る** — union ・掃引 ・重心を取り除いた掃引の 3 通りと，刻み幅
    Smear {
        #[arg(long, default_value = "testdata/grid-eval/seeds")]
        seeds: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// **外挿が真値に当たるかを測る** — 平行移動なら «t 倍動かした絵» が真値である
    Extrapolate {
        #[arg(long, default_value = "testdata/grid-eval/seeds")]
        seeds: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// **体積保存が画素の丸めでどれだけ崩れるかを測る** — 2 通りの決め方を並べる
    Squash {
        #[arg(long, default_value = "testdata/grid-eval/seeds")]
        seeds: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// **サブピクセルが実素材で効くかを測る** — 接線法 2 通りと高速法
    Subpixel {
        #[arg(long, default_value = "testdata/grid-eval/seeds")]
        seeds: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// **残像が «見える» のはどれくらい動いたときかを測る** — `pxsmith shade` に描かせた列で
    Afterimage {
        #[arg(long, default_value = "testdata/grid-eval/seeds")]
        seeds: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// **`pxsmith rotate` / `pxsmith scale` の品質を測る** — 往復の真値 ・対照 nearest ・ジャギー
    Rotate {
        #[arg(long, default_value = "testdata/grid-eval/seeds")]
        seeds: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// **投影の主張を測る** — 設計書 6.13 の 2 手順は同じ変換か ・30 度は格子に乗るか
    Project {
        #[arg(long, default_value = "testdata/grid-eval/seeds")]
        seeds: PathBuf,
        /// 段を刻む線の長さ
        #[arg(long, default_value_t = 64)]
        stair_len: u32,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// **ルール 19 ・20 ・21 の閾値を測る** — 付録 C 要調査事項 #2 を閉じる
    LintShape {
        #[arg(long, default_value = "testdata/grid-eval/seeds")]
        seeds: PathBuf,
        #[arg(long, default_value_t = 8)]
        min_area: u32,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// **フレーム間ルール (22 〜 27) の適用範囲と閾値を測る** — 正例 3 群 ・負例 6 種
    LintSeq {
        #[arg(long, default_value = "testdata/grid-eval/seeds")]
        seeds: PathBuf,
    },
    /// **空気遠近法が «効く» のかを測る** — 寄せた先がパレットに在る割合 ・真値 ・段の数
    Atmos {
        #[arg(long, default_value = "testdata/grid-eval/seeds")]
        seeds: PathBuf,
        /// $t$ を細かく掃いて «パレットが表せる段の数» を数えるときの刻み数
        #[arg(long, default_value_t = 50)]
        level_steps: usize,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// **中割りが実用になるかを測る (R11)** — 真値のある平行移動 ・余白 ・トポロジー
    Tween {
        #[arg(long, default_value = "testdata/grid-eval/seeds")]
        seeds: PathBuf,
        /// トポロジーを測る «別の絵» (省略時は Dungeon Crawl の生き物 6 枚)
        #[arg(long, num_args = 1..)]
        pair: Vec<String>,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// lint の負例を作る (良い絵に欠陥を 1 つだけ入れる)
    LintGen {
        /// 元にする良い絵
        #[arg(long, default_value = "testdata/grid-eval/seeds")]
        seeds: PathBuf,
        #[arg(long, default_value = "testdata/lint-cases/negative")]
        out: PathBuf,
        /// 欠陥 1 種類あたり何枚作るか
        #[arg(long, default_value_t = 8)]
        count: usize,
        #[arg(long, default_value_t = 0)]
        seed: u64,
    },
    /// 掃引の結果から指標を出す
    Report {
        #[arg(long, default_value = DEFAULT_DIR)]
        dir: PathBuf,
        /// 読み込む掃引 CSV．既定は <dir>/sweep.csv
        #[arg(long)]
        sweep: Option<PathBuf>,
        /// 目標精度 (実装計画書 M2 は 95%)
        #[arg(long, default_value_t = 0.95)]
        target: f32,
        /// 上位いくつを表示するか
        #[arg(long, default_value_t = 10)]
        top: usize,
    },
}

/// `dir:X,Y` だけを読む最小の解釈器 (`pxsmith` 側の `parse_light` の平行光源だけ)．
fn parse_light_spec(spec: &str) -> Result<pxsmith_core::ramp::LightSource> {
    let (kind, rest) = spec
        .split_once(':')
        .with_context(|| format!("光源 '{spec}' は 'dir:X,Y' の形で書くこと"))?;
    anyhow::ensure!(kind == "dir", "ここで扱うのは平行光源だけである ('{spec}')");
    let n: Vec<f32> = rest
        .split(',')
        .map(|v| {
            v.trim()
                .parse::<f32>()
                .with_context(|| format!("'{v}' を読めない"))
        })
        .collect::<Result<_>>()?;
    anyhow::ensure!(n.len() == 2, "dir は 2 つの数値が要る ('{spec}')");
    Ok(pxsmith_core::ramp::LightSource::Directional {
        dir: pxsmith_core::math::Vec2 { x: n[0], y: n[1] },
    })
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Gen {
            dir,
            seed,
            count,
            seeds,
        } => {
            let seeds = match seeds.filter(|p| p.exists()) {
                Some(p) => {
                    let s = sprite::load_seeds(&p)?;
                    println!(
                        "元絵に実物のドット絵 {} 件を使う ({})",
                        s.len(),
                        p.display()
                    );
                    s
                }
                None => {
                    println!("元絵を合成する (実物の種が無い)");
                    Vec::new()
                }
            };
            let manifest = dataset::build(seed, count, &seeds);
            dataset::generate(&dir, &manifest, &seeds)?;
            let with_grid = manifest
                .items
                .iter()
                .filter(|i| i.has_integer_grid())
                .count();
            let validation = manifest
                .items
                .iter()
                .filter(|i| i.split == Split::Validation)
                .count();
            println!(
                "{} 件を {} へ書いた (検証 {} / テスト {}，うち整数の格子があるもの {})",
                manifest.count,
                dir.display(),
                validation,
                manifest.count as usize - validation,
                with_grid,
            );
        }

        Command::Sweep {
            dir,
            out,
            split,
            max_scale,
            epsilon,
            delta,
            tau,
            phase_bands,
            phase_tolerance,
            phase_tolerance_floor,
            phase_agreement,
            phase_contrast_min,
            allow_unmeasurable,
            phase_subpixel,
            uniform_confidence,
            phase_min_cells,
            normalize_epsilon,
            edge_fit_order,
            edge_fit_curve_residual,
        } => {
            let manifest = dataset::read(&dir)?;
            let only = parse_split(&split)?;

            let default = ParamGrid::default();
            let grid = ParamGrid {
                max_scale,
                phase_bands,
                phase_tolerances: or_default(phase_tolerance, &default.phase_tolerances),
                phase_subpixel,
                confidence_per_scale: !uniform_confidence,
                phase_tolerance_floors: or_default(
                    phase_tolerance_floor,
                    &default.phase_tolerance_floors,
                ),
                phase_agreements: or_default(phase_agreement, &default.phase_agreements),
                phase_contrast_mins: or_default(phase_contrast_min, &default.phase_contrast_mins),
                phase_require_measurable: !allow_unmeasurable,
                phase_min_cells,
                normalize_epsilon: normalize_epsilon
                    .unwrap_or(pxsmith_core::grid::GridParams::default().normalize_epsilon),
                epsilons: or_default(epsilon, &default.epsilons),
                deltas: or_default(delta, &default.deltas),
                taus: or_default(tau, &default.taus),
                edge_fit_order,
                // 負の値は «肩代わりしない» の指定である (旗の有無で既定を潰さない)
                edge_fit_curve_residual: match edge_fit_curve_residual {
                    Some(v) if v < 0.0 => None,
                    Some(v) => Some(v),
                    None => default.edge_fit_curve_residual,
                },
                ..default
            };
            let combos = grid.combinations().len();
            let items = manifest
                .items
                .iter()
                .filter(|i| only.is_none_or(|s| i.split == s))
                .count();
            println!("{items} 件 x {combos} 通り = {} 回の推定", items * combos);

            let rows = sweep::run(&dir, &manifest, only, &grid)?;
            let path = out.unwrap_or_else(|| dir.join("sweep.csv"));
            sweep::write_csv(&path, &rows)?;
            println!("{} 行を {} へ書いた", rows.len(), path.display());
        }

        Command::Bands {
            dir,
            out,
            split,
            epsilon,
            delta,
            tau,
        } => {
            let manifest = dataset::read(&dir)?;
            let only = parse_split(&split)?;
            let records = bands::run(&dir, &manifest, only, epsilon, delta, tau)?;
            let path = out.unwrap_or_else(|| dir.join("bands.csv"));
            write_bands(&path, &records)?;
            report_bands(&records);
        }

        Command::Confidence {
            dir,
            out,
            split,
            epsilon,
            delta,
            tau,
        } => {
            let manifest = dataset::read(&dir)?;
            let only = parse_split(&split)?;
            let params = pxsmith_core::grid::GridParams {
                epsilon,
                delta,
                tau,
                ..Default::default()
            };
            let records = confidence::run(&dir, &manifest, only, &params)?;
            let path = out.unwrap_or_else(|| dir.join("confidence.csv"));
            write_confidence(&path, &records)?;
            report_confidence(&records);
        }

        Command::Ingest {
            images,
            dir,
            scale,
            crop,
            category,
            license,
            native_max,
            force_period,
            recover_native,
            native,
            degrade,
            keep_refused,
            negative_side,
        } => {
            ingest_images(
                &dir,
                &images,
                &IngestOptions {
                    scale,
                    crop: crop.as_deref(),
                    category: &category,
                    license: &license,
                    native_max,
                    force_period,
                    recover_native,
                    already_native: native,
                    degrade,
                    negative_side: keep_refused.then_some(negative_side),
                },
            )?;
        }

        Command::Scene {
            maps,
            out,
            max_tiles,
        } => {
            anyhow::ensure!(!maps.is_empty(), ".tmx を 1 つ以上指定すること");
            let (w, h) = max_tiles
                .split_once(',')
                .context("--max-tiles は W,H の形で書くこと")?;
            let max = (w.trim().parse()?, h.trim().parse()?);
            std::fs::create_dir_all(&out)?;
            for path in &maps {
                let scene = scene::render(path, max)?;
                // 出力名は「パック名_地図名」— どの見本地図か分かるようにする
                let stem = path.file_stem().unwrap_or_default().to_string_lossy();
                let pack = path
                    .ancestors()
                    .nth(2)
                    .and_then(|p| p.file_name())
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let name = format!("{pack}_{stem}.png");
                pxsmith_io::png::write_rgba(out.join(&name), &scene.canvas)?;
                println!(
                    "  {name:<46} {}x{} 画素 ({}x{} タイル{} / {} 層)",
                    scene.canvas.width(),
                    scene.canvas.height(),
                    scene.tiles.0,
                    scene.tiles.1,
                    if scene.tiles == scene.full {
                        String::new()
                    } else {
                        format!("．{}x{} から切った", scene.full.0, scene.full.1)
                    },
                    scene.layers,
                );
            }
            println!("\n{} 件を {} へ書いた", maps.len(), out.display());
        }

        Command::Render { dir, count, seed } => {
            let items = render_real_items(&dir, count, seed)?;
            println!("{} 件を {} へ書いた", items, dir.display());
        }

        Command::Real {
            dir,
            out,
            epsilon,
            delta,
            tau,
            min_confidence,
            normalize_epsilon,
            edge_fit_curve_residual,
            phase_tolerance,
            phase_agreement,
            phase_contrast_min,
            phase_tolerance_floor,
            allow_unmeasurable,
            edge_fit_order,
            edge_fit_slope,
            edge_fit_min_count,
            peak_suppression,
            peak_floor,
        } => {
            let d = pxsmith_core::grid::GridParams::default();
            let params = pxsmith_core::grid::GridParams {
                // **境界の峰の拾い方も実データで測れるようにした** (D173) —
                // 合成側だけ見て採ると «実データの安全側» が測れない
                peak_suppression: peak_suppression.unwrap_or(d.peak_suppression),
                peak_floor: peak_floor.unwrap_or(d.peak_floor),
                epsilon: epsilon.unwrap_or(d.epsilon),
                delta: delta.unwrap_or(d.delta),
                tau: tau.unwrap_or(d.tau),
                min_confidence: min_confidence.unwrap_or(d.min_confidence),
                // 負の値は «肩代わりしない» の指定である (旗の有無で既定を潰さない)
                edge_fit_curve_residual: match edge_fit_curve_residual {
                    Some(v) if v < 0.0 => None,
                    Some(v) => Some(v),
                    None => d.edge_fit_curve_residual,
                },
                normalize_epsilon: normalize_epsilon.unwrap_or(d.normalize_epsilon),
                phase_tolerance: phase_tolerance.unwrap_or(d.phase_tolerance),
                phase_agreement: phase_agreement.unwrap_or(d.phase_agreement),
                phase_contrast_min: phase_contrast_min.unwrap_or(d.phase_contrast_min),
                phase_tolerance_floor: phase_tolerance_floor.unwrap_or(d.phase_tolerance_floor),
                phase_require_measurable: !allow_unmeasurable.unwrap_or(false),
                edge_fit_order: edge_fit_order.unwrap_or(d.edge_fit_order),
                edge_fit_slope: edge_fit_slope.unwrap_or(d.edge_fit_slope),
                edge_fit_min_count: edge_fit_min_count.unwrap_or(d.edge_fit_min_count),
                ..d
            };
            println!(
                "ε = {}{} / δ = {} / τ = {} / min_confidence = {} / θ = {} / 曲線 {} / 測れない候補を{} / 境界 {} 階 傾き {} 本数 {}\n",
                params.epsilon,
                if params.normalize_epsilon {
                    " (画像分散に対する割合)"
                } else {
                    ""
                },
                params.delta,
                params.tau,
                params.min_confidence,
                params.phase_tolerance,
                params.phase_agreement,
                if params.phase_require_measurable {
                    "棄却"
                } else {
                    "素通し"
                },
                params.edge_fit_order,
                params.edge_fit_slope,
                params.edge_fit_min_count,
            );
            let manifest = real::read(&dir)?;
            let outcomes = real::run(&dir, &manifest, &params)?;
            let path = out.unwrap_or_else(|| dir.join("results.csv"));
            let mut text = String::from(real::HEADER);
            text.push('\n');
            for o in &outcomes {
                text.push_str(&o.to_csv());
                text.push('\n');
            }
            pxsmith_io::atomic::write(&path, text.as_bytes())
                .with_context(|| format!("{} を書けない", path.display()))?;
            println!("{} 件を {} へ書いた\n", outcomes.len(), path.display());

            for o in &outcomes {
                let answer = match (o.scale_hat, o.phase_hat) {
                    (Some(s), Some(p)) => format!("s={s} 位相=({},{})", p.0, p.1),
                    _ => format!("棄却 ({})", o.error.clone().unwrap_or_default()),
                };
                println!(
                    "  {:<34} {:>4}x{:<4} {answer:<24} 信頼度 {:<7} {}",
                    o.file,
                    o.width,
                    o.height,
                    o.confidence
                        .map(|c| format!("{c:.3}"))
                        .unwrap_or_else(|| "-".to_string()),
                    o.verdict.as_str(),
                );
            }
            let count = |v: real::Verdict| outcomes.iter().filter(|o| o.verdict == v).count();
            use real::Verdict::*;
            println!(
                "\n  正例 (格子あり): 完全一致 {} / s のみ一致 {} / 誤答 {} / 誤棄却 {}",
                count(Exact),
                count(ScaleOnly),
                count(Wrong),
                count(Rejected),
            );
            println!(
                "  負例 (格子なし): 正しい棄却 {} / 答えを返した {}",
                count(CorrectReject),
                count(FalseAccept),
            );
            println!(
                "  人が見る (正解が分からない): {}\n\n  **率で語らないこと** — 20〜30 件では 1 件が 3〜5% 動く",
                count(Unknown),
            );
        }

        Command::Recon {
            dir,
            out,
            split,
            include_resized,
            real,
            peak_suppression,
            peak_floor,
        } => {
            // **境界の峰の拾い方を掃けるようにした** (D173) — D72 の «残る的» の
            // 2 番目で，初期値のまま 1 通りしか試していなかった
            let params = pxsmith_core::grid::GridParams {
                peak_suppression,
                peak_floor,
                ..pxsmith_core::grid::GridParams::default()
            };
            let records = if real {
                recon::run_real(&dir, &crate::real::read(&dir)?, &params)?
            } else {
                let manifest = dataset::read(&dir)?;
                let only = parse_split(&split)?;
                recon::run(&dir, &manifest, only, &params, include_resized)?
            };
            let path = out.unwrap_or_else(|| dir.join("recon.csv"));
            let mut text = String::from(recon::HEADER);
            text.push('\n');
            for r in &records {
                text.push_str(&r.to_csv());
                text.push('\n');
            }
            pxsmith_io::atomic::write(&path, text.as_bytes())?;
            report_recon(&records);
            println!("\n{} 行を {} へ書いた", records.len(), path.display());
        }

        Command::Replay {
            dir,
            csv,
            split,
            verify,
            gates,
            epsilon,
            tau,
            phase_tolerance,
            phase_agreement,
            phase_contrast_min,
            min_confidence,
            edge_order,
            edge_mode,
            edge_slope,
            edge_residual,
            edge_min_count,
            edge_min_coverage,
            edge_require_measurable,
            edge_rescue,
            edge_curve_slope,
            edge_curve_residual,
            edge_curve_min_count,
            edge_drop_residual,
            edge_drop_any_axis,
            curve_lambda,
            curve_axis,
            edge_stat,
            top,
        } => {
            let manifest = dataset::read(&dir)?;
            let only = parse_split(&split)?;
            let path = csv.unwrap_or_else(|| dir.join("recon.csv"));
            let cases = replay::load(&path, &manifest, only)?;
            println!("{} 件を {} から読んだ", cases.len(), path.display());

            let base = replay::Gates::default();
            println!(
                "\n== 既定の運転点 (再現) ==\n  {}",
                replay::score(&cases, &base).line()
            );
            if verify {
                report_replay_verify(&dir, &manifest, only, &cases, &base)?;
            }
            if gates {
                report_replay_gates(&cases, &base);
            }

            let modes: Vec<replay::EdgeMode> = edge_mode
                .iter()
                .map(|m| {
                    replay::EdgeMode::parse(m)
                        .with_context(|| format!("掛け方は and / or / or-drift: {m}"))
                })
                .collect::<Result<_>>()?;
            let rescues: Vec<replay::Rescue> = edge_rescue
                .iter()
                .map(|r| {
                    replay::Rescue::parse(r).with_context(|| {
                        format!("肩代わりは none / eps / recon / contrast を + でつなぐ: {r}")
                    })
                })
                .collect::<Result<_>>()?;
            let axes: Vec<replay::CurveAxis> = curve_axis
                .iter()
                .map(|a| {
                    replay::CurveAxis::parse(a)
                        .with_context(|| format!("軸のまとめ方は mean / max: {a}"))
                })
                .collect::<Result<_>>()?;

            let mut grid = vec![replay::Gates {
                edge_require_measurable,
                ..base
            }];
            grid = expand(&grid, &epsilon, |g, v| g.epsilon = v);
            grid = expand(&grid, &tau, |g, v| g.tau = v);
            grid = expand(&grid, &phase_tolerance, |g, v| g.phase_tolerance = v);
            grid = expand(&grid, &phase_agreement, |g, v| g.phase_agreement = v);
            grid = expand(&grid, &phase_contrast_min, |g, v| g.phase_contrast_min = v);
            grid = expand(&grid, &min_confidence, |g, v| g.min_confidence = v);
            grid = expand(&grid, &edge_order, |g, v| g.edge_order = v);
            grid = expand(&grid, &modes, |g, v| g.edge_mode = v);
            grid = expand(&grid, &edge_slope, |g, v| g.edge_slope = v);
            grid = expand(&grid, &edge_residual, |g, v| g.edge_residual = v);
            grid = expand(&grid, &edge_min_count, |g, v| g.edge_min_count = v);
            grid = expand(&grid, &edge_min_coverage, |g, v| g.edge_min_coverage = v);
            grid = expand(&grid, &rescues, |g, v| g.rescue = v);
            grid = expand(&grid, &edge_curve_slope, |g, v| g.edge_curve_slope = v);
            grid = expand(&grid, &edge_curve_residual, |g, v| {
                g.edge_curve_residual = v
            });
            grid = expand(&grid, &edge_curve_min_count, |g, v| {
                g.edge_curve_min_count = v
            });
            grid = expand(&grid, &edge_drop_residual, |g, v| {
                g.edge_drop_residual = (v >= 0.0).then_some(v);
                g.edge_drop_both_axes = !edge_drop_any_axis;
            });
            grid = expand(&grid, &curve_lambda, |g, v| g.curve_lambda = v);
            grid = expand(&grid, &axes, |g, v| g.curve_axis = v);
            let stats: Vec<replay::EdgeStat> = edge_stat
                .iter()
                .map(|a| {
                    replay::EdgeStat::parse(a)
                        .with_context(|| format!("測り方は rms / median / folded: {a}"))
                })
                .collect::<Result<_>>()?;
            grid = expand(&grid, &stats, |g, v| g.edge_stat = v);
            report_replay_sweep(&cases, &grid, top);
        }

        Command::Recover {
            dir,
            out,
            split,
            seeds,
        } => {
            let manifest = dataset::read(&dir)?;
            let only = parse_split(&split)?;
            let seeds = match seeds.filter(|p| p.exists()) {
                Some(p) => sprite::load_seeds(&p)?,
                None => Vec::new(),
            };
            let records = recover::run(&dir, &manifest, only, &seeds)?;
            let path = out.unwrap_or_else(|| dir.join("recover.csv"));
            let mut text = String::from(recover::HEADER);
            text.push('\n');
            for r in &records {
                text.push_str(&r.to_csv());
                text.push('\n');
            }
            pxsmith_io::atomic::write(&path, text.as_bytes())?;
            report_recover(&records);
            println!("\n{} 件を {} へ書いた", records.len(), path.display());
        }

        Command::Diagnose { dir, out } => {
            let manifest = real::read(&dir)?;
            let params = pxsmith_core::grid::GridParams::default();
            let records = diagnose::run(&dir, &manifest, &params)?;
            let path = out.unwrap_or_else(|| dir.join("diagnose.csv"));
            let mut text = String::from(diagnose::HEADER);
            text.push('\n');
            for r in &records {
                text.push_str(&r.to_csv());
                text.push('\n');
            }
            pxsmith_io::atomic::write(&path, text.as_bytes())?;
            report_diagnose(&records);
            println!("\n{} 件を {} へ書いた", records.len(), path.display());
        }

        Command::Lint {
            dir,
            dataset,
            grid_like_ratio,
            ..
        } if dataset && !grid_like_ratio.is_empty() => {
            let manifest = dataset::read(&dir)?;
            let ratios = grid_like_ratio;
            for ratio in ratios {
                let cfg = pxsmith_lint::LintConfig {
                    grid_like_ratio: ratio,
                    ..pxsmith_lint::LintConfig::default()
                };
                let out = lintcal::run_dataset(&dir, &manifest, &cfg, Some(Split::Validation));
                let count = |grid: bool| -> (usize, usize) {
                    let recs: Vec<_> = out
                        .iter()
                        .filter(|(g, _)| *g == grid)
                        .filter_map(|(_, r)| r.as_ref().ok())
                        .collect();
                    let fired = recs.iter().filter(|r| r.hits.contains_key(&2)).count();
                    (fired, recs.len())
                };
                let (fc, nc) = count(true);
                let (fb, nb) = count(false);
                println!(
                    "  格子らしさ {ratio:<7} ルール 2: きれいな拡大 {fc:>3}/{nc} ({:>5.1}%) ・崩れた格子 {fb:>3}/{nb} ({:>5.1}%)",
                    fc as f32 / nc.max(1) as f32 * 100.0,
                    fb as f32 / nb.max(1) as f32 * 100.0
                );
            }
        }

        Command::Lint {
            dir,
            min_lightness_delta,
            ..
        } if !min_lightness_delta.is_empty() => {
            for delta in min_lightness_delta {
                let cfg = pxsmith_lint::LintConfig {
                    min_lightness_delta: delta,
                    ..pxsmith_lint::LintConfig::default()
                };
                let (records, _) = lintcal::run(&dir, &cfg)?;
                let fired = records.iter().filter(|r| r.hits.contains_key(&11)).count();
                let total: usize = records.iter().filter_map(|r| r.hits.get(&11)).sum();
                println!(
                    "  明度差の下限 {delta:<7} ルール 11: {fired:>3}/{} 枚 ({:>5.1}%) ・違反 {total}",
                    records.len(),
                    fired as f32 / records.len().max(1) as f32 * 100.0
                );
            }
        }

        Command::Lint {
            dir,
            shadow_hue,
            negative,
            ..
        } if !shadow_hue.is_empty() => {
            println!("  色相差 (度)   良い絵で鳴る    負例 mono-* で鳴る");
            for gap in shadow_hue {
                let cfg = pxsmith_lint::LintConfig {
                    min_shadow_hue_gap: gap,
                    ..pxsmith_lint::LintConfig::default()
                };
                let (good, _) = lintcal::run(&dir, &cfg)?;
                let good_fired = good.iter().filter(|r| r.hits.contains_key(&6)).count();
                let (bad, _) = lintcal::run(&negative, &cfg)?;
                let mono: Vec<_> = bad.iter().filter(|r| r.file.starts_with("mono-")).collect();
                let bad_fired = mono.iter().filter(|r| r.hits.contains_key(&6)).count();
                println!(
                    "  {gap:<10}   {good_fired:>3} / {:<3}       {bad_fired:>3} / {}",
                    good.len(),
                    mono.len()
                );
            }
        }

        Command::Lint {
            dir,
            outline_interior,
            negative,
            ..
        } if !outline_interior.is_empty() => {
            // **正例と負例を同時に出す** — 片方だけ見て決めない (D70)
            println!("  内側の割合 x 重なりの占有 (下限)   良い絵で鳴る    負例 corner-* で鳴る");
            for max in outline_interior {
                for overlaps in [0.05f32, 0.06, 0.1] {
                    let cfg = pxsmith_lint::LintConfig {
                        max_outline_interior: max,
                        min_outline_overlap_share: overlaps,
                        ..pxsmith_lint::LintConfig::default()
                    };
                    let (good, _) = lintcal::run(&dir, &cfg)?;
                    let good_fired = good.iter().filter(|r| r.hits.contains_key(&4)).count();
                    let (bad, _) = lintcal::run(&negative, &cfg)?;
                    let corner: Vec<_> = bad
                        .iter()
                        .filter(|r| r.file.starts_with("corner-"))
                        .collect();
                    let bad_fired = corner.iter().filter(|r| r.hits.contains_key(&4)).count();
                    println!(
                        "  {max:<6} x {overlaps:<3}        {good_fired:>3} / {:<3}       {bad_fired:>3} / {}",
                        good.len(),
                        corner.len()
                    );
                }
            }
        }

        Command::Lint {
            dir,
            grid_like_ratio,
            ..
        } if !grid_like_ratio.is_empty() => {
            // 原寸のドット絵は «格子を名乗っていない» はずである — 閾値ごとに確かめる
            for ratio in grid_like_ratio {
                let cfg = pxsmith_lint::LintConfig {
                    grid_like_ratio: ratio,
                    ..pxsmith_lint::LintConfig::default()
                };
                let (records, _) = lintcal::run(&dir, &cfg)?;
                let fired = records.iter().filter(|r| r.hits.contains_key(&2)).count();
                println!(
                    "  格子らしさ {ratio:<7} ルール 2: 原寸のドット絵 {fired:>3}/{} ({:>5.1}%)",
                    records.len(),
                    fired as f32 / records.len().max(1) as f32 * 100.0
                );
            }
        }

        Command::Lint { dir, dataset, .. } if dataset => {
            let manifest = dataset::read(&dir)?;
            let cfg = pxsmith_lint::LintConfig::default();
            let out = lintcal::run_dataset(&dir, &manifest, &cfg, Some(Split::Validation));
            for (grid, label) in [
                (true, "整数の格子がある (鳴ってはいけない)"),
                (false, "格子が無い (鳴るべき)"),
            ] {
                let recs: Vec<_> = out
                    .iter()
                    .filter(|(g, r)| *g == grid && r.is_ok())
                    .filter_map(|(_, r)| r.as_ref().ok().cloned())
                    .collect();
                let skipped = out.iter().filter(|(g, r)| *g == grid && r.is_err()).count();
                println!(
                    "\n== {label}: {} 枚 (添字にできなかった {skipped} 枚) ==",
                    recs.len()
                );
                for (id, name, blocking, files, total) in lintcal::by_rule(&recs) {
                    if files == 0 {
                        continue;
                    }
                    let sev = if blocking { "blocking" } else { "advisory" };
                    let rate = files as f32 / recs.len().max(1) as f32 * 100.0;
                    println!(
                        "  {id:>2} {name:<22} {sev:<10} {files:>4} 枚 ({rate:>5.1}%) {total:>8}"
                    );
                }
            }
        }

        Command::Lint { dir, out, .. } => {
            let cfg = pxsmith_lint::LintConfig::default();
            let (records, skipped) = lintcal::run(&dir, &cfg)?;
            println!(
                "== lint を掛けた {} 枚 ({}) ==",
                records.len(),
                dir.display()
            );
            if !skipped.is_empty() {
                println!("  添字にできなかった {} 枚:", skipped.len());
                for (f, why) in skipped.iter().take(5) {
                    println!("    {f}: {why}");
                }
            }
            println!("\n  ルール                     深刻度      鳴った枚数   違反の総数");
            for (id, name, blocking, files, total) in lintcal::by_rule(&records) {
                let sev = if blocking { "blocking" } else { "advisory" };
                let rate = if records.is_empty() {
                    0.0
                } else {
                    files as f32 / records.len() as f32 * 100.0
                };
                println!("  {id:>2} {name:<22} {sev:<10} {files:>4} 枚 ({rate:>5.1}%) {total:>8}");
            }
            let clean = records.iter().filter(|r| r.hits.is_empty()).count();
            let no_blocking = records
                .iter()
                .filter(|r| {
                    !r.hits.keys().any(|id| {
                        pxsmith_lint::rule(*id)
                            .is_some_and(|x| matches!(x.severity, pxsmith_lint::Severity::Blocking))
                    })
                })
                .count();
            println!(
                "\n  1 件も鳴らない絵 {clean} 枚 / blocking が鳴らない絵 {no_blocking} 枚 (全 {} 枚)",
                records.len()
            );
            let path = out.unwrap_or_else(|| dir.join("lint.csv"));
            let mut text = String::from(lintcal::HEADER);
            text.push('\n');
            for r in &records {
                text.push_str(&r.to_csv(&cfg));
                text.push('\n');
            }
            pxsmith_io::atomic::write(&path, text.as_bytes())?;
            println!("  {} 行を {} へ書いた", records.len(), path.display());
        }

        Command::Pillow {
            seeds,
            negative,
            steps,
            threshold,
            out,
        } => {
            let (good, bad, shaded) = pillow::run(&seeds, Some(&negative), steps)?;
            println!("== ルール 13 の特徴量 rho = corr(距離場, 明度) ==");
            println!("\n  群                 件数    最小    5%    中央    95%    最大");
            for (name, set) in [
                ("正例 (良い絵)", &good),
                ("負例 (pillow)", &bad),
                ("pxsmith shade の出力", &shaded),
            ] {
                match pillow::quantiles(set) {
                    Some((lo, p5, med, p95, hi)) => println!(
                        "  {name:<18} {:>4}  {lo:>6.3} {p5:>6.3} {med:>6.3} {p95:>6.3} {hi:>6.3}",
                        set.len()
                    ),
                    None => println!("  {name:<18} {:>4}  (測れる件が無い)", set.len()),
                }
            }

            // **`pxsmith shade` は光源プリセットごとに分けて見る** — 向きが変われば相関も動く
            println!("\n  pxsmith shade の内訳 (プリセットごと)");
            for preset in pxsmith_core::ramp::LightPreset::ALL {
                let set: Vec<_> = shaded
                    .iter()
                    .filter(|r| r.preset == Some(preset.as_str()))
                    .cloned()
                    .collect();
                if let Some((lo, p5, med, p95, hi)) = pillow::quantiles(&set) {
                    println!(
                        "  {:<18} {:>4}  {lo:>6.3} {p5:>6.3} {med:>6.3} {p95:>6.3} {hi:>6.3}",
                        preset.as_str(),
                        set.len()
                    );
                }
            }

            // **自己整合性** — 陰影を付けた 320 通りに lint を掛けて blocking を数える
            let blocking: usize = shaded.iter().map(|r| r.blocking).sum();
            println!(
                "\n  自己整合性: pxsmith shade の出力 {} 件で blocking {} 件",
                shaded.len(),
                blocking
            );
            for (id, n) in pillow::rule_hits(&shaded) {
                let r = pxsmith_lint::rule(id);
                let name = r.map(|r| r.name).unwrap_or("?");
                let sev =
                    if r.is_some_and(|r| matches!(r.severity, pxsmith_lint::Severity::Blocking)) {
                        "blocking"
                    } else {
                        "advisory"
                    };
                println!("    ルール {id:>2} {name} ({sev}) — {n} 件");
            }

            let grid = or_default(
                threshold,
                &[0.3, 0.4, 0.5, 0.6, 0.7, 0.75, 0.8, 0.85, 0.9, 0.95],
            );
            println!("\n  閾値    正例で誤爆   負例を捕捉   **pxsmith shade が鳴る**");
            for t in grid {
                println!(
                    "  {t:>5.2}   {:>7.1}%    {:>7.1}%    {:>10.1}%",
                    pillow::rate_at(&good, t) * 100.0,
                    pillow::rate_at(&bad, t) * 100.0,
                    pillow::rate_at(&shaded, t) * 100.0
                );
            }

            let path = out.unwrap_or_else(|| seeds.join("pillow.csv"));
            let mut text = String::from(pillow::HEADER);
            text.push('\n');
            for (group, set) in [("good", &good), ("bad", &bad), ("shade", &shaded)] {
                for r in set {
                    text.push_str(&pillow::to_csv(group, r));
                    text.push('\n');
                }
            }
            pxsmith_io::atomic::write(&path, text.as_bytes())?;
            println!(
                "\n  {} 行を {} へ書いた",
                good.len() + bad.len() + shaded.len(),
                path.display()
            );
        }

        Command::Jaggy {
            dir,
            max_move,
            out,
            top,
            apply,
        } => {
            let (records, skipped) = jaggycal::run(&dir, max_move, apply)?;
            let runs: usize = records.iter().map(|r| r.runs).sum();
            let jaggies: usize = records.iter().map(|r| r.jaggies).sum();
            let fixable: usize = records.iter().map(|r| r.fixable).sum();
            let dirty = records.iter().filter(|r| r.jaggies > 0).count();
            println!(
                "== ジャギー検出を {} 枚に掛けた ({}) ==",
                records.len(),
                dir.display()
            );
            if !skipped.is_empty() {
                println!("  測れなかった {} 枚 (例: {})", skipped.len(), skipped[0].1);
            }
            println!(
                "\n  ラン {runs} 本中 {jaggies} 件 ({:.2}%) ・移動上限 {max_move} に収まる {fixable} 件",
                if runs == 0 {
                    0.0
                } else {
                    jaggies as f32 / runs as f32 * 100.0
                }
            );
            println!("  **1 件でも鳴った絵 {dirty} / {} 枚**", records.len());

            let mut by_delta: std::collections::BTreeMap<i32, usize> = Default::default();
            for r in &records {
                for (d, n) in &r.by_delta {
                    *by_delta.entry(*d).or_default() += n;
                }
            }
            println!("\n  δ (目標 − 長さ) ごとの件数");
            for (d, n) in &by_delta {
                println!("    δ = {d:>2}  {n:>5} 件");
            }

            let mut by_shape: std::collections::BTreeMap<&'static str, usize> = Default::default();
            for r in &records {
                for (k, n) in &r.by_shape {
                    *by_shape.entry(k).or_default() += n;
                }
            }
            println!("\n  谷の周りの形 (**理想の単谷形の底を «ジャギー» と呼んでいないか**)");
            for (k, n) in &by_shape {
                println!("    {k:<24} {n:>5} 件");
            }

            let mut worst = records.clone();
            worst.sort_by_key(|r| (std::cmp::Reverse(r.jaggies), r.file.clone()));
            println!("\n  件数の多い絵");
            for r in worst.iter().take(top) {
                println!(
                    "    {:<34} ラン {:>5} 中 {:>4} 件 (直せる {})",
                    r.file, r.runs, r.jaggies, r.fixable
                );
            }

            if apply {
                let applied: Vec<_> = records.iter().filter_map(|r| r.applied.as_ref()).collect();
                let moved: usize = applied.iter().map(|a| a.moved).sum();
                let remaining: usize = applied.iter().map(|a| a.remaining).sum();
                let touched = applied.iter().filter(|a| a.moved > 0).count();
                let not_converged = applied.iter().filter(|a| a.second_pass_moved > 0).count();
                let area_over = applied
                    .iter()
                    .filter(|a| a.area_delta > a.moved as i64)
                    .count();
                let worse = applied
                    .iter()
                    .filter(|a| a.blocking_after > a.blocking_before)
                    .count();
                let max_passes = applied.iter().map(|a| a.passes).max().unwrap_or(0);
                println!("\n  == pxsmith smooth を実際に掛けた ==");
                println!(
                    "    動かした画素 {moved} ・触った絵 {touched} / {} 枚",
                    applied.len()
                );
                println!(
                    "    残ったジャギー {remaining} (元 {})",
                    records.iter().map(|r| r.jaggies).sum::<usize>()
                );
                println!("    最大の巡回数 {max_passes}");
                println!("    **収束していない絵 {not_converged} 枚** (2 回目で動く)");
                println!("    **面積が動かした画素数を超えて変わった絵 {area_over} 枚**");
                println!("    **lint の blocking が増えた絵 {worse} 枚**");
            }

            let path = out.unwrap_or_else(|| dir.join("jaggy.csv"));
            let mut text = String::from(jaggycal::HEADER);
            text.push('\n');
            for r in &records {
                text.push_str(&r.to_csv());
                text.push('\n');
            }
            pxsmith_io::atomic::write(&path, text.as_bytes())?;
            println!("\n  {} 行を {} へ書いた", records.len(), path.display());
        }

        Command::Mixel {
            dir,
            sheets,
            uniformity,
            out,
            dump,
        } => {
            let seeds = sprite::load_seeds(&dir)?;
            let params = pxsmith_core::grid::GridParams::default();
            let s = mixel::run(&seeds, sheets, uniformity, &params);
            println!("== 局所格子推定の窓を真値のある場面で測った ==");
            println!(
                "  種 {} 枚 ・場面 {} 枚 ・窓 {} 通り ・一致率の閾値 {:.2}",
                seeds.len(),
                s.cases.len(),
                mixel::WINDOWS.len(),
                uniformity
            );

            println!("\n  **一様な絵 (どこでも格子 s)** — 鳴ってはいけない ・最頻は s であるべき");
            println!("    倍率 \\ 窓 {:>50}", "");
            print!("    {:>4}", "s");
            for w in mixel::WINDOWS {
                print!(" {w:>10}");
            }
            println!();
            let by_scale = mixel::by_scale(&s);
            for scale in mixel::SCALES {
                print!("    {scale:>4}");
                for w in mixel::WINDOWS {
                    match by_scale.get(&(scale, w)) {
                        Some(t) => print!(" {:>4}/{:<5}", t.modal_ok, t.sheets),
                        None => print!(" {:>10}", "-"),
                    }
                }
                println!("   最頻が正解 / 枚数");
            }
            print!("    {:>4}", "誤爆");
            for w in mixel::WINDOWS {
                let (fired, sheets) = mixel::SCALES
                    .iter()
                    .filter_map(|s| by_scale.get(&(*s, w)))
                    .fold((0, 0), |(a, b), t| (a + t.fired, b + t.sheets));
                print!(" {fired:>4}/{sheets:<5}");
            }
            println!("   **一様なのに鳴った / 枚数**");

            println!("\n  **窓の下限を 1 画素刻みで掃いた** — 最頻が全枚数で正解になる最小の窓");
            println!(
                "    {:>4} {:>10} {:>10} {:>10}",
                "s", "帯 4 (既定)", "帯 2", "帯なし"
            );
            let laws = mixel::law(&seeds, sheets, &params);
            for scale in mixel::SCALES {
                print!("    {scale:>4}");
                for bands in [params.phase_bands, 2, 0] {
                    let l = laws
                        .iter()
                        .find(|l| l.scale == scale && l.bands == bands)
                        .and_then(|l| l.min_window);
                    match l {
                        Some(w) => print!(" {:>7} ({:.1}s)", w, w as f32 / scale as f32),
                        None => print!(" {:>10}", "届かない"),
                    }
                }
                println!();
            }

            println!("\n  **ミクセル (半々)** — 鳴るべき");
            println!(
                "    {:>4} {:>12} {:>12} {:>14}",
                "窓", "鳴った/枚数", "鳴りうる", "捕捉 (鳴りうる中)"
            );
            let t = mixel::tally(&s);
            for w in mixel::WINDOWS {
                if let Some(x) = t.get(&("ミクセル (半々)", w)) {
                    println!(
                        "    {w:>4} {:>7}/{:<4} {:>7}/{:<4} {:>10.1}%",
                        x.fired,
                        x.sheets,
                        x.can_fire,
                        x.sheets,
                        if x.can_fire == 0 {
                            0.0
                        } else {
                            x.caught as f32 * 100.0 / x.can_fire as f32
                        }
                    );
                }
            }

            println!("\n  **分解能** — 少数派 (2 倍の領域) が画布の何割なら捕まるか");
            print!("    {:>6}", "割合");
            for w in mixel::WINDOWS {
                print!(" {w:>8}");
            }
            println!();
            let by_frac = mixel::by_fraction(&s);
            let mut fracs: Vec<u32> = by_frac.keys().map(|(p, _)| *p).collect();
            fracs.sort_unstable();
            fracs.dedup();
            for p in fracs {
                print!("    {:>5.1}%", p as f32 / 10.0);
                for w in mixel::WINDOWS {
                    match by_frac.get(&(p, w)) {
                        Some(x) => print!(" {:>3}/{:<4}", x.fired, x.sheets),
                        None => print!(" {:>8}", "-"),
                    }
                }
                println!();
            }

            println!("\n  **L0 の画布 (16 〜 48 画素)** — lint ルール 9 の持ち場");
            println!(
                "    {:>4} {:>16} {:>16} {:>16}",
                "窓", "等倍で鳴った", "ミクセルで鳴った", "窓が 1 つだけ"
            );
            for w in mixel::WINDOWS {
                let clean = t.get(&("L0 の等倍 (鳴ってはいけない)", w));
                let mix = t.get(&("L0 にミクセル (鳴るべき)", w));
                println!(
                    "    {w:>4} {:>11}/{:<4} {:>11}/{:<4} {:>11}/{:<4}",
                    clean.map(|x| x.fired).unwrap_or(0),
                    clean.map(|x| x.sheets).unwrap_or(0),
                    mix.map(|x| x.fired).unwrap_or(0),
                    mix.map(|x| x.sheets).unwrap_or(0),
                    mix.map(|x| x.single_window).unwrap_or(0),
                    mix.map(|x| x.sheets).unwrap_or(0),
                );
            }

            println!(
                "\n  **実素材 {} 枚にそのまま掛けた** (`pxsmith lint` が受け取る形)",
                seeds.len()
            );
            println!(
                "    {:>4} {:>16} {:>18} {:>10}",
                "窓", "升が 2 つ以上", "**投票が 2 つ以上**", "鳴った"
            );
            for c in mixel::corpus(&seeds, uniformity, &params) {
                println!(
                    "    {:>4} {:>11}/{:<4} {:>13}/{:<4} {:>5}/{:<4}",
                    c.window,
                    c.cells_two_or_more,
                    c.sheets,
                    c.two_or_more,
                    c.sheets,
                    c.fired,
                    c.sheets
                );
            }

            if let Some(dir) = dump {
                println!("\n  端から端まで通すための場面を書き出した:");
                for name in mixel::dump(&seeds, &dir)? {
                    println!("    {name}");
                }
            }

            if let Some(path) = out {
                let mut text = String::from(mixel::HEADER);
                text.push('\n');
                for row in mixel::rows(&s) {
                    text.push_str(&row);
                    text.push('\n');
                }
                pxsmith_io::atomic::write(&path, text.as_bytes())?;
                println!("\n  {} 行を {} へ書いた", s.cases.len(), path.display());
            }
        }

        Command::MixelExact { dir, sheets, max_k } => {
            let seeds = sprite::load_seeds(&dir)?;
            let s = exactgrid::run(&seeds, sheets, max_k);
            println!("== ルール 9 を «厳密なブロック判定» に替えたときの上限 ==");
            println!(
                "  種 {} 枚 ・場面 {} 枚 ・窓 {} 通り ・升の上限 {max_k}",
                seeds.len(),
                s.cases.len(),
                exactgrid::WINDOWS.len()
            );
            println!(
                "  **平らな窓は «測れなかった» に数える** — «格子 1» と混ぜると\n\
                 \u{3000}\u{3000}実素材の背景がすべて 1 に投票して 2 倍の絵が誤爆する"
            );

            println!("\n  合成した場面 (正解あり)");
            println!(
                "    {:>4} {:>16} {:>16}",
                "窓", "混在で鳴った", "一様で鳴った (誤爆)"
            );
            for (window, (hit, mix, fp, uni)) in s.tally() {
                println!("    {window:>4} {hit:>10} / {mix:<3} {fp:>10} / {uni:<3}");
            }

            println!("\n  **群ごとに全部並べる** — 群名を手で当てにいくと取り違える");
            for window in exactgrid::WINDOWS {
                println!("    窓 {window}");
                for ((group, is_mixel), (hit, n)) in s.by_group(window) {
                    println!(
                        "      {group:<28} {}: {hit:>3} / {n:<3} 枚で鳴った",
                        if is_mixel {
                            "鳴るべき"
                        } else {
                            "鳴ってはいけない"
                        }
                    );
                }
            }

            // **鳴った場面の中身を出す** — «2 通りあった» だけでは何と何かが読めない
            if let Some(c) = s
                .cases
                .iter()
                .find(|c| c.truth.is_mixel() && c.obs.iter().any(|o| o.window == 16 && o.fires))
                && let Some(o) = c.obs.iter().find(|o| o.window == 16)
            {
                println!(
                    "\n  鳴った例 (窓 16): {} — 升ごとの窓数 {:?} ・平らな窓 {}",
                    c.name, o.by_k, o.flat
                );
            }

            println!("\n  実素材 (等倍のドット絵．**鳴ったら全件が誤検出**)");
            let rows = exactgrid::corpus(&seeds, max_k);
            for window in exactgrid::WINDOWS {
                let here: Vec<_> = rows.iter().filter(|(_, o)| o.window == window).collect();
                if here.is_empty() {
                    continue;
                }
                let fired = here.iter().filter(|(_, o)| o.fires).count();
                let pinned: usize = here.iter().map(|(_, o)| o.pinned).sum();
                let flat: usize = here.iter().map(|(_, o)| o.flat).sum();
                // **窓が 1 つしか並ばない絵は «鳴らなかった» ではなく «検査していない»**
                // (D164 と同じ形．窓が 2 つ無ければ格子は 2 通りになりようがない)
                let checkable = here.iter().filter(|(_, o)| o.pinned >= 2).count();
                println!(
                    "    窓 {window:>3}: {} 枚中**検査できたのは {checkable} 枚** \
                     (窓が 2 つ以上並ぶ絵) ・決まった窓 {pinned} ・平らな窓 {flat} ・\
                     **鳴った {fired} 枚**",
                    here.len()
                );
            }
        }

        Command::JaggyTruth { max_move, out } => {
            let s = jaggytruth::run(max_move);
            println!("== ジャギー検出に真値のある場面を掛けた ==");
            println!(
                "  **清書はすべて «凹凸が幾何で決まる» 絵なので，出た検出は全件が誤検出である**"
            );

            println!("\n  清書 (鳴ってはいけない)");
            for (kind, (sheets, runs, jaggies)) in jaggytruth::by_kind(&s) {
                println!(
                    "    {kind:<4} {sheets:>3} 枚 ・ラン {runs:>5} 本 ・**検出 {jaggies} 件** ({:.2}%)",
                    if runs == 0 {
                        0.0
                    } else {
                        jaggies as f32 * 100.0 / runs as f32
                    }
                );
            }
            println!(
                "    合計   {:>3} 枚 ・ラン {:>5} 本 ・**検出 {} 件** ・鳴った絵 {} 枚",
                s.clean.len(),
                s.clean_runs(),
                s.clean_jaggies(),
                s.clean_sheets_firing()
            );

            println!(
                "\n  **pxsmith smooth が清書を書き換えた画素 {} 個 ・壊した絵 {} / {} 枚**",
                s.clean_moved(),
                s.clean_sheets_damaged(),
                s.clean.len()
            );
            println!(
                "    鳴るだけなら助言で済むが，`smooth` は画素を動かす —\n\
                 \u{3000}\u{3000}**正しく描いた線が書き換えられている**",
            );

            println!("\n  対照: 段の境目を 1 画素手前へ動かした (崩した場所で鳴るべき)");
            println!(
                "    負例 {} 件のうち**実際に谷ができたのは {} 件**",
                s.defects.len(),
                s.defects_with_valley()
            );
            println!(
                "    **{} / {} 件で捕捉 ({:.1}%)**",
                s.caught(),
                s.defects_with_valley(),
                if s.defects_with_valley() == 0 {
                    0.0
                } else {
                    s.caught() as f32 * 100.0 / s.defects_with_valley() as f32
                }
            );

            println!("\n  比較: 縁の外へ 1 画素出っ張らせた (**谷にならない形**)");
            println!(
                "    {} 件のうち谷ができたのは {} 件 ・捕捉 {} 件",
                s.bumps.len(),
                s.bumps_with_valley(),
                s.bumps_caught()
            );
            println!(
                "    **出っ張りは向きの反転を作るので区間が切れ，谷が残らない** —\n\
                 \u{3000}\u{3000}谷を数える規則では原理的に拾えない (負例に使ってはいけない)"
            );

            if s.clean_jaggies() > 0 {
                println!("\n  **鳴った清書** (これが誤検出の中身である)");
                for c in s.clean.iter().filter(|c| c.jaggies > 0) {
                    println!(
                        "    {:<14} ラン {:>4} 中 {:>3} 件 ・smooth が動かした {} 画素",
                        c.name, c.runs, c.jaggies, c.moved
                    );
                }
            }
            let missed: Vec<_> = s
                .defects
                .iter()
                .filter(|b| b.has_valley && !b.caught)
                .collect();
            if !missed.is_empty() {
                println!("\n  **見逃した崩し** ({} 件．谷はできている)", missed.len());
                for b in missed.iter().take(12) {
                    println!("    {}", b.name);
                }
            }

            println!("\n  **谷の形は偽と真で分かれるか** (D168．偽 = 清書 ・真 = 崩した場所)");
            println!("    {:>6} {:>12} {:>12}", "深さ", "偽 (清書)", "真 (崩し)");
            let depths: std::collections::BTreeSet<i32> = s
                .clean_depths
                .keys()
                .chain(s.defect_depths.keys())
                .copied()
                .collect();
            for d in depths {
                println!(
                    "    {d:>6} {:>12} {:>12}",
                    s.clean_depths.get(&d).copied().unwrap_or(0),
                    s.defect_depths.get(&d).copied().unwrap_or(0)
                );
            }
            println!(
                "\n    {:>6} {:>12} {:>12}",
                "両隣の差", "偽 (清書)", "真 (崩し)"
            );
            let gaps: std::collections::BTreeSet<u32> = s
                .clean_neighbour_gap
                .keys()
                .chain(s.defect_neighbour_gap.keys())
                .copied()
                .collect();
            for g in gaps {
                println!(
                    "    {g:>6} {:>12} {:>12}",
                    s.clean_neighbour_gap.get(&g).copied().unwrap_or(0),
                    s.defect_neighbour_gap.get(&g).copied().unwrap_or(0)
                );
            }

            if let Some(path) = out {
                let mut text = String::from(jaggytruth::HEADER);
                text.push('\n');
                for c in &s.clean {
                    text.push_str(&format!("{},{},{},{}\n", c.name, c.kind, c.runs, c.jaggies));
                }
                pxsmith_io::atomic::write(&path, text.as_bytes())?;
                println!("\n  {} 行を {} へ書いた", s.clean.len(), path.display());
            }
        }

        Command::JaggySeam { max_move, dir, out } => {
            let real = if dir.is_dir() {
                Some(dir.as_path())
            } else {
                None
            };
            let s = jaggyseam::run(max_move, real)?;
            println!("== 継ぎ目の谷を許したときの «上限» ==");
            println!(
                "  **窓の取り方を全部試して両側が直線になれば継ぎ目とみなす** —\n\
                 \u{3000}\u{3000}認識器が達しうる最も緩い側である．ここで落ちるものは書いても守れない"
            );

            println!("\n  いま `pxsmith smooth` が動かしているもの (D169 の後)");
            for (kind, (sheets, movable, moved)) in jaggyseam::by_kind(&s) {
                println!(
                    "    {kind:<20} {sheets:>3} 枚 ・動かす谷 {movable:>3} ・動かした画素 {moved:>3}"
                );
            }
            println!(
                "    合計                 {:>3} 枚 ・動かす谷 {:>3} ・動かした画素 {:>3} ・壊した絵 {} 枚",
                s.clean.len(),
                s.clean_movable(),
                s.clean_moved(),
                s.clean_sheets_damaged()
            );

            if real.is_some() {
                println!(
                    "    実素材 {} 枚 ・動かす谷 {}",
                    s.real_files,
                    s.real_movable()
                );
            }
            println!(
                "\n  負例 {} 件のうち谷ができたのは {} 件 ・**`smooth` が直せたのは {} 件**",
                s.defects.len(),
                s.defects_with_valley(),
                s.repaired()
            );
            println!(
                "    (検出の捕捉は例外を入れても定義上動かない — 動くのは «直せたか» の側である)"
            );
            println!(
                "    崩した場所に谷があるのは {} 件 ・**D169 の直線の例外で動かせなくなった {} 件**",
                s.with_valley_near(),
                s.silenced_by_d169()
            );
            println!(
                "    動かす谷があるのに直らなかった {} 件 (当てる候補が無い) ・\n\
                 \u{3000}\u{3000}直ったのに近くに動かす谷が無い {} 件 (**数え上げが挙動を説明できていない分**)",
                s.movable_but_unrepaired(),
                s.repaired_unexplained()
            );
            println!(
                "    **1 か所崩すと巻き添えが出る**: 崩した場所で動かした {} 画素に対し，\n\
                 \u{3000}\u{3000}崩していない場所で **{} 画素** — 例外は区間まるごとに掛かるので，\n\
                 \u{3000}\u{3000}1 か所崩れると同じ縁の正しい部分まで書き換えの対象に戻る",
                s.moved_here(),
                s.moved_elsewhere()
            );

            println!("\n  **上限** — 継ぎ目に見える谷を全部許したら");
            println!(
                "    {:<20} {:>4} {:>12} {:>10} {:>12} {:>12}",
                "谷の配り方",
                "本数",
                "清書で守れる",
                "残る絵",
                "**直せなくなる**",
                "実素材で守れる"
            );
            for share in jaggyseam::SHARES {
                for min_runs in jaggyseam::MIN_RUNS {
                    let saved = s.clean_seam(share, min_runs);
                    let lost = s.lost(share, min_runs);
                    println!(
                        "    {:<20} {:>4} {:>8} / {:<3} {:>8} 枚 {:>8} / {:<3} {:>8} / {:<3}",
                        share.label(),
                        min_runs,
                        saved,
                        s.clean_movable(),
                        s.clean_sheets_left(share, min_runs),
                        lost,
                        s.repaired(),
                        s.real_seam(share, min_runs),
                        s.real_movable()
                    );
                }
            }

            // **一番よく効く取り方の中身を出す** — 表の 1 行を人が確かめられるように
            let best = jaggyseam::SHARES
                .iter()
                .flat_map(|&sh| jaggyseam::MIN_RUNS.iter().map(move |&m| (sh, m)))
                .filter(|&(sh, m)| s.lost(sh, m) == 0)
                .max_by_key(|&(sh, m)| s.clean_seam(sh, m));
            if let Some((share, min_runs)) = best {
                println!(
                    "\n  **直せなくなる件が 0 のまま一番効く取り方**: {} ・{} 本 — 清書 {} / {} 谷",
                    share.label(),
                    min_runs,
                    s.clean_seam(share, min_runs),
                    s.clean_movable()
                );
            }
            // **表の «一番効く行» の中身を出す** — 数字だけでは確かめようがない
            {
                let (share, min_runs) = (jaggyseam::Share::Right, 2usize);
                let names = s.lost_names(share, min_runs);
                if !names.is_empty() {
                    println!(
                        "\n  {} ・{} 本 で直せなくなる負例: {}",
                        share.label(),
                        min_runs,
                        names.join(" ")
                    );
                }
                let left = s.left_names(share, min_runs);
                println!(
                    "  {} ・{} 本 でも残る清書 ({} 枚): {}",
                    share.label(),
                    min_runs,
                    left.len(),
                    left.join(" ")
                );
            }
            if s.repaired_unexplained() > 0 {
                println!(
                    "  数え上げが説明できていない負例: {}",
                    s.unexplained_names().join(" ")
                );
            }

            if let Some(path) = out {
                let mut text = String::from(
                    "share,min_runs,clean_saved,clean_movable,sheets_left,lost,repaired,real_saved,real_movable\n",
                );
                for share in jaggyseam::SHARES {
                    for min_runs in jaggyseam::MIN_RUNS {
                        text.push_str(&format!(
                            "{},{},{},{},{},{},{},{},{}\n",
                            share.label(),
                            min_runs,
                            s.clean_seam(share, min_runs),
                            s.clean_movable(),
                            s.clean_sheets_left(share, min_runs),
                            s.lost(share, min_runs),
                            s.repaired(),
                            s.real_seam(share, min_runs),
                            s.real_movable()
                        ));
                    }
                }
                pxsmith_io::atomic::write(&path, text.as_bytes())?;
                println!("\n  {} へ書いた", path.display());
            }
        }

        Command::Aa { dir, outline } => {
            let opts = pxsmith_core::aa::AaAddOptions {
                include_outline: outline,
                ..pxsmith_core::aa::AaAddOptions::default()
            };
            let cfg = pxsmith_lint::LintConfig::default();
            let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")))
                .collect();
            files.sort();

            let (mut n, mut painted, mut added, mut touched) = (0usize, 0usize, 0usize, 0usize);
            let (mut worse, mut not_idempotent, mut silhouette_moved) = (0usize, 0usize, 0usize);
            let mut later: std::collections::BTreeMap<usize, usize> = Default::default();
            let mut over_256 = 0usize;
            // **ルール 14 (AA 過多) の分母**．中間色の画素 / 不透明画素を，掛ける前と
            // 掛けた後の両方で取る — 閾値は «自分の出力が鳴らない» ところに置く
            let (mut ratio_before, mut ratio_after) = (Vec::new(), Vec::new());
            for path in &files {
                let Ok(img) = pxsmith_io::png::read_rgba(path) else {
                    continue;
                };
                let Ok((canvas, palette)) = lintcal::index_exactly(&img) else {
                    continue;
                };
                n += 1;
                ratio_before.push((name_of(path), aa_ratio(&canvas, &palette)));
                let before_blocking = {
                    let mut r = pxsmith_lint::rules::lint_palette(&palette, &cfg);
                    r.extend(pxsmith_lint::lint_canvas(&canvas, &palette, &cfg));
                    r.blocking().count()
                };
                let silhouette = canvas.mask_of(canvas.transparent().unwrap_or(255));

                let (mut c, mut p) = (canvas.clone(), palette.clone());
                let Ok(report) = pxsmith_core::aa::add_antialiasing(&mut c, &mut p, &opts) else {
                    over_256 += 1;
                    continue;
                };
                painted += report.painted;
                added += report.added_colors;
                if report.painted > 0 {
                    touched += 1;
                }
                let after_blocking = {
                    let mut r = pxsmith_lint::rules::lint_palette(&p, &cfg);
                    r.extend(pxsmith_lint::lint_canvas(&c, &p, &cfg));
                    r.blocking().count()
                };
                if after_blocking > before_blocking {
                    worse += 1;
                }
                if c.mask_of(c.transparent().unwrap_or(255)) != silhouette {
                    silhouette_moved += 1;
                }
                ratio_after.push((name_of(path), aa_ratio(&c, &p)));
                let (mut again_c, mut again_p) = (c.clone(), p.clone());
                for pass in 1..4usize {
                    let Ok(again) =
                        pxsmith_core::aa::add_antialiasing(&mut again_c, &mut again_p, &opts)
                    else {
                        break;
                    };
                    if again.painted == 0 {
                        break;
                    }
                    if pass == 1 {
                        not_idempotent += 1;
                    }
                    *later.entry(pass + 1).or_insert(0) += again.painted;
                }
            }
            println!("== pxsmith aa を良い絵 {n} 枚に掛けた ==");
            println!("  置いた画素 {painted} ・作った色 {added} ・触った絵 {touched} 枚");
            println!("  **2 回目で塗る絵 {not_idempotent} 枚**");
            for (pass, n) in &later {
                println!("    {pass} 巡目に塗った画素 {n}");
            }
            println!("  **シルエットが動いた絵 {silhouette_moved} 枚**");
            println!("  **lint の blocking が増えた絵 {worse} 枚**");
            if over_256 > 0 {
                println!("  色数が 256 を超えて掛けられなかった絵 {over_256} 枚");
            }

            // **`pxsmith shade` の出力も測る (第 3 群)．**
            // 陰影の «段» は端の 2 色の間にあるので，そのまま中間色として数えられる —
            // 自分の理論どおりの出力が自分の検査に落ちていないかを見る (D58 ・D77)
            {
                use pxsmith_core::palette::ChromaCurve;
                use pxsmith_core::ramp::{LightPreset, build_lighting};
                use pxsmith_core::shade::{ShadeOptions, shade_to_canvas};
                let mut shaded: Vec<f32> = Vec::new();
                for path in &files {
                    let Ok(img) = pxsmith_io::png::read_rgba(path) else {
                        continue;
                    };
                    let mut mask = pxsmith_core::geom::Mask::new(img.width(), img.height());
                    for p in mask.bounds().iter() {
                        if img.get(p.x, p.y).is_some_and(|c| c.a != 0) {
                            mask.set(p, true);
                        }
                    }
                    if mask.count() < 64 {
                        continue;
                    }
                    let base = pxsmith_core::color::Rgba8::rgb(0x8a, 0x6a, 0x4a);
                    for preset in LightPreset::ALL {
                        let Ok((palette, model)) =
                            build_lighting(base, preset, 5, ChromaCurve::PeakMiddle)
                        else {
                            continue;
                        };
                        if let Ok((canvas, palette)) = shade_to_canvas(
                            &mask,
                            preset.default_source(),
                            &model,
                            &palette,
                            ShadeOptions::default(),
                        ) {
                            shaded.push(aa_ratio(&canvas, &palette).0);
                        }
                    }
                }
                shaded.sort_by(f32::total_cmp);
                if !shaded.is_empty() {
                    let at = |q: f32| shaded[((shaded.len() - 1) as f32 * q).round() as usize];
                    println!(
                        "\n  **pxsmith shade の出力** {} 件  中央 {:.4} ・90% {:.4} ・95% {:.4} ・最大 {:.4}",
                        shaded.len(),
                        at(0.5),
                        at(0.9),
                        at(0.95),
                        shaded[shaded.len() - 1]
                    );
                }
            }

            // ルール 14 の閾値を決めるための分布
            println!("\n  **中間色の割合 (不透明画素に対する)** — ルール 14 の特徴量");
            for (label, v) in [("掛ける前", &ratio_before), ("掛けた後", &ratio_after)] {
                let mut r: Vec<f32> = v.iter().map(|(_, x)| x.0).collect();
                r.sort_by(f32::total_cmp);
                if r.is_empty() {
                    continue;
                }
                let at = |q: f32| r[((r.len() - 1) as f32 * q).round() as usize];
                println!(
                    "    {label}  中央 {:.4} ・90% {:.4} ・95% {:.4} ・最大 {:.4}",
                    at(0.5),
                    at(0.9),
                    at(0.95),
                    r[r.len() - 1]
                );
            }
            let mut worst: Vec<&(String, (f32, usize))> = ratio_after.iter().collect();
            worst.sort_by(|a, b| b.1.0.total_cmp(&a.1.0));
            for (file, r) in worst.iter().take(200) {
                println!(
                    "      掛けた後の上位  {file}  割合 {:.4} ・中間色 {} 色",
                    r.0, r.1
                );
            }
            let mut cols: Vec<usize> = ratio_before.iter().map(|(_, r)| r.1).collect();
            cols.sort_unstable();
            if !cols.is_empty() {
                let at = |q: f32| cols[((cols.len() - 1) as f32 * q).round() as usize];
                println!(
                    "    中間色の色数 (掛ける前)  中央 {} ・90% {} ・95% {} ・最大 {}",
                    at(0.5),
                    at(0.9),
                    at(0.95),
                    cols[cols.len() - 1]
                );
            }
        }

        Command::Ao {
            seeds,
            threshold,
            passes,
            out,
        } => {
            let records = aocal::run(&seeds, &threshold, &passes)?;
            println!("== 環境遮蔽 (`pxsmith shade --ao`) を 3 群に掛けた ==");
            println!("\n  均し  閾値    群      件数   遮蔽の割合 (中央 / 最大)   ひとりぼっち");
            for (group, passes, t, n, median, max, scattered) in aocal::summarise(&records) {
                println!(
                    "  {passes:<4}  {t:<6}  {group:<6}  {n:>4}   {:>6.2}% / {:>6.2}%        {scattered:>5}",
                    median * 100.0,
                    max * 100.0
                );
            }
            if let Some(path) = out {
                let mut text = String::from(aocal::HEADER);
                text.push('\n');
                for r in &records {
                    text.push_str(&aocal::to_csv(r));
                    text.push('\n');
                }
                std::fs::write(&path, text)?;
                println!("\n  {} 行を {} へ書いた", records.len(), path.display());
            }
        }

        Command::Flip { seeds, steps } => {
            use pxsmith_core::palette::ChromaCurve;
            use pxsmith_core::ramp::{LightPreset, build_lighting};
            use pxsmith_core::shade::{ShadeOptions, shade_to_canvas};

            let mut files: Vec<PathBuf> = std::fs::read_dir(&seeds)?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")))
                .collect();
            files.sort();

            let (mut upright, mut mirrored) = (Vec::new(), Vec::new());
            let mut by_preset: std::collections::BTreeMap<&str, Vec<f32>> = Default::default();
            for path in &files {
                let Ok(img) = pxsmith_io::png::read_rgba(path) else {
                    continue;
                };
                let mut mask = pxsmith_core::geom::Mask::new(img.width(), img.height());
                for p in mask.bounds().iter() {
                    if img.get(p.x, p.y).is_some_and(|c| c.a != 0) {
                        mask.set(p, true);
                    }
                }
                if mask.count() < 64 {
                    continue;
                }
                let base = pxsmith_core::color::Rgba8::rgb(0x8a, 0x6a, 0x4a);
                for preset in LightPreset::ALL {
                    let Ok((palette, model)) =
                        build_lighting(base, preset, steps, ChromaCurve::PeakMiddle)
                    else {
                        continue;
                    };
                    let source = preset.default_source();
                    let Ok((canvas, palette)) =
                        shade_to_canvas(&mask, source, &model, &palette, ShadeOptions::default())
                    else {
                        continue;
                    };
                    // **宣言した光源はそのまま．絵だけを左右反転する** (自動ミラー)
                    let mut flipped = canvas.clone();
                    for p in canvas.bounds().iter() {
                        let q = pxsmith_core::math::ivec2(canvas.width() as i32 - 1 - p.x, p.y);
                        if let Some(i) = canvas.get_at(q) {
                            flipped.set_at(p, i);
                        }
                    }
                    if let Some(a) =
                        pxsmith_lint::rules::shading_agreement(&canvas, &palette, source)
                    {
                        upright.push(a);
                        by_preset.entry(preset.as_str()).or_default().push(a);
                    }
                    if let Some(a) =
                        pxsmith_lint::rules::shading_agreement(&flipped, &palette, source)
                    {
                        mirrored.push(a);
                    }
                }
            }
            let quant = |v: &mut Vec<f32>| {
                v.sort_by(f32::total_cmp);
                let at = |q: f32| v[((v.len() - 1) as f32 * q).round() as usize];
                format!(
                    "最小 {:.3} ・5% {:.3} ・中央 {:.3} ・95% {:.3} ・最大 {:.3}",
                    v[0],
                    at(0.05),
                    at(0.5),
                    at(0.95),
                    v[v.len() - 1]
                )
            };
            println!("== 明度勾配と光源方向の一致度 (pxsmith shade の出力) ==");
            for (name, v) in &mut by_preset {
                println!("  プリセット {name:<10} {}", quant(v));
            }
            println!("  そのまま  {} 件  {}", upright.len(), quant(&mut upright));
            println!(
                "  左右反転  {} 件  {}",
                mirrored.len(),
                quant(&mut mirrored)
            );
            for t in [-0.5f32, -0.3, -0.1, 0.0, 0.1, 0.3, 0.5] {
                let good = upright.iter().filter(|a| **a < t).count();
                let bad = mirrored.iter().filter(|a| **a < t).count();
                println!(
                    "  閾値 {t:>5}  正しい向きで鳴る {good:>3} / {}  ・反転を捕捉 {bad:>3} / {}",
                    upright.len(),
                    mirrored.len()
                );
            }
        }

        Command::Dither { tile } => {
            // 市松 (周期 2) を並べて，継ぎ目で同色が隣り合う行を数える．
            // **閾値は無い — 数え上げである**
            let touching = |w: u32, h: u32, mirror: bool| -> usize {
                (0..h as i32)
                    .filter(|&y| {
                        let left = ((w as i32 - 1) + y) % 2;
                        let right = if mirror {
                            // 鏡像なら継ぎ目の列は元の «右端» がもう一度来る
                            ((w as i32 - 1) + y) % 2
                        } else {
                            y % 2
                        };
                        left == right
                    })
                    .count()
            };
            println!("== ディザの位相 — 同じ絵を並べたときの継ぎ目 (市松，高さ 16) ==");
            println!("\n  一辺  偶奇  同一タイルの反復  鏡像を隣に置く");
            for &w in &tile {
                println!(
                    "  {w:>4}  {:<4} {:>13} / 16 {:>10} / 16",
                    if w % 2 == 0 { "偶数" } else { "奇数" },
                    touching(w, 16, false),
                    touching(w, 16, true)
                );
            }
            println!(
                "\n  設計書 4.3 / D45 は «タイルの幅は必ず偶数なので，同一タイルを並べると\n                   ディザのドットが連結する» と言うが，**偶数幅の反復では 0 である** —\n                   幅が偶数だからこそ位相が続く．連結するのは奇数幅と «鏡像を隣に置いたとき»．\n                   autotile は象限を鏡像で組むので，**タイルの内側の継ぎ目**で起きる (D105)．"
            );
        }

        Command::Tileset {
            seeds,
            tile,
            light,
            out,
        } => {
            let source = parse_light_spec(&light)?;
            let records = tilecal::run(&seeds, &tile, source)?;
            println!("== タイル分割と同値判定 (実素材 ・削減率は入力で決まる量である) ==");
            println!(
                "\n  一辺  モード        絵     縮約前   縮約後   削減率   反転で束ねた  測れた  鳴った  組み直し失敗"
            );
            for (t, mode, n, before, after, reduction, mirrored, measurable, fired, broken) in
                tilecal::summarise(&records)
            {
                println!(
                    "  {t:>4}  {mode:<12} {n:>4} {before:>9} {after:>8} {:>8.1}% {mirrored:>13} {measurable:>7} {fired:>7} {broken:>13}",
                    reduction * 100.0
                );
            }
            println!(
                "\n  «測れた» はルール 7 が勾配を取れた枚数．\n                   **8x8 では上限 6x6 = 36 画素で {} 画素の下限に構造的に届かない**                  — 鳴らないのではなく検査していない",
                pxsmith_lint::LintConfig::default().shading_min_pixels
            );
            if let Some(path) = out {
                let mut text = String::from(tilecal::HEADER);
                text.push('\n');
                for r in &records {
                    text.push_str(&tilecal::to_csv(r));
                    text.push('\n');
                }
                std::fs::write(&path, text)
                    .with_context(|| format!("{} を書き出せない", path.display()))?;
                println!("\n{} に書き出した", path.display());
            }
        }

        Command::Direction { seeds, steps, out } => {
            let records = dircal::run(&seeds, steps)?;
            let threshold = pxsmith_lint::LintConfig::default().min_shading_agreement;
            println!("== 方向展開 — 光源との一致度 (ルール 7 の閾値 {threshold:.2} 未満で鳴る) ==");
            println!("\n  群              段階            件数     最小    中央    最大  鳴る件数");
            for (group, stage, n, min, median, max, below) in dircal::summarise(&records, threshold)
            {
                println!(
                    "  {group:<15} {stage:<14} {n:>5} {min:>8.3} {median:>7.3} {max:>7.3} {below:>9}"
                );
            }
            println!("\n== 再導出が «反転しただけの絵» から書き換えた不透明画素 ==");
            println!("\n  群              件数    中央     最大");
            for (group, n, median, max) in dircal::summarise_rewritten(&records) {
                println!(
                    "  {group:<15} {n:>5} {:>7.1}% {:>7.1}%",
                    median * 100.0,
                    max * 100.0
                );
            }
            if let Some(path) = out {
                let mut text = String::from(dircal::HEADER);
                text.push('\n');
                for r in &records {
                    text.push_str(&dircal::to_csv(r));
                    text.push('\n');
                }
                std::fs::write(&path, text)
                    .with_context(|| format!("{} を書き出せない", path.display()))?;
                println!("\n{} に書き出した", path.display());
            }
        }

        Command::Compose {
            seeds,
            base,
            equip,
            clip,
            out,
        } => {
            let bases = if base.is_empty() {
                composecal::names(composecal::DEFAULT_BASES)
            } else {
                base.clone()
            };
            let equips = if equip.is_empty() {
                composecal::names(composecal::DEFAULT_EQUIPS)
            } else {
                equip.clone()
            };

            let margins = composecal::margins(&seeds)?;
            println!("== 画布の余白 (アンカーで動かしたときどれだけ余地があるか) ==");
            println!("\n  群            枚数  縁に接する  最小余白の中央  最大");
            for (label, n, touching, median, max) in composecal::summarise_margins(&margins) {
                println!("  {label:<12} {n:>5} {touching:>11} {median:>15} {max:>5}");
            }

            let merges = composecal::merges(&seeds, &bases, &equips)?;
            let (n, median, max, over62, over256, shared) = composecal::summarise_merges(&merges);
            println!("\n== 併合したパレットの色数 (胴体 x 装備 {n} 組) ==");
            println!("\n  中央 {median} 色 ・最大 {max} 色");
            println!("  L0 の 62 色を超える {over62} 組 ・256 色を超える {over256} 組");
            println!("  装備の色のうち胴体と共有しているものの割合 (中央) {shared:.2}");

            let shifts = composecal::default_shifts();
            let mut runs = composecal::runs(&seeds, &bases, &equips, &shifts, false)?;
            if clip {
                runs.extend(composecal::runs(&seeds, &bases, &equips, &shifts, true)?);
            }
            println!("\n== 実際に合成した ({} 組) ==", runs.len());
            println!("\n  ずらし    切る  組数  画布が広がった  捨てた画素  blocking 増");
            for (shift, clipped_mode, total, grew, clipped, worse) in
                composecal::summarise_runs(&runs)
            {
                println!(
                    "  ({:>2},{:>3}) {:>7} {total:>5} {grew:>15} {clipped:>11} {worse:>12}",
                    shift.x,
                    shift.y,
                    if clipped_mode { "切る" } else { "広げる" }
                );
            }

            let by_rule = composecal::blocking_by_rule(&runs);
            if by_rule.is_empty() {
                println!("\n  blocking は 1 件も増えていない");
            } else {
                println!("\n  増えた blocking の内訳");
                for (rule, delta, cases) in by_rule {
                    println!("    ルール {rule:>2}  +{delta} 件 ({cases} 組)");
                }
            }

            if let Some(path) = out {
                let mut text = String::new();
                text.push_str(composecal::MARGIN_HEADER);
                text.push('\n');
                for m in &margins {
                    text.push_str(&composecal::margin_csv(m));
                    text.push('\n');
                }
                text.push_str(composecal::MERGE_HEADER);
                text.push('\n');
                for m in &merges {
                    text.push_str(&composecal::merge_csv(m));
                    text.push('\n');
                }
                text.push_str(composecal::RUN_HEADER);
                text.push('\n');
                for r in &runs {
                    text.push_str(&composecal::run_csv(r));
                    text.push('\n');
                }
                std::fs::write(&path, text)
                    .with_context(|| format!("{} を書き出せない", path.display()))?;
                println!("\n{} に書き出した", path.display());
            }
        }

        Command::Smear { seeds, out } => {
            let shifts = [4i32, 8, 16, 24, 32];
            let rows = animcal::smear_rows(&seeds, &shifts)?;
            println!("== おばけ 3 通り (円板ではなく実素材を平行移動させたもの) ==");
            println!("\n  ずらし  件数  union が繋がる  掃引が繋がる  **重心を取り除いた掃引**");
            for (shift, n, u, p, c) in animcal::summarise_smear(&rows) {
                println!("  {shift:>6} {n:>6} {u:>15} {p:>13} {c:>24}");
            }

            let samples = [Some(1u32), Some(2), Some(4), Some(8), Some(16), None];
            println!("\n== 刻み幅の掃引 (ずらし 32 画素．None は «変位から決める») ==");
            for (n, broken, total) in animcal::sample_sweep(&seeds, 32, &samples)? {
                let label = n
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "変位から".into());
                println!("  標本 {label:<8} — 切れた {broken:>3} / {total} 枚");
            }

            if let Some(path) = out {
                let mut text = String::from(animcal::SMEAR_HEADER);
                text.push('\n');
                for r in &rows {
                    text.push_str(&animcal::smear_csv(r));
                    text.push('\n');
                }
                std::fs::write(&path, text)
                    .with_context(|| format!("{} を書き出せない", path.display()))?;
                println!("\n{} に書き出した", path.display());
            }
        }

        Command::Extrapolate { seeds, out } => {
            let shifts = [8i32, 16];
            let amounts = [0.25f32, 0.5, 1.0];
            let rows = animcal::extrapolate_rows(&seeds, &shifts, &amounts)?;
            println!("== 外挿 (平行移動なので真値がある — «t 倍だけ動かした絵») ==");
            println!(
                "\n  種類           振り幅  件数  場のまま  重心を取り除く  端のまま  切れた枚数"
            );
            for (kind, amount, n, plain, centroid, hold, clipped) in
                animcal::summarise_extrapolate(&rows)
            {
                println!(
                    "  {kind:<14} {amount:>5.2} {n:>6} {plain:>9.3} {centroid:>15.3} {hold:>9.3} {clipped:>11}"
                );
            }
            let changed = rows.iter().filter(|r| r.topology_changed).count();
            println!(
                "\n  {} 件中 {} 件でトポロジーが両端のどちらとも違う",
                rows.len(),
                changed
            );

            if let Some(path) = out {
                let mut text = String::from(animcal::EXTRAPOLATE_HEADER);
                text.push('\n');
                for r in &rows {
                    text.push_str(&animcal::extrapolate_csv(r));
                    text.push('\n');
                }
                std::fs::write(&path, text)
                    .with_context(|| format!("{} を書き出せない", path.display()))?;
                println!("\n{} に書き出した", path.display());
            }
        }

        Command::Squash { seeds, out } => {
            let amounts = [-0.5f32, -0.25, 0.25, 0.5];
            let rows = animcal::squash_rows(&seeds, &amounts)?;
            println!("== 体積保存 ($h \\times w$)．画素は整数なので丸めが残る ==");
            println!(
                "\n  決め方       画布  件数  矩形の誤差 (中央)  最悪  画素数の誤差  **拡縮だけ**  切れた  色が増えた"
            );
            for (rule, grew, n, med, worst, px, resample, clipped, added) in
                animcal::summarise_squash(&rows)
            {
                let canvas = if grew { "広げる" } else { "そのまま" };
                println!(
                    "  {rule:<12} {canvas:<6} {n:>4} {:>16.3}% {:>5.1}% {:>12.1}% {:>12.1}% {clipped:>7} {added:>11}",
                    med * 100.0,
                    worst * 100.0,
                    px * 100.0,
                    resample * 100.0
                );
            }

            if let Some(path) = out {
                let mut text = String::from(animcal::SQUASH_HEADER);
                text.push('\n');
                for r in &rows {
                    text.push_str(&animcal::squash_csv(r));
                    text.push('\n');
                }
                std::fs::write(&path, text)
                    .with_context(|| format!("{} を書き出せない", path.display()))?;
                println!("\n{} に書き出した", path.display());
            }
        }

        Command::Subpixel { seeds, out } => {
            let fractions = [0.25f32, 0.5, 0.75];
            let rows = animcal::subpixel_rows(&seeds, &fractions)?;
            let (lo, mid, hi) = animcal::intermediate_rates(&rows);
            println!("== サブピクセル (実素材) ==");
            println!(
                "\n  パレットの色の組に «間の色» がある割合 — 最小 {:.1}% ・中央 {:.1}% ・最大 {:.1}%",
                lo * 100.0,
                mid * 100.0,
                hi * 100.0
            );
            println!(
                "\n  方法      範囲        率  件数  動いた枚数  動いた画素 (中央)  中間色が無い  輪郭が動いた  色が増えた  blocking 増"
            );
            for (method, scope, fraction, n, moved, px, no_colour, sil, added, blocking) in
                animcal::summarise_subpixel(&rows)
            {
                println!(
                    "  {method:<9} {scope:<10} {fraction:>4.2} {n:>5} {moved:>11} {px:>18.0} {no_colour:>13} {sil:>13} {added:>11} {blocking:>12}"
                );
            }

            let (files, differing, med, multi) = animcal::fraction_sensitivity(&seeds)?;
            println!(
                "\n  移動率を 0.25 と 0.75 に変えて出力を突き合わせる — {differing} / {files} 枚で違う (違った画素の中央 {med:.0})"
            );
            println!(
                "  間の色が **2 色以上** ある組は，間の色がある組のうち {:.1}%",
                multi * 100.0
            );

            if let Some(path) = out {
                let mut text = String::from(animcal::SUBPIXEL_HEADER);
                text.push('\n');
                for r in &rows {
                    text.push_str(&animcal::subpixel_csv(r));
                    text.push('\n');
                }
                std::fs::write(&path, text)
                    .with_context(|| format!("{} を書き出せない", path.display()))?;
                println!("\n{} に書き出した", path.display());
            }
        }

        Command::Afterimage { seeds, out } => {
            let shifts = [1i32, 2, 4, 8];
            let trails = [1u32, 2, 3];
            let rows = animcal::afterimage_rows(&seeds, &shifts, &trails)?;
            println!("== 残像 (pxsmith shade に描かせた 3 コマ．ランプが宣言されている状態) ==");
            println!(
                "\n  ずらし  長さ  件数  見えた枚数  見えた画素 (中央)  隠れた画素  端に着いた"
            );
            for (shift, trail, n, visible, drawn, covered, sat) in
                animcal::summarise_afterimage(&rows)
            {
                println!(
                    "  {shift:>6} {trail:>5} {n:>5} {visible:>11} {drawn:>18.0} {covered:>11.0} {sat:>11.0}"
                );
            }

            if let Some(path) = out {
                let mut text = String::from(animcal::AFTERIMAGE_HEADER);
                text.push('\n');
                for r in &rows {
                    text.push_str(&animcal::afterimage_csv(r));
                    text.push('\n');
                }
                std::fs::write(&path, text)
                    .with_context(|| format!("{} を書き出せない", path.display()))?;
                println!("\n{} に書き出した", path.display());
            }
        }

        Command::Rotate { seeds, out } => {
            let angles = [15.0f32, 30.0, 45.0, 60.0];
            let (rows, skipped) = rotcal::build(&seeds, &angles)?;
            println!("== 拡縮 ・回転の品質 ==");
            println!("  {} 件 (飛ばした {skipped})", rows.len());
            for (algo, ok, total) in rotcal::integer_scale_is_exact(&seeds)? {
                println!("  3 倍拡大が厳密だった ({algo}): {ok} / {total}");
            }
            println!("\n  流儀        角度   件数   往復一致   影の一致  ジャギー(平均)  作った色");
            for (algo, deg, n, rt, sil, jag, created) in rotcal::summarise(&rows) {
                println!(
                    "  {algo:<10} {deg:>5.1} {n:>6} {:>9.1}% {:>9.1}% {jag:>13.1} {created:>9}",
                    rt * 100.0,
                    sil * 100.0
                );
            }
            println!("\n  拡大を伴う場面 (cleanEdge の本来の使い方)");
            println!("    流儀        場面            件数  ジャギー(平均)  作った色");
            for (algo, scene, n, jag, created) in rotcal::upscaled(&seeds)? {
                println!("    {algo:<10} {scene:<14} {n:>5} {jag:>13.1} {created:>9}");
            }

            if let Some(path) = out {
                let mut text = String::from(rotcal::ROT_HEADER);
                text.push('\n');
                for r in &rows {
                    text.push_str(&rotcal::rot_csv(r));
                    text.push('\n');
                }
                std::fs::write(&path, text)
                    .with_context(|| format!("{} を書き出せない", path.display()))?;
                println!("\n{} に書き出した", path.display());
            }
            return Ok(());
        }

        Command::Project {
            seeds,
            stair_len,
            out,
        } => {
            println!("== 設計書 6.13 の «または» は同じ変換か ==");
            let (pairs, skipped) = projcal::two_procedures(&seeds)?;
            let agree = pairs.iter().map(|r| r.agreement).sum::<f32>() / pairs.len().max(1) as f32;
            let same_size = pairs
                .iter()
                .filter(|r| r.halve_size == r.shear_size)
                .count();
            println!("  {} 件 (飛ばした {skipped})", pairs.len());
            println!("  シルエット一致 (中央合わせ)  {:.1}%", agree * 100.0);
            println!(
                "  出力の寸法が同じ            {same_size} / {}",
                pairs.len()
            );
            if let Some(best) = pairs
                .iter()
                .max_by(|a, b| a.agreement.total_cmp(&b.agreement))
            {
                println!(
                    "  最も近い 1 件でも          {:.1}% ({}．{}x{} 対 {}x{})",
                    best.agreement * 100.0,
                    best.file,
                    best.halve_size.0,
                    best.halve_size.1,
                    best.shear_size.0,
                    best.shear_size.1
                );
            }
            println!("  ** 一致しないなら «または» は誤りで，適用先の違う 2 つの変換である **");

            println!("\n== 段は格子に乗るか (走りの長さが 1 種類なら乗っている) ==");
            println!("  段                        傾き    角度   現れた走りの長さ");
            for s in projcal::stairs(stair_len) {
                let runs: Vec<String> = s.runs.iter().map(|r| r.to_string()).collect();
                println!(
                    "  {:<24} {:>5.3} {:>7.2} 度  {}",
                    s.label,
                    s.slope,
                    s.degrees,
                    runs.join(" ・")
                );
            }

            println!("\n== 実素材のジャギー (横から見た絵として写す) ==");
            println!("    名前                件数  ジャギー(平均)  作った色");
            for (label, n, jag, created) in projcal::grid_vs_thirty(&seeds)? {
                println!("    {label:<20} {n:>5} {jag:>13.1} {created:>9}");
            }

            let (rows, skipped) = projcal::build(&seeds)?;
            println!("\n== 投影 x 面 ({} 件 ・飛ばした {skipped}) ==", rows.len());
            println!(
                "  投影         面     件数   角度   垂直維持   面積比  切れ  切れ(広げず)  作色  blocking増  ジャギー"
            );
            for (p, plane, n, deg, vert, area, clip, clip_ng, created, dblock, jag) in
                projcal::summarise(&rows)
            {
                println!(
                    "  {p:<12} {plane:<6} {n:>4} {deg:>6.2} {:>9.0}% {area:>8.3} {clip:>5} {clip_ng:>13} {created:>5} {dblock:>11.1} {jag:>9.1}",
                    vert * 100.0
                );
            }
            println!(
                "\n  ** blocking が増えるのは道具の誤りではない ** — 投影は下地であり\n\
                 \u{3000}\u{3000}手修正が前提である (設計書 1.3) ．"
            );

            if let Some(path) = out {
                let mut text = String::from(projcal::PROJ_HEADER);
                text.push('\n');
                for r in &rows {
                    text.push_str(&projcal::proj_csv(r));
                    text.push('\n');
                }
                std::fs::write(&path, text)
                    .with_context(|| format!("{} を書き出せない", path.display()))?;
                println!("\n{} に書き出した", path.display());
            }
            return Ok(());
        }

        Command::LintShape {
            seeds,
            min_area,
            out,
        } => {
            let (rows, skipped) = shapecal::build(&seeds, min_area)?;
            println!("== 形の乱雑さ (ルール 19) の量を 2 通りで測る ==");
            println!(
                "  領域 {} 件 (面積 {min_area} 以上) ．飛ばした件 {skipped}",
                rows.len()
            );
            for (name, use_excess) in [
                ("P^2 / A (設計書のまま)", false),
                ("P / 外接矩形の周囲長", true),
            ] {
                println!("\n  -- {name} --");
                for g in ["good", "rough", "sil-good", "sil-rough"] {
                    let (med, p90, p99, n) = shapecal::quantiles(&rows, g, use_excess);
                    println!(
                        "    {:<6} {n:>5} 件  中央 {med:>7.2}  90% {p90:>7.2}  99% {p99:>7.2}",
                        match g {
                            "good" => "領域:良",
                            "rough" => "領域:荒",
                            "sil-good" => "影:良",
                            _ => "影:荒",
                        }
                    );
                }
                let values: Vec<f32> = if use_excess {
                    vec![1.2, 1.5, 2.0, 2.5, 3.0, 4.0]
                } else {
                    vec![16.0, 25.0, 40.0, 60.0, 100.0, 200.0]
                };
                for (good, bad, label) in [
                    ("good", "rough", "領域ごと"),
                    ("sil-good", "sil-rough", "シルエット"),
                ] {
                    println!("    [{label}] 値      良い絵で鳴る        荒らした絵で鳴る");
                    for (v, gf, gn, rf, rn) in
                        shapecal::sweep(&rows, &values, use_excess, good, bad)
                    {
                        println!(
                            "      {v:>6.2} {gf:>6} / {gn:<5} ({:>5.1}%) {rf:>6} / {rn:<5} ({:>5.1}%)",
                            gf as f32 / gn.max(1) as f32 * 100.0,
                            rf as f32 / rn.max(1) as f32 * 100.0
                        );
                    }
                }
            }
            // --- ルール 20 ・21 の面積の下限を掃く
            {
                use pxsmith_lint::rules::LintConfig;
                println!("\n  ルール 20 ・21 — 接触を見る «面積の下限» を掃く");
                println!(
                    "    下限   20:良い絵      20:角を付けた     21:良い絵      21:同化させた"
                );
                for min_touch in [1u32, 2, 4, 8, 16, 32] {
                    let cfg = LintConfig {
                        min_touch_area: min_touch,
                        ..Default::default()
                    };
                    let (mut g20, mut b20, mut g21, mut b21) = (0, 0, 0, 0);
                    let (mut n, mut nb20, mut nb21) = (0, 0, 0);
                    for path in crate::animcal::png_files(&seeds)? {
                        let Some((canvas, palette)) = crate::animcal::indexed(&path) else {
                            continue;
                        };
                        n += 1;
                        let fired = |c: &pxsmith_core::canvas::IndexedCanvas, id: u8| -> bool {
                            pxsmith_lint::lint_canvas(c, &palette, &cfg)
                                .violations
                                .iter()
                                .any(|v| v.rule == id)
                        };
                        if fired(&canvas, 20) {
                            g20 += 1;
                        }
                        if fired(&canvas, 21) {
                            g21 += 1;
                        }
                        if let Some(bad) = shapecal::add_tangent(&canvas, &palette, 4) {
                            nb20 += 1;
                            if fired(&bad, 20) {
                                b20 += 1;
                            }
                        }
                        // **負例は下限と無関係に固定する** — 下限ごとに作り直すと，
                        // 高い下限では «易しい部分集合» を見ることになる
                        if let Some(bad) = shapecal::merge_colours(&canvas, 16) {
                            nb21 += 1;
                            if fired(&bad, 21) {
                                b21 += 1;
                            }
                        }
                    }
                    println!(
                        "    {min_touch:>4} {g20:>4} / {n:<4} ({:>5.1}%) {b20:>4} / {nb20:<4} ({:>5.1}%) {g21:>4} / {n:<4} ({:>5.1}%) {b21:>4} / {nb21:<4} ({:>5.1}%)",
                        g20 as f32 / n.max(1) as f32 * 100.0,
                        b20 as f32 / nb20.max(1) as f32 * 100.0,
                        g21 as f32 / n.max(1) as f32 * 100.0,
                        b21 as f32 / nb21.max(1) as f32 * 100.0
                    );
                }
            }

            if let Some(path) = out {
                let mut text = String::from(shapecal::SHAPE_HEADER);
                text.push('\n');
                for r in &rows {
                    text.push_str(&shapecal::shape_csv(r));
                    text.push('\n');
                }
                std::fs::write(&path, text)
                    .with_context(|| format!("{} を書き出せない", path.display()))?;
                println!("\n{} に書き出した", path.display());
            }
            return Ok(());
        }

        Command::LintSeq { seeds } => {
            let (sequences, _, skipped) = seqcal::build(&seeds)?;
            let cfg = pxsmith_lint::rules::LintConfig::default();
            println!("== フレーム間ルール (22 〜 27) ==");
            println!(
                "  列 {} 本．飛ばした件: 添字にできない {} ・空 {} ・欠陥を入れられない {} ・中割りを作れない {}",
                sequences.len(),
                skipped.not_indexable,
                skipped.empty,
                skipped.not_corrupted,
                skipped.no_tween
            );
            println!(
                "  既定の閾値: 揺れ {:.3} ・ディザの位相 {:.3} ・新しい列の下限 {} ・体積 {:.3}",
                cfg.wobble_ratio, cfg.moving_dither_ratio, cfg.min_new_run, cfg.volume_error
            );

            println!("\n  群            本数    22    23    24    25    26    27");
            for (group, n, counts) in seqcal::measure(&sequences, &cfg) {
                println!(
                    "  {group:<12} {n:>5} {:>5} {:>5} {:>5} {:>5} {:>5} {:>5}",
                    counts[0], counts[1], counts[2], counts[3], counts[4], counts[5]
                );
            }

            {
                println!("\n  ルール 24 — ディザ領域を探す窓の一辺を変える");
                println!("    一辺    正例で鳴る        負例で捕捉");
                for w in [4u32, 6, 8, 12] {
                    let (mut fp, mut np, mut tp, mut nn) = (0, 0, 0, 0);
                    let cfg = pxsmith_lint::rules::LintConfig {
                        moving_dither_window: w,
                        ..Default::default()
                    };
                    for s in &sequences {
                        let hit = {
                            let (report, _) = pxsmith_lint::lint_sequence(&s.frames, &cfg);
                            report.violations.iter().any(|x| x.rule == 24)
                        };
                        if seqcal::positive_groups().contains(&s.group) {
                            np += 1;
                            if hit {
                                fp += 1;
                            }
                        } else if s.group == "ditherstuck" {
                            nn += 1;
                            if hit {
                                tp += 1;
                            }
                        }
                    }
                    println!(
                        "    {w:>4} {fp:>6} / {np:<5} ({:>5.1}%) {tp:>6} / {nn:<5} ({:>5.1}%)",
                        fp as f32 / np.max(1) as f32 * 100.0,
                        tp as f32 / nn.max(1) as f32 * 100.0
                    );
                }
            }

            for (rule, name, values) in [
                (
                    23u8,
                    "揺れ (面積比)",
                    vec![0.005f32, 0.01, 0.02, 0.05, 0.10, 0.20, 0.40],
                ),
                (
                    24,
                    "ディザの位相 (入れ替わり比)",
                    vec![0.02, 0.05, 0.10, 0.20, 0.35, 0.50],
                ),
                (27, "体積の誤差", vec![0.02, 0.05, 0.08, 0.12, 0.20, 0.30]),
            ] {
                println!("\n  ルール {rule} の掃引 — {name}");
                println!("    値      正例で鳴る        負例で捕捉");
                for (v, fp, np, tp, nn) in
                    seqcal::sweep(&sequences, rule, &values, |c, v| match rule {
                        23 => c.wobble_ratio = v,
                        24 => c.moving_dither_ratio = v,
                        _ => c.volume_error = v,
                    })
                {
                    println!(
                        "    {v:>5.3} {fp:>6} / {np:<5} ({:>5.1}%) {tp:>6} / {nn:<5} ({:>5.1}%)",
                        fp as f32 / np.max(1) as f32 * 100.0,
                        tp as f32 / nn.max(1) as f32 * 100.0
                    );
                }
            }

            println!("\n  検査できなかったもの (群ごとの列数)");
            let mut unchecked: std::collections::BTreeMap<(String, u8), usize> =
                std::collections::BTreeMap::new();
            let mut totals: std::collections::BTreeMap<String, usize> =
                std::collections::BTreeMap::new();
            for (group, cov) in seqcal::coverage(&sequences, &cfg) {
                *totals.entry(group.clone()).or_default() += 1;
                for id in cov.unchecked() {
                    *unchecked.entry((group.clone(), id)).or_default() += 1;
                }
            }
            for (group, total) in &totals {
                let list: Vec<String> = (22..=27u8)
                    .filter_map(|id| {
                        unchecked
                            .get(&(group.clone(), id))
                            .map(|n| format!("{id}: {n}"))
                    })
                    .collect();
                println!(
                    "  {group:<12} {total:>5} 本中 — {}",
                    if list.is_empty() {
                        "全部検査した".to_string()
                    } else {
                        list.join(" ・")
                    }
                );
            }
            return Ok(());
        }

        Command::Atmos {
            seeds,
            level_steps,
            out,
        } => {
            // 3 値の `Depth` に合わせた 3 点 + 端 (**寄せ具合そのものは宣言である**)
            let amounts = [0.0f32, 0.15, 0.3, 0.45, 0.6, 0.75, 1.0];
            let (rows, skipped) = atmoscal::atmos_rows(&seeds, &amounts)?;
            println!(
                "== 空気遠近法 (実素材 ・空の色 {} 通り ・寄せ具合 {} 通り) ==",
                atmoscal::SKIES.len(),
                amounts.len()
            );
            println!(
                "  飛ばした件: 添字にできない {} ・不透明画素が無い {} ・真値を作れない (絵, 空) {}",
                skipped.not_indexable, skipped.empty, skipped.extended_overflow
            );

            for group in ["own", "extended"] {
                println!(
                    "\n-- パレット: {} --",
                    if group == "own" {
                        "絵のまま"
                    } else {
                        "霞ませた先を足した真値"
                    }
                );
                println!(
                    "  寄せ  件数    色数  動いた(nearest)  動いた(between)  逆走  色差:対照  nearest  between  残った色N  逆転  明度幅N  外れ(中央)  外れ>許容  残った色B  明度幅B"
                );
                for (
                    t,
                    n,
                    colors,
                    mn,
                    mb,
                    wrong,
                    dc,
                    dn,
                    db,
                    kept,
                    non_mono,
                    spread,
                    detour,
                    off,
                    kept_b,
                    spread_b,
                ) in atmoscal::summarise(&rows, group)
                {
                    println!(
                        "  {t:>4.2} {n:>5} {colors:>7} {:>15.1}% {:>15.1}% {:>5.1}% {dc:>10.4} {dn:>8.4} {db:>8.4} {:>9.1}% {non_mono:>5} {:>7.2} {detour:>11.4} {:>10.1}% {:>10.1}% {:>7.2}",
                        mn * 100.0,
                        mb * 100.0,
                        wrong * 100.0,
                        kept * 100.0,
                        spread,
                        off * 100.0,
                        kept_b * 100.0,
                        spread_b
                    );
                }
            }

            let (en, eb, colors) = atmoscal::exact_rates(&rows);
            println!(
                "\n  真値との一致 ({colors} 色 x 段): nearest {:.1}% ・between {:.1}%",
                en * 100.0,
                eb * 100.0
            );

            println!(
                "\n  «線の上» と認める許容を変える (存在は $t$ に依らない．既定は pxsmith aa と同じ {:.2})",
                atmoscal::SEGMENT_TOLERANCE
            );
            for (tol, rate, total) in
                atmoscal::tolerance_sweep(&seeds, &[0.01, 0.02, 0.04, 0.08, 0.16, 0.32])?
            {
                println!(
                    "    許容 {tol:>4.2}: 置き換え先が在った {:>5.1}% ({total} 色 x 空)",
                    rate * 100.0
                );
            }

            for segment in [false, true] {
                let mut all = atmoscal::levels(&seeds, level_steps, segment)?;
                all.sort_unstable();
                let hist = |k: usize| all.iter().filter(|v| **v == k).count();
                println!(
                    "\n  パレットが表せる段の数 — 規則 {} ($t$ を {level_steps} 分割．{} 色 x 空)",
                    if segment { "between" } else { "nearest" },
                    all.len()
                );
                println!(
                    "    1 段 (効かない) {} ・2 段 {} ・3 段 {} ・4 段以上 {} ・中央 {} ・最大 {}",
                    hist(1),
                    hist(2),
                    hist(3),
                    all.iter().filter(|v| **v >= 4).count(),
                    all.get(all.len() / 2).copied().unwrap_or(0),
                    all.last().copied().unwrap_or(0)
                );
            }

            if let Some(path) = out {
                let mut text = String::from(atmoscal::ATMOS_HEADER);
                text.push('\n');
                for r in &rows {
                    text.push_str(&atmoscal::atmos_csv(r));
                    text.push('\n');
                }
                std::fs::write(&path, text)
                    .with_context(|| format!("{} を書き出せない", path.display()))?;
                println!("\n{} に書き出した", path.display());
            }
        }

        Command::Tween { seeds, pair, out } => {
            let names: Vec<String> = if pair.is_empty() {
                tweencal::DEFAULT_PAIRS
                    .iter()
                    .map(|s| s.to_string())
                    .collect()
            } else {
                pair.clone()
            };

            let shifts = tweencal::default_shifts();
            let truths = tweencal::truths(&seeds, &shifts)?;
            println!("== 真値のある場面 (平行移動．t = 0.5 の真値は «半分だけ動かした絵») ==");
            println!("\n  ずらし    件数  場のまま  重心を取り除く  動かさない  重心が勝った");
            for (shift, n, plain, centroid, hold, better) in tweencal::summarise_truths(&truths) {
                println!(
                    "  ({:>2},{:>3}) {n:>7} {plain:>9.3} {centroid:>15.3} {hold:>11.3} {better:>13}",
                    shift.x, shift.y
                );
            }
            let (total, plain_wins, centroid_wins) = tweencal::beats_hold(&truths);
            println!(
                "\n  «動かさない» に勝てた件数 — 場のまま {plain_wins} / {total} ・重心を取り除く {centroid_wins} / {total}"
            );

            let margins = [0u32, 2, 4, 8, 16, 32];
            let reference = 64u32;
            use pxsmith_core::tween::TweenAlign;
            println!(
                "\n== 余白の掃引 (基準は余白 {reference}．画像の外を «背景» とみなす穴を潰すため) =="
            );
            // **縁に接する絵も入れる** — 余白が要るとしたらそこである (D91 と同じ穴)
            for (label, align, pad) in [
                ("場のまま ・縁に余地あり", TweenAlign::None, 2u32),
                ("場のまま ・縁に接する", TweenAlign::None, 0),
                ("重心 ・縁に接する", TweenAlign::Centroid, 0),
            ] {
                let rows = tweencal::margin_sweep(&seeds, &margins, reference, align, pad)?;
                let moved: Vec<String> = rows
                    .iter()
                    .filter(|r| r.differing > 0)
                    .map(|r| format!("余白 {} で {} 画素 ({} 枚)", r.margin, r.differing, r.files))
                    .collect();
                if moved.is_empty() {
                    println!("  {label:<24} — 余白 0 まで含めて 1 画素も動かない");
                } else {
                    println!("  {label:<24} — {}", moved.join(" ・"));
                }
            }

            let ts = [0.25f32, 0.5, 0.75];
            let pairs = tweencal::pairs(&seeds, &names, &ts)?;
            let (n, changed, empty, split) = tweencal::summarise_pairs(&pairs);
            println!(
                "\n== 別の絵どうし (真値なし．{} 枚の総当たり x t 3 通り) ==",
                names.len()
            );
            println!(
                "\n  {n} 件 — トポロジーが変わった {changed} ・空になった {empty} ・成分が増えた {split}"
            );

            if let Some(path) = out {
                let mut text = String::new();
                text.push_str(tweencal::TRUTH_HEADER);
                text.push('\n');
                for r in &truths {
                    text.push_str(&tweencal::truth_csv(r));
                    text.push('\n');
                }
                text.push_str(tweencal::PAIR_HEADER);
                text.push('\n');
                for r in &pairs {
                    text.push_str(&tweencal::pair_csv(r));
                    text.push('\n');
                }
                std::fs::write(&path, text)
                    .with_context(|| format!("{} を書き出せない", path.display()))?;
                println!("\n{} に書き出した", path.display());
            }
        }

        Command::Outline { dir, outer } => {
            use pxsmith_core::outline::{OutlineOptions, OutlineStyle, outline};
            let cfg = pxsmith_lint::LintConfig::default();
            let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")))
                .collect();
            files.sort();

            println!(
                "== pxsmith outline を良い絵に掛けた ({} 側) ==",
                if outer { "外" } else { "内" }
            );
            println!(
                "\n  分類       描いた画素  作った色  触った絵  2 度目  シルエット移動  blocking 増"
            );
            for style in [
                OutlineStyle::Black,
                OutlineStyle::Tinted,
                OutlineStyle::Contrast,
                OutlineStyle::Shaded,
            ] {
                let opts = OutlineOptions {
                    style,
                    outer,
                    ..OutlineOptions::default()
                };
                let (mut painted, mut added, mut touched) = (0usize, 0usize, 0usize);
                let (mut again_n, mut moved, mut worse, mut n) = (0usize, 0usize, 0usize, 0usize);
                for path in &files {
                    let Ok(img) = pxsmith_io::png::read_rgba(path) else {
                        continue;
                    };
                    let Ok((canvas, palette)) = lintcal::index_exactly(&img) else {
                        continue;
                    };
                    n += 1;
                    let transparent = canvas.transparent().unwrap_or(255);
                    let before_silhouette = canvas.mask_of(transparent);
                    let before_blocking = {
                        let mut r = pxsmith_lint::rules::lint_palette(&palette, &cfg);
                        r.extend(pxsmith_lint::lint_canvas(&canvas, &palette, &cfg));
                        r.blocking().count()
                    };
                    let (mut c, mut p) = (canvas.clone(), palette.clone());
                    let Ok(report) = outline(&mut c, &mut p, &opts) else {
                        continue;
                    };
                    painted += report.painted;
                    added += report.added_colors;
                    if report.painted > 0 {
                        touched += 1;
                    }
                    if !outer && c.mask_of(transparent) != before_silhouette {
                        moved += 1;
                    }
                    let after = {
                        let mut r = pxsmith_lint::rules::lint_palette(&p, &cfg);
                        r.extend(pxsmith_lint::lint_canvas(&c, &p, &cfg));
                        r.blocking().count()
                    };
                    if after > before_blocking {
                        worse += 1;
                    }
                    let (mut c2, mut p2) = (c.clone(), p.clone());
                    if let Ok(again) = outline(&mut c2, &mut p2, &opts)
                        && again.painted > 0
                    {
                        again_n += 1;
                    }
                }
                println!(
                    "  {:<10} {painted:>10} {added:>9} {touched:>9} / {n} {again_n:>6} {moved:>14} {worse:>11}",
                    style.as_str()
                );
            }
        }

        Command::LintGen {
            seeds,
            out,
            count,
            seed,
        } => {
            let n = lintgen::generate(&seeds, &out, count, seed)?;
            println!(
                "{n} 枚を {} へ書いた ({} 種類 x {count} 枚)",
                out.display(),
                lintgen::Defect::ALL.len()
            );
            println!(
                "  狙い: {}",
                lintgen::Defect::ALL
                    .iter()
                    .map(|d| format!("{} → ルール {}", d.as_str(), d.rule()))
                    .collect::<Vec<_>>()
                    .join(" ・")
            );
        }

        Command::Report {
            dir,
            sweep: sweep_path,
            target,
            top,
        } => {
            let path = sweep_path.unwrap_or_else(|| dir.join("sweep.csv"));
            let rows = sweep::read_csv(&path)?;
            report(&dir, &rows, target, top)?;
        }
    }
    Ok(())
}

fn or_default(given: Vec<f32>, fallback: &[f32]) -> Vec<f32> {
    if given.is_empty() {
        fallback.to_vec()
    } else {
        given
    }
}

fn parse_split(split: &str) -> Result<Option<Split>> {
    match split {
        "validation" => Ok(Some(Split::Validation)),
        "test" => Ok(Some(Split::Test)),
        "all" => Ok(None),
        other => anyhow::bail!("分割は validation / test / all のいずれか: {other}"),
    }
}

/// 自作レンダで実データ枠の素材を作る．
///
/// 劣化のかけ方は合成データと同じ ([`degrade`]) だが，**元絵の作り方が違う** —
/// 平坦な数色ではなく陰影の階調を持つ．目録には正解を書く (自作なので分かる) ．
fn render_real_items(dir: &std::path::Path, count: u32, seed: u64) -> Result<usize> {
    use degrade::{COMPRESSIONS, Degradation, FILTERS, RESIZES, SCALES};

    std::fs::create_dir_all(dir.join("render"))
        .with_context(|| format!("{} を作れない", dir.display()))?;

    let mut items = Vec::new();
    for i in 0..count {
        let mut rng = rng::Rng::new(seed ^ u64::from(i).wrapping_mul(0x9e37_79b9_7f4a_7c15));
        let source = render::render(rng.next_u64());
        let scale = SCALES[rng.below(SCALES.len() as u32) as usize];
        let degradation = Degradation {
            scale,
            filter: FILTERS[rng.below(FILTERS.len() as u32) as usize],
            resize: RESIZES[rng.below(RESIZES.len() as u32) as usize],
            compression: COMPRESSIONS[rng.below(COMPRESSIONS.len() as u32) as usize],
            crop: (rng.below(scale), rng.below(scale)),
        };
        let img = degradation.apply(&source)?;
        let file = format!("render/{i:03}.png");
        pxsmith_io::png::write_rgba(dir.join(&file), &img)?;

        items.push(real::Item {
            file,
            category: real::Category::Render,
            license: "CC0 (自作)".to_string(),
            source: format!(
                "自作 — pxsmith-calib render (種 {seed}, {} 倍 / {} / リサイズ {} / {})",
                degradation.scale,
                degradation.filter.as_str(),
                degradation.resize.as_str(),
                degradation.compression.as_str(),
            ),
            // 非整数倍リサイズを挟んだ件に整数の格子は無い．正解を書かず，**負例**にする
            // — 「分からない」のではなく「無いと作り方から分かっている」件である
            truth: degradation.truth_phase().map(|p| real::Truth {
                scale: degradation.scale,
                phase: Some(p),
            }),
            no_grid: degradation.truth_phase().is_none(),
            note: degradation
                .truth_phase()
                .is_none()
                .then(|| "非整数倍リサイズ — 整数の格子は無い (棄却が正しい)".to_string()),
        });
    }

    let n = items.len();
    let json = serde_json::to_string_pretty(&real::Manifest { items })?;
    pxsmith_io::atomic::write(dir.join("manifest.json"), json.as_bytes())?;
    Ok(n)
}

/// `ingest` の引数一式．
struct IngestOptions<'a> {
    /// 拡大倍率．`None` なら 2〜12 を巡回する
    scale: Option<u32>,
    /// 切り落とし量 `DX,DY`．`None` なら倍率から散らす
    crop: Option<&'a str>,
    category: &'a str,
    license: &'a str,
    native_max: u32,
    /// 周期を測らずにこの値で縮小する
    force_period: Option<usize>,
    /// 元絵の解像度を復元してから拡大する
    recover_native: bool,
    /// 入力がすでに元絵である (縮小しない)
    already_native: bool,
    /// 拡大時に劣化を通すか
    degrade: bool,
    /// 負例として保存するときの一辺．**`None` なら負例を取らない**
    negative_side: Option<u32>,
}

/// ドット絵風の画像を正例へ仕立てて目録へ足す．
///
/// **拒否したものは黙って捨てない** — 理由を出す．`negative_side` を与えると
/// 「周期が読めない」で拒否したものを負例 (棄却が正解) として目録へ入れる．
///
/// 負例にするのは `NoPeriod` **だけ**である．`OutOfRange` は格子があっても大きさが
/// 枠外なだけ，`NonUniform` は縦横で読みが食い違っただけで，どちらも「格子が無い」
/// 根拠にならない．
fn ingest_images(dir: &std::path::Path, images: &[PathBuf], opts: &IngestOptions) -> Result<()> {
    let &IngestOptions {
        scale,
        crop,
        category,
        license,
        native_max,
        force_period,
        recover_native,
        already_native,
        degrade: with_degradation,
        negative_side,
    } = opts;
    anyhow::ensure!(!images.is_empty(), "入力画像を 1 つ以上指定すること");
    if let Some(p) = force_period {
        anyhow::ensure!(p >= 2, "--force-period は 2 以上にすること (指定 {p})");
    }
    let category = match category {
        "ai-output" => real::Category::AiOutput,
        "render" => real::Category::Render,
        "screenshot" => real::Category::Screenshot,
        "other" => real::Category::Other,
        other => anyhow::bail!("区分は ai-output / render / screenshot / other: {other}"),
    };
    let sub = match category {
        real::Category::AiOutput => "ai-output",
        real::Category::Render => "render",
        real::Category::Screenshot => "screenshot",
        real::Category::Other => "other",
    };
    std::fs::create_dir_all(dir.join(sub))?;

    // 既にある目録は残す．同じ出力名だけ差し替える
    let mut manifest = real::read(dir).unwrap_or(real::Manifest { items: Vec::new() });
    // **連番は既にある分の続きから振る．** 毎回 0 から振ると，同じ区分へ 2 回取り込んだ
    // ときに 1 回目を黙って上書きしてしまう (実際に踏んだ — 24 件が消えた)
    let mut accepted = next_index(&manifest, sub, "");
    let mut negatives = next_index(&manifest, sub, "neg-");
    let mut refused = Vec::new();

    for (i, path) in images.iter().enumerate() {
        // 倍率と位相を散らす — 1 つの条件に偏ると分布のずれを測れない
        let s = scale.unwrap_or(degrade::SCALES[i % degrade::SCALES.len()]);
        let c = match crop {
            Some(text) => {
                let (a, b) = text
                    .split_once(',')
                    .context("--crop は DX,DY の形で書くこと")?;
                (a.trim().parse()?, b.trim().parse()?)
            }
            None => ((i as u32) % s, (i as u32 * 2) % s),
        };

        // 劣化の水準も散らす — 補間法と圧縮が同じ周期で回らないよう別々に進める
        let levels = with_degradation.then(|| {
            (
                degrade::FILTERS[i % degrade::FILTERS.len()],
                degrade::COMPRESSIONS[(i / degrade::FILTERS.len()) % degrade::COMPRESSIONS.len()],
            )
        });
        let recipe = ingest::Recipe {
            force_period,
            recover_native,
            already_native,
            scale: s,
            crop: c,
            native_max,
            degrade: levels,
        };

        match ingest::ingest_one(path, &recipe)? {
            Err(reason) => {
                println!("  拒否 {:<40} {reason}", file_name(path));
                // 「周期が読めない」だけが負例になる — 格子が無いことの根拠がここにしかない
                if let (Some(side), ingest::Refusal::NoPeriod) = (negative_side, &reason) {
                    let (img, original) = ingest::negative_from(path, side)?;
                    let name = format!("{sub}/neg-{negatives:03}.png");
                    pxsmith_io::png::write_rgba(dir.join(&name), &img)?;
                    // **切り出したかどうかを目録に残す** — 面積は誤受理の出方を変える
                    let how = if (img.width(), img.height()) == original {
                        "原寸のまま".to_string()
                    } else {
                        format!("{}x{} の中央を切り出し", original.0, original.1)
                    };
                    println!(
                        "       → 負例 {name} ({}x{}．{how}．再標本化はしていない)",
                        img.width(),
                        img.height()
                    );
                    manifest.items.retain(|it| it.file != name);
                    manifest.items.push(real::Item {
                        file: name,
                        category,
                        license: license.to_string(),
                        source: format!("{} ({how})", file_name(path)),
                        truth: None,
                        no_grid: true,
                        note: Some(
                            "ingest が周期を読めなかった (縁が境界へ集中していない)．\
                             棄却が正しい"
                                .to_string(),
                        ),
                    });
                    negatives += 1;
                }
                refused.push((path.clone(), reason));
            }
            Ok((img, info)) => {
                let name = format!("{sub}/{:03}.png", accepted);
                pxsmith_io::png::write_rgba(dir.join(&name), &img)?;
                // 出来上がりが大きすぎる場合，倍率は指定より下がっている．
                // **正解には実際に使った方を書く**
                let s = info.scale;
                let phase = ingest::truth_phase(s, c);
                // 劣化を掛けたなら，どの水準かを記録する (成績を条件で切り分けられる)
                let how = match info.degrade {
                    None => "最近傍".to_string(),
                    Some((f, comp)) => format!("{} / {}", f.as_str(), comp.as_str()),
                };
                // 元絵をどうやって取り出したか (目録に書く根拠)
                let reduced = match info.reduction {
                    ingest::Reduction::Period(p) => format!("測った周期 {p} で平均縮小"),
                    ingest::Reduction::ForcedPeriod(p) => {
                        format!("指定した周期 {p} で平均縮小")
                    }
                    ingest::Reduction::Recovered => format!(
                        "{}x{} から元絵を復元 ({:.3} 倍で拡大されていた)",
                        info.original.0,
                        info.original.1,
                        f64::from(info.original.0) / f64::from(info.native.0),
                    ),
                    ingest::Reduction::AsIs => "入力をそのまま元絵として使用".to_string(),
                };
                println!(
                    "  取込 {:<40} {:<14} 元絵 {}x{} → {} 倍 {how} ({}x{}) 位相 ({},{})",
                    file_name(path),
                    info.reduction.as_str(),
                    info.native.0,
                    info.native.1,
                    s,
                    img.width(),
                    img.height(),
                    phase.0,
                    phase.1,
                );
                manifest.items.retain(|it| it.file != name);
                manifest.items.push(real::Item {
                    file: name,
                    category,
                    license: license.to_string(),
                    source: format!(
                        "{} を{reduced}し，{} 倍へ拡大 ({how}．pxsmith-calib ingest)",
                        file_name(path),
                        s,
                    ),
                    truth: Some(real::Truth {
                        scale: s,
                        phase: Some(phase),
                    }),
                    no_grid: false,
                    // **正解の出どころを明記する** — 格子がこちらの拡大で作られたことが
                    // 読み手に分からないと誤解を招く
                    note: match info.reduction {
                        ingest::Reduction::Period(_) => None,
                        ingest::Reduction::ForcedPeriod(_) => Some(
                            "元絵に格子は無い．縮小の粗さを指定し，こちらが決めた倍率で\
                             拡大して格子を作った (正解は拡大倍率)"
                                .to_string(),
                        ),
                        ingest::Reduction::Recovered => Some(
                            "非整数倍で拡大されて配られていた絵から元絵を厳密に復元し，\
                             こちらが決めた倍率で拡大し直した．**中身は本物のドット絵**"
                                .to_string(),
                        ),
                        ingest::Reduction::AsIs => Some(
                            "配布されている元絵 (縮小していない) を，こちらが決めた\
                             倍率で拡大した．**中身は本物のドット絵**"
                                .to_string(),
                        ),
                    },
                });
                accepted += 1;
            }
        }
    }

    let json = serde_json::to_string_pretty(&manifest)?;
    pxsmith_io::atomic::write(dir.join("manifest.json"), json.as_bytes())?;
    println!(
        "\n  正例 {accepted} 件 / 負例 {negatives} 件 / 拒否 {} 件．目録は {} 件になった",
        refused.len(),
        manifest.items.len()
    );
    let no_period = refused
        .iter()
        .filter(|(_, r)| *r == ingest::Refusal::NoPeriod)
        .count();
    if negative_side.is_none() && no_period > 0 {
        println!(
            "  「周期が読めない」{no_period} 件は**負例になる**．\
             `--keep-refused` で目録へ入れられる"
        );
    }
    if refused.len() > no_period {
        println!(
            "  残り {} 件は格子が無い根拠にならない (大きさが枠外 / 縦横の食い違い)．\
             負例にしない",
            refused.len() - no_period
        );
    }
    Ok(())
}

/// `<sub>/<prefix>NNN.png` の次に使える連番．
fn next_index(manifest: &real::Manifest, sub: &str, prefix: &str) -> usize {
    let head = format!("{sub}/{prefix}");
    manifest
        .items
        .iter()
        .filter_map(|it| {
            let rest = it.file.strip_prefix(&head)?.strip_suffix(".png")?;
            // 負例の "neg-000" を正例の連番として数えないよう，数字だけの名前に限る
            rest.parse::<usize>().ok()
        })
        .max()
        .map_or(0, |n| n + 1)
}

fn file_name(p: &std::path::Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// 正解の格子で縮小したとき，元絵がどれだけ戻るか．
fn report_recover(records: &[recover::Record]) {
    println!(
        "\n== 正解の格子を与えて縮小したときの復元 ({} 件) ==",
        records.len()
    );
    println!("  **推定器は通していない．** 「格子が当たった場合の上限」である\n");
    println!(
        "  {:<10} {:>5}  {:>12}  {:>16}  {:>10}  {:>12}",
        "補間", "件数", "完全一致", "パレットへ寄せた後", "色差 (中央)", "色数 元絵→復元"
    );
    for f in ["nearest", "bilinear", "bicubic", "lanczos"] {
        let sub: Vec<&recover::Record> = records.iter().filter(|r| r.filter == f).collect();
        if sub.is_empty() {
            continue;
        }
        let med = |mut v: Vec<f32>| {
            v.sort_by(f32::total_cmp);
            v[v.len() / 2]
        };
        let exact = med(sub.iter().map(|r| r.exact).collect());
        let snapped = med(sub.iter().map(|r| r.exact_snapped).collect());
        let de = med(sub.iter().map(|r| r.median_delta_e).collect());
        let cs = med(sub.iter().map(|r| r.colors_source as f32).collect());
        let cr = med(sub.iter().map(|r| r.colors_recovered as f32).collect());
        // 完全一致が 99% を超えた件 = 実質的に元絵が戻った件
        // 寄せた後に 99% を超えた件 = 量子化まで通せば実質的に元絵が戻る件
        let clean = sub.iter().filter(|r| r.exact_snapped > 0.99).count();
        println!(
            "  {f:<10} {:>5}  {:>11.1}%  {:>15.1}%  {de:>10.4}  {cs:>5.0} → {cr:<5.0}  寄せて戻る {clean}/{}",
            sub.len(),
            exact * 100.0,
            snapped * 100.0,
            sub.len(),
        );
    }
    // 圧縮の寄与を分けて見る (nearest なら劣化は圧縮だけである)
    println!("\n  nearest の圧縮別 (劣化は圧縮だけ)");
    for c in ["png", "jpeg95", "jpeg80", "jpeg60"] {
        let sub: Vec<&recover::Record> = records
            .iter()
            .filter(|r| r.filter == "nearest" && r.compression == c)
            .collect();
        if sub.is_empty() {
            continue;
        }
        let mut v: Vec<f32> = sub.iter().map(|r| r.exact).collect();
        let mut w: Vec<f32> = sub.iter().map(|r| r.exact_snapped).collect();
        v.sort_by(f32::total_cmp);
        w.sort_by(f32::total_cmp);
        println!(
            "    {c:<8} {:>3} 件  完全一致 {:.1}%  → パレットへ寄せた後 {:.1}%",
            sub.len(),
            v[v.len() / 2] * 100.0,
            w[w.len() / 2] * 100.0,
        );
    }
}

/// 誤棄却がどの関門で落ちたかを数える．
fn report_diagnose(records: &[diagnose::Record]) {
    use diagnose::Fallout;
    let rejected: Vec<&diagnose::Record> = records
        .iter()
        .filter(|r| r.fallout != Fallout::NotRejected)
        .collect();
    println!(
        "\n== 正解が分かっている {} 件のうち，棄却された {} 件の落ち方 ==",
        records.len(),
        rejected.len()
    );
    for f in [
        Fallout::Epsilon,
        Fallout::Recon,
        Fallout::PhaseDrift,
        Fallout::LostToLarger,
        Fallout::LowConfidence,
        Fallout::NoCandidate,
    ] {
        let hit: Vec<&&diagnose::Record> = rejected.iter().filter(|r| r.fallout == f).collect();
        if hit.is_empty() {
            continue;
        }
        // ε で落ちた件は「どれだけ超えたか」が直し方に直結する
        let ratios: Vec<f32> = hit.iter().map(|r| r.epsilon_ratio).collect();
        let (q1, q2, q3) = bands::quartiles(&ratios);
        println!(
            "  {:<16} {:>3} 件   真の s の分散 / 閾値 = {q1:.2} / {q2:.2} / {q3:.2} (Q1/中央/Q3)",
            f.as_str(),
            hit.len(),
        );
    }

    // **落ち方は 1 つとは限らない．** 上の表は最初に落ちた関門しか数えないので，
    // 2 つの関門が同時に落としている件が先頭の関門に付け替えられる
    println!("\n  関門ごとの関与 (重複あり — 1 件が複数の関門で落ちうる)");
    for gate in diagnose::GATES {
        let n = rejected
            .iter()
            .filter(|r| r.failed_gates.split('|').any(|g| g == gate))
            .count();
        let only = rejected.iter().filter(|r| r.failed_gates == gate).count();
        println!("    {gate:<12} {n:>3} 件 (この関門だけで落ちている件 {only})");
    }
}

/// どの統計が「真の $s$」を見分けられるかを並べる．
/// 再現が本物の `estimate_grid` とどれだけ食い違うか．
///
/// **完全には一致しない．** `scale_candidates` は全 $s$ を素通しで評価するが，
/// `estimate_grid` は自己相関で絞ってから全探索へ落ちる．食い違いの数を出しておかないと，
/// 再現の上で見た差が «関門の差» なのか «再現の粗さ» なのか分からない．
fn report_replay_verify(
    dir: &std::path::Path,
    manifest: &dataset::Manifest,
    only: Option<Split>,
    cases: &[replay::Case],
    gates: &replay::Gates,
) -> Result<()> {
    use rayon::prelude::*;
    let params = pxsmith_core::grid::GridParams::default();
    let by_id: std::collections::BTreeMap<u32, &dataset::Item> =
        manifest.items.iter().map(|i| (i.id, i)).collect();

    let diffs: Vec<(u32, String)> = cases
        .par_iter()
        .filter(|c| only.is_none_or(|s| by_id[&c.item_id].split == s))
        .map(|case| -> Result<Option<(u32, String)>> {
            let item = by_id[&case.item_id];
            let img = pxsmith_io::png::read_rgba(dir.join(&item.file))?;
            let real = pxsmith_core::grid::estimate_grid(&img, &params)
                .ok()
                .map(|e| (e.scale, e.phase.x, e.phase.y));
            let mine = case
                .decide(gates)
                .map(|c| (c.scale, c.phase.0 as i32, c.phase.1 as i32));
            Ok((real != mine).then(|| (case.item_id, format!("{real:?} 対 {mine:?}"))))
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect();

    println!("\n== 本物の estimate_grid との食い違い ==");
    println!("  {} / {} 件", diffs.len(), cases.len());
    for (id, d) in diffs.iter().take(10) {
        println!("    {id:04}  {d}");
    }
    Ok(())
}

/// 真の $s$ を落としている関門を数える．**落ち方は 1 つとは限らない**ので，
/// 落ちた関門をすべて数える (最初の 1 つだけだと犯人を取り違える) ．
/// 掃引の格子を 1 軸ぶん広げる．**入れ子の for を積み上げない** — 軸を足すたびに
/// 段が深くなると，足し忘れや順序の取り違えが起きる．
fn expand<T: Copy>(
    grid: &[replay::Gates],
    values: &[T],
    set: impl Fn(&mut replay::Gates, T),
) -> Vec<replay::Gates> {
    let mut out = Vec::with_capacity(grid.len() * values.len());
    for &v in values {
        for g in grid {
            let mut g = *g;
            set(&mut g, v);
            out.push(g);
        }
    }
    out
}

fn report_replay_gates(cases: &[replay::Case], gates: &replay::Gates) {
    // 区分ごとに数える．**B が唯一の未達**なので，どの関門が B を落としているかが要る
    let mut counts: std::collections::BTreeMap<String, [usize; 4]> =
        std::collections::BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut alive = [0usize; 4];
    let mut total = [0usize; 4];
    for case in cases.iter().filter(|c| c.has_integer_grid) {
        let Some(truth) = case.cands.iter().find(|c| c.scale == case.truth_scale) else {
            continue;
        };
        let slot = case.tier().map_or(3, |t| (t as u8 - b'A') as usize);
        total[slot] += 1;
        total[3] += 1;
        let failed = truth.failed_gates(gates, case.image_var);
        if failed.is_empty() {
            alive[slot] += 1;
            alive[3] += 1;
        }
        for name in failed {
            let entry = counts.entry(name.to_string()).or_insert_with(|| {
                order.push(name.to_string());
                [0; 4]
            });
            entry[slot] += 1;
            entry[3] += 1;
        }
    }
    println!("\n== 真の $s$ を落とした関門 (格子あり {} 件) ==", total[3]);
    println!("  {:<32} {:>5} {:>5} {:>5} {:>5}", "", "計", "A", "B", "C");
    println!(
        "  {:<32} {:>5} {:>5} {:>5} {:>5} — **完全一致の上限**",
        "すべて通る", alive[3], alive[0], alive[1], alive[2],
    );
    for name in &order {
        let n = counts[name];
        println!(
            "  {name:<32} {:>5} {:>5} {:>5} {:>5}",
            n[3], n[0], n[1], n[2],
        );
    }

    // 関門を通ったのに取り逃した件．**ここは関門を掛け替えても動かない**
    println!("\n== 関門を通ったのに取り逃した件 ==");
    let mut any = false;
    for case in cases {
        let Some(reason) = case.missed_reason(gates) else {
            continue;
        };
        any = true;
        let conf = case
            .confidence_at(gates)
            .map(|(c, f, s)| format!("信頼度 {c:.4} / 下限 {f:.4} (s = {s})"))
            .unwrap_or_default();
        println!(
            "  {:04}  区分 {}  {:<9} {reason}  {conf}",
            case.item_id,
            case.tier().unwrap_or('-'),
            case.filter,
        );
    }
    if !any {
        println!("  無し");
    }
}

/// 関門の組を総当たりして，選択規則を回した完全一致数で並べる．
///
/// **分離能では選ばない** (課題分析と戦略 8 節) ．並べる鍵はマクロ平均だが，採否は
/// D66 の区分 (A ・B ・D) と実データ枠で決めるので，区分をそのまま出す．
fn report_replay_sweep(cases: &[replay::Case], grid: &[replay::Gates], top: usize) {
    let mut rows: Vec<(replay::Gates, replay::Score)> =
        grid.iter().map(|g| (*g, replay::score(cases, g))).collect();
    rows.sort_by(|a, b| {
        b.1.macro_rate()
            .total_cmp(&a.1.macro_rate())
            .then(b.1.exact.cmp(&a.1.exact))
    });

    let show = |(g, s): &(replay::Gates, replay::Score)| {
        let rescue = format!(
            "床 {:<5} 肩代 {} (曲線は 傾き {} 残差 {} 本数 {})",
            g.edge_drop_residual
                .map(|v| format!("{v}"))
                .unwrap_or_else(|| "-".to_string()),
            g.rescue.label(),
            g.edge_curve_slope,
            g.edge_curve_residual,
            g.edge_curve_min_count
        );
        println!(
            "  ε {:<5} τ {:<5} θ {:<5} 曲線 {:<5} λ {:<4} 軸 {:<4} 比 {:<5} 信 {:<5} | 境界 {} {:<8} {:<7} 傾き {:<6} 残差 {:<6} 本数 {:<3} 割合 {:<5} 肩代 {} | {}",
            g.epsilon,
            g.tau,
            g.phase_tolerance,
            g.phase_agreement,
            g.curve_lambda,
            g.curve_axis.as_str(),
            g.phase_contrast_min,
            g.min_confidence,
            g.edge_order,
            g.edge_mode.as_str(),
            g.edge_stat.as_str(),
            g.edge_slope,
            g.edge_residual,
            g.edge_min_count,
            g.edge_min_coverage,
            rescue,
            s.line(),
        );
    };

    println!("\n== 関門を掛け替えた結果 ({} 組) ==", rows.len());
    for row in rows.iter().take(top) {
        show(row);
    }

    // 採否の基準 — **測る前に決めておく** (課題分析と戦略 10 節 ・11 節) ．
    // **再現は本物より 1〜2 件甘い**ので，本物で B 40 / 50 を狙うなら再現では 41 を見る
    // (再現の現行は B 39 ・本物は 38) ．
    let ok: Vec<&(replay::Gates, replay::Score)> = rows
        .iter()
        .filter(|(_, s)| s.tier[1].0 >= 41 && s.tier[0].0 >= 25 && s.wrong <= 12)
        .collect();
    println!("\n== 採否の基準を満たす組 (B >= 41 ・A >= 25 ・D <= 12) ==");
    if ok.is_empty() {
        println!("  無し");
    }
    for row in ok.iter().take(top) {
        show(row);
    }
}

fn report_recon(records: &[recon::Record]) {
    let truth = records.iter().filter(|r| r.is_truth).count();
    println!(
        "\n== 再構成統計の分離能 ==\n  候補 {} 件 (うち真の s は {truth} 件)",
        records.len(),
    );
    println!("  統計                     閾値      均衡正解率");
    for (name, key) in [
        (
            "全画素の不一致率 (現行)",
            &(|r: &recon::Record| r.stats.overall) as &dyn Fn(&recon::Record) -> f32,
        ),
        ("セル内側の不一致率", &|r: &recon::Record| {
            r.stats.interior
        }),
        ("セル境界の不一致率", &|r: &recon::Record| {
            r.stats.border
        }),
        ("色差の中央値", &|r: &recon::Record| {
            r.stats.median_delta_e
        }),
        ("内側の色差の中央値", &|r: &recon::Record| {
            r.stats.interior_median_delta_e
        }),
        ("内側 / 境界の比", &|r: &recon::Record| {
            r.stats.interior / r.stats.border.max(1.0e-6)
        }),
        ("V(s) / V(s/2)", &|r: &recon::Record| {
            r.v / r.v_half.max(1.0e-9)
        }),
        ("V(s) - V(s/2)", &|r: &recon::Record| r.v - r.v_half),
    ] {
        let (t, acc) = recon::separation(records, key);
        println!("  {name:<24} {t:<9.4} {:.1}%", acc * 100.0);
    }

    // 補間ごとに見る — 落ちるのは補間が掛かった件だけだった
    println!("\n  補間別 (現行 / V(s)/V(s/2))");
    for f in ["nearest", "bilinear", "bicubic", "lanczos"] {
        let subset: Vec<recon::Record> =
            records.iter().filter(|r| r.filter == f).cloned().collect();
        if subset.is_empty() {
            continue;
        }
        let (_, a) = recon::separation(&subset, |r| r.stats.overall);
        let (_, b) = recon::separation(&subset, |r| r.v / r.v_half.max(1.0e-9));
        println!("    {f:<10} {:.1}% / {:.1}%", a * 100.0, b * 100.0);
    }

    report_profile(records);
}

/// 差分エネルギーの折り畳みが $s_*$ と $2 s_*$ を分けるか．
///
/// **対比を分けて見る．** 再構成検査は「倍数を止めると真の $s$ も落ちる」という
/// 一本の閾値だったが，止めたい相手は倍数で，通したい相手は真の $s$ と (負けてよい)
/// 約数である．全部混ぜた均衡正解率だけを見ると，倍数だけに効く量が埋もれる．
fn report_profile(records: &[recon::Record]) {
    type Key = dyn Fn(&recon::Record) -> f32;
    // **向きは「小さいほど真の $s$」に揃える** (separation がそう解釈する)
    let stats: [(&str, &Key); 8] = [
        ("全画素の不一致率 (現行)", &|r| r.stats.overall),
        ("セル内側の不一致率", &|r| r.stats.interior),
        ("段差の割合 (符号反転)", &|r| {
            -(r.profile.edge_share[0] + r.profile.edge_share[1]) / 2.0
        }),
        ("echo1 max(x,y)", &|r| {
            r.profile.echo1[0].max(r.profile.echo1[1])
        }),
        ("echo1 平均", &|r| {
            (r.profile.echo1[0] + r.profile.echo1[1]) / 2.0
        }),
        ("echo2 max(x,y)", &|r| {
            r.profile.echo2[0].max(r.profile.echo2[1])
        }),
        ("echo2 平均", &|r| {
            (r.profile.echo2[0] + r.profile.echo2[1]) / 2.0
        }),
        ("echo1 と echo2 の max", &|r| {
            r.profile.echo1[0]
                .max(r.profile.echo1[1])
                .max(r.profile.echo2[0])
                .max(r.profile.echo2[1])
        }),
    ];

    type Pick = dyn Fn(&recon::Record) -> bool;
    let contrasts: [(&str, &Pick); 4] = [
        ("すべての s", &|_| true),
        ("2 倍だけ", &|r| r.ratio() == 2.0),
        ("倍数すべて", &|r| r.is_multiple()),
        ("約数すべて", &|r| r.is_divisor()),
    ];

    println!("\n== 折り畳んだ差分エネルギーの分離能 (真の s vs …) ==");
    print!("  {:<26}", "統計");
    for (name, pick) in &contrasts {
        let n = records.iter().filter(|r| pick(r)).count();
        print!(" {:>14}", format!("{name} ({n})"));
    }
    println!();

    for (name, key) in &stats {
        print!("  {name:<26}");
        for (_, pick) in &contrasts {
            let subset: Vec<recon::Record> = records
                .iter()
                .filter(|r| r.is_truth || pick(r))
                .cloned()
                .collect();
            let (_, acc) = recon::separation(&subset, key);
            print!(" {:>13.1}%", acc * 100.0);
        }
        println!();
    }

    // 相関が当てにならない場面がどれだけあるか — 平らな形の相関は意味を持たない
    let flat1 = records
        .iter()
        .filter(|r| r.profile.relief1[0] < 0.05)
        .count();
    let flat2 = records
        .iter()
        .filter(|r| r.profile.relief2[0] < 0.05)
        .count();
    println!(
        "\n  折り畳みが平ら (起伏 < 0.05，x 軸): 1 階 {flat1} 件 / 2 階 {flat2} 件 (全 {} 件)",
        records.len()
    );
}

fn write_bands(path: &std::path::Path, records: &[bands::Record]) -> Result<()> {
    let mut text = String::from(bands::RECORD_HEADER);
    text.push('\n');
    for r in records {
        text.push_str(&r.to_csv());
        text.push('\n');
    }
    pxsmith_io::atomic::write(path, text.as_bytes())
        .with_context(|| format!("{} を書けない", path.display()))?;
    println!("{} 件を {} へ書いた", records.len(), path.display());
    Ok(())
}

/// 帯の統計が 2 種類の件を分けられているかを表に出す．
fn report_bands(records: &[bands::Record]) {
    let (accept, reject): (Vec<&bands::Record>, Vec<&bands::Record>) =
        records.iter().partition(|r| r.should_accept);
    println!(
        "\n== 帯ごとの再構成誤差 ==\n  受け入れるべき件 {} / 棄却すべき件 {}",
        accept.len(),
        reject.len()
    );

    let show = |name: &str, key: &dyn Fn(&bands::Record) -> f32| {
        let q = |rows: &[&bands::Record]| {
            let v: Vec<f32> = rows.iter().map(|r| key(r)).collect();
            bands::quartiles(&v)
        };
        let (a1, a2, a3) = q(&accept);
        let (r1, r2, r3) = q(&reject);
        println!(
            "  {name:<16} 受け入れ Q1/中央/Q3 = {a1:.3} / {a2:.3} / {a3:.3}   棄却 = {r1:.3} / {r2:.3} / {r3:.3}"
        );
    };
    show("全体の不一致率", &|r| r.overall);
    show("帯のばらつき", &|r| r.spread);
    show("相対ばらつき", &|r| r.relative_spread);
    show("傾き", &|r| r.slope);
    show("位相のずれ (画素)", &|r| r.phase_spread as f32);
    show("位相のずれ (正規化)", &|r| r.phase_drift);

    println!("\n  単一閾値で分けたときの均衡正解率 (0.5 = 分けられていない)");
    for (name, key) in [
        (
            "全体の不一致率 (現行)",
            &(|r: &bands::Record| r.overall) as &dyn Fn(&bands::Record) -> f32,
        ),
        ("帯のばらつき", &|r: &bands::Record| r.spread),
        ("相対ばらつき", &|r: &bands::Record| r.relative_spread),
        ("位相のずれ", &|r: &bands::Record| r.phase_drift),
        ("位相のずれ + 不一致率", &|r: &bands::Record| {
            r.phase_drift.max(r.overall * 2.0)
        }),
    ] {
        let (t, acc) = bands::best_threshold(records, key);
        println!("    {name:<22} 閾値 {t:.4} で {:.1}%", acc * 100.0);
    }
}

/// 分割ごとに指標を出し，検証セットで運転点を選ぶ．
/// 当てはめる信頼度の下限の水準．
///
/// **$\varepsilon$ ・$\delta$ ・$\tau$ と同時に選ぶ．** 実データで測ると正例の誤棄却は
/// 「閾値で落ちる件」と「信頼度で落ちる件」に割れており，2 つは逆を向いている —
/// 下限を下げれば正例が戻る代わりに負例の誤受理が増える．片方ずつ動かすと，
/// 一方を直して他方を壊した分が打ち消し合って見えなくなる．
/// **運転点の周りは細かく刻む．** 0.08 と 0.10 の間が空いていたため «信頼度の下限は
/// 掃いても動かない» と読めていたが，実際には 0.09 で真の $s$ が 2 件戻る (D72) ．
/// 下限は $\hat{s}$ で割って使うので，$\hat{s} = 2 \ldots 3$ では 0.01 の差が
/// 0.003 〜 0.005 の差になる — **格子が粗いと «動かない» が «測れていない» になる．**
const CONFIDENCE_LEVELS: [f32; 14] = [
    0.0, 0.005, 0.01, 0.02, 0.03, 0.05, 0.08, 0.085, 0.09, 0.095, 0.10, 0.11, 0.12, 0.20,
];

fn report(dir: &std::path::Path, rows: &[Row], target: f32, top: usize) -> Result<()> {
    let validation: Vec<Row> = rows
        .iter()
        .filter(|r| r.split == Split::Validation)
        .cloned()
        .collect();
    anyhow::ensure!(
        !validation.is_empty(),
        "掃引 CSV に検証セットの行が無い．閾値は検証セットで決める"
    );

    // 閾値の組 x 信頼度の下限を同時に見る．下限は再推定なしで当てはめられる
    let summaries: Vec<Summary> = CONFIDENCE_LEVELS
        .iter()
        .flat_map(|&c| metrics::summarize_at(&validation, c))
        .collect();
    write_summary(&dir.join("summary.csv"), &summaries)?;

    // マクロ平均が最大のものを採る (素の正解率で選ばない理由は Summary::macro_rate)．
    // 同率なら ε ・δ ・τ が小さい方 — 閾値を緩めるほど過大推定の危険が上がるので，
    // 同じ成績なら緩めない
    let best = summaries
        .iter()
        .max_by(|a, b| {
            a.macro_rate()
                .total_cmp(&b.macro_rate())
                .then(b.epsilon.total_cmp(&a.epsilon))
                .then(b.delta.total_cmp(&a.delta))
                .then(b.tau.total_cmp(&a.tau))
                .then(b.min_confidence.total_cmp(&a.min_confidence))
        })
        .context("まとめが空である")?;

    println!(
        "\n== 検証セット 上位 {top} 件 (マクロ平均の順．ε は{}) ==",
        if summaries[0].normalized {
            "画像分散に対する割合"
        } else {
            "分散の絶対値"
        }
    );
    println!(
        "  ε        δ     τ     信頼度 マクロ  正解率  ECE    | 格子あり: 完全一致  s一致  誤棄却 | 格子なし: 正棄却 (惜しい誤答)"
    );
    let mut ranked: Vec<&Summary> = summaries.iter().collect();
    ranked.sort_by(|a, b| b.macro_rate().total_cmp(&a.macro_rate()));
    for s in ranked.iter().take(top) {
        println!(
            "  {:<8} {:<5} {:<5} {:<5} {:>5.1}% {:>6.1}% {:>6.3} | {:>15.1}% {:>6.1}% {:>6.1}% | {:>13.1}% ({:.1}%)",
            s.epsilon,
            s.delta,
            s.tau,
            s.min_confidence,
            s.macro_rate() * 100.0,
            s.correct_rate * 100.0,
            s.ece,
            s.grid_exact_rate * 100.0,
            s.grid_scale_rate * 100.0,
            s.grid_false_reject_rate * 100.0,
            s.resized_reject_rate * 100.0,
            s.resized_effective_rate * 100.0,
        );
    }

    // 選んだ組の risk-coverage 曲線を書き出す (全件が対象)
    let best_rows: Vec<&Row> = validation
        .iter()
        .filter(|r| r.param_id == best.param_id)
        .collect();
    let curve = metrics::risk_coverage(&best_rows);
    write_curve(&dir.join("risk-coverage.csv"), best.param_id, &curve)?;

    println!(
        "\n== 選んだ閾値 (検証セット) ==\n  ε = {}{}, δ = {}, τ = {}, min_confidence = {}\n  マクロ平均 {:.1}% / 正解率 {:.1}% ({} 件) / ECE {:.3}\n  格子あり {} 件: 完全一致 {:.1}% / s 一致 {:.1}% / 誤棄却 {:.1}%\n  格子なし {} 件: 正しい棄却 {:.1}%",
        best.epsilon,
        if best.normalized { " (割合)" } else { "" },
        best.delta,
        best.tau,
        best.min_confidence,
        best.macro_rate() * 100.0,
        best.correct_rate * 100.0,
        best.n,
        best.ece,
        best.grid_n,
        best.grid_exact_rate * 100.0,
        best.grid_scale_rate * 100.0,
        best.grid_false_reject_rate * 100.0,
        best.resized_n,
        best.resized_reject_rate * 100.0,
    );

    println!(
        "  D66 の区分: {}",
        metrics::tiers(&best_rows, best.min_confidence).line()
    );

    // **自動で選んだ組と出荷の既定値は一致しないことがある．**
    // マクロ平均だけで選ぶと実データ枠の後退が見えない (D69 ・D72) ので，
    // 既定値での成績も必ず並べる — 黙って «最良» へ寄せ直さないための歯止めである
    let shipped = pxsmith_core::grid::GridParams::default().min_confidence;
    if (shipped - best.min_confidence).abs() > f32::EPSILON {
        println!(
            "  ※ 出荷の既定は min_confidence = {shipped} である (この表の最良と違う) ．\n     既定での区分: {}\n     **実データ枠を見て決めた値なので，マクロ平均だけで書き換えないこと** (grid.rs の doc を読む)",
            metrics::tiers(&best_rows, shipped).line()
        );
    }

    match metrics::operating_point(&curve, target) {
        Some(p) => println!(
            "  min_confidence = {:.2} で誤り {:.1}% / 採用 {:.1}% ({} 件)  ← 目標 {:.0}% を満たす最小の閾値",
            p.threshold,
            p.risk * 100.0,
            p.coverage * 100.0,
            p.n_covered,
            target * 100.0,
        ),
        None => println!(
            "  目標 {:.0}% を満たす信頼度の閾値が無い．**この組では運転点を選べない**",
            target * 100.0
        ),
    }

    // テストセットが掃引に含まれていれば，選んだ組の成績だけを報告する
    let test: Vec<Row> = rows
        .iter()
        .filter(|r| r.split == Split::Test && r.param_id == best.param_id)
        .cloned()
        .collect();
    if !test.is_empty() {
        let refs: Vec<&Row> = test.iter().collect();
        // **出荷の既定で数える．** 自動で選んだ組ではなく «実際に出荷する運転点» が
        // テストセットでどうなるかが完了条件である
        println!(
            "  (テストセットの D66 区分 ・既定 min_confidence = {shipped}: {})",
            metrics::tiers(&refs, shipped).line()
        );
    }
    if test.is_empty() {
        println!(
            "\n(テストセットは掃引に含まれていない．`sweep --split test` を選んだ組だけで回すこと．\n 検証セットで決めた閾値をテストセットで選び直してはいけない)"
        );
    } else {
        // **信頼度の下限を当てはめてから数える．** `summarize` は下限 0 で数えるので，
        // そのまま出すと «運転点を通していない» 数字になる (M2 の完了条件を誤って報告する)
        let t = &metrics::summarize_at(&test, shipped)[0];
        println!(
            "\n== テストセット (選んだ組をそのまま適用) ==\n  マクロ平均 {:.1}% / 正解率 {:.1}% ({} 件) / ECE {:.3}\n  格子あり {} 件: 完全一致 {:.1}% / 誤棄却 {:.1}%\n  格子なし {} 件: 正しい棄却 {:.1}%",
            t.macro_rate() * 100.0,
            t.correct_rate * 100.0,
            t.n,
            t.ece,
            t.grid_n,
            t.grid_exact_rate * 100.0,
            t.grid_false_reject_rate * 100.0,
            t.resized_n,
            t.resized_reject_rate * 100.0,
        );
    }

    Ok(())
}

fn write_summary(path: &std::path::Path, summaries: &[Summary]) -> Result<()> {
    let mut text = String::from(metrics::SUMMARY_HEADER);
    text.push('\n');
    for s in summaries {
        text.push_str(&s.to_csv());
        text.push('\n');
    }
    pxsmith_io::atomic::write(path, text.as_bytes())
        .with_context(|| format!("{} を書けない", path.display()))?;
    println!(
        "{} 組のまとめを {} へ書いた",
        summaries.len(),
        path.display()
    );
    Ok(())
}

fn write_curve(path: &std::path::Path, param_id: usize, curve: &[metrics::Point]) -> Result<()> {
    let mut text = String::from(metrics::CURVE_HEADER);
    text.push('\n');
    for p in curve {
        text.push_str(&format!(
            "{},{:.2},{:.4},{:.4},{}\n",
            param_id, p.threshold, p.coverage, p.risk, p.n_covered
        ));
    }
    pxsmith_io::atomic::write(path, text.as_bytes())
        .with_context(|| format!("{} を書けない", path.display()))?;
    println!("risk-coverage 曲線を {} へ書いた", path.display());
    Ok(())
}

fn write_confidence(path: &std::path::Path, records: &[confidence::Record]) -> Result<()> {
    let mut text = String::from(confidence::HEADER);
    text.push('\n');
    for r in records {
        text.push_str(&r.to_csv());
        text.push('\n');
    }
    pxsmith_io::atomic::write(path, text.as_bytes())
        .with_context(|| format!("{} を書けない", path.display()))?;
    println!("{} 件を {} へ書いた", records.len(), path.display());
    Ok(())
}

/// 正解した件と誤答とで，信頼度の各項がどう違うのかを並べる．
fn report_confidence(records: &[confidence::Record]) {
    let (right, wrong): (Vec<&confidence::Record>, Vec<&confidence::Record>) =
        records.iter().partition(|r| r.correct);
    println!(
        "\n== 信頼度の中身 ==\n  答えを返した {} 件 (正解 {} / 誤答 {})",
        records.len(),
        right.len(),
        wrong.len()
    );

    let show = |name: &str, key: &dyn Fn(&confidence::Record) -> f32| {
        let q = |rows: &[&confidence::Record]| {
            let v: Vec<f32> = rows.iter().map(|r| key(r)).collect();
            bands::quartiles(&v)
        };
        let (a1, a2, a3) = q(&right);
        let (b1, b2, b3) = q(&wrong);
        println!(
            "  {name:<22} 正解 Q1/中央/Q3 = {a1:.5} / {a2:.5} / {a3:.5}   誤答 = {b1:.5} / {b2:.5} / {b3:.5}"
        );
    };
    show("信頼度", &|r| r.confidence);
    show("分子 (マージン)", &|r| r.margin);
    show("分母 (画像の分散)", &|r| r.v_image);
    show("V(ŝ)", &|r| r.v_hat);
    show("V(対戦相手)", &|r| r.v_rival);
    show("マージン / V(ŝ)", &|r| r.relative_margin);

    let clamped = |rows: &[&confidence::Record]| {
        rows.iter().filter(|r| r.margin <= 0.0).count() as f32 / rows.len().max(1) as f32
    };
    println!(
        "\n  マージンが 0 以下 (信頼度が 0 に潰れた) 割合: 正解 {:.1}% / 誤答 {:.1}%",
        clamped(&right) * 100.0,
        clamped(&wrong) * 100.0
    );

    let mut rivals: std::collections::BTreeMap<u32, usize> = std::collections::BTreeMap::new();
    for r in &right {
        *rivals.entry(r.rival_scale).or_default() += 1;
    }
    println!("  正解した件の対戦相手 (対照群の最小): {rivals:?}");
    rivals.clear();
    for r in &wrong {
        *rivals.entry(r.rival_scale).or_default() += 1;
    }
    println!("  誤答の対戦相手: {rivals:?}");

    for (name, key) in [
        (
            "信頼度 (現行)",
            &(|r: &confidence::Record| -r.confidence) as &dyn Fn(&confidence::Record) -> f32,
        ),
        ("マージン / V(ŝ)", &|r: &confidence::Record| {
            -r.relative_margin
        }),
        ("V(ŝ) の小ささ", &|r: &confidence::Record| r.v_hat),
    ] {
        let recs: Vec<bands::Record> = records
            .iter()
            .map(|r| bands::Record {
                item_id: r.item_id,
                has_integer_grid: r.has_integer_grid,
                filter: String::new(),
                resize: String::new(),
                compression: String::new(),
                truth_scale: r.truth_scale,
                scale_hat: r.scale_hat,
                should_accept: r.correct,
                overall: key(r),
                spread: 0.0,
                relative_spread: 0.0,
                slope: 0.0,
                phase_spread: 0,
                phase_drift: 0.0,
                by_x: Vec::new(),
                by_y: Vec::new(),
                phase_by_x: Vec::new(),
                phase_by_y: Vec::new(),
            })
            .collect();
        let (t, acc) = bands::best_threshold(&recs, |r| r.overall);
        println!(
            "  {name:<18} 単一閾値の均衡正解率 {:.1}% (閾値 {t:.5})",
            acc * 100.0
        );
    }
}

/// ファイル名だけを取り出す (表示用)．
fn name_of(path: &std::path::Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// **中間色として置かれている画素の割合** (不透明画素に対する)．ルール 14 の特徴量．
///
/// 数え方は **`pxsmith clean --remove-aa` が外す画素**そのものである — 付ける側 ・
/// 外す側 ・数える側で «中間色» の定義がずれると，付けた AA を自分の道具で外せなく
/// なる (D83 で実際に壊れた) ．
///
/// > [!warning] [`pxsmith_core::aa::strip_aa`] では数えられない．
/// > あちらは «8 近傍に現れる 2 色の間にあるか» を画素ごとに見るだけなので，
/// > 密な質感では色の大半が誰かの «間» に入る — 良い絵 61 枚で**中央 17.5% ・
/// > 最大 67.0%** になった．`pxsmith aa` が冪等性のために «元の角» を復元する用途では
/// > それでよいが，**割合の分子には使えない**．
fn aa_ratio(
    canvas: &pxsmith_core::canvas::IndexedCanvas,
    palette: &pxsmith_core::palette::Palette,
) -> (f32, usize) {
    let opaque = canvas
        .pixels()
        .iter()
        .filter(|i| canvas.transparent() != Some(**i))
        .count();
    if opaque == 0 {
        return (0.0, 0);
    }
    let mut areas: std::collections::BTreeMap<u8, u32> = Default::default();
    for &i in canvas.pixels() {
        if canvas.transparent() != Some(i) {
            *areas.entry(i).or_default() += 1;
        }
    }
    let tolerance = pxsmith_core::clean::AaOptions::default().tolerance;
    let (mut count, mut colours) = (0u32, 0usize);
    for (&i, &area) in &areas {
        let Some(mid) = palette.lab_of(i) else {
            continue;
        };
        let is_between = areas.iter().any(|(&a, &na)| {
            na > area
                && areas.iter().any(|(&b, &nb)| {
                    if b <= a || b == i || a == i || nb <= area {
                        return false;
                    }
                    let (Some(la), Some(lb)) = (palette.lab_of(a), palette.lab_of(b)) else {
                        return false;
                    };
                    let midpoint = pxsmith_core::color::Oklab::new(
                        (la.l + lb.l) * 0.5,
                        (la.a + lb.a) * 0.5,
                        (la.b + lb.b) * 0.5,
                    );
                    pxsmith_core::color::distance_sq(mid, midpoint, 1.0).sqrt() <= tolerance
                })
        });
        if is_between {
            count += area;
            colours += 1;
        }
    }
    (count as f32 / opaque as f32, colours)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_numbering_continues_after_what_is_already_there() {
        // 同じ区分へ 2 回取り込んでも 1 回目を上書きしないこと
        let item = |file: &str| real::Item {
            file: file.to_string(),
            category: real::Category::Other,
            license: String::new(),
            source: String::new(),
            truth: None,
            no_grid: false,
            note: None,
        };
        let m = real::Manifest {
            items: vec![
                item("other/000.png"),
                item("other/001.png"),
                item("other/neg-000.png"),
                item("ai-output/007.png"),
            ],
        };
        assert_eq!(next_index(&m, "other", ""), 2);
        assert_eq!(next_index(&m, "other", "neg-"), 1);
        // 区分をまたいで数えない
        assert_eq!(next_index(&m, "screenshot", ""), 0);
        assert_eq!(
            next_index(&real::Manifest { items: vec![] }, "other", ""),
            0
        );
    }

    #[test]
    fn empty_arguments_fall_back_to_the_default_levels() {
        assert_eq!(or_default(vec![], &[1.0, 2.0]), vec![1.0, 2.0]);
        assert_eq!(or_default(vec![9.0], &[1.0, 2.0]), vec![9.0]);
    }

    #[test]
    fn the_cli_parses_the_documented_invocations() {
        // 使い方の 3 行がそのまま通ること
        for args in [
            vec!["pxsmith-calib", "gen"],
            vec!["pxsmith-calib", "sweep"],
            vec!["pxsmith-calib", "report"],
            vec![
                "pxsmith-calib",
                "sweep",
                "--split",
                "test",
                "--epsilon",
                "0.005",
            ],
        ] {
            Cli::try_parse_from(args).expect("引数を解釈できない");
        }
    }
}
