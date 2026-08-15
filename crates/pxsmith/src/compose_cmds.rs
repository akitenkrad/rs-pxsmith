//! `pxsmith compose` — パーツ合成 ・variants 展開 (設計書 5 章 ・4.2 ・D42)．

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Args;
use pxsmith_core::compose::{
    Alignment, ComposeOptions, DelayMode, Part, compose, expand_template, expand_variants,
};
use pxsmith_core::math::ivec2;
use pxsmith_io::Document;
use pxsmith_io::l0::L0Document;

#[derive(Args, Clone)]
pub struct ComposeArgs {
    /// 出力先．`${変数}` を書ける (`--for-each` と組で使う)
    pub output: String,
    /// 重ねるパーツ．**宣言順が重ね順**で，先頭が一番下 ・基準になる
    ///
    /// 書き方は `パス`，`パス@アンカー`，`パス@自分のアンカー=基準のアンカー`．
    /// アンカーを書かないパーツは原点合わせになる (既に揃っている素材) ．
    #[arg(long = "part", required = true, num_args = 1..)]
    pub parts: Vec<String>,
    /// パーツを遅らせる — `パーツ名=フレーム数` (オーバーラップ / フォロースルー．D42)
    #[arg(long = "part-delay", num_args = 1..)]
    pub part_delay: Vec<String>,
    /// 遅らせたぶんをどう埋めるか．
    ///
    /// **既定は `hold`** (端のフレームを保つ) ．`wrap` は**列がループのときだけ
    /// 正しい** — ループかどうかは絵から決まらないので，既定にはしない
    #[arg(long, default_value = "hold")]
    pub delay_mode: String,
    /// 先頭のパーツの画布に切り揃える．
    ///
    /// **既定は広げる．** 実素材のシルエットは 38 枚中 33 枚が画布の縁に接して
    /// いるので (`pxsmith-calib compose`) ，切ると動かした側が必ず欠ける
    #[arg(long)]
    pub clip: bool,
    /// 変数の直積で展開する — `変数=値,値,値` (設計書 4.2 の `for_each`)
    #[arg(long = "for-each", num_args = 1..)]
    pub for_each: Vec<String>,
}

/// `名前=値` を割る．
fn split_pair(text: &str, what: &str) -> Result<(String, String)> {
    let (k, v) = text
        .split_once('=')
        .with_context(|| format!("{what} は '名前=値' の形で書くこと ('{text}')"))?;
    Ok((k.to_string(), v.to_string()))
}

/// `パス@自分のアンカー=基準のアンカー` を割る．
fn split_part(spec: &str) -> (String, Option<Alignment>) {
    match spec.split_once('@') {
        None => (spec.to_string(), None),
        Some((path, anchors)) => {
            let align = match anchors.split_once('=') {
                Some((mine, base)) => Alignment {
                    part: mine.to_string(),
                    base: base.to_string(),
                },
                // 片側だけなら同じ名前どうしを合わせる
                None => Alignment {
                    part: anchors.to_string(),
                    base: anchors.to_string(),
                },
            };
            (path.to_string(), Some(align))
        }
    }
}

/// パーツ名はファイル名の «語幹» にする — レイヤ名と `--part-delay` の鍵になる．
fn part_name(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().replace(".px.toml", ""))
        .map(|s| {
            s.trim_end_matches(".aseprite")
                .trim_end_matches(".toml")
                .to_string()
        })
        .unwrap_or_else(|| "part".to_string())
}

