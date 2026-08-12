//! `px` — pxforge の CLI．
//!
//! `px <group> <verb>` はレシピの `op = "<group>.<verb>"` に 1:1 対応する
//! (設計書 4.2)．

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use px_core::Frame;
use px_io::l0::{L0Document, Violation};
use px_io::{Document, FrameId};
use px_view::render::RenderOptions;

mod color_cmds;
mod watch;

#[derive(Parser)]
#[command(name = "px", version, about = "ドット絵アセットのためのパイプライン")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// `.aseprite` ⇄ L0 テキスト形式の相互変換 (設計書 4.1)
    Text {
        #[command(subcommand)]
        command: TextCommand,
    },
    /// パレットの操作 (設計書 4.5 / 6.6)
    Palette {
        #[command(subcommand)]
        command: color_cmds::PaletteCommand,
    },
    /// 色数を削減する (設計書 6.6)
    Quantize {
        input: PathBuf,
        output: PathBuf,
        /// 目標色数
        #[arg(long, default_value_t = 16)]
        colors: u8,
        #[arg(long, value_enum, default_value_t = color_cmds::MethodArg::Wu)]
        method: color_cmds::MethodArg,
        /// k-means の seed (決定論性のため必須)
        #[arg(long, default_value_t = 0)]
        seed: u64,
        /// 量子化の後に規則ベースの段階削減もかける (D49)
        #[arg(long)]
        strategy: bool,
    },
    /// 整形する (設計書 6.3 / 6.14)
    Clean {
        input: PathBuf,
        output: PathBuf,
        /// この画素数未満の連結成分を周囲へ溶かす．
        ///
        /// **既定は 0 (溶かさない)．** 2 にすると «面積 2 未満» をすべて溶かすが，
        /// ドット絵の質感は 1 画素の色違いでできており，CC0 の実物のタイルでは
        /// 1024 画素中 655 画素が対象になった — **既定で絵を壊していた**．
        #[arg(long, default_value_t = 0)]
        min_area: u32,
        /// 低可視の中間色を落とす
        #[arg(long)]
        remove_aa: bool,
        /// 規則性のないディザ状ノイズを正規化する
        #[arg(long)]
        denoise_dither: bool,
    },
    /// 格子を復元して縮小する (設計書 6.1)
    Conform {
        input: PathBuf,
        output: PathBuf,
        #[command(flatten)]
        grid: color_cmds::GridArgs,
        /// 縮小の後にこのパレットへ強制する
        #[arg(long)]
        palette: Option<PathBuf>,
        /// 非一様格子の検査に使う窓の一辺．0 で検査を飛ばす
        #[arg(long, default_value_t = 32)]
        window: u32,
        /// 局所推定の一致率がこれを下回ったら非一様として棄却する
        #[arg(long, default_value_t = 0.8)]
        uniformity: f32,
    },
    /// 端末に表示して確かめる
    View {
        path: PathBuf,
        #[command(flatten)]
        display: DisplayArgs,
    },
    /// 変更を監視して表示し直す
    Watch {
        path: PathBuf,
        #[command(flatten)]
        display: DisplayArgs,
    },
    /// 2 つの絵の変化ピクセルを示す
    Diff {
        a: PathBuf,
        b: PathBuf,
        /// 一覧に出す件数の上限
        #[arg(long, default_value_t = 40)]
        limit: usize,
    },
    /// 品質検査 (設計書 7 章)
    Lint {
        input: PathBuf,
        /// JSON で出す
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        grid: color_cmds::GridArgs,
    },
    /// 環境と保持層の診断
    Verify {
        #[command(subcommand)]
        command: VerifyCommand,
    },
}

#[derive(Args, Clone)]
struct DisplayArgs {
    /// 表示するフレーム
    #[arg(long, default_value_t = 0)]
    frame: usize,
    /// 整数倍の拡大率
    #[arg(long, default_value_t = 8)]
    zoom: u32,
    /// 明度のみで表示する
    #[arg(long)]
    luma: bool,
    /// 単色化して形だけ見る
    #[arg(long)]
    silhouette: bool,
    /// 透明部分の市松模様を消す
    #[arg(long)]
    no_checkerboard: bool,
}

impl DisplayArgs {
    fn options(&self) -> RenderOptions {
        RenderOptions {
            zoom: self.zoom,
            checkerboard: !self.no_checkerboard,
            luma: self.luma,
            silhouette: self
                .silhouette
                .then_some(px_core::Rgba8::rgb(0x1a, 0x1c, 0x2c)),
            ..RenderOptions::default()
        }
    }
}

