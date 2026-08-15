//! **`pxsmith atmos` を実装する前に «効くのか» を測る** (`pxsmith-calib atmos`)．
//!
//! 空気遠近法は «遠いものほど空の色へ寄せ，明暗差を落とす» ものである [^upc]．
//! ところがこちらは**インデックスカラーの固定パレット**なので混色ができない
//! (D2 ・D4) ．並べ替えるだけの道具は色を作ってはいけない (D94) から，
//! **atmos にできるのは添字の置き換えだけ**である．
//!
//! したがって «寄せた先の色がパレットに在るか» が先に立つ．無ければ atmos は
//! **効かない** — `pxsmith aa` の 81.3% (D83) ・サブピクセルの 53.6% (D124) と
//! 同じ量をここで測る．
//!
//! # 測る規則は 2 つ．対照は «何もしない»
//!
//! | 規則 | 選び方 |
//! | --- | --- |
//! | `nearest` | 狙い $\mathrm{lerp}(c, s, t)$ に最も近いパレットの色 (`Palette::nearest`) |
//! | `between` | **$c$ と空を結ぶ線の上**にあるものだけを候補にし，狙いに最も近いものを選ぶ ([`pxsmith_core::atmos::nearest_toward`]) |
//! | 対照 | **動かさない** (`pxsmith anim tween` の «動かさない» と同じ役目) |
//!
//! `between` は `pxsmith aa` の [`pxsmith_core::aa::nearest_between`] と同じ形の条件だが，
//! **終点がパレットの添字ではなく «宣言された空の色»** なので別に書いてある．
//! **実体は道具の側 (`pxsmith_core::atmos`) にあり，ここはそれを呼ぶ** — 測る口が
//! 自前の写しを持つと «測ったのと違うものが出荷される» (D110)．
//!
//! # 真値のある場面
//!
//! パレットに «霞ませた先» を足したものを作れば，**正解の添字が決まる**．
//! 規則がそれを引けなければ規則の側が誤りである．合わせて，元のパレットのままの
//! 場合と並べれば «効かないのはパレットのせいか規則のせいか» が分かれる．
//!
//! [^upc]: ULTIMATE PIXEL CREW REPORT PAGE:038 «遠くのものには白青みをかけ空との
//!   色の差を少なくし，より近くのものは色味が濃くなるように描く»．

use anyhow::{Context, Result};
use pxsmith_core::atmos::nearest_toward;
use pxsmith_core::color::{Oklab, Rgba8, distance_sq, oklab_of};
use pxsmith_core::palette::Palette;
use pxsmith_core::quantize::oklab_to_rgba;
use std::path::Path;

use crate::animcal::{indexed, name_of, png_files};

/// 測るときに使う空の色．**宣言である** — 絵からは決まらない (D89 と同じ理由)．
///
/// 発明を避けるため，同梱の CC0 パレット `palettes/sweetie-16.hex` から
/// 書籍の 3 場面 (晴天 ・曇天 ・夕方) に当たる色を取っている．
pub const SKIES: [(&str, &str); 3] = [
    ("clear", "41a6f6"),
    ("overcast", "f4f4f4"),
    ("sunset", "ef7d57"),
];

/// «線の上» と認める遠回りの許容．**道具の側から引く** — ここで別の値を書くと
/// «測ったのと違うものが出荷される» (D110 と同じ形の誤り)．
pub const SEGMENT_TOLERANCE: f32 = pxsmith_core::atmos::AtmosOptions::DEFAULT_TOLERANCE;

fn lerp(a: Oklab, b: Oklab, t: f32) -> Oklab {
    Oklab::new(
        a.l + (b.l - a.l) * t,
        a.a + (b.a - a.a) * t,
        a.b + (b.b - a.b) * t,
    )
}

fn de(x: Oklab, y: Oklab) -> f32 {
    distance_sq(x, y, 1.0).sqrt()
}

// ------------------------------------------------------------------ 行

