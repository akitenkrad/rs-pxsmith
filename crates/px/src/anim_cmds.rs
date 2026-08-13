//! `px anim` — 中割り ・タイミング ・周期アニメ (設計書 6.9 ・6.11 ・6.12)．
//!
//! **判定は CLI に置く** — `px-core` は絵を作るだけで，lint を掛けるのは
//! `px-lint` を呼べるこちら側である (`px direction` ・`px validate` と同じ形) ．

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use px_core::Rgba8;
use px_core::anim::{CycleSpec, ModTarget, Wave, cycle, ease, reverse_derive};
use px_core::frame::{Frame, FrameKind, Layer, LayerMeta, Surface, unify_palettes};
use px_core::geom::Mask;
use px_core::math::ivec2;
use px_core::palette::{ChromaCurve, Ramp};
use px_core::ramp::{LightPreset, build_lighting};
use px_core::shade::{ShadeOptions, shade_to_canvas};
use px_core::tween::{TweenAlign, TweenOptions, tween_series};
use px_io::l0::L0Document;
use px_io::{Document, hex};

use crate::color_cmds::{CurveArg, PresetArg};
use crate::shape_cmds::parse_light;

#[derive(Subcommand)]
pub enum AnimCommand {
    /// 中割りを作る (設計書 6.9)．**形だけ補間し，色は補間しない**
    Tween {
        #[command(flatten)]
        args: TweenArgs,
    },
    /// コマ打ちと FPS から表示時間を付ける (設計書 6.11 の D40)
    Ease {
        #[command(flatten)]
        args: EaseArgs,
    },
    /// 周期アニメを作る (設計書 6.12)．**変調対象 x 波形の 2 軸**
    Cycle {
        #[command(flatten)]
        args: CycleArgs,
    },
}

pub fn anim(command: AnimCommand) -> Result<()> {
    match command {
        AnimCommand::Tween { args } => tween_cmd(&args),
        AnimCommand::Ease { args } => ease_cmd(&args),
        AnimCommand::Cycle { args } => cycle_cmd(&args),
    }
}

/// フレーム列を書き出す．
///
/// > [!warning] **`.aseprite` は `kind` を持てない** (D81 と同じ形の穴．3 度目) ．
/// > 設計書 7.1 ・D47 の «`inbetween` にはジャギー ・AA 系の lint を掛けない»
/// > は，**フレームの `kind` がファイルに残っていて初めて効く**．保持層の
/// > `.aseprite` にはその欄が無いので，`.aseprite` で書き出すと `px lint` は
/// > 中割りにもジャギーを鳴らす．
/// >
/// > **端から端まで CLI で通して見つけた** — 単体試験は作業層のフレームしか
/// > 見ていないので `kind` が落ちない．
/// >
/// > 欄を持っているのは L0 (`.px.toml`) だけなので，**L0 でも書き出せるように
/// > して，`.aseprite` だけのときは «スコープが効かない» と併記する**．
fn write_frames(path: &std::path::Path, frames: &[Frame], name: &str) -> Result<()> {
    let text = path.to_string_lossy();
    if text.ends_with(".px.toml") || text.ends_with(".toml") {
        let palette_ref = path.with_extension("hex");
        let exported = L0Document::from_frames(name, &palette_ref, frames, 0)
            .map_err(|v| anyhow::anyhow!("L0 として書き出せない: {v}"))?;
        for a in &exported.advisories {
            println!("  L0 の注意: {a}");
        }
        exported
            .document
            .write(path)
            .with_context(|| format!("{} を書き出せない", path.display()))?;
        hex::write(&palette_ref, &frames[0].palette)
            .with_context(|| format!("{} を書き出せない", palette_ref.display()))?;
        println!("  パレット -> {}", palette_ref.display());
        return Ok(());
    }
    let doc = Document::from_frames(frames).context("フレーム列を組み立てられない")?;
    doc.write(path)
        .with_context(|| format!("{} を書き出せない", path.display()))?;
    if frames.iter().any(|f| f.kind != FrameKind::Key) {
        println!(
            "  ** .aseprite は kind を持てない ** — 書いた中割りは読み直すと key に\n\
             なるので，px lint のスコープ (D47) が効かない．\n\
             スコープが要るなら .px.toml で書き出すこと"
        );
    }
    Ok(())
}