#[derive(Subcommand)]
enum TextCommand {
    /// `.aseprite` の 1 レイヤを L0 テキストへ書き出す
    Export {
        input: PathBuf,
        output: PathBuf,
        /// 書き出すレイヤ名 (省略時は最初のインデックスカラーレイヤ)
        #[arg(long)]
        layer: Option<String>,
        /// L0 が参照するパレット (`.hex`)．無ければ作る
        #[arg(long)]
        palette: PathBuf,
    },
    /// L0 テキストから `.aseprite` を作る
    Import { input: PathBuf, output: PathBuf },
}

#[derive(Subcommand)]
enum VerifyCommand {
    /// 端末が 1 画素の確認に耐えるか調べる (R1・R18)
    Terminal,
    /// `.aseprite` の読み書きがバイト一致するか確かめる
    Roundtrip {
        files: Vec<PathBuf>,
        /// 射影と書き戻しを挟んでも一致するかも確かめる
        #[arg(long)]
        via_frame: bool,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Text { command } => text(command),
        Command::Palette { command } => color_cmds::palette(command),
        Command::Quantize {
            input,
            output,
            colors,
            method,
            seed,
            strategy,
        } => color_cmds::quantize(&input, &output, colors, method, seed, strategy),
        Command::Clean {
            input,
            output,
            min_area,
            remove_aa,
            denoise_dither,
        } => color_cmds::clean(&input, &output, min_area, remove_aa, denoise_dither),
        Command::Conform {
            input,
            output,
            grid,
            palette,
            window,
            uniformity,
        } => color_cmds::conform(
            &input,
            &output,
            &grid,
            palette.as_deref(),
            window,
            uniformity,
        ),
        Command::View { path, display } => view(&path, &display),
        Command::Watch { path, display } => watch::run(&path, &display.options(), display.frame),
        Command::Diff { a, b, limit } => diff(&a, &b, limit),
        Command::Lint { input, json, grid } => color_cmds::lint(&input, json, &grid),
        Command::Verify { command } => verify(command),
    }
}

/// 拡張子から入力の種類を見分けてフレーム列を読む．
pub(crate) fn load_frames(path: &Path) -> Result<Vec<Frame>> {
    let name = path.to_string_lossy();
    if name.ends_with(".px.toml") || name.ends_with(".toml") {
        let doc = L0Document::read(path)
            .with_context(|| format!("{} を L0 として読めない", path.display()))?;
        return doc
            .to_frames(path)
            .with_context(|| format!("{} を作業層へ変換できない", path.display()));
    }
    let doc = Document::read(path).with_context(|| format!("{} を読めない", path.display()))?;
    (0..doc.frame_count())
        .map(|i| {
            doc.project(FrameId(i))
                .with_context(|| format!("{} のフレーム {i} を射影できない", path.display()))
        })
        .collect()
}

fn text(command: TextCommand) -> Result<()> {
    match command {
        TextCommand::Export {
            input,
            output,
            layer,
            palette: palette_path,
        } => {
            let frames = load_frames(&input)?;
            let first = frames.first().context("フレームが 1 つも無い")?;

            let index = match &layer {
                Some(name) => first
                    .layer_by_name(name)
                    .with_context(|| format!("レイヤ '{name}' が無い"))?,
                None => first
                    .layers
                    .iter()
                    .position(|l| l.surface.as_indexed().is_some())
                    .context("インデックスカラーのレイヤが無い．L0 では表せない")?,
            };

            // L0 が参照するパレットを先に確定させる．無ければ元の絵から作る
            if !palette_path.exists() {
                px_io::hex::write(&palette_path, &first.palette)
                    .with_context(|| format!("{} を書き出せない", palette_path.display()))?;
                println!("パレットを作成: {}", palette_path.display());
            }

            let stem = output
                .file_name()
                .map(|s| s.to_string_lossy().replace(".px.toml", ""))
                .unwrap_or_else(|| "sprite".to_string());
            let relative = relative_from(&output, &palette_path);

            let exported = L0Document::from_frames(&stem, relative, &frames, index)
                .map_err(|v: Violation| anyhow::anyhow!("L0 の制約に反する — {v}"))?;
            for a in &exported.advisories {
                eprintln!("注意: {a}");
            }
            exported
                .document
                .write(&output)
                .with_context(|| format!("{} を書き出せない", output.display()))?;
            println!(
                "{} -> {} ({} フレーム，レイヤ '{}')",
                input.display(),
                output.display(),
                frames.len(),
                first.layers[index].meta.name
            );
            Ok(())
        }
        TextCommand::Import { input, output } => {
            let frames = load_frames(&input)?;
            let doc = Document::from_frames(&frames).with_context(|| {
                format!("{} から .aseprite を組み立てられない", input.display())
            })?;
            doc.write(&output)
                .with_context(|| format!("{} を書き出せない", output.display()))?;
            println!(
                "{} -> {} ({} フレーム)",
                input.display(),
                output.display(),
                frames.len()
            );
            Ok(())
        }
    }
}

