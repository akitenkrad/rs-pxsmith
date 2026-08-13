//! `px direction` — 方向展開 (設計書 4.3 ・L2 «8 方向»)．
//!
//! 設計書 4.3 は «**自動ミラーで生成したタイルには lint ルール 7 を blocking で
//! 適用する**» と定める．**その適用側がここである** — `px-core` は絵を作るだけで，
//! 判定は lint を呼べるこちらが持つ．
//!
//! > [!warning] **何を検査していないかを必ず併記する** (D92 の作法) ．
//! > 反転で裏返るのは明度勾配の $x$ 成分だけなので，**横成分の小さい光源では
//! > 反転しても矛盾が起きない** — 真上から照らされた絵は反転しても真上から
//! > 照らされたままである．鳴るのは $\lvert \ell_x \rvert > 0.474$ のときに限り，
//! > これは閾値 $0.55$ から代数的に決まる (校正の対象ではない．D96) ．

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Args;
use px_core::compose::expand_template;
use px_core::direction::{
    Direction, ExpandMode, ExpandOptions, ReshadeSpec, expand, mirror_is_checkable,
};
use px_core::frame::Frame;
use px_core::ramp::{LightPreset, LightSource};
use px_io::Document;

use crate::color_cmds::{CurveArg, PresetArg};
use crate::shape_cmds::parse_light;

#[derive(Args, Clone)]
pub struct DirectionArgs {
    /// 出力先．**`${dir}` を書くこと** — 方向ごとに 1 ファイル出す
    pub output: String,
    /// 描いてある方向 — `方向=パス`．方向は n / ne / e / se / s / sw / w / nw
    #[arg(long = "from", required = true, num_args = 1..)]
    pub from: Vec<String>,
    /// 宣言する光源 (`dir:X,Y` など)．
    ///
    /// **無ければルール 7 は掛からない** — 絵だけから光源方向は決まらないので，
    /// 宣言があるときにしか矛盾を言えない (D89)
    #[arg(long)]
    pub light: Option<String>,
    /// 反転したシルエットへ陰影を導出し直す．
    ///
    /// > [!warning] **元の絵の画素は残らない．** 実素材では中央 100% が書き換わる
    /// > (`px-calib direction`) ．`px shade` で作った素材のための道具である
    #[arg(long)]
    pub reshade: bool,
    /// 再導出の固有色 (`RRGGBB`)．`--reshade` のときだけ使う
    #[arg(long)]
    pub base: Option<String>,
    #[arg(long, value_enum, default_value_t = PresetArg::Clear)]
    pub preset: PresetArg,
    #[arg(long, default_value_t = 5)]
    pub steps: u8,
    #[arg(long, value_enum, default_value_t = CurveArg::PeakMiddle)]
    pub curve: CurveArg,
    /// ルール 7 が鳴っても非ゼロで終わらない (**既定は止める**．設計書 4.3)
    #[arg(long)]
    pub allow_inconsistent: bool,
}

fn load_frames(path: &Path) -> Result<Vec<Frame>> {
    crate::load_frames(path)
}