/// (絵，空，寄せ具合) 1 件．
#[derive(Clone, Debug)]
pub struct AtmosRow {
    pub file: String,
    pub sky: &'static str,
    /// パレットの出どころ — `own` (絵のまま) か `extended` (霞ませた先を足した真値)．
    pub group: &'static str,
    pub amount: f32,
    /// パレットの色数．
    pub palette_len: usize,
    /// 検査した «使っている不透明な添字» の数．
    pub colors: usize,
    /// 画素の数 (不透明のみ)．
    pub pixels: usize,
    /// `nearest` が添字を動かした色の数．
    pub moved_nearest: usize,
    /// `between` が置き換え先を見つけた色の数．
    pub moved_between: usize,
    /// `nearest` が選んだ先が **空に近づいていない** 色の数．
    pub wrong_way: usize,
    /// 真値があるとき，正解の添字を引けた色の数 (`group = "extended"` のみ)．
    pub exact_nearest: usize,
    pub exact_between: usize,
    /// 画素で重み付けした色差の中央値．`ideal` は連続色の $\mathrm{lerp}(c, s, t)$．
    pub de_control: f32,
    pub de_nearest: f32,
    pub de_between: f32,
    /// 出力に残った相異なる添字の数 (**明暗差が落ちると減る**)．
    pub distinct_nearest: usize,
    /// **前の段より空から遠ざかった色の数** — 奥へ行くほど霞むはずなので 0 でなければ
    /// «奥の方が濃い» が起きている．
    pub non_monotone: usize,
    /// 使っている色の明度の幅 (元 / `nearest` の出力)．**空気遠近法は明暗差を落とす**
    /// ものなので，増えていたら «寄せている» と言えない．
    pub spread_before: f32,
    pub spread_after: f32,
    /// `nearest` が **線から外れた分**の中央値 — $d(c, o) + d(o, s) - d(c, s)$．
    /// 0 なら «元の色と空を結ぶ線» の上に落ちている．大きいと «霞んだ» のではなく
    /// **色が変わった**ことになる．
    pub detour_nearest: f32,
    /// **線から許容 (`SEGMENT_TOLERANCE`) より外れた色の数** — 中央値では見えない裾．
    /// ここが多いと «霞んだ» ではなく «色が変わった» になる．
    pub off_line: usize,
    /// `between` 側の «残った色» と明度の幅．**選ぶ規則の側で数えないと意味が無い**．
    pub distinct_between: usize,
    pub spread_after_between: f32,
}

pub const ATMOS_HEADER: &str = "file,sky,group,amount,palette_len,colors,pixels,moved_nearest,moved_between,wrong_way,exact_nearest,exact_between,de_control,de_nearest,de_between,distinct_nearest,non_monotone,spread_before,spread_after,detour_nearest,off_line,distinct_between,spread_after_between";

pub fn atmos_csv(r: &AtmosRow) -> String {
    format!(
        "{},{},{},{:.2},{},{},{},{},{},{},{},{},{:.4},{:.4},{:.4},{},{},{:.4},{:.4},{:.4},{},{},{:.4}",
        r.file,
        r.sky,
        r.group,
        r.amount,
        r.palette_len,
        r.colors,
        r.pixels,
        r.moved_nearest,
        r.moved_between,
        r.wrong_way,
        r.exact_nearest,
        r.exact_between,
        r.de_control,
        r.de_nearest,
        r.de_between,
        r.distinct_nearest,
        r.non_monotone,
        r.spread_before,
        r.spread_after,
        r.detour_nearest,
        r.off_line,
        r.distinct_between,
        r.spread_after_between
    )
}

/// 飛ばした件の内訳．**測る口が «飛ばした件» を数えていないと，落ちているのが
/// 見えない** (D128)．
#[derive(Clone, Debug, Default)]
pub struct AtmosSkipped {
    /// 色が 256 を超えて添字にできなかった絵．
    pub not_indexable: usize,
    /// 不透明な画素が 1 つも無い絵．
    pub empty: usize,
    /// 霞ませた先を足すと 256 色を超えるので真値を作れなかった (絵，空) の組．
    pub extended_overflow: usize,
}

// ------------------------------------------------------------------ 測る

struct Used {
    /// (添字, 画素数)．
    entries: Vec<(u8, usize)>,
    pixels: usize,
}

