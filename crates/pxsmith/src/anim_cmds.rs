//! `pxsmith anim` — 中割り ・タイミング ・周期アニメ (設計書 6.9 ・6.11 ・6.12)．
//!
//! **判定は CLI に置く** — `pxsmith-core` は絵を作るだけで，lint を掛けるのは
//! `pxsmith-lint` を呼べるこちら側である (`pxsmith direction` ・`pxsmith validate` と同じ形) ．

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use pxsmith_core::Rgba8;
use pxsmith_core::afterimage::{AfterimageOptions, TrailEdge, afterimage};
use pxsmith_core::anim::{CycleSpec, ModTarget, Wave, cycle, ease, reverse_derive};
use pxsmith_core::canvas::IndexedCanvas;
use pxsmith_core::deform::{SquashAnchor, SquashOptions, VolumeRule, squash};
use pxsmith_core::frame::{Frame, FrameKind, Layer, LayerMeta, Surface, unify_palettes};
use pxsmith_core::geom::Mask;
use pxsmith_core::math::{IVec2, ivec2};
use pxsmith_core::palette::{ChromaCurve, Ramp};
use pxsmith_core::ramp::{LightPreset, build_lighting};
use pxsmith_core::shade::{ShadeOptions, shade_to_canvas};
use pxsmith_core::smear::{SmearMethod, SmearOptions, covers_ends, smear_mask};
use pxsmith_core::subpixel::{SubpixelMethod, SubpixelOptions, SubpixelScope};
use pxsmith_core::tween::{
    ExtrapolateKind, TweenAlign, TweenOptions, extrapolate_mask, tween_series,
};

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
    /// おばけを作る (設計書 6.11 の D43)．**2 枚を繋ぐ伸びた形を 1 コマ**
    Smear {
        #[command(flatten)]
        args: SmearArgs,
    },
    /// 予備動作 / オーバーシュートを作る (設計書 6.11)
    Extrapolate {
        #[command(flatten)]
        args: ExtrapolateArgs,
    },
    /// 潰しと伸ばし (設計書 6.11 の D41)．**体積 ($h \times w$) を保つ**
    Squash {
        #[command(flatten)]
        args: SquashArgs,
    },
    /// サブピクセル (設計書 6.10．D38 ・D39)．**接線方向へ色を渡す**
    Subpixel {
        #[command(flatten)]
        args: SubpixelArgs,
    },
    /// 残像を敷く．**ランプの宣言が要る**
    Afterimage {
        #[command(flatten)]
        args: AfterimageArgs,
    },
}

pub fn anim(command: AnimCommand) -> Result<()> {
    match command {
        AnimCommand::Tween { args } => tween_cmd(&args),
        AnimCommand::Ease { args } => ease_cmd(&args),
        AnimCommand::Cycle { args } => cycle_cmd(&args),
        AnimCommand::Smear { args } => smear_cmd(&args),
        AnimCommand::Extrapolate { args } => extrapolate_cmd(&args),
        AnimCommand::Squash { args } => squash_cmd(&args),
        AnimCommand::Subpixel { args } => subpixel_cmd(&args),
        AnimCommand::Afterimage { args } => afterimage_cmd(&args),
    }
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
    crate::save_frames(&args.output, &frames, "tween")?;

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
    crate::save_frames(&args.output, &frames, "eased")?;

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
         16 ms にするとコマ打ちのぶん誤差が積もり pxsmith validate が鳴る)"
    );
    println!("  実機の表示周期に合っているかは pxsmith validate --target で確かめること");
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
    #[arg(long, default_value_t = pxsmith_core::anim::DEFAULT_FRAMES)]
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
    crate::save_frames(&args.output, &out, "cycle")?;

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
         実装していない (回転は pxsmith rotate の仕事で，ここに 2 つ目の回転を書かない)"
    );
    Ok(())
}

// -------------------------------------------------------------------- smear

/// 形から絵を作る共通の引数 (`tween` と同じ組)．
#[derive(Args, Clone, Debug)]
pub struct ShapeShadeArgs {
    /// 固有色 `RRGGBB`．**形だけを作る道具なので色は宣言する**
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
}

