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
mod ingest;
mod metrics;
mod real;
mod render;
mod rng;
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
        /// 帯どうしの位相のずれの許容 ($s$ に対する割合)
        #[arg(long, default_value_t = 1.0 / 6.0)]
        phase_tolerance: f32,
        /// 位相ずれ検査に要る帯あたりのセル数
        #[arg(long, default_value_t = 2)]
        phase_min_cells: usize,
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
        Command::Gen { dir, seed, count } => {
            let manifest = dataset::build(seed, count);
            dataset::generate(&dir, &manifest)?;
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
            phase_min_cells,
        } => {
            let manifest = dataset::read(&dir)?;
            let only = parse_split(&split)?;

            let default = ParamGrid::default();
            let grid = ParamGrid {
                max_scale,
                phase_bands,
                phase_tolerance,
                phase_min_cells,
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
        } => {
            ingest_images(&dir, &images, scale, crop.as_deref(), &category, &license)?;
        }

        Command::Render { dir, count, seed } => {
            let items = render_real_items(&dir, count, seed)?;
            println!("{} 件を {} へ書いた", items, dir.display());
        }

        Command::Real { dir, out } => {
            let manifest = real::read(&dir)?;
            let outcomes = real::run(&dir, &manifest, &px_core::grid::GridParams::default())?;
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
            let unknown = outcomes
                .iter()
                .filter(|o| o.verdict == real::Verdict::Unknown)
                .count();
            println!(
                "\n  正解が分かっている {} 件 / 人が見る {unknown} 件．\n  **率で語らないこと** — 20〜30 件では 1 件が 3〜5% 動く",
                outcomes.len() - unknown
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
            // 非整数倍リサイズを挟んだ件に整数の格子は無い．正解を書かない
            truth: degradation.truth_phase().map(|p| real::Truth {
                scale: degradation.scale,
                phase: Some(p),
            }),
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

/// ドット絵風の画像を正例へ仕立てて目録へ足す．
///
/// **拒否したものは黙って捨てない** — 理由を出す．負例の候補になる．
fn ingest_images(
    dir: &std::path::Path,
    images: &[PathBuf],
    scale: Option<u32>,
    crop: Option<&str>,
    category: &str,
    license: &str,
) -> Result<()> {
    anyhow::ensure!(!images.is_empty(), "入力画像を 1 つ以上指定すること");
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
    let mut accepted = 0usize;
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

        match ingest::ingest_one(path, s, c)? {
            Err(reason) => {
                println!("  拒否 {:<40} {reason}", file_name(path));
                refused.push((path.clone(), reason));
            }
            Ok((img, info)) => {
                let name = format!("{sub}/{:03}.png", accepted);
                px_io::png::write_rgba(dir.join(&name), &img)?;
                let phase = ingest::truth_phase(s, c);
                println!(
                    "  取込 {:<40} 周期 {:>2} → 元絵 {}x{} → {} 倍 ({}x{}) 位相 ({},{})",
                    file_name(path),
                    info.period,
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
                        "{} を周期 {} で縮小し {} 倍へ拡大 (px-calib ingest)",
                        file_name(path),
                        info.period,
                        s
                    ),
                    truth: Some(real::Truth {
                        scale: s,
                        phase: Some(phase),
                    }),
                    note: None,
                });
                accepted += 1;
            }
        }
    }

    let json = serde_json::to_string_pretty(&manifest)?;
    px_io::atomic::write(dir.join("manifest.json"), json.as_bytes())?;
    println!(
        "\n  取り込み {accepted} 件 / 拒否 {} 件．目録は {} 件になった",
        refused.len(),
        manifest.items.len()
    );
    if !refused.is_empty() {
        println!("  拒否したものは**負例の候補**である．捨てずに取っておくこと");
    }
    Ok(())
}

fn file_name(p: &std::path::Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
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

    let summaries = metrics::summarize(&validation);
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
        })
        .context("まとめが空である")?;

    println!("\n== 検証セット 上位 {top} 件 (マクロ平均の順) ==");
    println!(
        "  ε        δ     τ     マクロ  正解率  ECE    | 格子あり: 完全一致  s一致  誤棄却 | 格子なし: 正棄却 (惜しい誤答)"
    );
    let mut ranked: Vec<&Summary> = summaries.iter().collect();
    ranked.sort_by(|a, b| b.macro_rate().total_cmp(&a.macro_rate()));
    for s in ranked.iter().take(top) {
        println!(
            "  {:<8} {:<5} {:<5} {:>5.1}% {:>6.1}% {:>6.3} | {:>15.1}% {:>6.1}% {:>6.1}% | {:>13.1}% ({:.1}%)",
            s.epsilon,
            s.delta,
            s.tau,
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
        "\n== 選んだ閾値 (検証セット) ==\n  ε = {}, δ = {}, τ = {}\n  マクロ平均 {:.1}% / 正解率 {:.1}% ({} 件) / ECE {:.3}\n  格子あり {} 件: 完全一致 {:.1}% / s 一致 {:.1}% / 誤棄却 {:.1}%\n  格子なし {} 件: 正しい棄却 {:.1}%",
        best.epsilon,
        best.delta,
        best.tau,
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
