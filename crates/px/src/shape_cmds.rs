//! 形と陰影のサブコマンド (M3)．
//!
//! 入力から**シルエットだけ**を取り出して扱うのが共通の作法である (設計書の判断 4 —
//! 形状と陰影を分離し，陰影は常に導出する) ．入力の色は捨て，`--base` の固有色と
//! 光源プリセットから作ったパレットを新しく貼る．

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use px_core::Rgba8;
use px_core::aa::{AaAddOptions, add_antialiasing};
use px_core::frame::Surface;
use px_core::geom::Mask;
use px_core::guide::GuideOptions;
use px_core::outline::{OutlineOptions, OutlineStyle, outline};
use px_core::project::{Facing, ProjectOptions, Projection, SourcePlane, Step};
use px_core::ramp::{LightSource, build_lighting};
use px_core::resample::{ResampleAlgo, ResampleOptions};
use px_core::shade::{DEFAULT_AMBIENT_OCCLUSION, ShadeOptions, shade_to_canvas};
use px_core::smooth::{SmoothOptions, smooth_canvas};
use px_io::hex;

use crate::color_cmds::{CurveArg, PresetArg, is_png, load_indexed, write_indexed};

/// `px shade` の引数．
#[derive(Args, Clone, Debug)]
pub struct ShadeArgs {
    /// 陰影を付ける形．**色は使わず，透明でない画素だけをシルエットとして読む**
    pub input: PathBuf,
    /// 出力．**添字を保ちたいなら `.aseprite`** (PNG は確認用．設計書 4.1)
    pub output: PathBuf,
    /// 固有色 `RRGGBB`．ここから 3 ランプを作る
    #[arg(long)]
    pub base: String,
    /// 光源プリセット (設計書 3.3)
    #[arg(long, value_enum, default_value_t = PresetArg::Clear)]
    pub preset: PresetArg,
    /// 1 ランプの段数
    #[arg(long, default_value_t = 5)]
    pub steps: u8,
    /// 彩度カーブ
    #[arg(long, value_enum, default_value_t = CurveArg::PeakMiddle)]
    pub curve: CurveArg,
    /// 光源を明示する．省略するとプリセットの既定の向きを使う．
    ///
    /// `dir:x,y` ・`point:x,y[,強さ]` ・`line:x1,y1,x2,y2[,強さ]` ・
    /// `area:x,y,w,h[,強さ]` ・`ambient` の 5 型 (設計書 3.3)
    #[arg(long)]
    pub light: Option<String>,
    /// 環境遮蔽を掛ける (凹んだところを遮蔽色へ落とす)
    #[arg(long)]
    pub ao: bool,
    /// 環境遮蔽の閾値．**暫定値である** ([`DEFAULT_AMBIENT_OCCLUSION`] の説明)
    #[arg(long, default_value_t = DEFAULT_AMBIENT_OCCLUSION)]
    pub ao_threshold: f32,
    /// `HasBounceNeighbor` の探索距離 (下方向の画素数)
    #[arg(long, default_value_t = ShadeOptions::default().bounce_reach)]
    pub bounce_reach: u32,
    /// 作ったパレットを `.hex` として書き出す
    #[arg(long)]
    pub emit_palette: Option<PathBuf>,
}

/// `px shade` — シルエットへ陰影を導出する (設計書 6.2)．
pub fn shade(args: &ShadeArgs) -> Result<()> {
    let base = Rgba8::from_hex_str(&args.base)
        .with_context(|| format!("固有色 '{}' を読めない", args.base))?;
    let mask = load_mask(&args.input)?;
    if mask.is_empty() {
        bail!(
            "{} に透明でない画素が 1 つも無い．陰影を付ける形が無い",
            args.input.display()
        );
    }

    let preset = args.preset.into();
    let source = match &args.light {
        Some(spec) => parse_light(spec)?,
        None => px_core::ramp::LightPreset::default_source(preset),
    };
    let (palette, model) = build_lighting(base, preset, args.steps, args.curve.into())?;
    let opts = ShadeOptions {
        bounce_reach: args.bounce_reach,
        ambient_occlusion: args.ao.then_some(args.ao_threshold),
        ..ShadeOptions::default()
    };

    let (canvas, palette) = shade_to_canvas(&mask, source, &model, &palette, opts)?;

    println!(
        "シルエット {} 画素 / 光源 {} — 光面 {:?} ・影面 {:?} ・反射光 {:?} ・遮蔽 {}",
        mask.count(),
        describe(source),
        model.key.entries(),
        model.shadow.entries(),
        model.bounce.entries(),
        model.occlusion
    );
    if args.ao {
        println!("環境遮蔽: 閾値 {} (暫定値)", args.ao_threshold);
    }

    write_indexed(&args.output, &canvas, &palette)?;
    println!(
        "{} -> {} ({} 色)",
        args.input.display(),
        args.output.display(),
        palette.len()
    );
    if let Some(path) = &args.emit_palette {
        hex::write(path, &palette).with_context(|| format!("{} を書き出せない", path.display()))?;
        println!("パレット -> {}", path.display());
    }
    Ok(())
}