// -------------------------------------------------------------------- tween

#[derive(Args, Clone, Debug)]
pub struct TweenArgs {
    /// 出力．中割りは `kind = "inbetween"` で書く (D47．lint のスコープが効く)
    pub output: PathBuf,
    /// 始点のキーフレーム
    #[arg(long)]
    pub from: PathBuf,
    /// 終点のキーフレーム
    #[arg(long)]
    pub to: PathBuf,
    /// 中割りの枚数
    #[arg(long, default_value_t = 1)]
    pub steps: u32,
    /// 補間する前に重心を合わせるか (`centroid` ・`none`)
    ///
    /// **既定は `centroid`．** 場をそのまま補間すると動きに直交する向きに痩せ，
    /// 移動が形の差し渡しの 2 倍を超えると消える (R11) ．
    #[arg(long, default_value = "centroid")]
    pub align: String,
    /// 中割りの固有色 `RRGGBB`．**中割りの色は元の絵から取らない** (設計書 6.9)
    #[arg(long)]
    pub base: String,
    #[arg(long, value_enum, default_value_t = PresetArg::Clear)]
    pub preset: PresetArg,
    /// 1 ランプの段数
    #[arg(long, default_value_t = 5)]
    pub ramp_steps: u8,
    #[arg(long, value_enum, default_value_t = CurveArg::PeakMiddle)]
    pub curve: CurveArg,
    /// 光源．省略するとプリセットの既定を使う
    #[arg(long)]
    pub light: Option<String>,
    /// 両端のキーフレームを出力に含めない
    #[arg(long)]
    pub no_ends: bool,
}

fn silhouette_of(frame: &Frame) -> Mask {
    let mut m = Mask::new(frame.size.x, frame.size.y);
    for layer in &frame.layers {
        let Some(c) = layer.surface.as_indexed() else {
            continue;
        };
        for y in 0..c.height() as i32 {
            for x in 0..c.width() as i32 {
                let p = ivec2(x, y);
                if !c.is_transparent_at(p) {
                    m.set(p, true);
                }
            }
        }
    }
    m
}

