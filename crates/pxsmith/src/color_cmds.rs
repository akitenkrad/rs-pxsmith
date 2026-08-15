//! 色とパレットのサブコマンド (M2)．
//!
//! 入出力の型を拡張子で見分ける．`.png` は RGBA，`.aseprite` と `.px.toml` は
//! インデックスカラーである．

use std::num::NonZeroU8;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use pxsmith_core::canvas::{IndexedCanvas, RgbaCanvas};
use pxsmith_core::clean::{AaOptions, DenoiseOptions};
use pxsmith_core::grid::{GridParams, downscale_modal, estimate_grid, local_grid, uniformity};
use pxsmith_core::quantize::{ApplyOptions, Dither, QuantizeMethod, ReduceOptions};
use pxsmith_core::ramp::{LightPreset, RampSpec, build_lighting, generate_ramp};
use pxsmith_core::{ChromaCurve, Palette, Rgba8};
use pxsmith_io::{FrameId, hex, png};

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
///
/// > [!warning] 既定値を書き写さない
/// > ここに数値を直書きすると校正した値と黙って食い違う — 実際 $\varepsilon$ が
/// > 0.02 対 0.2 ・$\theta$ が 0.25 対 0.35 とずれており，**CLI だけ校正前の閾値で
/// > 動いていた**．すべて [`GridParams::default`] から引く．
#[derive(Args, Clone, Debug)]
pub struct GridArgs {
    #[arg(long, default_value_t = 16)]
    pub max_scale: u32,
    /// セル内平均分散の許容 $\varepsilon$．
    #[arg(long, default_value_t = GridParams::default().epsilon)]
    pub epsilon: f32,
    /// 再構成の画素色差の許容 $\delta$．
    #[arg(long, default_value_t = GridParams::default().delta)]
    pub delta: f32,
    /// 再構成の不一致画素率の許容 $\tau$．
    #[arg(long, default_value_t = GridParams::default().tau)]
    pub tau: f32,
    /// 位相ずれ検査で画像を切る帯の数．0 で検査を飛ばす
    #[arg(long, default_value_t = GridParams::default().phase_bands)]
    pub phase_bands: usize,
    /// 帯どうしの位相のずれの許容 ($s$ に対する割合)
    #[arg(long, default_value_t = GridParams::default().phase_tolerance)]
    pub phase_tolerance: f32,
    /// 帯のずれの許容の下限 (画素)．**既定の 0 は「下限なし」** — 上げると非整数の
    /// 周期を受け入れてしまう (校正記録)
    #[arg(long, default_value_t = GridParams::default().phase_tolerance_floor)]
    pub phase_tolerance_floor: f32,
    /// 帯ごとの位相を副画素で求める
    #[arg(long)]
    pub phase_subpixel: bool,
    /// 帯ごとの位相**曲線**の食い違いの許容．1.0 以上でこの検査を外す
    #[arg(long, default_value_t = GridParams::default().phase_agreement)]
    pub phase_agreement: f32,
    /// 半セルずらしたときにセル内分散が最低これだけ悪化することを求める．
    /// 1.0 以下でこの検査を外す
    #[arg(long, default_value_t = GridParams::default().phase_contrast_min)]
    pub phase_contrast_min: f32,
    /// 位相ずれ検査に要る帯あたりのセル数
    #[arg(long, default_value_t = GridParams::default().phase_min_cells)]
    pub phase_min_cells: usize,
    /// これ未満の信頼度は棄却する．**$\hat{s}$ で割って使う** (既定)
    #[arg(long, default_value_t = GridParams::default().min_confidence)]
    pub min_confidence: f32,
}