/// `px smooth` の引数．
#[derive(Args, Clone, Debug)]
pub struct SmoothArgs {
    /// 整形する絵．**インデックスカラーが要る** (色境界を見るため)
    pub input: PathBuf,
    pub output: PathBuf,
    /// 画素の移動上限．**これを超える谷は直さずに報告する** (意図的なディテールの
    /// 可能性がある)
    #[arg(long, default_value_t = SmoothOptions::default().max_move)]
    pub max_move: u32,
    /// 走査を繰り返す上限 (収束するまで回る．**安全網**)
    #[arg(long, default_value_t = SmoothOptions::default().max_passes)]
    pub max_passes: usize,
    /// 何が起きるかだけ見る (書き出さない)
    #[arg(long)]
    pub dry_run: bool,
}

/// `px smooth` — ジャギーを正規化する (設計書 6.4)．
pub fn smooth(args: &SmoothArgs) -> Result<()> {
    let (mut canvas, palette) = load_indexed(&args.input)?;
    let opts = SmoothOptions {
        max_move: args.max_move,
        max_passes: args.max_passes,
    };
    let report = smooth_canvas(&mut canvas, &opts);

    println!(
        "{} — {} 画素を動かした ({} 巡)",
        args.input.display(),
        report.moved,
        report.passes
    );
    if report.remaining > 0 {
        println!(
            "  残り {} 件: 移動上限 {} を超える {} 件 ・**幾何が決めた刻み {} 件** ・直し方が無い {} 件",
            report.remaining,
            args.max_move,
            report.over_limit,
            report.geometric,
            report.no_candidate
        );
    }
    // **«触らないと決めた» を黙らない** (D169．D77 ・D104 ・D164 の作法)
    if report.geometric > 0 {
        println!(
            "  ** {} 件は一定の傾きの直線として説明できるので触っていない ** —\n\
             \u{3000}\u{3000}直線の digitization には谷が必ず現れる．**動かすと正しく描いた線が壊れる**\n\
             \u{3000}\u{3000}(lint ルール 8 は advisory なので今までどおり助言としては鳴る)",
            report.geometric
        );
    }
    if args.dry_run {
        println!("  (--dry-run なので書き出さない)");
        return Ok(());
    }

    write_indexed(&args.output, &canvas, &palette)?;
    println!("-> {}", args.output.display());
    Ok(())
}

/// `px aa` の引数．
#[derive(Args, Clone, Debug)]
pub struct AaArgs {
    /// AA を付ける絵．**インデックスカラーが要る**
    pub input: PathBuf,
    pub output: PathBuf,
    /// **外郭にも付ける** (既定は内部境界のみ．D34 — 外郭は背景色が不定で
    /// AA が機能せず，ゲーム内で縁が汚れる)
    #[arg(long)]
    pub outline: bool,
    /// この色距離より近い 2 色には付けない
    #[arg(long, default_value_t = AaAddOptions::default().min_span)]
    pub min_span: f32,
    /// «2 色の間にある» とみなす遠回りの許容
    #[arg(long, default_value_t = AaAddOptions::default().tolerance)]
    pub tolerance: f32,
    /// 中間色を新しく作ってよい色数．0 なら既にある色だけを使う
    #[arg(long, default_value_t = AaAddOptions::default().max_new_colors)]
    pub max_new_colors: usize,
    /// 中間色を中点からずらす量．**暫定値である** (`AaAddOptions::offset` の説明)
    #[arg(long, default_value_t = AaAddOptions::default().offset)]
    pub offset: f32,
    /// これより短い段には付けない (45° に近い段は効き目より色数が目立つ)
    #[arg(long, default_value_t = AaAddOptions::default().min_run)]
    pub min_run: u32,
    /// 何が起きるかだけ見る (書き出さない)
    #[arg(long)]
    pub dry_run: bool,
    /// 作ったパレットを `.hex` として書き出す
    #[arg(long)]
    pub emit_palette: Option<PathBuf>,
}