/// マスクに陰影を付けて 1 フレームにする．
fn shade_frame(mask: &Mask, args: &ShapeShadeArgs, kind: FrameKind) -> Result<Frame> {
    let base = Rgba8::from_hex_str(&args.base)
        .with_context(|| format!("固有色 '{}' を読めない", args.base))?;
    let preset: LightPreset = args.preset.into();
    let source = match &args.light {
        Some(spec) => parse_light(spec)?,
        None => LightPreset::default_source(preset),
    };
    let (palette, model) = build_lighting(base, preset, args.ramp_steps, args.curve.into())?;
    let (canvas, palette) =
        shade_to_canvas(mask, source, &model, &palette, ShadeOptions::default())?;
    let mut frame = Frame::new(mask.size(), palette);
    frame.layers.push(Layer::new(
        LayerMeta::named("art"),
        Surface::Indexed(canvas),
    ));
    frame.kind = kind;
    Ok(frame)
}

#[derive(Args, Clone, Debug)]
pub struct SmearArgs {
    pub output: PathBuf,
    #[arg(long)]
    pub from: PathBuf,
    #[arg(long)]
    pub to: PathBuf,
    /// 掃引する前に重心を合わせるか (`centroid` ・`none`)
    ///
    /// **既定は `centroid`．** `none` は設計書 6.11 のままだが，
    /// **包含定理から «union と同じ集合» にしかならない** (繋がらない)．
    #[arg(long, default_value = "centroid")]
    pub align: String,
    /// 掃引の作り方 (`sweep` ・`union`)．`union` は設計書が «採らない» と言う方
    #[arg(long, default_value = "sweep")]
    pub method: String,
    /// 標本の数．**省略すると重心変位から決める** (1 画素に 1 標本)
    #[arg(long)]
    pub samples: Option<u32>,
    /// 両端のキーフレームを出力に含めない
    #[arg(long)]
    pub no_ends: bool,
    #[command(flatten)]
    pub shade: ShapeShadeArgs,
}

fn ends_of(from: &std::path::Path, to: &std::path::Path) -> Result<(Frame, Frame)> {
    let first = crate::load_frames(from)?;
    let last = crate::load_frames(to)?;
    let (Some(a), Some(b)) = (first.first(), last.first()) else {
        bail!("両端のキーフレームが要る");
    };
    Ok((a.clone(), b.clone()))
}

fn smear_cmd(args: &SmearArgs) -> Result<()> {
    let align = TweenAlign::parse(&args.align)
        .with_context(|| format!("--align は centroid か none ('{}')", args.align))?;
    let method = SmearMethod::parse(&args.method)
        .with_context(|| format!("--method は sweep か union ('{}')", args.method))?;
    let (a, b) = ends_of(&args.from, &args.to)?;
    let (ma, mb) = (silhouette_of(&a), silhouette_of(&b));

    let opts = SmearOptions {
        method,
        align,
        samples: args.samples,
    };
    let smear = smear_mask(&ma, &mb, &opts)?;
    // **おばけは 1 コマ相当** (設計書 6.11)
    let mut frame = shade_frame(&smear.mask, &args.shade, FrameKind::Inbetween)?;
    frame.duration_ms = a.duration_ms;

    let mut frames: Vec<Frame> = Vec::new();
    if !args.no_ends {
        frames.push(a.clone());
    }
    frames.push(frame);
    if !args.no_ends {
        frames.push(b.clone());
    }
    let colors = unify_palettes(&mut frames)?.len();
    crate::save_frames(&args.output, &frames, "smear")?;

    println!(
        "{} — おばけ 1 コマ ({} フレーム ・{} 色)",
        args.output.display(),
        frames.len(),
        colors
    );
    println!(
        "  重心変位 {:.1} 画素 ・標本 {} ・面積 {} ・成分 {} → {}",
        smear.displacement,
        smear.samples,
        smear.mask.count(),
        smear.components.0,
        smear.components.2
    );
    if !smear.connects() {
        println!(
            "  ** {} つに切れている ** — 繋がった形になっていない．\n\
             --align none は包含定理から «2 枚の和集合» にしかならないので，\n\
             速い動きでは必ず切れる (--align centroid にすること)",
            smear.components.2
        );
    }
    if !covers_ends(&ma, &mb, &smear.mask) {
        println!("  ** 両端を含んでいない ** — 符号の規約が壊れている");
    }
    // **書いていないものを黙らない** (D92 の作法)
    println!(
        "  書いていない: おばけの色は宣言した固有色と光源から作った (中割りと同じ)．\n\
         トポロジーは保証しない — 成分数を上に出してある"
    );
    Ok(())
}

// -------------------------------------------------------------- extrapolate