pub fn direction_cmd(args: &DirectionArgs) -> Result<()> {
    if !args.output.contains("${dir}") {
        bail!("出力先に ${{dir}} を書くこと (方向ごとに 1 ファイル出す)");
    }

    let mut drawn: BTreeMap<Direction, Vec<Frame>> = BTreeMap::new();
    for spec in &args.from {
        let (name, path) = spec
            .split_once('=')
            .with_context(|| format!("--from は '方向=パス' の形で書くこと ('{spec}')"))?;
        let dir = Direction::parse(name).with_context(|| {
            format!("'{name}' は方向ではない (n / ne / e / se / s / sw / w / nw)")
        })?;
        if drawn.contains_key(&dir) {
            bail!("方向 '{name}' が 2 度指定されている");
        }
        drawn.insert(dir, load_frames(Path::new(path))?);
    }

    let light: Option<LightSource> = match &args.light {
        Some(spec) => Some(parse_light(spec)?),
        None => None,
    };

    let mode = if args.reshade {
        let base = match &args.base {
            Some(hex) => px_core::color::Rgba8::from_hex_str(hex)
                .with_context(|| format!("--base '{hex}' を読めない"))?,
            None => bail!("--reshade には --base RRGGBB が要る (シルエットから塗り直すため)"),
        };
        let preset: LightPreset = args.preset.into();
        ExpandMode::Reshade(Box::new(ReshadeSpec {
            base,
            preset,
            steps: args.steps,
            curve: args.curve.into(),
            shade: px_core::shade::ShadeOptions::default(),
        }))
    } else {
        ExpandMode::Mirror
    };

    let (all, report) = expand(&drawn, &ExpandOptions { mode })?;

    println!(
        "描いてある {} 方向 → 反転で {} 方向を作った",
        report.drawn.len(),
        report.generated.len()
    );

    // 設計書 4.3 — **反転で作った方向にルール 7 を blocking で掛ける**
    let cfg = px_lint::LintConfig {
        light,
        ..px_lint::LintConfig::default()
    };
    let threshold = cfg.min_shading_agreement;
    let checkable = light.is_some_and(|l| mirror_is_checkable(l, threshold));

    let mut failed: Vec<Direction> = Vec::new();
    for g in &report.generated {
        let frames = &all[&g.direction];
        let mut hits = 0usize;
        if checkable {
            for frame in frames {
                hits += px_lint::lint_frame(frame, &cfg)
                    .blocking()
                    .filter(|v| v.rule == 7)
                    .count();
            }
        }
        if hits > 0 {
            failed.push(g.direction);
        }
        println!(
            "  {} ← {}{}{}",
            g.direction.as_str(),
            g.from.as_str(),
            if g.reshaded {
                format!(
                    " (再導出．元の画素の {:.0}% を書き換えた)",
                    g.rewritten * 100.0
                )
            } else {
                String::new()
            },
            if hits > 0 {
                format!("  **ルール 7 が {hits} 件**")
            } else {
                String::new()
            }
        );
    }

    // **検査していないことを黙らない** (D92 の作法)
    match (light, checkable) {
        (None, _) => println!(
            "\n検査していない: 光源を宣言していないのでルール 7 を掛けていない (--light で宣言する)"
        ),
        (Some(l), false) => {
            let lx = px_core::outline::light_direction(l).x;
            println!(
                "\n検査していない: この光源は横成分が {:.3} で，|ℓx| > {:.3} でないと\n\
                 反転しても光源と矛盾しない (閾値 {:.2} から代数的に決まる)．\n\
                 見逃しではなく «矛盾が起きない» という意味である",
                lx,
                ((1.0 - threshold) / 2.0f32).sqrt(),
                threshold
            );
        }
        (Some(_), true) => println!(
            "\nルール 7 を掛けた (反転で作った {} 方向)",
            report.generated.len()
        ),
    }

    if !report.missing.is_empty() {
        println!(
            "\n埋まらなかった方向: {} — **反転では作れない**ので描くこと\n\
             (n と s は軸の上にあるので鏡像が自分自身である)",
            report
                .missing
                .iter()
                .map(|d| d.as_str())
                .collect::<Vec<_>>()
                .join(" ・")
        );
    }

    for (dir, frames) in &all {
        let mut vars = BTreeMap::new();
        vars.insert("dir".to_string(), dir.as_str().to_string());
        let path = PathBuf::from(expand_template(&args.output, &vars)?);
        let doc = Document::from_frames(frames)
            .with_context(|| format!("{} を組み立てられない", path.display()))?;
        doc.write(&path)
            .with_context(|| format!("{} を書き出せない", path.display()))?;
    }
    println!("\n{} ファイルを書き出した", all.len());

    if !failed.is_empty() && !args.allow_inconsistent {
        bail!(
            "反転で作った {} 方向が宣言した光源と矛盾している ({})．\
             --reshade で陰影を導出し直すか，その方向を手で描くこと\
             (承知の上なら --allow-inconsistent)",
            failed.len(),
            failed
                .iter()
                .map(|d| d.as_str())
                .collect::<Vec<_>>()
                .join(" ・")
        );
    }
    Ok(())
}