/// `px aa` — アンチエイリアスを付ける (設計書 6.5)．
pub fn aa(args: &AaArgs) -> Result<()> {
    let (mut canvas, mut palette) = load_indexed(&args.input)?;
    let opts = AaAddOptions {
        include_outline: args.outline,
        min_span: args.min_span,
        tolerance: args.tolerance,
        max_new_colors: args.max_new_colors,
        offset: args.offset,
        min_run: args.min_run,
    };
    let before = palette.len();
    let report = add_antialiasing(&mut canvas, &mut palette, &opts)?;

    println!(
        "{} — {} 画素に中間色を置いた (色 {} → {})",
        args.input.display(),
        report.painted,
        before,
        palette.len()
    );
    println!(
        "  飛ばした: 45° の境界 {} 本 ・外郭 {} 画素 ・中間色を用意できない {} 画素",
        report.skipped_diagonal, report.skipped_outline, report.no_colour
    );
    if report.painted > 0 {
        // **掛けるのは 1 度だけにする．** AA は輪郭の形を変えるので，2 度目には
        // その先に新しい角ができる (量は巡ごとに減るが 0 にはならない)
        println!("  注意: 同じ絵に 2 度掛けると縁が太る．掛けるのは 1 度だけにすること");
    }
    if args.dry_run {
        println!("  (--dry-run なので書き出さない)");
        return Ok(());
    }

    write_indexed(&args.output, &canvas, &palette)?;
    println!("-> {}", args.output.display());
    if let Some(path) = &args.emit_palette {
        hex::write(path, &palette).with_context(|| format!("{} を書き出せない", path.display()))?;
        println!("パレット -> {}", path.display());
    }
    Ok(())
}

/// 縁取りの分類 (設計書 D36 の «5 分類»)．
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum OutlineStyleArg {
    /// 縁取りを剥がす
    None,
    /// 純黒
    Black,
    /// 内側の色を暗くした色
    Tinted,
    /// 背景想定に対して明暗を逆に取る
    Contrast,
    /// 光の当たる側は明るく ・影の側は暗く
    Shaded,
}

impl From<OutlineStyleArg> for OutlineStyle {
    fn from(v: OutlineStyleArg) -> Self {
        match v {
            OutlineStyleArg::None => Self::None,
            OutlineStyleArg::Black => Self::Black,
            OutlineStyleArg::Tinted => Self::Tinted,
            OutlineStyleArg::Contrast => Self::Contrast,
            OutlineStyleArg::Shaded => Self::Shaded,
        }
    }
}

/// `px outline` の引数．
#[derive(Args, Clone, Debug)]
pub struct OutlineArgs {
    /// 縁取りを付ける絵．**インデックスカラーが要る**
    pub input: PathBuf,
    pub output: PathBuf,
    /// 分類 (設計書 D36)
    #[arg(long, value_enum, default_value_t = OutlineStyleArg::Tinted)]
    pub style: OutlineStyleArg,
    /// **光の当たる側を描かない** (選択的輪郭線)
    #[arg(long)]
    pub selective: bool,
    /// **背景想定** `RRGGBB` (D36)．`--style contrast` が明暗を決めるのに使う
    #[arg(long)]
    pub background: Option<String>,
    /// 光源 (`--style shaded` と `--selective` が使う)．書式は `px shade` と同じ
    #[arg(long)]
    pub light: Option<String>,
    /// **外側に描く**．既定は内側 — 実物 61 枚のうち 56 枚が画像の縁に接しており，
    /// 外へ太らせると切れる
    #[arg(long)]
    pub outer: bool,
    /// 縁の色を作ってよい色数
    #[arg(long, default_value_t = OutlineOptions::default().max_new_colors)]
    pub max_new_colors: usize,
    /// 内側の色を暗くする量．**暫定値である** (`OutlineOptions::darken` の説明)
    #[arg(long, default_value_t = OutlineOptions::default().darken)]
    pub darken: f32,
    /// 何が起きるかだけ見る (書き出さない)
    #[arg(long)]
    pub dry_run: bool,
    /// 作ったパレットを `.hex` として書き出す
    #[arg(long)]
    pub emit_palette: Option<PathBuf>,
}

