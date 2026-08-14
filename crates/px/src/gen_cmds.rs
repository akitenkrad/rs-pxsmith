//! 生成 AI 連携のサブコマンド (M6．設計書 8 章)．
//!
//! **書いたのは経路 B (`px gen prog`) だけである** (D156) ．`px gen image` は
//! 8.1 «汎用拡散モデルの出力は非一様格子になることがあり，その場合は棄却する»
//! のとおり成功率がモデル依存で上限が読めないので後回しにした — D92 の作法で
//! «書いていない» と報告する．

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use px_gen::anthropic::AnthropicGenerator;
use px_gen::request::{Backend, Constraints, Effort, GenKind, GenRequest};
use px_gen::session::{Session, describe_frames, now_iso8601};
use px_gen::{Attempt, Generator};

#[derive(Subcommand)]
pub enum GenCommand {
    /// LLM に L0 を書かせる (経路 B．設計書 8.2)
    Prog {
        #[command(flatten)]
        args: ProgArgs,
    },
}

/// `px gen prog` の引数．
///
/// **バックエンドもモデルも宣言させる** (D156 ・D89) — 推測した宣言は外れた
/// ときだけ静かに壊れる．
#[derive(Args, Clone)]
pub struct ProgArgs {
    /// 出力先 (`.px.toml`)．パレットと素性は隣に置く
    pub output: PathBuf,
    /// 作らせるものの説明
    #[arg(long)]
    pub prompt: String,
    /// 使ってよい色 `RRGGBB,RRGGBB,...`．**L0 は 62 色まで**
    #[arg(long, value_delimiter = ',')]
    pub palette: Vec<String>,
    /// 大きさ `WxH`
    #[arg(long, default_value = "16x16")]
    pub size: String,
    /// コマ数
    #[arg(long, default_value_t = 1)]
    pub frames: u32,
    /// モデル
    #[arg(long, default_value = "claude-opus-5")]
    pub model: String,
    /// 叩き先
    #[arg(long, default_value = "https://api.anthropic.com/v1/messages")]
    pub endpoint: String,
    /// 思考の深さ
    #[arg(long, default_value = "high", value_parser = ["low", "medium", "high", "xhigh", "max"])]
    pub effort: String,
    /// 検査に落ちたときに作り直す上限 (設計書 8.2 の n_max)
    #[arg(long, default_value_t = 3)]
    pub attempts: u32,
    /// 叩かずに，何をどう頼むかだけ出す
    #[arg(long)]
    pub dry_run: bool,
}

pub fn generate(command: GenCommand) -> Result<()> {
    match command {
        GenCommand::Prog { args } => prog(&args),
    }
}

fn parse_size(spec: &str) -> Result<(u32, u32)> {
    let (w, h) = spec
        .split_once(['x', 'X'])
        .with_context(|| format!("大きさ '{spec}' を読めない (`WxH` のはず)"))?;
    Ok((
        w.trim().parse().with_context(|| format!("幅 '{w}'"))?,
        h.trim().parse().with_context(|| format!("高さ '{h}'"))?,
    ))
}