#[derive(Args, Clone, Debug)]
pub struct ExtrapolateArgs {
    pub output: PathBuf,
    #[arg(long)]
    pub from: PathBuf,
    #[arg(long)]
    pub to: PathBuf,
    /// `anticipation` (予備動作) か `overshoot` (オーバーシュート)
    #[arg(long, default_value = "anticipation")]
    pub kind: String,
    /// 振り幅．**変位に対する比**で，0.25 なら 2 枚の変位の 1/4 だけ外へ出す
    #[arg(long, default_value_t = 0.25)]
    pub amount: f32,
    /// 外挿する前に重心を合わせるか (`centroid` ・`none`)
    #[arg(long, default_value = "centroid")]
    pub align: String,
    /// 両端のキーフレームを出力に含めない
    #[arg(long)]
    pub no_ends: bool,
    #[command(flatten)]
    pub shade: ShapeShadeArgs,
}

fn extrapolate_cmd(args: &ExtrapolateArgs) -> Result<()> {
    let align = TweenAlign::parse(&args.align)
        .with_context(|| format!("--align は centroid か none ('{}')", args.align))?;
    let kind = ExtrapolateKind::parse(&args.kind)
        .with_context(|| format!("--kind は anticipation か overshoot ('{}')", args.kind))?;
    let (a, b) = ends_of(&args.from, &args.to)?;
    let (ma, mb) = (silhouette_of(&a), silhouette_of(&b));

    let opts = TweenOptions { margin: 0, align };
    let got = extrapolate_mask(&ma, &mb, kind, args.amount, &opts)?;
    let mut frame = shade_frame(&got.mask, &args.shade, FrameKind::Inbetween)?;
    frame.duration_ms = a.duration_ms;

    // **予備動作は前に，オーバーシュートは後ろに置く**
    let mut frames: Vec<Frame> = Vec::new();
    match kind {
        ExtrapolateKind::Anticipation => {
            frames.push(frame);
            if !args.no_ends {
                frames.push(a.clone());
                frames.push(b.clone());
            }
        }
        ExtrapolateKind::Overshoot => {
            if !args.no_ends {
                frames.push(a.clone());
                frames.push(b.clone());
            }
            frames.push(frame);
        }
    }
    let colors = unify_palettes(&mut frames)?.len();
    crate::save_frames(&args.output, &frames, "extrapolate")?;

    println!(
        "{} — {} ({} フレーム ・{} 色)",
        args.output.display(),
        kind.as_str(),
        frames.len(),
        colors
    );
    println!(
        "  t = {:.2} ・取り除いた移動 ({},{}) ・面積 {} ・成分 {} → {}",
        got.t,
        got.shift.x,
        got.shift.y,
        got.mask.count(),
        got.components.0,
        got.components.2
    );
    if got.clipped > 0 {
        // **黙って消さない** — 外挿では包含が成り立たない
        println!(
            "  ** 画布の外へ出て {} 画素が切れた ** — 中割りと違い外挿は\n\
             «2 枚の外接矩形の中に収まる» ことが保証されない．振り幅を下げるか，\n\
             元の絵に余白を足すこと",
            got.clipped
        );
    }
    println!(
        "  書いていない: 形が違う 2 枚の外挿がどう暴れるかは測っていない\n\
         (真値の作りようが無いため)．平行移動なら真値と画素単位一致する"
    );
    Ok(())
}

// ------------------------------------------------------------------- squash

#[derive(Args, Clone, Debug)]
pub struct SquashArgs {
    pub input: PathBuf,
    pub output: PathBuf,
    /// **縦の倍率から 1 を引いたもの．** `-0.25` で潰し，`+0.25` で伸ばし
    #[arg(long, allow_negative_numbers = true)]
    pub amount: f32,
    /// どこを固定するか (`bottom` ・`center` ・`top`)
    #[arg(long, default_value = "bottom")]
    pub anchor: String,
    /// もう一方の辺の決め方 (`derived` ・`independent`)
    #[arg(long, default_value = "derived")]
    pub rule: String,
    /// **画布を広げない．** 広げないと実素材の 136 / 140 通りで切れる
    #[arg(long)]
    pub no_grow: bool,
}