fn used_indices(canvas: &pxsmith_core::canvas::IndexedCanvas, palette: &Palette) -> Used {
    let mut count = [0usize; 256];
    for &i in canvas.pixels() {
        if palette.get(i).is_some_and(|c| c.a == 0) {
            continue;
        }
        count[i as usize] += 1;
    }
    let entries: Vec<(u8, usize)> = (0..=255u8)
        .filter(|i| count[*i as usize] > 0)
        .map(|i| (i, count[i as usize]))
        .collect();
    let pixels = entries.iter().map(|(_, n)| n).sum();
    Used { entries, pixels }
}

fn weighted_median(mut v: Vec<(f32, usize)>) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    let total: usize = v.iter().map(|(_, w)| w).sum();
    let mut acc = 0usize;
    for (x, w) in &v {
        acc += w;
        if acc * 2 >= total {
            return *x;
        }
    }
    v.last().map(|(x, _)| *x).unwrap_or(0.0)
}

/// (元の添字, 段の番号) → 正解の添字．
type Truth = Vec<((u8, u32), u8)>;

/// «霞ませた先» をパレットへ足す — **真値のある場面を作る**．
///
/// 返り値は拡張したパレットと，(元の添字, 寄せ具合) → 正解の添字の対応．
fn extend_palette(
    palette: &Palette,
    used: &Used,
    sky: Oklab,
    amounts: &[f32],
) -> Option<(Palette, Truth)> {
    let mut entries: Vec<Rgba8> = palette.entries().to_vec();
    let mut truth = Vec::new();
    for &(c, _) in &used.entries {
        let lab = palette.lab_of(c)?;
        for (k, &t) in amounts.iter().enumerate() {
            let want = oklab_to_rgba(lerp(lab, sky, t));
            let at = match entries.iter().position(|e| *e == want) {
                Some(i) => i,
                None => {
                    if entries.len() >= 256 {
                        return None;
                    }
                    entries.push(want);
                    entries.len() - 1
                }
            };
            truth.push(((c, k as u32), at as u8));
        }
    }
    Palette::new(entries).ok().map(|p| (p, truth))
}

#[allow(clippy::too_many_arguments)]
fn measure(
    file: &str,
    sky_name: &'static str,
    group: &'static str,
    palette: &Palette,
    used: &Used,
    sky: Oklab,
    amounts: &[f32],
    truth: Option<&Truth>,
) -> Vec<AtmosRow> {
    let mut rows = Vec::new();
    // 前の段でどこへ落ちたか (単調性を見るため)
    let mut previous: std::collections::BTreeMap<u8, f32> = std::collections::BTreeMap::new();
    for (k, &t) in amounts.iter().enumerate() {
        let mut row = AtmosRow {
            file: file.to_string(),
            sky: sky_name,
            group,
            amount: t,
            palette_len: palette.len(),
            colors: used.entries.len(),
            pixels: used.pixels,
            moved_nearest: 0,
            moved_between: 0,
            wrong_way: 0,
            exact_nearest: 0,
            exact_between: 0,
            de_control: 0.0,
            de_nearest: 0.0,
            de_between: 0.0,
            distinct_nearest: 0,
            non_monotone: 0,
            spread_before: 0.0,
            spread_after: 0.0,
            detour_nearest: 0.0,
            off_line: 0,
            distinct_between: 0,
            spread_after_between: 0.0,
        };
        let (mut dc, mut dn, mut db) = (Vec::new(), Vec::new(), Vec::new());
        let mut detour: Vec<(f32, usize)> = Vec::new();
        let mut distinct = std::collections::BTreeSet::new();
        let mut distinct_b = std::collections::BTreeSet::new();
        let mut l_after_b: Vec<f32> = Vec::new();
        let (mut l_before, mut l_after): (Vec<f32>, Vec<f32>) = (Vec::new(), Vec::new());
        let mut current: std::collections::BTreeMap<u8, f32> = std::collections::BTreeMap::new();

        for &(c, weight) in &used.entries {
            let Some(lab) = palette.lab_of(c) else {
                continue;
            };
            let ideal = lerp(lab, sky, t);
            let span = de(lab, sky);

            let a = palette
                .nearest(oklab_to_rgba(ideal), 1.0)
                .filter(|i| palette.get(*i).is_some_and(|c| c.a != 0))
                .unwrap_or(c);
            let b = nearest_toward(palette, c, sky, ideal, SEGMENT_TOLERANCE, None);

            if a != c {
                row.moved_nearest += 1;
                // 空へ近づいていない先を選んでいないか
                if let Some(la) = palette.lab_of(a)
                    && de(la, sky) >= span
                {
                    row.wrong_way += 1;
                }
            }
            if b.is_some() {
                row.moved_between += 1;
            }
            distinct.insert(a);
            distinct_b.insert(b.unwrap_or(c));

            if let Some(truth) = truth
                && let Some((_, want)) = truth.iter().find(|((i, j), _)| *i == c && *j == k as u32)
            {
                if a == *want {
                    row.exact_nearest += 1;
                }
                // **«動かさない» は «元の添字を選んだ» ことである** — $t = 0$ で
                // `None` を «外した» と数えると 1 / 7 だけ低く出る
                if b.unwrap_or(c) == *want {
                    row.exact_between += 1;
                }
            }

            dc.push((de(lab, ideal), weight));
            l_before.push(lab.l);
            if let Some(la) = palette.lab_of(a) {
                dn.push((de(la, ideal), weight));
                l_after.push(la.l);
                let reached = de(la, sky);
                let off = de(lab, la) + reached - span;
                detour.push((off, weight));
                if off > SEGMENT_TOLERANCE {
                    row.off_line += 1;
                }
                if previous.get(&c).is_some_and(|p| reached > *p + 1e-6) {
                    row.non_monotone += 1;
                }
                current.insert(c, reached);
            }
            let bl = b.and_then(|i| palette.lab_of(i)).unwrap_or(lab);
            db.push((de(bl, ideal), weight));
            l_after_b.push(bl.l);
        }

        let range = |v: &[f32]| -> f32 {
            let (lo, hi) = v
                .iter()
                .fold((f32::MAX, f32::MIN), |(lo, hi), x| (lo.min(*x), hi.max(*x)));
            if v.is_empty() { 0.0 } else { hi - lo }
        };
        row.detour_nearest = weighted_median(detour);
        row.spread_before = range(&l_before);
        row.spread_after = range(&l_after);
        row.spread_after_between = range(&l_after_b);
        row.distinct_between = distinct_b.len();
        row.de_control = weighted_median(dc);
        row.de_nearest = weighted_median(dn);
        row.de_between = weighted_median(db);
        row.distinct_nearest = distinct.len();
        previous = current;
        rows.push(row);
    }
    rows
}

