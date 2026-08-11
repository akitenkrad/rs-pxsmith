//! ドット絵風の画像を実データ枠の**正例**へ仕立てる．
//!
//! 生成 AI の出力はドット絵風に見えても連続階調で，整数の格子を持たない (実測した 4 件は
//! 同一行率 0.0% ・色数 13 万〜40 万) ．そのままでは「格子あり」の評価に使えない．
//!
//! ここでは 3 段で正例にする．
//!
//! | 段 | 内容 |
//! | --- | --- |
//! | 1 | 見かけのブロック周期を測る (自己相関) |
//! | 2 | その周期で縮小し，ドット絵の解像度へ落とす |
//! | 3 | **こちらが決めた整数倍**で拡大し，位相をずらして切る |
//!
//! > [!note] 循環参照にならない
//! > 正解は段 3 で**こちらが選んだ倍率**であって，段 1 の推定結果ではない．段 1 が多少
//! > ずれても最終画像の格子は厳密に正しく，ずれはリサンプル痕として元絵に残るだけである
//! > (実データらしさとしてはむしろ望ましい) ．**推定器を自分の出力で採点することには
//! > ならない．**
//!
//! 周期が読めない画像は**拒否する**．黙って正例に仕立てると，格子の無いものを
//! 「格子あり」として採点してしまう．拒否したものは負例の候補として報告する．
//!
//! > [!warning] 「ドット絵風」は目視で判定できない
//! > 配布素材の背景画 22 件は，縮小表示ではブロックが並んで見えるが，拡大すると縁が
//! > 滑らかで境界が無かった．画風であって格子ではない．**隣接行の近似一致率も当てに
//! > ならない** — 平坦な領域が広いと 90% を超え，格子があるように見えてしまう．
//! > 縁のエネルギーが境界へ集中しているかを見ること．

use std::path::Path;

use anyhow::{Context, Result};
use px_core::{Rgba8, RgbaCanvas};

/// 探す周期の上限 (見かけのブロックの一辺)．
pub const MAX_PERIOD: usize = 64;

/// 縮小後に許す一辺の下限．
pub const NATIVE_MIN: u32 = 12;

/// 縮小後に許す一辺の上限 (既定)．スプライトを想定した値で，背景画は 160x90 などに
/// なるので呼び出し側で広げる．
pub const NATIVE_MAX: u32 = 64;

/// 仕立てた画像の一辺の上限．**推定の費用は面積 x $s^2$ で効く** — 870x493 で
/// `px conform` が 20 秒かかった．これを超えないよう倍率を抑える．
pub const OUTPUT_MAX: u32 = 1000;

/// 境界と内部の縁の比をどれだけ要求するか．**下回ったら格子は無いと見なす**．
const MIN_CONTRAST: f32 = 1.8;

/// 集中度 (縁がどれだけ境界へ寄っているか) の下限．1.0 が理想で，外れた周期では
/// $1/p$ 程度まで落ちる．JPEG の雑音を見込んで少し緩めてある．
const MIN_CONCENTRATION: f32 = 0.6;

/// 取り込めなかった理由．
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// 周期が読めない — 格子らしい構造が無い．
    NoPeriod,
    /// 縦横で周期が食い違う — 画面内でブロックの大きさが一定でない．
    NonUniform { x: usize, y: usize },
    /// 縮小後が小さすぎる / 大きすぎる．
    ///
    /// 大きい画像で `period` が 2 ・3 と出た場合はブロック構造ではなく高周波の雑音を
    /// 拾っている．周期も一緒に報告しないと，そのことが読み取れない．
    OutOfRange {
        native: u32,
        period: usize,
        max: u32,
    },
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoPeriod => write!(f, "周期が読めない (ドット絵風だが格子が無い)"),
            Self::NonUniform { x, y } => {
                write!(f, "ブロックの大きさが一定でない (横 {x} / 縦 {y})")
            }
            Self::OutOfRange {
                native,
                period,
                max,
            } => write!(
                f,
                "周期 {period} → 縮小後が {native} 画素角 (受け入れは {}〜{max}){}",
                NATIVE_MIN,
                if *period <= 3 {
                    "．ブロック構造ではなく雑音を拾っている"
                } else {
                    ""
                }
            ),
        }
    }
}