fn squash_cmd(args: &SquashArgs) -> Result<()> {
    let anchor = SquashAnchor::parse(&args.anchor)
        .with_context(|| format!("--anchor は bottom ・center ・top ('{}')", args.anchor))?;
    let rule = VolumeRule::parse(&args.rule)
        .with_context(|| format!("--rule は derived か independent ('{}')", args.rule))?;
    let opts = SquashOptions {
        anchor,
        rule,
        grow: !args.no_grow,
    };

    let mut frames = crate::load_frames(&args.input)?;
    // **フレームの寸法は揃っていなければならない．** 画布を広げるなら，
    // 広げた量が違うフレームどうしを共通の画布へ載せ直す
    let mut placed: Vec<Vec<(IndexedCanvas, IVec2)>> = Vec::new();
    let (mut volume_error, mut resample_error) = (0.0f32, 0.0f32);
    let (mut clipped, mut added) = (0usize, 0i64);
    let mut sizes: Vec<((u32, u32), (u32, u32))> = Vec::new();
    for frame in &frames {
        let mut per_layer = Vec::new();
        for layer in &frame.layers {
            let Some(canvas) = layer.surface.as_indexed() else {
                bail!("{} は指標付きでない", layer.meta.name);
            };
            let (out, r) = squash(canvas, args.amount, &opts)?;
            volume_error = volume_error.max(r.volume_error());
            resample_error = resample_error.max(r.resample_error());
            clipped += r.clipped;
            added = added.max(r.colors.1 as i64 - r.colors.0 as i64);
            sizes.push(r.canvas_size);
            per_layer.push((out, r.origin_shift));
        }
        placed.push(per_layer);
    }

    // 共通の画布 — いちばん大きいずらしに合わせて全部を同じだけ動かす
    let shift = placed.iter().flatten().fold(ivec2(0, 0), |acc, (_, s)| {
        ivec2(acc.x.max(s.x), acc.y.max(s.y))
    });
    let (mut cw, mut ch) = (0u32, 0u32);
    for (canvas, s) in placed.iter().flatten() {
        cw = cw.max((canvas.width() as i32 + shift.x - s.x) as u32);
        ch = ch.max((canvas.height() as i32 + shift.y - s.y) as u32);
    }
    for (frame, layers) in frames.iter_mut().zip(&placed) {
        for (layer, (canvas, s)) in frame.layers.iter_mut().zip(layers) {
            let transparent = canvas.transparent();
            let mut out = IndexedCanvas::filled(cw, ch, transparent.unwrap_or(0))
                .with_transparent(transparent);
            out.blit(canvas, ivec2(shift.x - s.x, shift.y - s.y), true);
            *layer = Layer::new(layer.meta.clone(), Surface::Indexed(out));
        }
        frame.size = pxsmith_core::math::uvec2(cw, ch);
    }

    let colors = unify_palettes(&mut frames)?.len();
    crate::save_frames(&args.output, &frames, "squash")?;

    let before = sizes.first().map(|s| s.0).unwrap_or((0, 0));
    println!(
        "{} -> {} — {} フレーム ・{} 色",
        args.input.display(),
        args.output.display(),
        frames.len(),
        colors
    );
    println!(
        "  量 {:+.2} ・錨 {} ・決め方 {} ・画布 {}x{} -> {}x{}",
        args.amount,
        anchor.as_str(),
        rule.as_str(),
        before.0,
        before.1,
        cw,
        ch
    );
    println!(
        "  体積 ($h \\times w$) の誤差 (最悪) {:.2}% ・拡縮が動かした画素 (最悪) {:.2}%",
        volume_error * 100.0,
        resample_error * 100.0
    );
    if clipped > 0 {
        println!(
            "  ** {clipped} 画素が切れた ** — --no-grow を外すと切れない\n\
             (体積を保つなら片方の辺は必ず伸びる)"
        );
    }
    if added > 0 {
        println!("  ** 色が {added} 増えた ** — 拡縮は最近傍なのでここは 0 のはず");
    }
    if (cw, ch) != before {
        // **端から端まで通して分かった** — 32x32 を潰すと 42x32 になり，
        // 8 の倍数でなくなる (GB / NES 系はこれを制約に持つ)
        println!(
            "  ** 画布の寸法が変わった ** — 出力先の «寸法はタイルの倍数» 制約を\n\
             壊すことがある．pxsmith validate --target で確かめること"
        );
    }
    // **書いていないものを黙らない** (D92 の作法)
    println!(
        "  書いていない: 体積は «外接矩形の面積» で測っている (設計書 6.11 ・\n\
         ルール 27 の定義)．画素は整数なので丸めのぶんは 0 にならない"
    );
    Ok(())
}

// ---------------------------------------------------------------- subpixel