fn tween_cmd(args: &TweenArgs) -> Result<()> {
    let align = TweenAlign::parse(&args.align)
        .with_context(|| format!("--align は centroid か none ('{}')", args.align))?;
    let base = Rgba8::from_hex_str(&args.base)
        .with_context(|| format!("固有色 '{}' を読めない", args.base))?;

    let first = crate::load_frames(&args.from)?;
    let last = crate::load_frames(&args.to)?;
    let (Some(a), Some(b)) = (first.first(), last.first()) else {
        bail!("両端のキーフレームが要る");
    };
    let (ma, mb) = (silhouette_of(a), silhouette_of(b));

    let opts = TweenOptions { margin: 0, align };
    let series = tween_series(&ma, &mb, args.steps, &opts)?;

    let preset: LightPreset = args.preset.into();
    let source = match &args.light {
        Some(spec) => parse_light(spec)?,
        None => LightPreset::default_source(preset),
    };
    let (shade_palette, model) = build_lighting(base, preset, args.ramp_steps, args.curve.into())?;

    let mut frames: Vec<Frame> = Vec::new();
    if !args.no_ends {
        frames.push(a.clone());
    }
    let mut changed = 0usize;
    for step in &series {
        let (canvas, palette) = shade_to_canvas(
            &step.mask,
            source,
            &model,
            &shade_palette,
            ShadeOptions::default(),
        )?;
        let mut frame = Frame::new(step.mask.size(), palette);
        frame.layers.push(Layer::new(
            LayerMeta::named("art"),
            Surface::Indexed(canvas),
        ));
        // **中割りは中割りだと書く** (D47) — ジャギー ・AA 系の lint が外れる
        frame.kind = FrameKind::Inbetween;
        frame.duration_ms = a.duration_ms;
        frames.push(frame);
        if step.topology_changed() {
            changed += 1;
        }
    }
    if !args.no_ends {
        frames.push(b.clone());
    }

    let colors = unify_palettes(&mut frames)?.len();
    write_frames(&args.output, &frames, "tween")?;

    let shift = series.first().map(|s| s.shift).unwrap_or(ivec2(0, 0));
    println!(
        "{} — 中割り {} 枚 ({} フレーム ・{} 色)",
        args.output.display(),
        series.len(),
        frames.len(),
        colors
    );
    println!(
        "  補間の前に取り除いた移動 ({},{}) ・揃え方 {}",
        shift.x,
        shift.y,
        align.as_str()
    );
    for s in &series {
        println!(
            "    t={:.2}  面積 {:>6}  成分 {} → {}  穴 {} → {}{}",
            s.t,
            s.mask.count(),
            s.components.0,
            s.components.2,
            s.holes.0,
            s.holes.2,
            if s.topology_changed() {
                "  ** トポロジーが変わった **"
            } else {
                ""
            }
        );
    }
    if changed > 0 {
        // **扱えないことを黙らない** (設計書 6.9)
        println!(
            "  {changed} 枚でオイラー標数が両端のどちらとも違う．\n\
             SDF 補間はトポロジー変化を扱えないので，そこは手で描くこと"
        );
    }
    // **書いていないものを黙らない** (D92 の作法)
    println!(
        "  書いていない: 中割りの色は宣言した固有色と光源から作った — 元の絵の色は\n\
         使っていない (設計書 6.9 «形状のみ補間し，色は補間しない»)．\n\
         Parts 補間 ・Manual 補間は実装していない"
    );
    Ok(())
}

// --------------------------------------------------------------------- ease

#[derive(Args, Clone, Debug)]
pub struct EaseArgs {
    pub input: PathBuf,
    pub output: PathBuf,
    /// 表示周期 (フレーム毎秒)
    #[arg(long, default_value_t = 24.0)]
    pub fps: f32,
    /// コマ打ち．1 つなら全フレームに，複数ならフレームごとに当てる (イージング)
    #[arg(long, value_delimiter = ',', default_values_t = vec![2u32])]
    pub hold: Vec<u32>,
}

fn ease_cmd(args: &EaseArgs) -> Result<()> {
    let mut frames = crate::load_frames(&args.input)?;
    let report = ease(&mut frames, args.fps, &args.hold)?;
    write_frames(&args.output, &frames, "eased")?;

    println!(
        "{} -> {} — {} フレーム ・{} FPS ・合計 {} ms",
        args.input.display(),
        args.output.display(),
        frames.len(),
        report.fps,
        report.total_ms
    );
    let shown: Vec<String> = report
        .holds
        .iter()
        .map(|(h, ms)| format!("{h}コマ={ms}ms"))
        .collect();
    println!("  {}", shown.join(" ・"));
    println!(
        "  丸めは逆数の四捨五入 (60 FPS の 1 コマ = 17 ms．\n\
         16 ms にするとコマ打ちのぶん誤差が積もり px validate が鳴る)"
    );
    println!("  実機の表示周期に合っているかは px validate --target で確かめること");
    Ok(())
}

// -------------------------------------------------------------------- cycle