/// 明度 (自己相関は明度だけで足りる)．
fn luma(c: Rgba8) -> f32 {
    0.299 * f32::from(c.r) + 0.587 * f32::from(c.g) + 0.114 * f32::from(c.b)
}

/// 列ごとの差分エネルギー．ブロックの境界で山になる．
fn edge_profile(img: &RgbaCanvas, horizontal: bool) -> Vec<f32> {
    let (w, h) = (img.width() as i32, img.height() as i32);
    let n = if horizontal { w } else { h };
    (1..n)
        .map(|i| {
            let mut acc = 0.0;
            let across = if horizontal { h } else { w };
            for j in 0..across {
                let (a, b) = if horizontal {
                    (img.get(i - 1, j), img.get(i, j))
                } else {
                    (img.get(j, i - 1), img.get(j, i))
                };
                if let (Some(a), Some(b)) = (a, b) {
                    acc += (luma(a) - luma(b)).abs();
                }
            }
            acc / across.max(1) as f32
        })
        .collect()
}

/// 縁のエネルギーがブロック境界へどれだけ集中しているかで周期を読む．
///
/// **自己相関は JPEG に耐えない．** 圧縮が 8x8 の格子を持ち込むうえ，高周波の雑音とも
/// 相関するので，周期 2 ・4 を掴んでしまう (実測: 1920 画素の背景画で周期 2) ．
///
/// 代わりに「境界の縁 / 内部の縁」の比を見る．本物の格子なら縁は境界に集まり，内部は
/// 平坦になる．多数の行で平均するので，JPEG の雑音は打ち消し合う．
fn boundary_stats(profile: &[f32], p: usize) -> Option<(f32, f32)> {
    // 返り値は (持ち上がり, 境界と内部の比)
    if p < 2 || profile.len() < p * 4 {
        return None;
    }
    // 位相ごとに「その位置に来る縁の平均」を出し，最も強い位相を境界とみなす
    let mut sums = vec![0.0f32; p];
    let mut counts = vec![0u32; p];
    for (i, v) in profile.iter().enumerate() {
        sums[i % p] += v;
        counts[i % p] += 1;
    }
    let means: Vec<f32> = sums
        .iter()
        .zip(&counts)
        .map(|(s, c)| if *c == 0 { 0.0 } else { s / *c as f32 })
        .collect();
    let boundary = means.iter().copied().fold(f32::MIN, f32::max);
    let interior = (means.iter().sum::<f32>() - boundary) / (p - 1) as f32;
    if boundary <= f32::EPSILON {
        return None;
    }
    let overall = means.iter().sum::<f32>() / p as f32;
    // 内部が 0 (完璧な格子) は**最良の証拠**である．0 除算を避けるために下限を置くが，
    // 「情報なし」として捨ててはいけない
    let contrast = boundary / interior.max(boundary * 1.0e-4);
    // 持ち上がり = 境界の強さ / 全体の平均．**倍数では飽和し，半分では半減する** —
    // これが基本周期を選ぶ手がかりになる
    let lift = boundary / overall.max(f32::EPSILON);
    Some((lift, contrast))
}

/// 基本周期を読む．
///
/// 手がかりは**集中度** $= \mathrm{lift} / p$ である．周期 $p$ に縁がすべて乗っていれば
/// 境界の平均は全体平均の $p$ 倍になるので集中度は 1 に届く．外れた $p$ では 1/p 程度に
/// 落ちる (実測: 真の周期 4 の画像で $p = 4$ が 1.00 ・$p = 8$ が 0.51 ・$p = 12$ が 0.35) ．
///
/// **真の周期の約数もすべて集中度 1 になる** ($4 \mid x$ なら $2 \mid x$) ので，
/// 条件を満たす中で**最大**の $p$ を採る．
fn fundamental_period(profile: &[f32]) -> Option<usize> {
    let max_p = MAX_PERIOD.min(profile.len() / 4);
    if max_p < 2 {
        return None;
    }
    (2..=max_p)
        .filter_map(|p| boundary_stats(profile, p).map(|(l, c)| (p, l, c)))
        // 境界と内部に差が無ければ格子は無い
        .filter(|(_, _, c)| *c >= MIN_CONTRAST)
        .filter(|(p, l, _)| *l / *p as f32 >= MIN_CONCENTRATION)
        .map(|(p, _, _)| p)
        .max()
}

