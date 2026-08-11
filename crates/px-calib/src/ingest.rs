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

use std::path::Path;

use anyhow::{Context, Result};
use px_core::{Rgba8, RgbaCanvas};

/// 探す周期の上限 (見かけのブロックの一辺)．
pub const MAX_PERIOD: usize = 64;

/// 縮小後に許す一辺の範囲．外れたら拒否する．
pub const NATIVE_RANGE: std::ops::RangeInclusive<u32> = 12..=64;

/// 自己相関のピークをどれだけ強く要求するか (最大値に対する比)．
const PEAK_RATIO: f32 = 0.55;

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
    OutOfRange { native: u32, period: usize },
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoPeriod => write!(f, "周期が読めない (ドット絵風だが格子が無い)"),
            Self::NonUniform { x, y } => {
                write!(f, "ブロックの大きさが一定でない (横 {x} / 縦 {y})")
            }
            Self::OutOfRange { native, period } => write!(
                f,
                "周期 {period} → 縮小後が {native} 画素角 (受け入れは {}〜{}){}",
                NATIVE_RANGE.start(),
                NATIVE_RANGE.end(),
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

/// 自己相関から基本周期を読む．
///
/// **倍数も相関するので，強い山のうち最小のものを採る．** 最大値だけを見ると
/// 2 倍・3 倍の位置を掴む．
fn fundamental_period(profile: &[f32]) -> Option<usize> {
    // 周期 k を読むには最低でも数周期分の長さが要る．上限は画像に追随させる —
    // 固定にすると小さい画像が「周期なし」になってしまう
    let max_k = MAX_PERIOD.min(profile.len() / 4);
    if max_k < 2 {
        return None;
    }
    let mean = profile.iter().sum::<f32>() / profile.len() as f32;
    let centered: Vec<f32> = profile.iter().map(|v| v - mean).collect();
    let energy: f32 = centered.iter().map(|v| v * v).sum();
    if energy <= f32::EPSILON {
        return None;
    }

    let corr: Vec<f32> = (2..=max_k)
        .map(|k| {
            let s: f32 = centered
                .iter()
                .zip(centered.iter().skip(k))
                .map(|(a, b)| a * b)
                .sum();
            s / energy
        })
        .collect();

    let peak = corr.iter().copied().fold(f32::MIN, f32::max);
    // 相関がそもそも弱ければ格子は無い
    if peak < 0.15 {
        return None;
    }
    // 強い山のうち最小の $k$ が基本周期
    corr.iter()
        .position(|c| *c >= peak * PEAK_RATIO)
        .map(|i| i + 2)
}

/// 見かけのブロック周期．縦横がずれていたら非一様として拒否する．
pub fn detect_period(img: &RgbaCanvas) -> std::result::Result<usize, Refusal> {
    let px = fundamental_period(&edge_profile(img, true)).ok_or(Refusal::NoPeriod)?;
    let py = fundamental_period(&edge_profile(img, false)).ok_or(Refusal::NoPeriod)?;
    // 1 画素のずれは丸めの範囲として許す
    if px.abs_diff(py) > 1 {
        return Err(Refusal::NonUniform { x: px, y: py });
    }
    Ok(px.min(py))
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
}

/// 1 枚を正例へ仕立てる．
pub fn ingest_one(
    path: &Path,
    scale: u32,
    crop: (u32, u32),
) -> Result<std::result::Result<(RgbaCanvas, Ingested), Refusal>> {
    let img =
        px_io::png::read_rgba(path).with_context(|| format!("{} を読めない", path.display()))?;

    let period = match detect_period(&img) {
        Ok(p) => p,
        Err(r) => return Ok(Err(r)),
    };
    let native = downscale_mean(&img, period);
    let side = native.width().min(native.height());
    if !NATIVE_RANGE.contains(&side) {
        return Ok(Err(Refusal::OutOfRange {
            native: side,
            period,
        }));
    }

    let out = upscale(&native, scale, crop);
    Ok(Ok((
        out,
        Ingested {
            period,
            native: (native.width(), native.height()),
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
        let (out, info) = ingest_one(&write_temp(&big, "ingest_case.png"), 6, (2, 1))
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