/// `px outline` — 縁取りを描く (設計書 D36)．
pub fn outline_cmd(args: &OutlineArgs) -> Result<()> {
    let (mut canvas, mut palette) = load_indexed(&args.input)?;
    let background = match &args.background {
        Some(hex) => {
            Some(Rgba8::from_hex_str(hex).with_context(|| format!("背景想定 '{hex}' を読めない"))?)
        }
        None => None,
    };
    let style: OutlineStyle = args.style.into();
    let opts = OutlineOptions {
        style,
        selective: args.selective,
        background,
        light: match &args.light {
            Some(spec) => parse_light(spec)?,
            None => OutlineOptions::default().light,
        },
        outer: args.outer,
        max_new_colors: args.max_new_colors,
        darken: args.darken,
        ..OutlineOptions::default()
    };
    if style == OutlineStyle::Contrast && background.is_none() {
        println!("注意: --background が無いので «明るい背景» を想定する (D36)");
    }

    let before = palette.len();
    let report = outline(&mut canvas, &mut palette, &opts)?;
    println!(
        "{} — 分類 {} / {} 側 — 描いた {} 画素 ・剥がした {} 画素 (色 {} → {})",
        args.input.display(),
        style.as_str(),
        if args.outer { "外" } else { "内" },
        report.painted,
        report.removed,
        before,
        palette.len()
    );
    if report.skipped_lit > 0 {
        println!("  選択的輪郭線で飛ばした {} 画素", report.skipped_lit);
    }
    if report.no_room > 0 {
        println!(
            "  **外に余地が無くて描けなかった {} 画素** (絵が画像の縁に接している)",
            report.no_room
        );
    }
    if args.dry_run {
        println!("  (--dry-run なので書き出さない)");
        return Ok(());
    }

    write_indexed(&args.output, &canvas, &palette)?;
    println!("-> {}", args.output.display());
    if let Some(path) = &args.emit_palette {
        hex::write(path, &palette).with_context(|| format!("{} を書き出せない", path.display()))?;
        println!("パレット -> {}", path.display());
    }
    Ok(())
}

/// 入力からシルエットを取り出す．**色は見ない．**
///
/// PNG はアルファ 0 でない画素，インデックスカラーは透明添字でない画素を採る．
fn load_mask(path: &Path) -> Result<Mask> {
    if is_png(path) {
        let img = px_io::png::read_rgba(path)
            .with_context(|| format!("{} を読めない", path.display()))?;
        let mut m = Mask::new(img.width(), img.height());
        for p in m.bounds().iter() {
            if img.get(p.x, p.y).is_some_and(|c| c.a != 0) {
                m.set(p, true);
            }
        }
        return Ok(m);
    }
    let (canvas, _) = load_indexed(path)?;
    Ok(canvas.silhouette())
}

/// `--light` の書式を読む (設計書 3.3 の光源 5 型)．
///
/// **座標は画素の中心を基準にした画像の座標系である** (`y` は下向き) ．
pub(crate) fn parse_light(spec: &str) -> Result<LightSource> {
    let (kind, rest) = spec.split_once(':').unwrap_or((spec, ""));
    let n: Vec<f32> = if rest.is_empty() {
        Vec::new()
    } else {
        rest.split(',')
            .map(|v| {
                v.trim()
                    .parse::<f32>()
                    .with_context(|| format!("光源 '{spec}' の数値 '{v}' を読めない"))
            })
            .collect::<Result<_>>()?
    };
    // 強さの既定は 1．点光源だけは $I / r^2$ なので画素の尺度で薄くなりすぎる —
    // それでも**既定では 1 のままにする**．明るさは利用者が測って決める量である
    let intensity = |at: usize| n.get(at).copied().unwrap_or(1.0);

    let source = match (kind, n.len()) {
        ("dir", 2) => LightSource::Directional {
            dir: px_core::vec2(n[0], n[1]),
        },
        ("point", 2..=3) => LightSource::Point {
            pos: px_core::vec2(n[0], n[1]),
            intensity: intensity(2),
        },
        ("line", 4..=5) => LightSource::Line {
            a: px_core::vec2(n[0], n[1]),
            b: px_core::vec2(n[2], n[3]),
            intensity: intensity(4),
        },
        ("area", 4..=5) => LightSource::Area {
            rect: px_core::math::Rect {
                x: n[0],
                y: n[1],
                w: n[2],
                h: n[3],
            },
            intensity: intensity(4),
        },
        ("ambient", 0) => LightSource::Ambient,
        _ => bail!(
            "光源 '{spec}' を読めない．\
             dir:x,y ・point:x,y[,強さ] ・line:x1,y1,x2,y2[,強さ] ・\
             area:x,y,w,h[,強さ] ・ambient のいずれか"
        ),
    };
    Ok(source)
}