/// 見かけのブロック周期．縦横がずれていたら非一様として拒否する．
pub fn detect_period(img: &RgbaCanvas) -> std::result::Result<usize, Refusal> {
    let px = fundamental_period(&edge_profile(img, true)).ok_or(Refusal::NoPeriod)?;
    let py = fundamental_period(&edge_profile(img, false)).ok_or(Refusal::NoPeriod)?;

    // 1 画素のずれは丸めの範囲として許す
    if px.abs_diff(py) <= 1 {
        return Ok(px.min(py));
    }
    // 一方が他方の倍数なら，**元絵自身がその方向に繰り返しを持っている**だけである
    // (縦に 2 行周期の模様があれば，4 倍に拡大した画像は縦に周期 8 を持つ) ．
    // 格子の周期は小さい方
    if px.max(py) % px.min(py) == 0 {
        return Ok(px.min(py));
    }
    Err(Refusal::NonUniform { x: px, y: py })
}

/// 周期で縮小する．**平均を採る** — 数十万色あるので最頻色は意味を持たない．
pub fn downscale_mean(img: &RgbaCanvas, period: usize) -> RgbaCanvas {
    let p = period.max(1) as u32;
    let (nw, nh) = ((img.width() / p).max(1), (img.height() / p).max(1));
    let mut pixels = Vec::with_capacity((nw * nh) as usize);
    for cy in 0..nh {
        for cx in 0..nw {
            let mut acc = [0u32; 4];
            let mut n = 0u32;
            for y in 0..p {
                for x in 0..p {
                    if let Some(c) = img.get((cx * p + x) as i32, (cy * p + y) as i32) {
                        acc[0] += u32::from(c.r);
                        acc[1] += u32::from(c.g);
                        acc[2] += u32::from(c.b);
                        acc[3] += u32::from(c.a);
                        n += 1;
                    }
                }
            }
            let n = n.max(1);
            pixels.push(Rgba8::new(
                (acc[0] / n) as u8,
                (acc[1] / n) as u8,
                (acc[2] / n) as u8,
                (acc[3] / n) as u8,
            ));
        }
    }
    RgbaCanvas::from_pixels(nw, nh, pixels).expect("画素数は nw*nh で作っている")
}

/// 最近傍で整数倍に拡大し，位相をずらして切る．**ここで格子が生まれる**．
pub fn upscale(img: &RgbaCanvas, scale: u32, crop: (u32, u32)) -> RgbaCanvas {
    let s = scale.max(1);
    let (dx, dy) = (crop.0 % s, crop.1 % s);
    let (w, h) = (img.width() * s - dx, img.height() * s - dy);
    let mut pixels = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let c = img
                .get(((x + dx) / s) as i32, ((y + dy) / s) as i32)
                .unwrap_or(Rgba8::TRANSPARENT);
            pixels.push(c);
        }
    }
    RgbaCanvas::from_pixels(w, h, pixels).expect("画素数は w*h で作っている")
}

/// 切り落とし量から正解の位相を求める (`degrade` と同じ規約)．
pub fn truth_phase(scale: u32, crop: (u32, u32)) -> (u32, u32) {
    let s = scale.max(1);
    ((s - crop.0 % s) % s, (s - crop.1 % s) % s)
}

/// 取り込みの結果．**目録に書く根拠**として周期と元絵の大きさを持ち回る．
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Ingested {
    /// 見かけのブロック周期 (段 1 で測った値)．
    pub period: usize,
    /// 縮小後の大きさ．
    pub native: (u32, u32),
    /// **実際に使った倍率**．出来上がりが大きくなりすぎる場合は指定より下がる．
    pub scale: u32,
}

