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
use px_core::geom::Mask;
use px_core::outline::{OutlineOptions, OutlineStyle, outline};
use px_core::ramp::{LightSource, build_lighting};
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
            "  残り {} 件: 移動上限 {} を超える {} 件 ・直し方が無い {} 件",
            report.remaining, args.max_move, report.over_limit, report.no_candidate
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
fn parse_light(spec: &str) -> Result<LightSource> {
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
