//! `pxsmith atmos` — 空気遠近法と多重スクロールメタ (設計書 4.4 ・5 章)．
//!
//! **奥行きを持てるのは L0 (`.px.toml`) だけである** — `.aseprite` には対応する
//! 概念が無く (`pxsmith-io` の `project_meta`) ，L0 は 1 ファイル 1 レイヤ (D9) なので
//! `[meta] depth` は**ファイル 1 枚に 1 つ**である．多重スクロールの単位が
//! ちょうどそれなので，**複数の絵を 1 度に受け取り，1 つのメタを書く**．
//!
//! `.aseprite` を渡すときは `--depth` で宣言する — D81 «ランプの宣言が残らない» ・
//! D119 «`.aseprite` は kind を持てない» と**同じ形の 5 度目**なので，
//! 黙って既定へ倒さず落とす．
//!
//! 判断の根拠は [`pxsmith_core::atmos`] のモジュール文書にまとめてある．

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Args;
use pxsmith_core::atmos::{AtmosOptions, HazeTable, ScrollDoc, ScrollLayer, atmos};
use pxsmith_core::color::Rgba8;
use pxsmith_core::frame::Depth;
use pxsmith_core::ramp::LightPreset;

use crate::{load_frames, save_frames};

#[derive(Args, Clone)]
pub struct AtmosArgs {
    /// 出力先のひな型．`${name}` は入力のファイル名，`${depth}` は奥行きになる
    pub output: String,
    /// 霞ませる絵．**奥行きを持つのは L0 だけ**なので，`.aseprite` には `--depth` が要る
    #[arg(long = "input", num_args = 1.., required = true)]
    pub inputs: Vec<PathBuf>,
    /// **宣言する空の色** (`RRGGBB`)．絵からは決まらない (D89)
    #[arg(long)]
    pub sky: String,
    /// 奥行きごとの寄せ具合．`--haze background=0.6` のように書く (0.0 〜 1.0)
    ///
    /// **既定は «何もしない»**．霞の厚みは «どれだけ遠いか» であって絵に書かれて
    /// いないので，呼ぶ側が宣言する (D126 が `--ramp` を必須にしたのと同じ)
    #[arg(long = "haze", num_args = 1..)]
    pub haze: Vec<String>,
    /// 入力が奥行きを持たないときに宣言する (`foreground` / `midground` / `background`)
    #[arg(long)]
    pub depth: Option<String>,
    /// **宣言する**視差速度係数．`--speed background=0.25` のように書く
    ///
    /// **導出しない** — 視差の速さはゲーム側の選択である (D92 ・D95)
    #[arg(long = "speed", num_args = 1..)]
    pub speed: Vec<String>,
    /// 多重スクロールメタの書き出し先 (JSON)
    #[arg(long)]
    pub scroll_meta: Option<PathBuf>,
    /// 光源プリセット．**空の色相を並べて見せるだけ** — ここから空の色は作らない
    #[arg(long)]
    pub light: Option<String>,
    /// «線の上» と認める遠回りの許容．既定は `pxsmith aa` と同じ
    #[arg(long)]
    pub tolerance: Option<f32>,
}

fn stem(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().replace(".px.toml", ""))
        .map(|s| {
            s.trim_end_matches(".aseprite")
                .trim_end_matches(".toml")
                .to_string()
        })
        .unwrap_or_default()
}

/// `key=value` の並びを読む．**知らない鍵は落とす** — 黙って無視すると，
/// 綴りを間違えた宣言が «何もしない» になって気付けない．
fn parse_by_depth(items: &[String], what: &str) -> Result<BTreeMap<Depth, f32>> {
    let mut out = BTreeMap::new();
    for item in items {
        let (k, v) = item
            .split_once('=')
            .with_context(|| format!("--{what} は '奥行き=値' で書く: {item}"))?;
        let depth = Depth::parse(k.trim()).with_context(|| {
            format!("奥行き '{k}' を解釈できない (foreground / midground / background)")
        })?;
        let value: f32 = v
            .trim()
            .parse()
            .with_context(|| format!("{what} の値を読めない: {v}"))?;
        if out.insert(depth, value).is_some() {
            bail!("--{what} に {} が 2 度出てくる", depth.as_str());
        }
    }
    Ok(out)
}

/// メタから見た相対パスにする．
///
/// **絶対パスを書くとプロジェクトを移した瞬間に読めなくなる** — D131 が
/// ステップキーで «レシピからの相対パスだけを混ぜる» と決めたのと同じ理由．
/// 共通の親が無いときは書き換えない (推測しない)．
fn relative_to(path: &Path, meta: Option<&Path>) -> String {
    let base = meta
        .and_then(|m| m.parent())
        .filter(|p| !p.as_os_str().is_empty());
    match base.and_then(|b| path.strip_prefix(b).ok()) {
        Some(rel) => rel.to_string_lossy().to_string(),
        None => path.to_string_lossy().to_string(),
    }
}

fn hue_of(c: Rgba8) -> f32 {
    let lab = c.to_oklab();
    lab.b.atan2(lab.a).to_degrees().rem_euclid(360.0)
}