/// `from` のあるディレクトリから見た `to` への相対パス．
///
/// 共通の親を辿れないときは `to` をそのまま返す．L0 の `ref` は L0 ファイルからの
/// 相対で解決されるので，ここを間違えるとパレットが引けなくなる．
fn relative_from(from: &Path, to: &Path) -> PathBuf {
    let base = from.parent().unwrap_or(Path::new("."));
    let (base, to) = match (base.canonicalize(), to.canonicalize()) {
        (Ok(b), Ok(t)) => (b, t),
        _ => return to.to_path_buf(),
    };
    let mut b = base.components().peekable();
    let mut t = to.components().peekable();
    while b.peek().is_some() && b.peek() == t.peek() {
        b.next();
        t.next();
    }
    let mut out = PathBuf::new();
    for _ in b {
        out.push("..");
    }
    out.extend(t);
    if out.as_os_str().is_empty() { to } else { out }
}

fn view(path: &Path, display: &DisplayArgs) -> Result<()> {
    let frames = load_frames(path)?;
    let frame = frames
        .get(display.frame)
        .with_context(|| format!("フレーム {} が無い (全 {} 件)", display.frame, frames.len()))?;
    let img = px_view::render::to_rgba_image(frame, &display.options());
    px_view::show(&img, px_view::term::Placement::default())?;
    println!(
        "{} フレーム {} / {} — {}x{}，{} ms，{}",
        path.display(),
        display.frame,
        frames.len(),
        frame.size.x,
        frame.size.y,
        frame.duration_ms,
        frame.kind.as_str()
    );
    Ok(())
}

fn diff(a: &Path, b: &Path, limit: usize) -> Result<()> {
    let (fa, fb) = (load_frames(a)?, load_frames(b)?);
    if fa.len() != fb.len() {
        println!("フレーム数が違う: {} と {}", fa.len(), fb.len());
    }

    let mut total = 0usize;
    for (i, (x, y)) in fa.iter().zip(&fb).enumerate() {
        if x.layers.len() != y.layers.len() {
            println!(
                "フレーム {i}: レイヤ数が違う ({} と {})",
                x.layers.len(),
                y.layers.len()
            );
            continue;
        }
        for (j, (lx, ly)) in x.layers.iter().zip(&y.layers).enumerate() {
            let (Some(cx), Some(cy)) = (lx.surface.as_indexed(), ly.surface.as_indexed()) else {
                continue;
            };
            let d = px_view::diff::diff_indexed(cx, cy)?;
            if d.is_empty() {
                continue;
            }
            total += d.count();
            println!(
                "フレーム {i} / レイヤ {j} '{}'\n{}",
                lx.meta.name,
                px_view::diff::format(&d, limit)
            );
        }
    }
    if total == 0 {
        println!("差分なし");
    } else {
        println!("合計 {total} 画素が変化");
    }
    Ok(())
}

fn verify(command: VerifyCommand) -> Result<()> {
    match command {
        VerifyCommand::Terminal => {
            let kind = px_view::detect();
            println!("{}", kind.report());
            if !kind.is_pixel_accurate() {
                bail!("この端末では 1 画素単位の確認ができない");
            }
            Ok(())
        }
        VerifyCommand::Roundtrip { files, via_frame } => {
            if files.is_empty() {
                bail!("検査するファイルを 1 つ以上指定すること");
            }
            let mut failed = 0usize;
            for path in &files {
                let original = std::fs::read(path)
                    .with_context(|| format!("{} を読めない", path.display()))?;
                let mut doc = Document::from_bytes(&original)
                    .with_context(|| format!("{} を解釈できない", path.display()))?;

                if via_frame {
                    for i in 0..doc.frame_count() {
                        let frame = doc.project(FrameId(i))?;
                        doc.merge_back(FrameId(i), &frame)?;
                    }
                }

                let written = doc.to_bytes()?;
                if written == original {
                    println!("一致    {} ({} バイト)", path.display(), original.len());
                } else {
                    failed += 1;
                    let at = written
                        .iter()
                        .zip(&original)
                        .position(|(a, b)| a != b)
                        .unwrap_or(original.len().min(written.len()));
                    println!(
                        "不一致  {} (元 {} バイト / 出力 {} バイト，最初の相違 {} バイト目)",
                        path.display(),
                        original.len(),
                        written.len(),
                        at
                    );
                }
            }
            if failed > 0 {
                bail!("{failed} / {} 件がバイト一致しなかった", files.len());
            }
            Ok(())
        }
    }
}