/// 実素材を掃く．
pub fn atmos_rows(dir: &Path, amounts: &[f32]) -> Result<(Vec<AtmosRow>, AtmosSkipped)> {
    let mut rows = Vec::new();
    let mut skipped = AtmosSkipped::default();

    for path in png_files(dir)? {
        let file = name_of(&path);
        let Some((canvas, palette)) = indexed(&path) else {
            skipped.not_indexable += 1;
            continue;
        };
        let used = used_indices(&canvas, &palette);
        if used.entries.is_empty() {
            skipped.empty += 1;
            continue;
        }

        for (sky_name, hex) in SKIES {
            let sky = oklab_of(
                Rgba8::from_hex_str(hex).with_context(|| format!("空の色 {hex} を読めない"))?,
            );
            rows.extend(measure(
                &file, sky_name, "own", &palette, &used, sky, amounts, None,
            ));

            match extend_palette(&palette, &used, sky, amounts) {
                Some((ext, truth)) => {
                    let ext_used = Used {
                        entries: used.entries.clone(),
                        pixels: used.pixels,
                    };
                    rows.extend(measure(
                        &file,
                        sky_name,
                        "extended",
                        &ext,
                        &ext_used,
                        sky,
                        amounts,
                        Some(&truth),
                    ));
                }
                None => skipped.extended_overflow += 1,
            }
        }
    }
    Ok((rows, skipped))
}