/// フレーム列とアンカーを読む．**アンカーを持てるのは L0 だけである** (設計書 4.1) ．
fn load_part(path: &Path) -> Result<Part> {
    let name = path.to_string_lossy();
    if name.ends_with(".px.toml") || name.ends_with(".toml") {
        let doc = L0Document::read(path)
            .with_context(|| format!("{} を L0 として読めない", path.display()))?;
        let frames = doc
            .to_frames(path)
            .with_context(|| format!("{} を作業層へ変換できない", path.display()))?;
        let mut part = Part::new(part_name(path), frames);
        for (k, v) in &doc.meta.anchors {
            part.anchors.insert(k.clone(), ivec2(v[0], v[1]));
        }
        return Ok(part);
    }
    let doc = Document::read(path).with_context(|| format!("{} を読めない", path.display()))?;
    let frames = (0..doc.frame_count())
        .map(|i| {
            doc.project(pxsmith_io::FrameId(i))
                .with_context(|| format!("{} のフレーム {i} を射影できない", path.display()))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Part::new(part_name(path), frames))
}

pub fn compose_cmd(args: &ComposeArgs) -> Result<()> {
    let delay_mode = DelayMode::parse(&args.delay_mode)
        .with_context(|| format!("--delay-mode は hold か wrap ('{}')", args.delay_mode))?;

    let mut delays: BTreeMap<String, u32> = BTreeMap::new();
    for spec in &args.part_delay {
        let (name, value) = split_pair(spec, "--part-delay")?;
        let n: u32 = value
            .parse()
            .with_context(|| format!("--part-delay のフレーム数を読めない ('{value}')"))?;
        delays.insert(name, n);
    }

    let mut vars: Vec<(String, Vec<String>)> = Vec::new();
    for spec in &args.for_each {
        let (name, values) = split_pair(spec, "--for-each")?;
        let list: Vec<String> = values.split(',').map(|s| s.trim().to_string()).collect();
        if list.iter().any(|s| s.is_empty()) {
            bail!("--for-each の値に空のものがある ('{spec}')");
        }
        vars.push((name, list));
    }

    let variants = expand_variants(&vars);
    let opts = ComposeOptions {
        delay_mode,
        clip: args.clip,
    };

    // **遅延の鍵は variants をまたいで数える．**
    // `--for-each cap=red,green` のとき，パーツ名は展開ごとに `cap_red` ・`cap_green`
    // と変わる．展開 1 つずつで «当たらなければ誤り» と言うと，**正しい指定が
    // 必ず片方で落ちる** (端から端まで CLI で通して初めて出た) ．鍵の側も
    // `${}` を展開したうえで，**どの展開でも 1 度も当たらなかったものだけ**を誤りとする．
    let mut delay_hits: BTreeMap<String, usize> = delays.keys().map(|k| (k.clone(), 0)).collect();

    for variant in &variants {
        let output = expand_template(&args.output, variant)?;
        // 鍵の側も同じ展開に掛ける — `--part-delay 'cap_${cap}=1'` と書ける
        let resolved: BTreeMap<String, (String, u32)> = delays
            .iter()
            .map(|(k, v)| Ok((k.clone(), (expand_template(k, variant)?, *v))))
            .collect::<Result<_>>()?;

        let mut parts: Vec<Part> = Vec::new();
        for spec in &args.parts {
            let expanded = expand_template(spec, variant)?;
            let (path, align) = split_part(&expanded);
            let mut part = load_part(Path::new(&path))?;
            part.align = align;
            for (key, (name, n)) in &resolved {
                if *name == part.name {
                    part.delay = *n;
                    *delay_hits.entry(key.clone()).or_default() += 1;
                }
            }
            parts.push(part);
        }

        let (frames, report) = compose(&parts, &opts)?;
        let out_path = PathBuf::from(&output);
        crate::save_frames(&out_path, &frames, "compose")?;

        println!(
            "{output} — {}x{} ・{} フレーム ・{} 色{}",
            report.canvas.x,
            report.canvas.y,
            report.frames,
            report.colors,
            if report.grew {
                format!(
                    " (画布を広げた．元の原点は ({},{}))",
                    report.origin.x, report.origin.y
                )
            } else {
                String::new()
            }
        );
        for p in &report.placements {
            let mut notes: Vec<String> = Vec::new();
            if p.covered > 0 {
                notes.push(format!("{} 画素が隠れている", p.covered));
            }
            if p.clipped > 0 {
                notes.push(format!("**{} 画素を切り捨てた**", p.clipped));
            }
            println!(
                "  {:<16} ({:>3},{:>3}){}",
                p.name,
                p.offset.x,
                p.offset.y,
                if notes.is_empty() {
                    String::new()
                } else {
                    format!("  {}", notes.join(" ・"))
                }
            );
        }
        if report.colors > 62 {
            eprintln!(
                "注意: {} 色は L0 テキスト形式の上限 62 色を超えている (`.aseprite` では扱える)",
                report.colors
            );
        }
    }

    // **1 度も当たらなかった鍵は打ち間違いである．** 黙って «遅延なし» で出すと，
    // オーバーラップを指定したつもりの絵がそのまま出荷される
    let missed: Vec<&String> = delay_hits
        .iter()
        .filter(|(_, n)| **n == 0)
        .map(|(k, _)| k)
        .collect();
    if !missed.is_empty() {
        bail!(
            "--part-delay の {} に当たるパーツがどの展開にも無い",
            missed
                .iter()
                .map(|s| format!("'{s}'"))
                .collect::<Vec<_>>()
                .join(" ・")
        );
    }
    Ok(())
}