#[derive(Args, Clone, Debug)]
pub struct SubpixelArgs {
    pub input: PathBuf,
    pub output: PathBuf,
    /// `tangent` (接線方向．既定) か `fast` (200% → 移動 → 50% → パレット強制)
    #[arg(long, default_value = "tangent")]
    pub method: String,
    /// 移動率．**オフセットではなく «どれだけ色を渡すか»**
    #[arg(long, default_value_t = pxsmith_core::subpixel::DEFAULT_FRACTION)]
    pub fraction: f32,
    /// どの輪郭から接線を取るか (`colours` ・`silhouette`)
    #[arg(long, default_value = "colours")]
    pub scope: String,
    /// 高速法で動かす向き `x,y`．**接線を見ないので指定が要る**
    #[arg(long, value_delimiter = ',', default_values_t = vec![1i32, 0])]
    pub direction: Vec<i32>,
    /// 新しい列に何ドット未満なら «孤立» とみなすか
    #[arg(long, default_value_t = pxsmith_core::subpixel::DEFAULT_MIN_RUN)]
    pub min_run: u32,
}

fn subpixel_cmd(args: &SubpixelArgs) -> Result<()> {
    let method = SubpixelMethod::parse(&args.method)
        .with_context(|| format!("--method は tangent か fast ('{}')", args.method))?;
    let scope = SubpixelScope::parse(&args.scope)
        .with_context(|| format!("--scope は colours か silhouette ('{}')", args.scope))?;
    if args.direction.len() != 2 {
        bail!("--direction は x,y の 2 つ");
    }

    let mut frames = crate::load_frames(&args.input)?;
    let (mut changed, mut no_colour, mut candidates) = (0usize, 0usize, 0usize);
    let (mut isolated, mut skipped, mut added) = (0usize, 0usize, 0i64);
    // **輪郭が動いたら中間フレームではない** (付録 C #5)
    let (mut silhouette, mut silhouette_layers) = (0usize, 0usize);
    for frame in &mut frames {
        let palette = frame.palette.clone();
        for layer in &mut frame.layers {
            // **除外マスクはレイヤの宣言から引く** (設計書 3.5 の `subpixel_exclude`)
            if layer.meta.subpixel_exclude {
                skipped += 1;
                continue;
            }
            let Some(canvas) = layer.surface.as_indexed() else {
                bail!("{} は指標付きでない", layer.meta.name);
            };
            let (out, r) = pxsmith_core::subpixel::subpixel(
                canvas,
                &palette,
                &SubpixelOptions {
                    fraction: args.fraction,
                    method,
                    scope,
                    exclude: None,
                    tolerance: pxsmith_core::subpixel::DEFAULT_TOLERANCE,
                    min_run: args.min_run,
                    direction: ivec2(args.direction[0], args.direction[1]),
                },
            )?;
            changed += r.changed;
            no_colour += r.no_colour;
            candidates += r.candidates;
            isolated += r.isolated_fixed;
            added = added.max(r.colors.1 as i64 - r.colors.0 as i64);
            if r.silhouette_moved > 0 {
                silhouette += r.silhouette_moved;
                silhouette_layers += 1;
            }
            *layer = Layer::new(layer.meta.clone(), Surface::Indexed(out));
        }
    }

    let colors = unify_palettes(&mut frames)?.len();
    crate::save_frames(&args.output, &frames, "subpixel")?;

    println!(
        "{} -> {} — {} ・{} フレーム ・{} 色",
        args.input.display(),
        args.output.display(),
        method.as_str(),
        frames.len(),
        colors
    );
    println!(
        "  移動率 {:.2} ・候補 {candidates} ・動かした画素 {changed} ・直した孤立列 {isolated}",
        args.fraction
    );
    if no_colour > 0 {
        // **作らずに数える** (設計書 6.10)
        println!(
            "  中間色がパレットに無くて動かせなかった対 {no_colour} — 新色は作らない\n\
             (実素材では候補の 4 割ほどがここに落ちる)"
        );
    }
    if changed == 0 {
        println!("  ** 1 画素も動いていない ** — 接線方向に色が変わっている画素が無い");
    }
    // **確率ではなく «この 1 回がどちらだったか» を言う** (付録 C #5) ．
    // 高速法が使えるかは絵ごとに変わり事前には読めないが，**事後には確実に読める**．
    // 輪郭がずれた絵は «壊れた絵» ではなく «正しく描かれた別の絵» なので
    // lint には掛からない — だから処方せず報告する (D101 ・D107 ・D138) ．
    if silhouette > 0 {
        println!(
            "  ** 輪郭が {silhouette} 画素動いた ({silhouette_layers} レイヤ) ** — \
             これは中間フレームではない\n\
             (設計書 6.10 は単位ステップ分のシフトを «サブピクセルではなく通常の移動» と\
             定めている．lint はこの形を見ない)"
        );
        if method == SubpixelMethod::Fast {
            println!(
                "  高速法は実素材 61 枚中 32 枚で輪郭を動かす (動かなかったのは 47.5%)．\
                 中間フレームが要るなら --method tangent にすること (付録 C #5)"
            );
        }
    } else if method == SubpixelMethod::Fast {
        println!(
            "  輪郭は動かなかった — 高速法が中間フレームとして通る 47.5% の側である\n\
             (実素材 61 枚中 29 枚．どちらに落ちるかは絵ごとに変わるので，毎回ここを見ること)"
        );
    }
    if added > 0 {
        // **これは警告ではない** — 中間色を置くとは «パレットの中の，まだ使って
        // いない色を使い始める» ことなので，増えるのが正しい
        println!("  パレットの中で新しく使い始めた色 {added} (新色は作っていない)");
    }
    if skipped == 0 {
        // **運用上必須** (設計書 6.10) なので，宣言が 1 つも無いことを言う
        println!(
            "  ** 除外レイヤが 1 つも宣言されていない ** — 設計書 6.10 は顔 ・目に\n\
             サブピクセルを使わないことを運用上必須としている．L0 の\n\
             subpixel_exclude = true で宣言すること"
        );
    } else {
        println!("  除外レイヤ {skipped} 枚は触っていない");
    }
    Ok(())
}