/// 寄せ具合ごとにまとめる — (寄せ具合, 件数, 色の数, 動いた率 nearest, 動いた率
/// between, 逆走率, 色差 対照 / nearest / between, 残った色の割合)．
#[allow(clippy::type_complexity)]
pub fn summarise(
    rows: &[AtmosRow],
    group: &str,
) -> Vec<(
    f32,
    usize,
    usize,
    f32,
    f32,
    f32,
    f32,
    f32,
    f32,
    f32,
    usize,
    f32,
    f32,
    f32,
    f32,
    f32,
)> {
    let mut amounts: Vec<f32> = rows
        .iter()
        .filter(|r| r.group == group)
        .map(|r| r.amount)
        .collect();
    amounts.sort_by(f32::total_cmp);
    amounts.dedup_by(|a, b| (*a - *b).abs() < 1e-6);

    amounts
        .into_iter()
        .map(|t| {
            let set: Vec<&AtmosRow> = rows
                .iter()
                .filter(|r| r.group == group && (r.amount - t).abs() < 1e-6)
                .collect();
            let colors: usize = set.iter().map(|r| r.colors).sum();
            let rate = |f: &dyn Fn(&AtmosRow) -> usize| -> f32 {
                if colors == 0 {
                    0.0
                } else {
                    set.iter().map(|r| f(r)).sum::<usize>() as f32 / colors as f32
                }
            };
            let med = |f: &dyn Fn(&AtmosRow) -> f32| -> f32 {
                let mut v: Vec<f32> = set.iter().map(|r| f(r)).collect();
                v.sort_by(f32::total_cmp);
                v.get(v.len() / 2).copied().unwrap_or(0.0)
            };
            let kept = if colors == 0 {
                0.0
            } else {
                set.iter().map(|r| r.distinct_nearest).sum::<usize>() as f32 / colors as f32
            };
            (
                t,
                set.len(),
                colors,
                rate(&|r| r.moved_nearest),
                rate(&|r| r.moved_between),
                rate(&|r| r.wrong_way),
                med(&|r| r.de_control),
                med(&|r| r.de_nearest),
                med(&|r| r.de_between),
                kept,
                set.iter().map(|r| r.non_monotone).sum::<usize>(),
                med(&|r| {
                    if r.spread_before <= f32::EPSILON {
                        1.0
                    } else {
                        r.spread_after / r.spread_before
                    }
                }),
                med(&|r| r.detour_nearest),
                rate(&|r| r.off_line),
                if colors == 0 {
                    0.0
                } else {
                    set.iter().map(|r| r.distinct_between).sum::<usize>() as f32 / colors as f32
                },
                med(&|r| {
                    if r.spread_before <= f32::EPSILON {
                        1.0
                    } else {
                        r.spread_after_between / r.spread_before
                    }
                }),
            )
        })
        .collect()
}

/// 真値のある場面での一致率 (nearest, between)．
pub fn exact_rates(rows: &[AtmosRow]) -> (f32, f32, usize) {
    let set: Vec<&AtmosRow> = rows.iter().filter(|r| r.group == "extended").collect();
    let colors: usize = set.iter().map(|r| r.colors).sum();
    if colors == 0 {
        return (0.0, 0.0, 0);
    }
    let n = set.iter().map(|r| r.exact_nearest).sum::<usize>() as f32;
    let b = set.iter().map(|r| r.exact_between).sum::<usize>() as f32;
    (n / colors as f32, b / colors as f32, colors)
}

// -------------------------------------------------- パレットが持てる段の数

/// **パレットが表せる «霞の段» はいくつあるか** — `Depth` の 3 値で足りるかを
/// 決めるための数え上げ．
///
/// $t$ を細かく掃いて出力が何通りになるかを色ごとに数える．出力が 1 通り
/// (自分だけ) なら **その色に atmos は効かない**．
///
/// `segment` が真なら選ぶ規則は [`nearest_toward`]，偽なら `Palette::nearest`．
pub fn levels(dir: &Path, steps: usize, segment: bool) -> Result<Vec<usize>> {
    let mut out = Vec::new();
    for path in png_files(dir)? {
        let Some((canvas, palette)) = indexed(&path) else {
            continue;
        };
        let used = used_indices(&canvas, &palette);
        if used.entries.is_empty() {
            continue;
        }
        for (_, hex) in SKIES {
            let sky = oklab_of(Rgba8::from_hex_str(hex).map_err(anyhow::Error::from)?);
            for &(c, _) in &used.entries {
                let Some(lab) = palette.lab_of(c) else {
                    continue;
                };
                let mut seen = std::collections::BTreeSet::new();
                for k in 0..=steps {
                    let t = k as f32 / steps as f32;
                    let ideal = lerp(lab, sky, t);
                    let a = if segment {
                        nearest_toward(&palette, c, sky, ideal, SEGMENT_TOLERANCE, None)
                            .unwrap_or(c)
                    } else {
                        palette
                            .nearest(oklab_to_rgba(ideal), 1.0)
                            .filter(|i| palette.get(*i).is_some_and(|c| c.a != 0))
                            .unwrap_or(c)
                    };
                    seen.insert(a);
                }
                out.push(seen.len());
            }
        }
    }
    Ok(out)
}

