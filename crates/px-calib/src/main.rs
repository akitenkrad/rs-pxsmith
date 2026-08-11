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

mod dataset;
mod degrade;
mod metrics;
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
        } => {
            let manifest = dataset::read(&dir)?;
            let only = match split.as_str() {
                "validation" => Some(Split::Validation),
                "test" => Some(Split::Test),
                "all" => None,
                other => anyhow::bail!("分割は validation / test / all のいずれか: {other}"),
            };

            let default = ParamGrid::default();
            let grid = ParamGrid {
                max_scale,
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