fn prog(args: &ProgArgs) -> Result<()> {
    if args.palette.is_empty() {
        bail!(
            "--palette が空である．**使ってよい色は絵からは決まらないので宣言させる** \
             (D89) — 例: --palette 1a1c2c,566c86,b13e53,f4f4f4"
        );
    }
    if args.palette.len() > 62 {
        bail!(
            "{} 色を宣言しているが L0 の上限は 62 色である (使える文字が 0-9 a-z A-Z のため)",
            args.palette.len()
        );
    }
    let (width, height) = parse_size(&args.size)?;
    let effort = Effort::parse(&args.effort)
        .with_context(|| format!("思考の深さ '{}' を知らない", args.effort))?;

    let req = GenRequest {
        kind: GenKind::Prog,
        backend: Backend {
            name: if args.endpoint.contains("api.anthropic.com") {
                "anthropic".to_string()
            } else {
                "custom".to_string()
            },
            endpoint: args.endpoint.clone(),
            model: args.model.clone(),
        },
        effort,
        prompt: args.prompt.clone(),
        constraints: Constraints {
            width,
            height,
            palette: args.palette.clone(),
            frames: args.frames,
        },
        max_attempts: args.attempts,
    };

    let (dir, stem) = split_output(&args.output)?;
    let session = Session::prepare(&dir, &stem, &req)?;

    println!("  {} ・{}", req.backend.model, req.backend.endpoint);
    println!(
        "    {}x{} ・{} コマ ・{} 色 ・思考 {} ・作り直し上限 {}",
        width,
        height,
        args.frames,
        args.palette.len(),
        effort.as_str(),
        args.attempts
    );
    println!("    依頼の鍵 {}", &req.key()[..16]);
    println!(
        "    パレット -> {}",
        session.l0_path().with_extension("hex").display()
    );

    if args.dry_run {
        println!(
            "\n  (--dry-run なので叩かない)．**この依頼には種も温度も無い** — \n\
             \u{3000}\u{3000}同じ鍵でも同じ絵が返る保証は無く，決定論を担うのはキャッシュである (D157)"
        );
        return Ok(());
    }

    let backend = AnthropicGenerator::from_env(&session.palette_ref())?;
    let report = session.run(&backend, &req, &now_iso8601())?;

    for (i, attempt) in report.attempts.iter().enumerate() {
        let n = i + 1;
        match attempt {
            Attempt::Unparsed { error } => println!("    {n} 回目: L0 として読めない — {error}"),
            Attempt::Blocking { findings } => {
                println!("    {n} 回目: blocking {} 件", findings.len());
                for f in findings.iter().take(5) {
                    println!("      - {f}");
                }
            }
            Attempt::Accepted { advisory } => {
                println!("    {n} 回目: 通った (advisory {advisory} 件)")
            }
        }
    }

    match &report.verified {
        Some(v) => {
            println!(
                "\n  {} ({})",
                args.output.display(),
                describe_frames(&v.frames)
            );
            println!("    素性 -> {}", session.provenance_path().display());
            // **advisory を «直った» と読ませない** (D77 ・D104)
            println!(
                "    ** 通ったのは blocking が 0 だからで，advisory {} 件は残っている ** — \n\
                 \u{3000}\u{3000}設計書 8.2 が «advisory 違反は許容する» と決めている．\n\
                 \u{3000}\u{3000}`px lint` で中身を見ること",
                v.advisory
            );
            println!("    ** 下地である ** — 設計書 1.3 のとおり，最後は手で直すことが前提である");
            println!(
                "    ** 生成物はコミットすること ** — 決定論を担うのはキャッシュと，\n\
                 \u{3000}\u{3000}この出力がリポジトリに残ることである (D157)．{}",
                backend.describe()
            );
            Ok(())
        }
        None => bail!(
            "{} 回作り直しても検査を通らなかった．**上限を上げるより先に依頼を書き直すこと** — \
             同じ依頼を繰り返しても同じところで落ちる",
            report.attempts.len()
        ),
    }
}

/// `out/hero.px.toml` -> (`out`, `hero`)．
fn split_output(path: &Path) -> Result<(PathBuf, String)> {
    let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .with_context(|| format!("{} から名前を取れない", path.display()))?;
    let stem = name
        .strip_suffix(".px.toml")
        .or_else(|| name.strip_suffix(".toml"))
        .unwrap_or(&name)
        .to_string();
    if stem.is_empty() {
        bail!("{} から名前を取れない", path.display());
    }
    Ok((dir, stem))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **壊れると: パレットと素性が生成物の隣に置かれなくなる．**
    #[test]
    fn the_output_name_splits_into_a_directory_and_a_stem() {
        let (d, s) = split_output(Path::new("out/hero.px.toml")).unwrap();
        assert_eq!(d, Path::new("out"));
        assert_eq!(s, "hero");

        let (d, s) = split_output(Path::new("hero.px.toml")).unwrap();
        assert_eq!(d, Path::new(""));
        assert_eq!(s, "hero");
    }

    #[test]
    fn a_size_needs_both_sides() {
        assert_eq!(parse_size("16x16").unwrap(), (16, 16));
        assert_eq!(parse_size("32X8").unwrap(), (32, 8));
        assert!(parse_size("16").is_err());
        assert!(parse_size("axb").is_err());
    }
}