/// **許容を変えると «線の上にある» 色はどれだけ増えるか．**
///
/// 返り値は (許容, 置き換え先が在った色の割合, 検査した色数)．
pub fn tolerance_sweep(dir: &Path, tolerances: &[f32]) -> Result<Vec<(f32, f32, usize)>> {
    let mut out = Vec::new();
    for &tol in tolerances {
        let (mut hit, mut total) = (0usize, 0usize);
        for path in png_files(dir)? {
            let Some((canvas, palette)) = indexed(&path) else {
                continue;
            };
            let used = used_indices(&canvas, &palette);
            for (_, hex) in SKIES {
                let sky = oklab_of(Rgba8::from_hex_str(hex).map_err(anyhow::Error::from)?);
                for &(c, _) in &used.entries {
                    let Some(lab) = palette.lab_of(c) else {
                        continue;
                    };
                    total += 1;
                    // 存在は $t$ に依らない (D124 と同じ性質) ので中点で代表させる
                    let ideal = lerp(lab, sky, 0.5);
                    if nearest_toward(&palette, c, sky, ideal, tol, None).is_some() {
                        hit += 1;
                    }
                }
            }
        }
        out.push((tol, hit as f32 / total.max(1) as f32, total));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pal(hexes: &[&str]) -> Palette {
        Palette::new(
            hexes
                .iter()
                .map(|h| Rgba8::from_hex_str(h).unwrap())
                .collect(),
        )
        .unwrap()
    }

    /// **壊れると: «寄せた» と言いながら空から遠ざかる色を選ぶ．**
    #[test]
    fn nearest_toward_only_moves_towards_the_sky() {
        // 暗い青 → 空 (明るい青)．間に中くらいの青がある
        let p = pal(&["1a1c2c", "3b5dc9", "41a6f6", "b13e53"]);
        let sky = oklab_of(Rgba8::from_hex_str("41a6f6").unwrap());
        let from = 0u8;
        let target = lerp(p.lab_of(from).unwrap(), sky, 0.5);
        let got = nearest_toward(&p, from, sky, target, SEGMENT_TOLERANCE, None);
        assert_eq!(got, Some(1), "線の上にある中間の青を引くはず");
    }

    /// **壊れると: 線から外れた色 (赤) を «霞» として選んでしまう．**
    #[test]
    fn a_colour_off_the_line_is_not_a_haze_step() {
        let p = pal(&["1a1c2c", "b13e53"]);
        let sky = oklab_of(Rgba8::from_hex_str("41a6f6").unwrap());
        let target = lerp(p.lab_of(0).unwrap(), sky, 0.5);
        assert_eq!(
            nearest_toward(&p, 0, sky, target, SEGMENT_TOLERANCE, None),
            None,
            "赤は暗い青と空を結ぶ線の上に無い"
        );
    }

    /// **壊れると: 真値を作る側が «正解» を持たないまま一致率を出す．**
    #[test]
    fn the_extended_palette_contains_every_haze_step() {
        let p = pal(&["1a1c2c", "b13e53"]);
        let used = Used {
            entries: vec![(0, 1), (1, 1)],
            pixels: 2,
        };
        let sky = oklab_of(Rgba8::from_hex_str("41a6f6").unwrap());
        let amounts = [0.25f32, 0.5, 0.75];
        let (ext, truth) = extend_palette(&p, &used, sky, &amounts).unwrap();
        assert_eq!(truth.len(), 6, "2 色 x 3 段");
        for ((c, k), want) in truth {
            let ideal = lerp(p.lab_of(c).unwrap(), sky, amounts[k as usize]);
            assert_eq!(
                ext.get(want),
                Some(oklab_to_rgba(ideal)),
                "正解の添字が狙いの色を指していない"
            );
        }
    }
}
