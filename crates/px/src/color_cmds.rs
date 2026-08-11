//! 色とパレットのサブコマンド (M2)．
//!
//! 入出力の型を拡張子で見分ける．`.png` は RGBA，`.aseprite` と `.px.toml` は
//! インデックスカラーである．

use std::num::NonZeroU8;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use px_core::canvas::{IndexedCanvas, RgbaCanvas};
use px_core::clean::{AaOptions, DenoiseOptions};
use px_core::grid::{GridParams, downscale_modal, estimate_grid, local_grid, uniformity};
use px_core::quantize::{ApplyOptions, Dither, QuantizeMethod, ReduceOptions};
use px_core::ramp::{LightPreset, RampSpec, build_lighting, generate_ramp};
use px_core::{ChromaCurve, Palette, Rgba8};
use px_io::{Document, FrameId, hex, png};

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum MethodArg {
    Wu,
    Kmeans,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum DitherArg {
    None,
    Ordered,
    /// 誤差拡散．フレーム間で模様が踊るのでオプトイン．
    Error,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum CurveArg {
    Uniform,
    PeakMiddle,
    ShadowHeavy,
    LightHeavy,
}

impl From<CurveArg> for ChromaCurve {
    fn from(v: CurveArg) -> Self {
        match v {
            CurveArg::Uniform => Self::Uniform,
            CurveArg::PeakMiddle => Self::PeakMiddle,
            CurveArg::ShadowHeavy => Self::ShadowHeavy,
            CurveArg::LightHeavy => Self::LightHeavy,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum PresetArg {
    Clear,
    Overcast,
    Sunset,
    Night,
    Moonlight,
}

impl From<PresetArg> for LightPreset {
    fn from(v: PresetArg) -> Self {
        match v {
            PresetArg::Clear => Self::Clear,
            PresetArg::Overcast => Self::Overcast,
            PresetArg::Sunset => Self::Sunset,
            PresetArg::Night => Self::Night,
            PresetArg::Moonlight => Self::Moonlight,
        }
    }
}

/// 格子推定の閾値．既定値は合成 500 件の検証セットで校正した (`GridParams` の説明) ．
/// **目標 (95%) には届いていないので暫定である．**
#[derive(Args, Clone, Debug)]
pub struct GridArgs {
    #[arg(long, default_value_t = 16)]
    pub max_scale: u32,
    /// セル内平均分散の許容 $\varepsilon$．
    #[arg(long, default_value_t = 0.02)]
    pub epsilon: f32,
    /// 再構成の画素色差の許容 $\delta$．
    #[arg(long, default_value_t = 0.1)]
    pub delta: f32,
    /// 再構成の不一致画素率の許容 $\tau$．
    #[arg(long, default_value_t = 0.1)]
    pub tau: f32,
    /// 位相ずれ検査で画像を切る帯の数．0 で検査を飛ばす
    #[arg(long, default_value_t = 4)]
    pub phase_bands: usize,
    /// 帯どうしの位相のずれの許容 ($s$ に対する割合)
    #[arg(long, default_value_t = 1.0 / 6.0)]
    pub phase_tolerance: f32,
    /// これ未満の信頼度は棄却する．
    #[arg(long, default_value_t = 0.03)]
    pub min_confidence: f32,
}

impl From<&GridArgs> for GridParams {
    fn from(a: &GridArgs) -> Self {
        Self {
            max_scale: a.max_scale,
            epsilon: a.epsilon,
            delta: a.delta,
            tau: a.tau,
            phase_bands: a.phase_bands,
            phase_tolerance: a.phase_tolerance,
            min_confidence: a.min_confidence,
        }
    }
}

#[derive(Subcommand)]
pub enum PaletteCommand {
    /// パレットの内容を表示する
    Info { path: PathBuf },
    /// `.gpl` / `.pal` / `.act` を正規形 `.hex` へ変換する
    Convert { input: PathBuf, output: PathBuf },
    /// 複数の絵から共通パレットを作る
    Extract {
        /// `.aseprite` / `.px.toml`
        inputs: Vec<PathBuf>,
        #[arg(long)]
        output: PathBuf,
    },
    /// 複数のパレットを 1 つに束ねる
    Merge {
        inputs: Vec<PathBuf>,
        #[arg(long)]
        output: PathBuf,
        /// ランプの端点を寄せて共有する (D48)
        #[arg(long)]
        share_endpoints: bool,
        /// 端点を同じとみなす色距離
        #[arg(long, default_value_t = 0.06)]
        threshold: f32,
    },
    /// 色傾斜 (ランプ) を作る
    Ramp {
        output: PathBuf,
        /// 固有色 `RRGGBB`
        #[arg(long)]
        base: String,
        #[arg(long, default_value_t = 5)]
        steps: u8,
        #[arg(long, value_enum, default_value_t = CurveArg::PeakMiddle)]
        curve: CurveArg,
        /// 色相をずらす量 (度)．明→黄・暗→紫
        #[arg(long, default_value_t = 25.0)]
        hue_shift: f32,
        /// 光源プリセット．指定すると光面・影面・反射光の 3 ランプを作る
        #[arg(long, value_enum)]
        preset: Option<PresetArg>,
        /// 純黒を許す (既定は避ける)
        #[arg(long)]
        allow_pure_black: bool,
    },
    /// 既存のパレットへ強制する
    Apply {
        input: PathBuf,
        output: PathBuf,
        #[arg(long)]
        palette: PathBuf,
        #[arg(long, value_enum, default_value_t = DitherArg::None)]
        dither: DitherArg,
        /// 明度の重み $w_L$
        #[arg(long, default_value_t = 1.0)]
        weight_l: f32,
    },
}

pub fn palette(command: PaletteCommand) -> Result<()> {
    match command {
        PaletteCommand::Info { path } => info(&path),
        PaletteCommand::Convert { input, output } => {
            let p = load_palette(&input)?;
            hex::write(&output, &p)?;
            println!(
                "{} -> {} ({} 色)",
                input.display(),
                output.display(),
                p.len()
            );
            Ok(())
        }
        PaletteCommand::Extract { inputs, output } => extract(&inputs, &output),
        PaletteCommand::Merge {
            inputs,
            output,
            share_endpoints,
            threshold,
        } => {
            if inputs.is_empty() {
                bail!("束ねるパレットを 1 つ以上指定すること");
            }
            let palettes: Vec<Palette> = inputs
                .iter()
                .map(|p| load_palette(p))
                .collect::<Result<_>>()?;
            let merged = Palette::merged(&palettes, share_endpoints, threshold)?;
            hex::write(&output, &merged)?;
            println!(
                "{} 個のパレット -> {} ({} 色{})",
                palettes.len(),
                output.display(),
                merged.len(),
                if share_endpoints {
                    "，端点を共有"
                } else {
                    ""
                }
            );
            Ok(())
        }
        PaletteCommand::Ramp {
            output,
            base,
            steps,
            curve,
            hue_shift,
            preset,
            allow_pure_black,
        } => ramp(
            &output,
            &base,
            steps,
            curve,
            hue_shift,
            preset,
            allow_pure_black,
        ),
        PaletteCommand::Apply {
            input,
            output,
            palette,
            dither,
            weight_l,
        } => apply(&input, &output, &palette, dither, weight_l),
    }
}

fn info(path: &Path) -> Result<()> {
    let palette = load_palette(path)?;
    println!("{} — {} 色", path.display(), palette.len());
    for (i, (c, lab)) in palette.entries().iter().zip(palette.lab()).enumerate() {
        println!(
            "{i:>3}  {}  L={:.3} a={:+.3} b={:+.3} C={:.3}{}",
            c.to_hex_string(),
            lab.l,
            lab.a,
            lab.b,
            lab.chroma(),
            if c.a == 0 { "  (透明)" } else { "" }
        );
    }
    Ok(())
}

pub fn load_palette(path: &Path) -> Result<Palette> {
    let is_hex = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("hex"));
    if is_hex {
        hex::read(path).with_context(|| format!("{} を読めない", path.display()))
    } else {
        hex::import(path).with_context(|| format!("{} を読めない", path.display()))
    }
}

fn extract(inputs: &[PathBuf], output: &Path) -> Result<()> {
    if inputs.is_empty() {
        bail!("抽出元を 1 つ以上指定すること");
    }
    let mut owned: Vec<(IndexedCanvas, Palette)> = Vec::new();
    for path in inputs {
        for frame in crate::load_frames(path)? {
            for layer in &frame.layers {
                if let Some(c) = layer.surface.as_indexed() {
                    owned.push((c.clone(), frame.palette.clone()));
                }
            }
        }
    }
    let palette = Palette::extract_from(owned.iter().map(|(c, p)| (c, p)))?;
    hex::write(output, &palette)?;
    println!(
        "{} 個の入力 -> {} ({} 色)",
        inputs.len(),
        output.display(),
        palette.len()
    );
    Ok(())
}

fn ramp(
    output: &Path,
    base: &str,
    steps: u8,
    curve: CurveArg,
    hue_shift: f32,
    preset: Option<PresetArg>,
    allow_pure_black: bool,
) -> Result<()> {
    let base = Rgba8::from_hex_str(base).with_context(|| format!("固有色 '{base}' を読めない"))?;

    let palette = match preset {
        Some(p) => {
            let preset: LightPreset = p.into();
            let (palette, model) = build_lighting(base, preset, steps, curve.into())?;
            println!(
                "光源プリセット {} — 光面 {:?} / 影面 {:?} / 反射光 {:?} / 遮蔽 {}",
                preset.as_str(),
                model.key.entries(),
                model.shadow.entries(),
                model.bounce.entries(),
                model.occlusion
            );
            palette
        }
        None => {
            let spec = RampSpec {
                base,
                steps,
                chroma_curve: curve.into(),
                hue_shift,
                avoid_pure_black: !allow_pure_black,
                ..RampSpec::default()
            };
            Palette::new(generate_ramp(&spec))?
        }
    };

    hex::write(output, &palette)?;
    println!("{} ({} 色)", output.display(), palette.len());
    for (i, c) in palette.entries().iter().enumerate() {
        println!("{i:>3}  {}", c.to_hex_string());
    }
    Ok(())
}

fn apply(
    input: &Path,
    output: &Path,
    palette_path: &Path,
    dither: DitherArg,
    weight_l: f32,
) -> Result<()> {
    let palette = load_palette(palette_path)?;
    let img = load_rgba(input)?;
    let opts = ApplyOptions {
        w_l: weight_l,
        dither: match dither {
            DitherArg::None => Dither::None,
            DitherArg::Ordered => Dither::ORDERED,
            DitherArg::Error => Dither::ErrorDiffusion { strength: 1.0 },
        },
    };
    let indexed = px_core::quantize::apply_palette(&img, &palette, &opts);
    write_indexed(output, &indexed, &palette)?;
    println!(
        "{} -> {} ({} 色のパレットへ強制，w_L={weight_l})",
        input.display(),
        output.display(),
        palette.len()
    );
    Ok(())
}

/// `px quantize`．
pub fn quantize(
    input: &Path,
    output: &Path,
    colors: u8,
    method: MethodArg,
    seed: u64,
    strategy: bool,
) -> Result<()> {
    let colors = NonZeroU8::new(colors).context("色数は 1 以上")?;
    let img = load_rgba(input)?;
    let method = match method {
        MethodArg::Wu => QuantizeMethod::Wu,
        MethodArg::Kmeans => QuantizeMethod::Kmeans { seed },
    };

    let palette = px_core::quantize::quantize(&img, colors, method)?;
    let mut indexed = px_core::quantize::apply_palette(&img, &palette, &ApplyOptions::default());
    let mut palette = palette;

    if strategy {
        // 規則ベースの段階削減 (D49)．量子化の後にもう一段かける
        let r = px_core::quantize::reduce_colors(
            &indexed,
            &palette,
            colors.get() as usize,
            &ReduceOptions::default(),
        )?;
        for (name, dropped) in &r.steps {
            println!("  {name}: {dropped} 色");
        }
        indexed.remap(&r.map)?;
        palette = r.palette;
    }

    write_indexed(output, &indexed, &palette)?;
    println!(
        "{} -> {} ({} 色)",
        input.display(),
        output.display(),
        palette.len()
    );
    Ok(())
}

/// `px clean`．
pub fn clean(
    input: &Path,
    output: &Path,
    min_area: u32,
    remove_aa: bool,
    denoise: bool,
) -> Result<()> {
    let (mut canvas, palette) = load_indexed(input)?;
    let mut report = Vec::new();

    if min_area > 0 {
        let n = px_core::clean::remove_isolated(&mut canvas, min_area);
        report.push(format!("孤立ピクセル除去: {n} 画素"));
    }
    if remove_aa {
        let n = px_core::clean::remove_antialiasing(&mut canvas, &palette, &AaOptions::default());
        report.push(format!("AA 除去: {n} 画素"));
    }
    if denoise {
        let found = px_core::clean::detect_dither_noise(&canvas, &DenoiseOptions::default());
        let n = px_core::clean::denoise_dither(&mut canvas, &DenoiseOptions::default());
        report.push(format!("脱ディザノイズ: {} 領域 / {n} 画素", found.len()));
    }

    write_indexed(output, &canvas, &palette)?;
    println!("{} -> {}", input.display(), output.display());
    for line in report {
        println!("  {line}");
    }
    Ok(())
}

/// `px conform`．
pub fn conform(
    input: &Path,
    output: &Path,
    grid: &GridArgs,
    palette_path: Option<&Path>,
    window: u32,
    uniformity_threshold: f32,
) -> Result<()> {
    let img = load_rgba(input)?;
    let params = GridParams::from(grid);

    // 非一様格子は復元できないので，検出して棄却し人に返す (D29)
    if window > 0 {
        let local = local_grid(&img, window, &params);
        match uniformity(&local) {
            Some((scale, ratio)) => {
                println!("局所格子: 最頻 {scale} / 一致率 {:.1}%", ratio * 100.0);
                if ratio < uniformity_threshold {
                    bail!(
                        "格子が非一様である (一致率 {:.1}% < {:.1}%)．\
                         この画像は決定論的には直せないので人に返す",
                        ratio * 100.0,
                        uniformity_threshold * 100.0
                    );
                }
            }
            None => println!("局所格子: 推定できる窓が無い"),
        }
    }

    let estimate = estimate_grid(&img, &params)
        .map_err(|e| anyhow::anyhow!("{} の格子を推定できない: {e}", input.display()))?;
    println!(
        "格子: {} 倍 / 位相 ({}, {}) / 信頼度 {:.3} / セル内分散 {:.2e}",
        estimate.scale,
        estimate.phase.x,
        estimate.phase.y,
        estimate.confidence,
        estimate.mean_variance
    );

    let small = downscale_modal(&img, estimate.scale, estimate.phase);
    println!("{}x{} へ縮小", small.width(), small.height());

    match palette_path {
        Some(path) => {
            let palette = load_palette(path)?;
            let indexed =
                px_core::quantize::apply_palette(&small, &palette, &ApplyOptions::default());
            write_indexed(output, &indexed, &palette)?;
        }
        None => png::write_rgba(output, &small)?,
    }
    println!("-> {}", output.display());
    Ok(())
}

/// 入力を RGBA として読む．
fn load_rgba(path: &Path) -> Result<RgbaCanvas> {
    if is_png(path) {
        return png::read_rgba(path).with_context(|| format!("{} を読めない", path.display()));
    }
    let (canvas, palette) = load_indexed(path)?;
    Ok(png::resolve(&canvas, &palette))
}

/// 入力をインデックスカラーとして読む．最初のインデックスカラーレイヤを採る．
fn load_indexed(path: &Path) -> Result<(IndexedCanvas, Palette)> {
    if is_png(path) {
        bail!(
            "{} は PNG なのでインデックスカラーとして読めない．\
             先に px quantize か px palette apply を通すこと",
            path.display()
        );
    }
    let frames = crate::load_frames(path)?;
    let frame = frames.first().context("フレームが 1 つも無い")?;
    let layer = frame
        .layers
        .iter()
        .find_map(|l| l.surface.as_indexed())
        .context("インデックスカラーのレイヤが無い")?;
    Ok((layer.clone(), frame.palette.clone()))
}

fn is_png(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("png"))
}

/// 出力の拡張子に合わせて書き出す．
fn write_indexed(path: &Path, canvas: &IndexedCanvas, palette: &Palette) -> Result<()> {
    if is_png(path) {
        return png::write_indexed(path, canvas, palette)
            .with_context(|| format!("{} を書き出せない", path.display()));
    }
    // `.aseprite` なら添字をそのまま保てる
    let mut frame = px_core::Frame::new(canvas.size(), palette.clone());
    frame.layers.push(px_core::Layer::new(
        px_core::LayerMeta::named("art"),
        px_core::Surface::Indexed(canvas.clone()),
    ));
    let doc = Document::from_frames(&[frame])?;
    doc.write(path)
        .with_context(|| format!("{} を書き出せない", path.display()))?;
    let _ = FrameId(0);
    Ok(())
}

/// `px lint`．
///
/// PNG は格子系のルール (2 ・9) だけ，インデックスカラーは色・ディザ系のルールを
/// 見る．**blocking 違反があれば非ゼロで終わる** — 生成ループと CI が同じ判定を
/// 使えるようにするためである．
pub fn lint(input: &Path, json: bool, grid: &GridArgs) -> Result<()> {
    let mut cfg = px_lint::LintConfig {
        grid: GridParams::from(grid),
        ..px_lint::LintConfig::default()
    };
    cfg.grid_window = if grid.max_scale >= 2 { 32 } else { 0 };

    let mut report = px_lint::Report::default();
    if is_png(input) {
        let img = png::read_rgba(input)?;
        report.extend(px_lint::rules::lint_grid(&img, &cfg));
    } else {
        for (i, frame) in crate::load_frames(input)?.iter().enumerate() {
            let mut r = px_lint::lint_frame(frame, &cfg);
            for v in &mut r.violations {
                v.message = format!("フレーム {i}: {}", v.message);
            }
            report.extend(r);
        }
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{report}");
        println!(
            "blocking {} 件 / advisory {} 件",
            report.blocking().count(),
            report.advisory().count()
        );
    }

    if report.has_blocking() {
        bail!("blocking 違反がある");
    }
    Ok(())
}
