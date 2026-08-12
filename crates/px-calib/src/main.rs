//! `px-calib` — 閾値を決めるための校正ツール (D59)．
//!
//! **出荷物ではない．** `pyo3` + `maturin` で Python から呼ぶ案を採らず，校正を Rust に
//! 閉じるための道具である．指標は CSV へ書き出し，作図は任意のツールに任せる．
//!
//! ```sh
//! # 1. 合成 500 件を作る (正解つき)
//! cargo run -p px-calib --release -- gen
//! # 2. 閾値を掃引する (検証セット 300 件)
//! cargo run -p px-calib --release -- sweep
//! # 3. 指標を出して運転点を選ぶ
//! cargo run -p px-calib --release -- report
//! ```
//!
//! `sweep` は件数 x パラメータ組の回数だけ格子推定を回すので，**必ず `--release` で
//! 実行する**．

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

mod bands;
mod confidence;
mod dataset;
mod degrade;
mod diagnose;
mod ingest;
mod lintcal;
mod metrics;
mod real;
mod recon;
mod recover;
mod render;
mod rng;
mod scene;
mod sprite;
mod sweep;

use dataset::Split;
use metrics::Summary;
use sweep::{ParamGrid, Row};

/// 既定の作業場所．
const DEFAULT_DIR: &str = "grid-eval";

#[derive(Parser)]
#[command(
    name = "px-calib",
    version,
    about = "格子推定の閾値を決めるための校正ツール"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

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
                    .unwrap_or(px_core::grid::GridParams::default().normalize_epsilon),
                epsilons: or_default(epsilon, &default.epsilons),
                deltas: or_default(delta, &default.deltas),
                taus: or_default(tau, &default.taus),
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
            let params = px_core::grid::GridParams {
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
                px_io::png::write_rgba(out.join(&name), &scene.canvas)?;
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
            phase_tolerance,
            phase_agreement,
            phase_contrast_min,
            phase_tolerance_floor,
            allow_unmeasurable,
        } => {
            let d = px_core::grid::GridParams::default();
            let params = px_core::grid::GridParams {
                epsilon: epsilon.unwrap_or(d.epsilon),
                delta: delta.unwrap_or(d.delta),
                tau: tau.unwrap_or(d.tau),
                min_confidence: min_confidence.unwrap_or(d.min_confidence),
                normalize_epsilon: normalize_epsilon.unwrap_or(d.normalize_epsilon),
                phase_tolerance: phase_tolerance.unwrap_or(d.phase_tolerance),
                phase_agreement: phase_agreement.unwrap_or(d.phase_agreement),
                phase_contrast_min: phase_contrast_min.unwrap_or(d.phase_contrast_min),
                phase_tolerance_floor: phase_tolerance_floor.unwrap_or(d.phase_tolerance_floor),
                phase_require_measurable: !allow_unmeasurable.unwrap_or(false),
                ..d
            };
            println!(
                "ε = {}{} / δ = {} / τ = {} / min_confidence = {} / θ = {} / 曲線 {} / 測れない候補を{}\n",
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
            px_io::atomic::write(&path, text.as_bytes())
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
        } => {
            let manifest = dataset::read(&dir)?;
            let only = parse_split(&split)?;
            let params = px_core::grid::GridParams::default();
            let records = recon::run(&dir, &manifest, only, &params, include_resized)?;
            let path = out.unwrap_or_else(|| dir.join("recon.csv"));
            let mut text = String::from(recon::HEADER);
            text.push('\n');
            for r in &records {
                text.push_str(&r.to_csv());
                text.push('\n');
            }
            px_io::atomic::write(&path, text.as_bytes())?;
            report_recon(&records);
            println!("\n{} 行を {} へ書いた", records.len(), path.display());
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
            px_io::atomic::write(&path, text.as_bytes())?;
            report_recover(&records);
            println!("\n{} 件を {} へ書いた", records.len(), path.display());
        }

        Command::Diagnose { dir, out } => {
            let manifest = real::read(&dir)?;
            let params = px_core::grid::GridParams::default();
            let records = diagnose::run(&dir, &manifest, &params)?;
            let path = out.unwrap_or_else(|| dir.join("diagnose.csv"));
            let mut text = String::from(diagnose::HEADER);
            text.push('\n');
            for r in &records {
                text.push_str(&r.to_csv());
                text.push('\n');
            }
            px_io::atomic::write(&path, text.as_bytes())?;
            report_diagnose(&records);
            println!("\n{} 件を {} へ書いた", records.len(), path.display());
        }

        Command::Lint { dir, out } => {
            let cfg = px_lint::LintConfig::default();
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
                        px_lint::rule(*id)
                            .is_some_and(|x| matches!(x.severity, px_lint::Severity::Blocking))
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
            px_io::atomic::write(&path, text.as_bytes())?;
            println!("  {} 行を {} へ書いた", records.len(), path.display());
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
        px_io::png::write_rgba(dir.join(&file), &img)?;

        items.push(real::Item {
            file,
            category: real::Category::Render,
            license: "CC0 (自作)".to_string(),
            source: format!(
                "自作 — px-calib render (種 {seed}, {} 倍 / {} / リサイズ {} / {})",
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
    px_io::atomic::write(dir.join("manifest.json"), json.as_bytes())?;
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
                    px_io::png::write_rgba(dir.join(&name), &img)?;
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
                px_io::png::write_rgba(dir.join(&name), &img)?;
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
                        "{} を{reduced}し，{} 倍へ拡大 ({how}．px-calib ingest)",
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
    px_io::atomic::write(dir.join("manifest.json"), json.as_bytes())?;
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
    px_io::atomic::write(path, text.as_bytes())
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
const CONFIDENCE_LEVELS: [f32; 10] = [0.0, 0.005, 0.01, 0.02, 0.03, 0.05, 0.08, 0.10, 0.12, 0.20];

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
    if test.is_empty() {
        println!(
            "\n(テストセットは掃引に含まれていない．`sweep --split test` を選んだ組だけで回すこと．\n 検証セットで決めた閾値をテストセットで選び直してはいけない)"
        );
    } else {
        let t = &metrics::summarize(&test)[0];
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
    px_io::atomic::write(path, text.as_bytes())
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
    px_io::atomic::write(path, text.as_bytes())
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
    px_io::atomic::write(path, text.as_bytes())
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
            vec!["px-calib", "gen"],
            vec!["px-calib", "sweep"],
            vec!["px-calib", "report"],
            vec!["px-calib", "sweep", "--split", "test", "--epsilon", "0.005"],
        ] {
            Cli::try_parse_from(args).expect("引数を解釈できない");
        }
    }
}