// -------------------------------------------------------------- afterimage

#[derive(Args, Clone, Debug)]
pub struct AfterimageArgs {
    pub input: PathBuf,
    pub output: PathBuf,
    /// ランプの添字．**必須** — 絵だけから «どの色が何段目か» は決まらない
    #[arg(long, value_delimiter = ',', required = true)]
    pub ramp: Vec<u8>,
    /// 何コマ前まで残すか
    #[arg(long, default_value_t = pxsmith_core::afterimage::DEFAULT_TRAIL)]
    pub trail: u32,
    /// 1 コマ古くなるごとにランプを何段落とすか
    #[arg(long, default_value_t = pxsmith_core::afterimage::DEFAULT_STEP)]
    pub step: u32,
    /// 列の先頭の埋め方 (`none` ・`wrap`)．**`wrap` はループのときだけ正しい**
    #[arg(long, default_value = "none")]
    pub edge: String,
}

fn afterimage_cmd(args: &AfterimageArgs) -> Result<()> {
    let edge = TrailEdge::parse(&args.edge)
        .with_context(|| format!("--edge は none か wrap ('{}')", args.edge))?;
    let frames = crate::load_frames(&args.input)?;
    let ramp = Ramp::new(args.ramp.clone(), ChromaCurve::PeakMiddle);
    let (mut out, report) = afterimage(
        &frames,
        &ramp,
        &AfterimageOptions {
            trail: args.trail,
            step: args.step,
            edge,
        },
    )?;
    let colors = unify_palettes(&mut out)?.len();
    crate::save_frames(&args.output, &out, "afterimage")?;

    println!(
        "{} -> {} — {} フレーム ・{} 色",
        args.input.display(),
        args.output.display(),
        out.len(),
        colors
    );
    println!(
        "  長さ {} コマ ・1 コマあたり {} 段 ・端 {} ・置いた画素 {}",
        args.trail,
        args.step,
        edge.as_str(),
        report.drawn
    );
    let shown: Vec<String> = report.per_frame.iter().map(|n| n.to_string()).collect();
    println!("  フレームごと: {}", shown.join(" ・"));
    if report.invisible() {
        // **«効かない» と «効いた» を分ける** (D77 の作法)
        println!(
            "  ** 1 画素も見えていない ** — 前のコマが現在の絵に隠れている．\n\
             実素材でも 1 画素の動きでは 64 枚中 55 枚が «見えない» になる"
        );
    }
    if report.covered > 0 {
        println!("  現在の絵に隠れて置かなかった画素 {}", report.covered);
    }
    if report.not_in_ramp > 0 {
        println!(
            "  宣言したランプに無い添字だった画素 {} — 落とす先が決まらない",
            report.not_in_ramp
        );
    }
    if report.saturated > 0 {
        println!(
            "  ランプの端に着いて置けなかった画素 {} — **もっと暗い色は作らない** (D94)",
            report.saturated
        );
    }
    Ok(())
}