/// 光源を 1 行で説明する (`--light` と同じ書式で返すので，そのまま貼り直せる)．
fn describe(source: LightSource) -> String {
    match source {
        LightSource::Directional { dir } => format!("dir:{},{}", dir.x, dir.y),
        LightSource::Point { pos, intensity } => format!("point:{},{},{intensity}", pos.x, pos.y),
        LightSource::Line { a, b, intensity } => {
            format!("line:{},{},{},{},{intensity}", a.x, a.y, b.x, b.y)
        }
        LightSource::Area { rect, intensity } => {
            format!(
                "area:{},{},{},{},{intensity}",
                rect.x, rect.y, rect.w, rect.h
            )
        }
        LightSource::Ambient => "ambient".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use px_core::math::Rect;
    use px_core::vec2;

    /// 光源 5 型がすべて読める．**書式を間違えたら黙って既定へ落ちない．**
    #[test]
    fn every_kind_of_light_source_parses() {
        assert_eq!(
            parse_light("dir:1,-2").unwrap(),
            LightSource::Directional {
                dir: vec2(1.0, -2.0)
            }
        );
        assert_eq!(
            parse_light("point:3,4").unwrap(),
            LightSource::Point {
                pos: vec2(3.0, 4.0),
                intensity: 1.0
            }
        );
        assert_eq!(
            parse_light("point:3,4,9").unwrap(),
            LightSource::Point {
                pos: vec2(3.0, 4.0),
                intensity: 9.0
            }
        );
        assert_eq!(
            parse_light("line:0,0,1,1").unwrap(),
            LightSource::Line {
                a: vec2(0.0, 0.0),
                b: vec2(1.0, 1.0),
                intensity: 1.0
            }
        );
        assert_eq!(
            parse_light("area:0,1,2,3,0.5").unwrap(),
            LightSource::Area {
                rect: Rect {
                    x: 0.0,
                    y: 1.0,
                    w: 2.0,
                    h: 3.0
                },
                intensity: 0.5
            }
        );
        assert_eq!(parse_light("ambient").unwrap(), LightSource::Ambient);
    }

    /// **数が足りない ・多い ・型を知らないときはエラーにする．**
    /// 黙って既定の光源へ落ちると，指定したつもりの絵が別物になる．
    #[test]
    fn a_malformed_light_is_an_error_not_a_default() {
        for bad in [
            "dir:1",
            "dir:1,2,3",
            "point:1",
            "line:1,2,3",
            "area:1,2,3",
            "ambient:1",
            "spot:1,2",
            "dir:x,y",
        ] {
            assert!(parse_light(bad).is_err(), "'{bad}' が通ってしまう");
        }
    }

    /// 説明はそのまま `--light` へ貼り直せる (往復する)．
    #[test]
    fn the_description_round_trips_through_the_parser() {
        for source in [
            LightSource::Directional {
                dir: vec2(-0.6, 0.8),
            },
            LightSource::Point {
                pos: vec2(0.0, -8.0),
                intensity: 1.0,
            },
            LightSource::Line {
                a: vec2(-8.0, -8.0),
                b: vec2(8.0, -8.0),
                intensity: 1.0,
            },
            LightSource::Area {
                rect: Rect {
                    x: -8.0,
                    y: -10.0,
                    w: 16.0,
                    h: 4.0,
                },
                intensity: 1.0,
            },
            LightSource::Ambient,
        ] {
            assert_eq!(parse_light(&describe(source)).unwrap(), source);
        }
    }
}

// --------------------------------------------------------- px scale / px rotate

/// `px scale` — 拡縮する (設計書 5 章 ・D18)．
#[derive(Args, Clone)]
pub struct ScaleArgs {
    pub input: PathBuf,
    pub output: PathBuf,
    /// 倍率．**整数倍なら厳密である**
    #[arg(long)]
    pub factor: f32,
    /// 流儀
    #[arg(long, default_value = "nearest", value_parser = ["nearest", "cleanedge"])]
    pub algo: String,
}

/// `px rotate` — 回転する (設計書 5 章 ・D18)．
#[derive(Args, Clone)]
pub struct RotateArgs {
    pub input: PathBuf,
    pub output: PathBuf,
    /// 角度 (度)．**90 度の倍数は厳密である**
    #[arg(long)]
    pub degrees: f32,
    /// 流儀
    #[arg(long, default_value = "nearest", value_parser = ["nearest", "cleanedge"])]
    pub algo: String,
    /// 画布を広げない．**回転すると外接矩形は必ず伸びるので切れる**
    #[arg(long)]
    pub no_grow: bool,
}

fn resample_options(algo: &str, grow: bool) -> Result<ResampleOptions> {
    Ok(ResampleOptions {
        algo: ResampleAlgo::parse(algo)
            .with_context(|| format!("流儀 '{algo}' を知らない (nearest / cleanedge)"))?,
        grow,
    })
}

fn report_resample(r: &px_core::resample::ResampleReport, input: &Path, output: &Path) {
    println!(
        "  {} -> {} ({}x{} -> {}x{}．流儀 {})",
        input.display(),
        output.display(),
        r.size.0.0,
        r.size.0.1,
        r.size.1.0,
        r.size.1.1,
        r.algo_name
    );
    println!(
        "    不透明な画素 {} -> {} ・切れた画素 {}",
        r.opaque.0, r.opaque.1, r.clipped
    );
    if r.clipped > 0 {
        println!(
            "    ** {} 画素が画布からはみ出て切れた ** — --no-grow を外すこと",
            r.clipped
        );
    }
    if r.filled_opaque > 0 {
        // **«不透明な画素が増えた» の中身を分ける** — 透明添字を持たない絵には
        // «何も無い» が無いので，広げた分は実色で埋まる (D92 ・D107 のとおり
        // 処方せず数えて言う)
        println!(
            "    ** {} 画素は絵ではなく埋め草である ** — この絵は透明添字を宣言して\n\
             \u{3000}\u{3000}いないので，広げた画布は実色 (添字 {}) で埋まった．透明にしたいなら\n\
             \u{3000}\u{3000}入力に透明を 1 色宣言すること",
            r.filled_opaque, 0
        );
    }
    println!("    ** 下地である ** — 設計書 1.3 のとおり，最後は手で直すことが前提である");
    if r.algo_name == "cleanedge" {
        // **回転は必ず画布が伸びるので «広がったか» では解像度を測れない．**
        // 等倍かどうかは呼ぶ側しか知らないので，助言として毎回言う
        println!(
            "    ** cleanedge は «拡大してから回す» と効く ** — 実測で 4 倍 + 30 度なら\n\
             \u{3000}\u{3000}ジャギーが 138.0 -> 122.4 に減る．**等倍では nearest とほぼ同じで**\n\
             \u{3000}\u{3000}往復の差は +1 ポイント程度，ジャギーはむしろ 0.5 〜 1.1 多い"
        );
    }
}

pub fn scale_cmd(args: &ScaleArgs) -> Result<()> {
    let frames = crate::load_frames(&args.input)?;
    if args.algo == "cleanedge" && (args.factor - args.factor.round()).abs() < 1e-4 {
        println!(
            "  ** 整数倍なら nearest が厳密である ** — cleanedge は縁を作り直すので\n\
             \u{3000}\u{3000}1 画素が k x k にならない (実測で 61 枚すべてが厳密でない)．\n\
             \u{3000}\u{3000}拡大だけが目的なら --algo nearest を使うこと"
        );
    }
    let opts = resample_options(&args.algo, true)?;
    let mut out = Vec::with_capacity(frames.len());
    let mut last = None;
    for frame in &frames {
        let mut next = frame.clone();
        for layer in &mut next.layers {
            if let Surface::Indexed(c) = &layer.surface {
                let (n, r) = px_core::resample::scale(c, &frame.palette, args.factor, &opts)?;
                next.size = px_core::math::uvec2(n.width(), n.height());
                layer.surface = Surface::Indexed(n);
                last = Some(r);
            }
        }
        out.push(next);
    }
    let name = resample_stem(&args.output);
    crate::save_frames(&args.output, &out, &name)?;
    if let Some(r) = &last {
        report_resample(r, &args.input, &args.output);
    }
    Ok(())
}

pub fn rotate_cmd(args: &RotateArgs) -> Result<()> {
    let frames = crate::load_frames(&args.input)?;
    let opts = resample_options(&args.algo, !args.no_grow)?;
    let mut out = Vec::with_capacity(frames.len());
    let mut last = None;
    for frame in &frames {
        let mut next = frame.clone();
        for layer in &mut next.layers {
            if let Surface::Indexed(c) = &layer.surface {
                let (n, r) = px_core::resample::rotate(c, &frame.palette, args.degrees, &opts)?;
                next.size = px_core::math::uvec2(n.width(), n.height());
                layer.surface = Surface::Indexed(n);
                last = Some(r);
            }
        }
        out.push(next);
    }
    let name = resample_stem(&args.output);
    crate::save_frames(&args.output, &out, &name)?;
    if let Some(r) = &last {
        report_resample(r, &args.input, &args.output);
    }
    Ok(())
}

/// `px project` — 投影変換 (設計書 6.13)．
///
/// **どの面を写すのか ・どちらへ倒すのかは絵からは決まらないので宣言させる**
/// (D89 ・設計書 6.13 «歪める方向はオブジェクトが向いている方向に合わせる») ．
#[derive(Args, Clone)]
pub struct ProjectArgs {
    pub input: PathBuf,
    pub output: PathBuf,
    /// 投影．**段は名前が決めている** (iso = 2:1 ・dimetric45 = 1:1)
    #[arg(long = "to", value_parser = ["iso", "dimetric45", "oblique"])]
    pub to: String,
    /// 入力がどの面を描いた絵か．**絵からは決まらない**
    ///
    /// `top` は真上から見た絵で 2 軸とも倒れる．`side` は横から見た絵で
    /// **垂直線は立ったまま**である
    #[arg(long, value_parser = ["top", "side"])]
    pub from: String,
    /// 歪める向き．**オブジェクトが向いている方向に合わせること (逆にしない)**
    #[arg(long, value_parser = ["right", "left"])]
    pub facing: String,
    /// 段 `走り:上がり`．**選べるのは oblique だけ** (6.13 の表で 2 通り挙がる行)
    #[arg(long)]
    pub step: Option<String>,
    /// 流儀
    #[arg(long, default_value = "nearest", value_parser = ["nearest", "cleanedge"])]
    pub algo: String,
    /// 画布を広げない．**投影すると外接矩形は伸びるので切れる**
    #[arg(long)]
    pub no_grow: bool,
}

pub fn project_cmd(args: &ProjectArgs) -> Result<()> {
    let projection =
        Projection::parse(&args.to).with_context(|| format!("投影 '{}' を知らない", args.to))?;
    let plane =
        SourcePlane::parse(&args.from).with_context(|| format!("面 '{}' を知らない", args.from))?;
    let facing = Facing::parse(&args.facing)
        .with_context(|| format!("向き '{}' を知らない", args.facing))?;
    let step = args.step.as_deref().map(Step::parse).transpose()?;

    let opts = ProjectOptions {
        projection,
        plane,
        facing,
        step,
        resample: resample_options(&args.algo, !args.no_grow)?,
    };

    let frames = crate::load_frames(&args.input)?;
    let mut out = Vec::with_capacity(frames.len());
    let mut last = None;
    for frame in &frames {
        let mut next = frame.clone();
        for layer in &mut next.layers {
            if let Surface::Indexed(c) = &layer.surface {
                let (n, r) = px_core::project::project(c, &frame.palette, &opts)?;
                next.size = px_core::math::uvec2(n.width(), n.height());
                layer.surface = Surface::Indexed(n);
                last = Some(r);
            }
        }
        out.push(next);
    }
    let name = resample_stem(&args.output);
    crate::save_frames(&args.output, &out, &name)?;
    if let Some(r) = &last {
        report_project(r, &args.input, &args.output);
    }
    Ok(())
}

fn report_project(r: &px_core::project::ProjectReport, input: &Path, output: &Path) {
    println!(
        "  {} -> {} ({} ・{} から ・{} 向き ・段 {})",
        input.display(),
        output.display(),
        r.projection,
        r.plane,
        r.facing,
        r.step.label()
    );
    println!(
        "    受ける軸 {:.2} 度 ・垂直線は{} ・面積比 {:.3}",
        r.degrees,
        if r.keeps_vertical {
            "立ったまま"
        } else {
            "倒れる"
        },
        r.area_ratio
    );
    report_resample(&r.resample, input, output);

    // **段が格子に乗ることを言う．** 設計書 6.13 は手順の方で tan 30 度 を使うが，
    // 同じ節の表が «正確な 30 度は引けないため 2:1 で代用» と書いている．
    // 採ったのは表の側で，実測でもジャギーが 6.4 -> 3.0 に減る
    println!(
        "    ** 段は {} なので格子に乗る ** — 走りの長さが 1 種類になる．\n\
         \u{3000}\u{3000}30 度 (tan 30 = 0.577) は走りが 1 と 2 に割れ，実素材の\n\
         \u{3000}\u{3000}ジャギーが 3.0 -> 6.4 に増える",
        r.step.label()
    );
    println!(
        "    ** 向きは {} で歪めた ** — オブジェクトが向いている方向と\n\
         \u{3000}\u{3000}逆なら --facing を替えること (絵からは決まらない)",
        r.facing
    );
}

/// `px guide` — 投影ガイドグリッドを引く (設計書 6.13)．
#[derive(Args, Clone)]
pub struct GuideArgs {
    pub output: PathBuf,
    /// 投影
    #[arg(long, value_parser = ["iso", "dimetric45", "oblique"])]
    pub projection: String,
    /// どの面のガイドか
    #[arg(long, default_value = "top", value_parser = ["top", "side"])]
    pub from: String,
    /// 倒す向き
    #[arg(long, default_value = "right", value_parser = ["right", "left"])]
    pub facing: String,
    /// 段 `走り:上がり`．**選べるのは oblique だけ**
    #[arg(long)]
    pub step: Option<String>,
    /// **整数の刻みを何回繰り返すか**．等角の刻みは (2, 1) なので 16 なら 1 升が 32x16
    #[arg(long, default_value_t = 16)]
    pub cell: u32,
    /// 画布の大きさ `WxH`
    #[arg(long, default_value = "256x256")]
    pub size: String,
    /// 升をチェス盤状に塗り分ける (設計書 6.13)
    #[arg(long)]
    pub checker: bool,
}

fn parse_size(spec: &str) -> Result<px_core::math::UVec2> {
    let (w, h) = spec
        .split_once(['x', 'X'])
        .with_context(|| format!("画布 '{spec}' を読めない (`WxH` のはず)"))?;
    Ok(px_core::math::uvec2(
        w.trim()
            .parse()
            .with_context(|| format!("幅 '{w}' を読めない"))?,
        h.trim()
            .parse()
            .with_context(|| format!("高さ '{h}' を読めない"))?,
    ))
}

pub fn guide_cmd(args: &GuideArgs) -> Result<()> {
    let projection = Projection::parse(&args.projection)
        .with_context(|| format!("投影 '{}' を知らない", args.projection))?;
    let plane =
        SourcePlane::parse(&args.from).with_context(|| format!("面 '{}' を知らない", args.from))?;
    let facing = Facing::parse(&args.facing)
        .with_context(|| format!("向き '{}' を知らない", args.facing))?;
    let step = args.step.as_deref().map(Step::parse).transpose()?;

    let opts = GuideOptions {
        projection,
        plane,
        facing,
        step,
        cell: args.cell,
        size: parse_size(&args.size)?,
        checker: args.checker,
    };
    let (canvas, palette, r) = px_core::guide::guide(&opts)?;

    let name = resample_stem(&args.output);
    let mut frame = px_core::frame::Frame::new(canvas.size(), palette);
    frame.layers.push(px_core::frame::Layer::new(
        px_core::frame::LayerMeta::named("guide"),
        Surface::Indexed(canvas),
    ));
    crate::save_frames(&args.output, std::slice::from_ref(&frame), &name)?;

    println!(
        "  {} ({} ・{} から ・{} 向き ・段 {})",
        args.output.display(),
        r.projection,
        r.plane,
        r.facing,
        r.step.label()
    );
    println!(
        "    刻み ({}, {}) と ({}, {}) ・1 升 {}x{} ・線 {} 画素",
        r.basis.0.x,
        r.basis.0.y,
        r.basis.1.x,
        r.basis.1.y,
        r.cell_size.x,
        r.cell_size.y,
        r.line_pixels
    );
    // **またぐ枚数は数え上げなので校正しない** — 設計書 6.13 の «2 枚 / 4 枚» が
    // 本当かをそのまま数えて出す (D92 ・D101 と同じ側)
    if r.cells == 0 {
        println!(
            "    ** 画布に収まりきった升が 1 つも無い ** — --cell を小さくするか\n\
             \u{3000}\u{3000}--size を大きくすること (またぐ枚数は数えられない)"
        );
    } else {
        let spans: Vec<String> = r
            .tile_span
            .iter()
            .map(|(tiles, n)| format!("{tiles} 枚 = {n} 升"))
            .collect();
        println!(
            "    一辺 {} の正方形タイルへのまたがり ({} 升): {}",
            r.tile,
            r.cells,
            spans.join(" ・")
        );
    }
    if args.checker {
        println!(
            "    チェス盤の塗り分け: 辺を接して同色になった組 {}",
            r.same_colour_adjacent
        );
    } else {
        println!(
            "    ** 塗り分けていない ** — 設計書 6.13 は交点のドット連結を避けるため\n\
             \u{3000}\u{3000}チェス盤状の塗り分けを使えと言う (--checker)"
        );
    }
    println!("    ** 下地である ** — 設計書 1.3 のとおり，最後は手で直すことが前提である");
    Ok(())
}

fn resample_stem(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().replace(".px.toml", ""))
        .map(|s| {
            s.trim_end_matches(".aseprite")
                .trim_end_matches(".toml")
                .to_string()
        })
        .unwrap_or_default()
}