#[derive(Args, Clone, Debug)]
pub struct CycleArgs {
    pub input: PathBuf,
    pub output: PathBuf,
    /// プリセット (`flicker` ・`sway` ・`rotate` ・`ripple`)．**`--target` / `--wave` より先に効く**
    #[arg(long)]
    pub preset: Option<String>,
    /// 何を変調するか (`ramp` ・`offset` ・`mask` ・`rotate`)
    #[arg(long, default_value = "offset")]
    pub target: String,
    /// どう変調するか (`sine` ・`square` ・`noise` ・`random-blink`)
    #[arg(long, default_value = "sine")]
    pub wave: String,
    /// フレーム数．**3 枚未満は受け付けない** (D44)
    #[arg(long, default_value_t = px_core::anim::DEFAULT_FRAMES)]
    pub frames: u32,
    #[arg(long, default_value_t = 1.0)]
    pub amplitude: f32,
    #[arg(long, default_value_t = 0.0)]
    pub phase: f32,
    /// **必須** — ノイズ系を含むので，無いと決定論性が崩れる (設計書 6.12)
    #[arg(long)]
    pub seed: u64,
    /// 平行移動の向き `x,y`
    #[arg(long, value_delimiter = ',', default_values_t = vec![1i32, 0])]
    pub direction: Vec<i32>,
    /// ランプの添字 (`--target ramp` に要る)．例 `--ramp 3,4,5,6,7`
    #[arg(long, value_delimiter = ',')]
    pub ramp: Vec<u8>,
    /// 逆再生を後ろへ足して往復にする (D44)
    #[arg(long)]
    pub reverse_derive: bool,
}

fn cycle_cmd(args: &CycleArgs) -> Result<()> {
    let mut spec = match &args.preset {
        Some(name) => CycleSpec::preset(name, args.seed).with_context(|| {
            format!(
                "'{name}' はプリセットではない (使えるのは {})",
                CycleSpec::PRESETS.join(" ・")
            )
        })?,
        None => CycleSpec {
            target: ModTarget::parse(&args.target)
                .with_context(|| format!("'{}' は変調対象ではない", args.target))?,
            wave: Wave::parse(&args.wave)
                .with_context(|| format!("'{}' は波形ではない", args.wave))?,
            seed: args.seed,
            ..CycleSpec::default()
        },
    };
    spec.frames = args.frames;
    spec.amplitude = args.amplitude;
    spec.phase = args.phase;
    if args.direction.len() != 2 {
        bail!("--direction は x,y の 2 つ");
    }
    spec.direction = ivec2(args.direction[0], args.direction[1]);

    let frames = crate::load_frames(&args.input)?;
    let Some(src) = frames.first() else {
        bail!("{} にフレームが 1 つも無い", args.input.display());
    };
    let ramp =
        (!args.ramp.is_empty()).then(|| Ramp::new(args.ramp.clone(), ChromaCurve::PeakMiddle));

    let (mut out, report) = cycle(src, &spec, ramp.as_ref())?;
    let mut reversed = 0;
    if args.reverse_derive {
        let before = out.len();
        out = reverse_derive(&out);
        reversed = out.len() - before;
    }

    let colors = unify_palettes(&mut out)?.len();
    write_frames(&args.output, &out, "cycle")?;

    println!(
        "{} -> {} — {} x {} ・{} フレーム{} ・{} 色",
        args.input.display(),
        args.output.display(),
        spec.target.as_str(),
        spec.wave.as_str(),
        out.len(),
        if reversed > 0 {
            format!(" (逆再生 {reversed} 枚を足した)")
        } else {
            String::new()
        },
        colors
    );
    let shown: Vec<String> = report
        .steps
        .iter()
        .map(|(v, s)| format!("{v:+.2}→{s:+}"))
        .collect();
    println!("  波 {} (種 {})", shown.join(" ・"), spec.seed);
    if report.all_still {
        // **«鳴らない» と «効いていない» を分ける** (D77 と同じ作法)
        println!(
            "  ** 1 枚も動いていない ** — 振幅 {} が小さすぎて丸めるとどのフレームも 0 になる",
            spec.amplitude
        );
    }
    // **書いていないものを黙らない** (D92 の作法)
    println!(
        "  書いていない: 16 通り (変調対象 4 x 波形 4) のうち rotate の 4 通りは\n\
         実装していない (回転は px rotate の仕事で，ここに 2 つ目の回転を書かない)"
    );
    Ok(())
}