pub fn atmos_cmd(args: &AtmosArgs) -> Result<()> {
    let sky = Rgba8::from_hex_str(&args.sky)
        .with_context(|| format!("空の色 '{}' を読めない (RRGGBB)", args.sky))?;

    let table = parse_by_depth(&args.haze, "haze")?;
    let haze = HazeTable {
        foreground: table.get(&Depth::Foreground).copied().unwrap_or_default(),
        midground: table.get(&Depth::Midground).copied().unwrap_or_default(),
        background: table.get(&Depth::Background).copied().unwrap_or_default(),
    };
    if haze.is_empty() {
        bail!(
            "--haze が 1 つも無い．霞の厚みは絵からは決まらないので宣言する\n\
             例: --haze midground=0.3 --haze background=0.6"
        );
    }
    let speed = parse_by_depth(&args.speed, "speed")?;

    let declared = args
        .depth
        .as_deref()
        .map(|s| {
            Depth::parse(s).with_context(|| {
                format!("--depth '{s}' を解釈できない (foreground / midground / background)")
            })
        })
        .transpose()?;

    let opts = AtmosOptions {
        sky,
        haze,
        tolerance: args.tolerance.unwrap_or(AtmosOptions::DEFAULT_TOLERANCE),
    };

    // **空の色相を並べて見せる．** プリセットから引けるのは色相だけで，明度と
    // 彩度は無い — だから «導出» はせず，2 つの宣言を突き合わせるに留める
    if let Some(name) = &args.light {
        let preset =
            LightPreset::parse(name).with_context(|| format!("光源プリセット '{name}' が無い"))?;
        println!(
            "  空の色相: 宣言 {:.0}° 対 プリセット {} の空 {:.0}° (差 {:.0}°)\n\
             \u{3000}\u{3000}** プリセットから引けるのは色相だけである ** — 明度と彩度を\n\
             \u{3000}\u{3000}持っていないので，ここから空の色は作らない (--sky が要る)",
            hue_of(sky),
            preset.as_str(),
            preset.sky_hue(),
            (hue_of(sky) - preset.sky_hue())
                .abs()
                .min(360.0 - (hue_of(sky) - preset.sky_hue()).abs()),
        );
    }

    let mut layers = Vec::new();
    let mut ineffective = 0usize;

    for input in &args.inputs {
        let frames = load_frames(input)?;
        let first = frames.first().context("フレームが 1 つも無い")?;

        let from_file = first.layers.iter().find_map(|l| l.meta.depth);
        let depth = match (from_file, declared) {
            (Some(d), None) => d,
            (None, Some(d)) => d,
            (Some(a), Some(b)) => {
                if a != b {
                    bail!(
                        "{} は depth = '{}' と書いてあるのに --depth {} と宣言されている\n\
                         どちらが正しいか決まらないので落とす",
                        input.display(),
                        a.as_str(),
                        b.as_str()
                    );
                }
                a
            }
            (None, None) => bail!(
                "{} は奥行きを持っていない．\n\
                 ** 欄を持っているのは L0 (.px.toml) の [meta] depth だけである ** —\n\
                 .aseprite には対応する概念が無いので (D81 ・D119 と同じ形) ，\n\
                 --depth で宣言すること",
                input.display()
            ),
        };

        let (out, report) = atmos(&frames, depth, &opts)?;

        let name = stem(input);
        let vars = BTreeMap::from([
            ("name".to_string(), name.clone()),
            ("depth".to_string(), depth.as_str().to_string()),
        ]);
        let path = PathBuf::from(pxsmith_core::compose::expand_template(&args.output, &vars)?);
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("{} を作れない", dir.display()))?;
        }
        save_frames(&path, &out, &name)?;

        println!(
            "  {} ({}, 寄せ {:.2}) -> {}",
            input.display(),
            depth.as_str(),
            report.amount,
            path.display()
        );
        println!(
            "    色 {} 中 {} を置き換え ・段が無くて動かさなかった {} ・潰れた {} ・画素 {}",
            report.colors, report.moved, report.no_step, report.collapsed, report.pixels
        );
        println!(
            "    明度の幅 {:.4} -> {:.4} ({:.2} 倍)",
            report.spread.0,
            report.spread.1,
            report.spread_ratio()
        );
        if report.ineffective() && report.amount > 0.0 {
            ineffective += 1;
            println!(
                "    ** 1 色も動かなかった ** — このパレットに «空へ寄せた先» が無い．\n\
                 \u{3000}\u{3000}道具は色を作らないので (D94) ，直すならパレットに霞の段を足すこと"
            );
        }
        if report.collapsed * 2 >= report.colors && report.colors > 0 {
            println!(
                "    ** 色の半分以上が潰れた ** — そのパレットにその霞は無いという結果で\n\
                 \u{3000}\u{3000}あって，失敗ではない．濃くしたいならパレットに段を足すこと．\n\
                 \u{3000}\u{3000}ディザの 2 色が 1 色へ潰れると縞が塊になるので，pxsmith lint の\n\
                 \u{3000}\u{3000}ルール 10 (ディザの塊化) で確かめること"
            );
        }

        layers.push(ScrollLayer {
            file: relative_to(&path, args.scroll_meta.as_deref()),
            depth: depth.as_str().to_string(),
            haze: report.amount,
            speed: speed.get(&depth).copied(),
        });
    }

    if let Some(meta) = &args.scroll_meta {
        let doc = ScrollDoc::new(sky, layers);
        std::fs::write(meta, doc.to_json()?)
            .with_context(|| format!("{} を書き出せない", meta.display()))?;
        println!("  多重スクロールメタ -> {}", meta.display());
        if doc.undeclared() > 0 {
            println!(
                "    ** 速度係数を書いていないレイヤが {} ある ** — 視差の速さは\n\
                 \u{3000}\u{3000}ゲーム側の選択であって絵からも depth からも決まらないので，\n\
                 \u{3000}\u{3000}推測せずに空けてある．要るなら --speed で宣言すること (D92)",
                doc.undeclared()
            );
        }
    }

    if ineffective > 0 {
        println!("\n  {} 枚で 1 色も動かなかった (効かなかった)", ineffective);
    }
    Ok(())
}