impl From<&GridArgs> for GridParams {
    fn from(a: &GridArgs) -> Self {
        Self {
            max_scale: a.max_scale,
            // CLI からは切り替えない (校正で決まるまでは既定のまま)
            normalize_epsilon: GridParams::default().normalize_epsilon,
            epsilon: a.epsilon,
            delta: a.delta,
            tau: a.tau,
            phase_bands: a.phase_bands,
            phase_tolerance: a.phase_tolerance,
            phase_tolerance_floor: a.phase_tolerance_floor,
            phase_subpixel: a.phase_subpixel,
            phase_min_cells: a.phase_min_cells,
            min_confidence: a.min_confidence,
            confidence_per_scale: GridParams::default().confidence_per_scale,
            phase_agreement: a.phase_agreement,
            phase_contrast_min: a.phase_contrast_min,
            // 校正で決めた形をそのまま使う (CLI からは切り替えない)．
            // **数値を書き写さない** — 校正値と黙って食い違う (D68 で実際にやった)
            phase_require_measurable: GridParams::default().phase_require_measurable,
            ..GridParams::default()
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
    /// 面積上位色とコントラストを報告する (設計書 5 章 ・G5)
    Report {
        /// 絵．**インデックスカラーが要る**
        input: PathBuf,
        /// 一覧に出す色数の上限
        #[arg(long, default_value_t = 12)]
        top: usize,
        /// **«主な色» として突き合わせる上位の数**．書籍は 2 〜 3 と言う
        #[arg(long, default_value_t = 3)]
        main: usize,
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
        PaletteCommand::Report { input, top, main } => report(&input, top, main),
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

/// `pxsmith palette report` — 面積上位色とコントラストを報告する (設計書 5 章)．
///
/// **処方はしない** — 何色に絞るべきかは絵の狙いで決まるので，数えて出すだけに
/// とどめる (D101 «削減率は入力で決まるので報告するだけ» と同じ側)．
fn report(input: &Path, top: usize, main: usize) -> Result<()> {
    let (canvas, palette) = load_indexed(input)?;
    let r = pxsmith_core::palreport::report(&canvas, &palette, main);
    if r.opaque == 0 {
        bail!("{} に不透明な画素が 1 つも無い", input.display());
    }

    println!(
        "{} — 不透明 {} 画素 ・パレット {} 色のうち使った色 {}",
        input.display(),
        r.opaque,
        r.palette_len,
        r.used
    );

    println!("\n  面積上位色");
    println!("    添字  色        面積    割合   最大の塊  領域数  明度");
    for c in r.by_area.iter().take(top) {
        println!(
            "    {:>4}  {}  {:>6} {:>6.1}% {:>9} {:>7} {:>6.3}",
            c.index,
            c.colour.to_hex_string(),
            c.area,
            c.share * 100.0,
            c.largest_region,
            c.regions,
            c.lightness
        );
    }
    if r.used > top {
        println!("    (残り {} 色は省いた．--top で増やせる)", r.used - top);
    }

    println!("\n  面積を覆うのに要る色数");
    for percent in [50u32, 80, 90, 95] {
        println!("    {percent:>3}% -> {} 色", r.cover(percent));
    }
    // **処方せず突き合わせるだけ** — 書籍が言っているのは «主な色を 2 〜 3 色に
    // 収める» であって，«超えたら直せ» ではない．
    //
    // **どの割合で読むかで «主な色» の数が変わる**ので，実素材 61 枚で測った
    // 中央値を並べて基準にする — 書籍の 2 〜 3 色に合うのは **50% の側**である
    // (80% で読むと中央 4 色で，2 〜 3 色に収まるのは 45.9% しかない)
    println!(
        "    参考: 書籍第四章 «色のデザイン» は主な色を 2 〜 3 色に収めることを勧める．\n\
         \u{3000}\u{3000}実素材 61 枚の中央値は **50% で 2 色 ・80% で 4 色 ・90% で 5 色** で，\n\
         \u{3000}\u{3000}書籍の 2 〜 3 色に合うのは «面積の半分» で読んだときである\n\
         \u{3000}\u{3000}(80% で読むと 2 〜 3 色に収まる絵は 45.9% しかない)"
    );

    if r.contrast.is_empty() {
        println!("\n  ** 主な色が 1 色しかないので比べる組が無い ** — 上位 {main} 色で見ている");
    } else {
        println!("\n  主な色どうしの隔たり (上位 {main} 色)");
        println!("    組        明度差   色距離");
        for c in &r.contrast {
            println!(
                "    {:>3} - {:<3} {:>7.3} {:>8.3}",
                c.a, c.b, c.lightness, c.delta_e
            );
        }
        // **見るのは明度である** — 色距離は色相の差も混ぜてしまうので，
        // «明度が同じで色相だけ違う 2 色» を «離れている» と数える
        if let Some(min) = r.closest_main_lightness() {
            println!(
                "    最も近い組の明度差 {min:.3} — **形が読めるかを決めるのは明度差**であり，\n\
                 \u{3000}\u{3000}色距離が大きくても明度が並んでいれば形は読めない"
            );
        }
    }
    Ok(())
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
    let indexed = pxsmith_core::quantize::apply_palette(&img, &palette, &opts);
    write_indexed(output, &indexed, &palette)?;
    println!(
        "{} -> {} ({} 色のパレットへ強制，w_L={weight_l})",
        input.display(),
        output.display(),
        palette.len()
    );
    Ok(())
}

/// `pxsmith quantize`．
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

    let palette = pxsmith_core::quantize::quantize(&img, colors, method)?;
    let mut indexed =
        pxsmith_core::quantize::apply_palette(&img, &palette, &ApplyOptions::default());
    let mut palette = palette;

    if strategy {
        // 規則ベースの段階削減 (D49)．量子化の後にもう一段かける
        let r = pxsmith_core::quantize::reduce_colors(
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

/// `pxsmith clean`．
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
        let n = pxsmith_core::clean::remove_isolated(&mut canvas, min_area);
        report.push(format!("孤立ピクセル除去: {n} 画素"));
    }
    if remove_aa {
        let n =
            pxsmith_core::clean::remove_antialiasing(&mut canvas, &palette, &AaOptions::default());
        report.push(format!("AA 除去: {n} 画素"));
    }
    if denoise {
        let found = pxsmith_core::clean::detect_dither_noise(&canvas, &DenoiseOptions::default());
        let n = pxsmith_core::clean::denoise_dither(&mut canvas, &DenoiseOptions::default());
        report.push(format!("脱ディザノイズ: {} 領域 / {n} 画素", found.len()));
    }

    write_indexed(output, &canvas, &palette)?;
    println!("{} -> {}", input.display(), output.display());
    for line in report {
        println!("  {line}");
    }
    Ok(())
}

/// `pxsmith conform`．
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
        let voting = local.data().iter().filter(|v| v.is_some()).count();
        match uniformity(&local) {
            Some((scale, ratio)) => {
                println!(
                    "局所格子: 最頻 {scale} / 一致率 {:.1}% ({voting} 窓が投票)",
                    ratio * 100.0
                );
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
        // **«一様だった» と «測っていない» を分ける** (D164)
        if voting < 2 {
            println!(
                "  ** 非一様格子の検査をしていない: 投票した窓が {voting} つしかない **\n\
                 \u{3000}\u{3000}一致率は投票した窓だけで出るので，これでは定義から 1.0 になる"
            );
        }
    }

    let estimate = estimate_grid(&img, &params)
        .map_err(|e| anyhow::anyhow!("{} の格子を推定できない: {e}", input.display()))?;

    // **窓は倍率の 4 倍要る** (D164)．先に窓を決めているので，推定できた倍率が
    // 大きいと «一様だと確かめた» が空約束になる — 測ってから言う
    if window > 0 {
        let need = pxsmith_core::grid::min_window_for(estimate.scale);
        if window < need {
            println!(
                "  ** 非一様格子の検査は当てにならない: 窓 {window} は {} 倍の格子に足りない **\n\
                 \u{3000}\u{3000}窓の一辺にセルが {} つ入らないと局所推定は当たらない — \
                 --window {need} 以上で掛け直す",
                estimate.scale,
                pxsmith_core::grid::MIN_CELLS_PER_WINDOW
            );
        }
    }
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
                pxsmith_core::quantize::apply_palette(&small, &palette, &ApplyOptions::default());
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
pub(crate) fn load_indexed(path: &Path) -> Result<(IndexedCanvas, Palette)> {
    if is_png(path) {
        bail!(
            "{} は PNG なのでインデックスカラーとして読めない．\
             先に pxsmith quantize か pxsmith palette apply を通すこと",
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

pub(crate) fn is_png(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("png"))
}

/// 出力の拡張子に合わせて書き出す．
pub(crate) fn write_indexed(path: &Path, canvas: &IndexedCanvas, palette: &Palette) -> Result<()> {
    if is_png(path) {
        return png::write_indexed(path, canvas, palette)
            .with_context(|| format!("{} を書き出せない", path.display()));
    }
    // `.aseprite` ・`.px.toml` なら添字をそのまま保てる
    let mut frame = pxsmith_core::Frame::new(canvas.size(), palette.clone());
    frame.layers.push(pxsmith_core::Layer::new(
        pxsmith_core::LayerMeta::named("art"),
        pxsmith_core::Surface::Indexed(canvas.clone()),
    ));
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().replace(".px", ""))
        .unwrap_or_else(|| "sprite".to_string());
    let _ = FrameId(0);
    crate::save_frames(path, &[frame], &stem)
}

/// `pxsmith lint`．
///
/// PNG は格子系のルール (2 ・9) だけ，インデックスカラーは色・ディザ系のルールを
/// 見る．**blocking 違反があれば非ゼロで終わる** — 生成ループと CI が同じ判定を
/// 使えるようにするためである．
pub fn lint(input: &Path, json: bool, grid: &GridArgs) -> Result<()> {
    let mut cfg = pxsmith_lint::LintConfig {
        grid: GridParams::from(grid),
        ..pxsmith_lint::LintConfig::default()
    };
    // **見る升の上限は利用者の宣言から引く** (D172)．
    //
    // 以前はここで «max_scale < 2 ならルール 9 を切る» という門を掛けていたが，
    // 厳密判定に替えた時点でその理由が消えた — **ミクセルとは «無いはずの拡大が
    // 混ざっていること»** なので，«拡大は無いはず» という宣言こそ検査の前提である．
    // 少なくとも 1 と 2 を見分けられないと書籍のミクセルが取れないので 2 で下げ止める
    cfg.mixel_max_k = grid.max_scale.max(2);

    let mut report = pxsmith_lint::Report::default();
    let mut coverage = None;
    let mut mixel = None;
    if is_png(input) {
        let img = png::read_rgba(input)?;
        report.extend(pxsmith_lint::rules::lint_grid(&img, &cfg));
        // **«鳴らなかった» と «検査していない» を分ける** (D164 ・D172)
        mixel = Some(pxsmith_lint::mixel_coverage(&img, &cfg));
    } else {
        let frames = crate::load_frames(input)?;
        for (i, frame) in frames.iter().enumerate() {
            let mut r = pxsmith_lint::lint_frame(frame, &cfg);
            for v in &mut r.violations {
                v.message = format!("フレーム {i}: {}", v.message);
            }
            report.extend(r);
        }
        // **列があればフレーム間のルールも掛ける** (設計書 7.1 の `sequence`)
        if frames.len() >= 2 {
            let (r, cov) = pxsmith_lint::lint_sequence(&frames, &cfg);
            report.extend(r);
            coverage = Some(cov);
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
        // **«鳴らなかった» と «検査していない» を分ける** (D77 ・D92 ・D104)
        if let Some(cov) = &coverage {
            let unchecked = cov.unchecked();
            if !unchecked.is_empty() {
                let names: Vec<String> = unchecked
                    .iter()
                    .map(|id| {
                        let why = match id {
                            22 => "kind = inbetween のフレームが無い",
                            26 => "subpixel_exclude のレイヤが無い",
                            _ => "伸び縮みしているコマ対が無い",
                        };
                        format!("{id} ({why})")
                    })
                    .collect();
                println!(
                    "** 掛からなかったフレーム間ルール: {} **\n\
                     \u{3000}\u{3000}欄を持てるのは L0 (.px.toml) だけである — .aseprite で\n\
                     \u{3000}\u{3000}書き出すと kind と subpixel_exclude が落ちる (D119)",
                    names.join(" ・")
                );
            }
        }
        // **ルール 9 も同じ扱いにする** (D164 ・D172) — 升は窓ごとに決まるので，
        // 窓が 2 つ決まらなければ 2 通りになりようがない
        if let Some(m) = &mixel {
            if let Some(why) = m.why_not() {
                println!(
                    "** 掛からなかったルール: 9 ミクセル ({why}) **\n\
                     \u{3000}\u{3000}升が 2 つ以上決まらないと «場所により違う» と言えない (D164 ・D172)"
                );
            } else {
                // **«検査した» だけでは足りない** — 窓より小さい混入は原理的に
                // 見えないので，何が見えなかったかを言う (D172)
                println!(
                    "ルール 9 ミクセル: 升が決まった窓 {} つを見た\n\
                     \u{3000}\u{3000}** {} 画素角より小さい拡大の混入は見えない ** — \
                     窓 1 つがまるごと拡大側に入らないと升が立たない",
                    m.pinned,
                    m.resolution()
                );
            }
        }
    }

    if report.has_blocking() {
        bail!("blocking 違反がある");
    }
    Ok(())
}

/// `pxsmith validate` — **出力先の制約に照らす** (設計書 5 章)．
///
/// `--target` は組み込みの名前 (`gb` / `nes` / `snes` / `gba` / `pico8`) か，
/// プロファイルの TOML へのパスを取る．**違反があれば非ゼロで終わる** — CI と
/// レシピが同じ判定を使えるようにするためである (`pxsmith lint` と同じ作法) ．
///
/// > [!note] **PNG は受け取らない．**
/// > 制約はパレットとタイルの色数で決まるので，**添字の面が要る**．PNG を
/// > その場で添字化すると «こちらが選んだ量子化» を検査することになり，
/// > 出力先に載るかどうかの答えが変わってしまう．
pub fn validate(input: &Path, target: &str, json: bool) -> Result<()> {
    use pxsmith_core::validate::{Target, validate_frames};

    let target = match Target::builtin(target) {
        Some(t) => t,
        None => {
            let path = Path::new(target);
            if !path.exists() {
                bail!(
                    "出力先 {target} を知らない (組み込み: {}) ．\
                     プロファイルの TOML を渡すこともできる",
                    Target::BUILTIN.join(" / ")
                );
            }
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("{} を読めない", path.display()))?;
            toml::from_str(&text).with_context(|| format!("{} を読み取れない", path.display()))?
        }
    };

    if is_png(input) {
        bail!("PNG は検査できない — 添字の面 (.aseprite / .px.toml) を渡すこと");
    }
    let frames = crate::load_frames(input)?;
    let report = validate_frames(&frames, &target);

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{report}");
        println!("違反 {} 件", report.violations.len());
    }

    if !report.is_ok() {
        bail!("{} の制約に違反している", report.target);
    }
    Ok(())
}
