//! `pxsmith anchor` — L0 のアンカートラックを抽出・適用する (D182)．

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use pxsmith_io::l0::L0Document;
use serde_json::{Value, json};

#[derive(Subcommand)]
pub enum AnchorCommand {
    /// L0 の各フレームで解決したアンカーを JSON へ取り出す
    Track(TrackArgs),
    /// JSON のアンカートラックを L0 の各フレームへ書き込む
    Apply(ApplyArgs),
}

#[derive(Args)]
pub struct TrackArgs {
    /// 入力 (`.px.toml` / `.toml` / `.aseprite` / `.ase`)
    pub input: PathBuf,
    /// 取り出すアンカー名．省略時は全て
    #[arg(long = "anchor")]
    pub anchors: Vec<String>,
    /// 出力先．省略時は標準出力
    #[arg(short, long)]
    pub output: Option<PathBuf>,
}

#[derive(Args)]
pub struct ApplyArgs {
    /// `anchor track` が書いた JSON
    pub track: PathBuf,
    /// 適用先 (`.px.toml` / `.toml` / `.aseprite` / `.ase`)
    pub target: PathBuf,
    /// 書き出す L0
    #[arg(short, long)]
    pub output: PathBuf,
    /// 適用するアンカー名．省略時は全て
    #[arg(long = "anchor")]
    pub anchors: Vec<String>,
}

pub fn anchor(command: AnchorCommand) -> Result<()> {
    match command {
        AnchorCommand::Track(args) => track(&args),
        AnchorCommand::Apply(args) => apply(&args),
    }
}

fn require_l0(path: &Path) -> Result<()> {
    if path.to_string_lossy().ends_with(".px.toml") {
        return Ok(());
    }
    bail!(
        "{} は L0 (.px.toml) ではない．アンカーを持てるのは L0 だけ",
        path.display()
    )
}

fn selected_names(requested: &[String], available: BTreeSet<String>) -> BTreeSet<String> {
    if requested.is_empty() {
        available
    } else {
        requested.iter().cloned().collect()
    }
}

fn track(args: &TrackArgs) -> Result<()> {
    require_l0(&args.input)?;
    let doc = L0Document::read(&args.input)
        .with_context(|| format!("{} を L0 として読めない", args.input.display()))?;
    let available = doc
        .meta
        .anchors
        .keys()
        .chain(doc.frames.iter().flat_map(|frame| frame.anchors.keys()))
        .cloned()
        .collect();
    let names = selected_names(&args.anchors, available);

    let mut anchors: BTreeMap<String, Vec<[i32; 2]>> = BTreeMap::new();
    for name in names {
        let mut values = Vec::with_capacity(doc.frames.len());
        for (frame_index, frame) in doc.frames.iter().enumerate() {
            let value = frame
                .anchors
                .get(&name)
                .or_else(|| doc.meta.anchors.get(&name))
                .copied()
                .with_context(|| {
                    format!(
                        "{} のフレーム {frame_index} にアンカー '{name}' が無い",
                        args.input.display()
                    )
                })?;
            values.push(value);
        }
        anchors.insert(name, values);
    }

    let value = json!({
        "format": 1,
        "source": args.input.to_string_lossy(),
        "frames": doc.frames.len(),
        "anchors": anchors,
    });
    let mut text = serde_json::to_string_pretty(&value)?;
    text.push('\n');
    if let Some(output) = &args.output {
        std::fs::write(output, text)
            .with_context(|| format!("{} を書き出せない", output.display()))?;
    } else {
        print!("{text}");
    }
    Ok(())
}

fn apply(args: &ApplyArgs) -> Result<()> {
    require_l0(&args.target)?;
    let bytes = std::fs::read(&args.track)
        .with_context(|| format!("{} を読めない", args.track.display()))?;
    let track: Value = serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "{} をアンカートラック JSON として読めない",
            args.track.display()
        )
    })?;
    if track.get("format").and_then(Value::as_u64) != Some(1) {
        bail!("{} の format は 1 でなければならない", args.track.display());
    }
    let track_frames = track
        .get("frames")
        .and_then(Value::as_u64)
        .and_then(|frames| usize::try_from(frames).ok())
        .with_context(|| format!("{} の frames を読めない", args.track.display()))?;
    let tracks = track
        .get("anchors")
        .and_then(Value::as_object)
        .with_context(|| format!("{} の anchors を読めない", args.track.display()))?;

    let mut target = L0Document::read(&args.target)
        .with_context(|| format!("{} を L0 として読めない", args.target.display()))?;
    if track_frames != target.frames.len() {
        bail!(
            "トラックは {track_frames} フレームだが，対象は {} フレーム",
            target.frames.len()
        );
    }

    let available = tracks.keys().cloned().collect();
    let names = selected_names(&args.anchors, available);
    for name in names {
        let values = tracks
            .get(&name)
            .and_then(Value::as_array)
            .with_context(|| format!("トラックにアンカー '{name}' が無い"))?;
        if values.len() != track_frames {
            bail!(
                "アンカー '{name}' は {} フレームだが，トラックは {track_frames} フレーム",
                values.len()
            );
        }
        for (frame_index, (frame, value)) in target.frames.iter_mut().zip(values).enumerate() {
            let pair = value.as_array().with_context(|| {
                format!("アンカー '{name}' のフレーム {frame_index} が [x, y] ではない")
            })?;
            let [x, y] = pair.as_slice() else {
                bail!("アンカー '{name}' のフレーム {frame_index} が [x, y] ではない");
            };
            let x = x
                .as_i64()
                .and_then(|n| i32::try_from(n).ok())
                .with_context(|| {
                    format!("アンカー '{name}' のフレーム {frame_index} の x を読めない")
                })?;
            let y = y
                .as_i64()
                .and_then(|n| i32::try_from(n).ok())
                .with_context(|| {
                    format!("アンカー '{name}' のフレーム {frame_index} の y を読めない")
                })?;
            frame.anchors.insert(name.clone(), [x, y]);
        }
    }

    target
        .write(&args.output)
        .with_context(|| format!("{} を書き出せない", args.output.display()))
}