/// 1 枚を正例へ仕立てる．
pub fn ingest_one(
    path: &Path,
    scale: u32,
    crop: (u32, u32),
    native_max: u32,
) -> Result<std::result::Result<(RgbaCanvas, Ingested), Refusal>> {
    let img =
        px_io::png::read_rgba(path).with_context(|| format!("{} を読めない", path.display()))?;

    let period = match detect_period(&img) {
        Ok(p) => p,
        Err(r) => return Ok(Err(r)),
    };
    let native = downscale_mean(&img, period);
    let side = native.width().min(native.height());
    if !(NATIVE_MIN..=native_max).contains(&side) {
        return Ok(Err(Refusal::OutOfRange {
            native: side,
            period,
            max: native_max,
        }));
    }

    // 出来上がりが大きすぎると推定に時間がかかるので倍率を抑える
    let longest = native.width().max(native.height());
    let scale = scale.min((OUTPUT_MAX / longest.max(1)).max(2));
    let out = upscale(&native, scale, crop);
    Ok(Ok((
        out,
        Ingested {
            period,
            native: (native.width(), native.height()),
            scale,
        },
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sprite;

    /// 最近傍で `scale` 倍に拡大する (試験用の素直な実装)．
    fn blow_up(img: &RgbaCanvas, scale: u32) -> RgbaCanvas {
        upscale(img, scale, (0, 0))
    }

    #[test]
    fn the_period_of_a_clean_upscale_is_the_scale() {
        for scale in [4u32, 6, 8, 12] {
            let src = sprite::synthesize(21);
            let big = blow_up(&src, scale);
            assert_eq!(
                detect_period(&big),
                Ok(scale as usize),
                "{scale} 倍の周期を読み違えた"
            );
        }
    }

    #[test]
    fn noise_has_no_period() {
        // ブロック構造の無い画像は拒否する — 黙って正例にしない
        let mut rng = crate::rng::Rng::new(5);
        let pixels: Vec<Rgba8> = (0..200 * 200)
            .map(|_| {
                let v = rng.below(256) as u8;
                Rgba8::rgb(v, v, v)
            })
            .collect();
        let img = RgbaCanvas::from_pixels(200, 200, pixels).unwrap();
        assert_eq!(detect_period(&img), Err(Refusal::NoPeriod));
    }

    #[test]
    fn a_flat_image_has_no_period() {
        let img = RgbaCanvas::filled(200, 200, Rgba8::rgb(20, 30, 40));
        assert_eq!(detect_period(&img), Err(Refusal::NoPeriod));
    }

    #[test]
    fn downscaling_by_the_period_recovers_the_original() {
        let src = sprite::synthesize(22);
        let big = blow_up(&src, 6);
        let back = downscale_mean(&big, 6);
        assert_eq!((back.width(), back.height()), (src.width(), src.height()));
        assert_eq!(back.pixels(), src.pixels(), "平均でも完全に戻るはず");
    }

    #[test]
    fn the_upscale_creates_an_exact_grid() {
        // ここで格子が生まれる — 同一行が s-1/s の割合で並ぶ
        let src = sprite::synthesize(23);
        let out = upscale(&src, 5, (0, 0));
        let same = (1..out.height() as i32)
            .filter(|y| (0..out.width() as i32).all(|x| out.get(x, *y) == out.get(x, y - 1)))
            .count();
        // 1 ブロック 5 行につき 4 本が同一．元絵に同じ行が隣り合っていれば更に増えるので
        // 下限で見る
        let least = (out.height() - src.height()) as usize;
        assert!(same >= least, "同一行が {same} 本しかない (最低 {least})");
    }

    #[test]
    fn the_crop_shifts_the_phase_the_same_way_as_degrade() {
        // 正解の規約は評価データセットと揃える
        assert_eq!(truth_phase(6, (2, 3)), (4, 3));
        assert_eq!(truth_phase(6, (0, 0)), (0, 0));
        assert_eq!(truth_phase(6, (6, 6)), (0, 0));
    }

    #[test]
    fn the_ingested_image_can_be_recovered_by_the_estimator() {
        // 仕立てた正例が実際に推定できること — これが通らないと正例と呼べない
        use px_core::grid::{GridParams, estimate_grid};
        // 大きめの元絵を選ぶ (周期を読むには数周期分の長さが要る)
        let src = (0..)
            .map(sprite::synthesize)
            .find(|c| c.width() >= 40 && c.height() >= 40)
            .expect("大きい元絵がある");
        let big = blow_up(&src, 8); // AI 出力の代わり (周期 8)
        let (out, info) = ingest_one(&write_temp(&big, "ingest_case.png"), 6, (2, 1), NATIVE_MAX)
            .unwrap()
            .expect("拒否された");
        assert_eq!(info.period, 8);

        let e = estimate_grid(&out, &GridParams::default()).expect("推定できない");
        assert_eq!(e.scale, 6);
        assert_eq!((e.phase.x as u32, e.phase.y as u32), truth_phase(6, (2, 1)));
    }

    fn write_temp(img: &RgbaCanvas, name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("pxforge-ingest-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        px_io::png::write_rgba(&path, img).unwrap();
        path
    }
}
