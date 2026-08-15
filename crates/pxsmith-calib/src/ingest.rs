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
//! ## 拒否された画像からでも正例は作れる ([`Recipe::force_period`])
//!
//! 段 1 が読めないのは「元絵の解像度が分からない」というだけで，**正例が作れない
//! 理由にはならない**．正解は段 3 でこちらが決める倍率だからである．そこで縮小の
//! 粗さを指定してしまえば，段 1 を飛ばして同じ 3 段を通せる．
//!
//! 出来上がりは「実在の絵を素材にした，格子が厳密に正しい画像」になる．**外から来た
//! 正例そのものではない** — 測れるのは絵の中身の分布のずれだけで，格子の作られ方の
//! ずれは測れない．それでも合成スプライトより実運用に近い．
//!
//! | 素材 | 画像分散 $\bar{V}_{\mathrm{image}}$ の中央値 |
//! | --- | --- |
//! | 合成スプライト (劣化後) | 0.089 |
//! | 自作レンダ (劣化後) | 0.050 |
//! | AI 出力を周期 20 で縮小 | **0.040** |
//!
//! $\varepsilon$ は分散の絶対値に対する閾値なので，**この低分散の領域が今いちばん
//! 足りていない**．
//!
//! 最近傍で拡大するだけだと補間も圧縮も無い「きれいな格子」になり実運用より易しいので，
//! [`Recipe::degrade`] で合成データと同じ劣化を通せる (非整数倍リサイズだけは掛けない
//! — 掛けると整数の格子が消える) ．
//!
//! ## 配布素材からは元絵そのものを取り戻せる ([`Recipe::recover_native`])
//!
//! ドット絵の配布サイトは「元絵を幅 500 画素へ」のように**非整数倍で拡大して配る**．
//! 整数の格子は無い (実測: DOT ILLUST 23 件中 22 件) が，**最近傍なら画素の値は元の
//! ままなので元絵は失われていない**．
//!
//! [`recover_native_size`] が元絵の解像度を復元する．戻した元絵は平均でぼかしたもの
//! ではなく**本物のドット絵そのもの**なので，こちらが決めた倍率で拡大し直せば
//! 「中身も本物」の正例になる — 手打ちのアンチエイリアス ・ディザ ・1 画素の輪郭線を
//! 持つ入力は，これ以外の道では作れない．
//!
//! > [!warning] 「ドット絵風」は目視で判定できない
//! > 配布素材の背景画 22 件は，縮小表示ではブロックが並んで見えるが，拡大すると縁が
//! > 滑らかで境界が無かった．画風であって格子ではない．**隣接行の近似一致率も当てに
//! > ならない** — 平坦な領域が広いと 90% を超え，格子があるように見えてしまう．
//! > 縁のエネルギーが境界へ集中しているかを見ること．

use std::path::Path;

use anyhow::{Context, Result};
use pxsmith_core::{Rgba8, RgbaCanvas};

use crate::degrade::{Compression, Degradation, Filter, Resize};

/// 探す周期の上限 (見かけのブロックの一辺)．
pub const MAX_PERIOD: usize = 64;

/// 縮小後に許す一辺の下限．
pub const NATIVE_MIN: u32 = 12;

/// 縮小後に許す一辺の上限 (既定)．スプライトを想定した値で，背景画は 160x90 などに
/// なるので呼び出し側で広げる．
pub const NATIVE_MAX: u32 = 64;

/// 仕立てた画像の一辺の上限．**推定の費用は面積 x $s^2$ で効く** — 870x493 で
/// `pxsmith conform` が 20 秒かかった．これを超えないよう倍率を抑える．
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
        /// 周期で縮小した場合のみ．元絵を復元した場合は `None`．
        period: Option<usize>,
        max: u32,
    },
    /// 元絵の解像度が読めない ([`Recipe::recover_native`])．
    ///
    /// 最近傍で拡大したものではない (補間が掛かっている ・圧縮で画素が動いている) か，
    /// 元絵が `native_max` より大きい．
    NoNativeSize { max: u32 },
    /// 「元絵である」と言われた入力が，実際には拡大されたものだった
    /// ([`Recipe::already_native`])．
    ///
    /// そのまま $s$ 倍すると，元の 1 画素が $k \times k$ として残るので $s$ と $ks$ の
    /// 格子が両方成立し，**正解が一意に決まらない**．
    NotNative { native: (u32, u32) },
    /// 指定した周期が**実際のブロック構造の約数**で，正解が一意に決まらない．
    ///
    /// 周期 8 の画像を 4 で縮小すると，元の 1 画素が縮小後の 2x2 として残る．これを
    /// $s$ 倍すると $s$ の格子と $2s$ の格子が**両方とも厳密に成立する**．推定器の
    /// 規約は「閾値を満たす最大の $s$」なので答えは $2s$ になり，こちらが書いた正解
    /// $s$ とは食い違う．**どちらかが間違っているのではなく，正解が 1 つに決まらない．**
    AmbiguousTruth { forced: usize, detected: usize },
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
                "{}縮小後が {native} 画素角 (受け入れは {}〜{max}){}",
                match period {
                    Some(p) => format!("周期 {p} → "),
                    None => String::new(),
                },
                NATIVE_MIN,
                if period.is_some_and(|p| p <= 3) {
                    "．ブロック構造ではなく雑音を拾っている"
                } else {
                    ""
                }
            ),
            Self::NoNativeSize { max } => write!(
                f,
                "元絵の解像度が読めない (最近傍で拡大したものではない，\
                 または元絵が {max} 画素角より大きい)"
            ),
            Self::NotNative { native } => write!(
                f,
                "入力は元絵ではない — 実際は {}x{} から拡大されている\
                 (そのまま拡大すると正解が一意に決まらない)．--recover-native を使うこと",
                native.0, native.1,
            ),
            Self::AmbiguousTruth { forced, detected } => write!(
                f,
                "指定した周期 {forced} は実際のブロック {detected} の約数である\
                 (縮小後に {}x{} のブロックが残り，正解が一意に決まらない)．\
                 --force-period {detected} を使うこと",
                detected / forced,
                detected / forced,
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

/// 1 つ前の行 (列) と中身が違う位置．
///
/// **完全一致で見る．** 近似で見ると，元絵に隣り合う似た色があるだけで境界を見落とす．
fn change_positions(img: &RgbaCanvas, vertical: bool) -> Vec<usize> {
    let (w, h) = (img.width() as i32, img.height() as i32);
    let n = if vertical { h } else { w };
    let across = if vertical { w } else { h };
    (1..n)
        .filter(|&i| {
            !(0..across).all(|j| {
                let (a, b) = if vertical {
                    (img.get(j, i - 1), img.get(j, i))
                } else {
                    (img.get(i - 1, j), img.get(i, j))
                };
                a == b
            })
        })
        .map(|i| i as usize)
        .collect()
}

/// 最近傍で $N \to L$ に拡大されたと見なせる**最小の** $N$．
///
/// 配布素材は「元絵を幅 500 画素へ」のように**非整数倍で拡大されて配られている**ことが
/// 多い．整数の格子は無いが，元絵そのものは失われていない — 最近傍なら画素の値は
/// 元のままだからである．
///
/// 判定はこうする．最近傍で $N \to L$ に拡大した画像では，行 $i$ は元絵の
/// $\lfloor i N / L \rfloor$ 行目である．したがって**中身が変わる位置は，この商が
/// 繰り上がる位置に限られる**．変化位置がすべてその条件を満たす $N$ なら，各セル内の
/// 行はすべて同一であり，元絵を標本化して拡大し直すと**元画像に完全に一致する**．
///
/// 最小を採るのは，$N = L$ (恒等) を含めどんな倍数も条件を満たすためである．
/// 上限を `max` で切るのは費用のためと，恒等解を除くためでもある．
pub fn recover_native_size(img: &RgbaCanvas, vertical: bool, max: u32) -> Option<u32> {
    let l = if vertical { img.height() } else { img.width() } as usize;
    let changes = change_positions(img, vertical);
    // 変化が無い (完全に平坦) なら，どの N でも条件を満たしてしまう．読めたことにしない
    if changes.is_empty() {
        return None;
    }
    (2..=max.min(l as u32)).find(|&n| {
        let n = n as usize;
        changes.iter().all(|&i| i * n / l != (i - 1) * n / l)
    })
}

/// **整数倍**で拡大されているか — $k \times k$ のブロックがすべて一様な最大の $k \ge 2$．
///
/// 正解が割れるのはここだけである．$k$ 画素角のブロックでできた絵を $s$ 倍すると
/// $s$ と $ks$ の格子が**両方とも厳密に成立し**，推定器は規約どおり大きい方を返す．
/// 非整数倍で拡大されている場合 (ラン長が $n$ と $n+1$ に割れる場合) は，そもそも
/// 粗い側に整数の格子が無いので正解は割れない．
///
/// [`recover_native_size`] をこの判定に使ってはいけない — 16 画素角のような短い軸では
/// 変化の数が少なく，**偶然条件を満たす小さい $N$ が見つかる** (実測: CC0 の 16x16
/// タイルが「14x15 からの拡大」と判定された) ．
pub fn integer_block_size(img: &RgbaCanvas) -> Option<u32> {
    let (w, h) = (img.width(), img.height());
    (2..=w.min(h))
        .rev()
        .filter(|k| w.is_multiple_of(*k) && h.is_multiple_of(*k))
        .find(|&k| {
            (0..h as i32).all(|y| {
                (0..w as i32).all(|x| img.get(x, y) == img.get(x - x % k as i32, y - y % k as i32))
            })
        })
}

/// 復元した大きさで元絵を標本化する．
///
/// 各セルの**先頭**の画素を採る．セル内はすべて同一なのでどこを採っても同じである
/// ([`recover_native_size`] がそれを保証している)．
pub fn sample_native(img: &RgbaCanvas, native: (u32, u32)) -> RgbaCanvas {
    let (nw, nh) = native;
    // セル k の先頭は ceil(k * L / N)
    let first =
        |k: u32, n: u32, l: u32| (u64::from(k) * u64::from(l)).div_ceil(u64::from(n)) as i32;
    let mut pixels = Vec::with_capacity((nw * nh) as usize);
    for y in 0..nh {
        for x in 0..nw {
            pixels.push(
                img.get(first(x, nw, img.width()), first(y, nh, img.height()))
                    .unwrap_or(Rgba8::TRANSPARENT),
            );
        }
    }
    RgbaCanvas::from_pixels(nw, nh, pixels).expect("画素数は nw*nh で作っている")
}

/// 負例として保存するときの一辺 (既定)．
///
/// 素材はリポジトリへ入るので大きさが効く (28 件で 原寸 79 MB / 384 画素角 8.5 MB /
/// 256 画素角 3.8 MB) ．**256 と 384 で採点結果は変わらなかった**ので小さい方を採る．
///
/// $s = 16$ でも 16x16 セルあり，位相ずれ検査の帯 (4) あたり 4 セルで最小セル数 2 を
/// 満たす．**これ以上小さくすると帯の検査が働かなくなる**．
pub const NEGATIVE_SIDE: u32 = 256;

/// 負例を作る．**中央を切り出すだけで，再標本化はしない**．
///
/// 縮小を掛けると補間が入り，「格子が無い」という性質そのものを触ってしまう．切り出しは
/// 画素をそのまま残すので，元画像の一部を見せているだけになる．
pub fn center_crop(img: &RgbaCanvas, side: u32) -> RgbaCanvas {
    let w = side.min(img.width());
    let h = side.min(img.height());
    let (ox, oy) = ((img.width() - w) / 2, (img.height() - h) / 2);
    let mut pixels = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            pixels.push(
                img.get((ox + x) as i32, (oy + y) as i32)
                    .unwrap_or(Rgba8::TRANSPARENT),
            );
        }
    }
    RgbaCanvas::from_pixels(w, h, pixels).expect("画素数は w*h で作っている")
}

/// 拒否された画像を負例に仕立てる (中央の切り出し)．
///
/// **切り出しは中立ではない．** 面積が減ると候補が落ちやすくなり，元の大きさでは
/// 通ってしまう誤受理が消えることがある (実測: 1672x941 で $\hat{s} = 2$ を返す件が，
/// 768 画素角以下では棄却される) ．目録の `source` に元の大きさを残すのはこのためである．
///
/// 返り値は (切り出した画像, 元の大きさ)．
pub fn negative_from(path: &Path, side: u32) -> Result<(RgbaCanvas, (u32, u32))> {
    let img = pxsmith_io::png::read_rgba(path)
        .with_context(|| format!("{} を読めない", path.display()))?;
    let size = (img.width(), img.height());
    Ok((center_crop(&img, side), size))
}

/// 元絵をどうやって取り出したか．**目録に書く根拠**になる．
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Reduction {
    /// 見かけのブロック周期を測り，その周期で平均縮小した．
    Period(usize),
    /// 周期を**指定して**平均縮小した ([`Recipe::force_period`])．
    ForcedPeriod(usize),
    /// 元絵の解像度を復元した ([`Recipe::recover_native`])．**平均を採っていない** —
    /// 画素をそのまま抜いているので元絵が厳密に戻る．
    Recovered,
    /// 縮小していない — **入力がすでに元絵**である ([`Recipe::already_native`])．
    AsIs,
}

impl Reduction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Period(_) => "測った周期",
            Self::ForcedPeriod(_) => "指定した周期",
            Self::Recovered => "元絵を復元",
            Self::AsIs => "入力が元絵",
        }
    }
}

/// 取り込みの結果．
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Ingested {
    pub reduction: Reduction,
    /// 入力の大きさ．
    pub original: (u32, u32),
    /// 縮小後 (元絵) の大きさ．
    pub native: (u32, u32),
    /// **実際に使った倍率**．出来上がりが大きくなりすぎる場合は指定より下がる．
    pub scale: u32,
    /// 拡大のしかた．`None` なら最近傍のまま (劣化なし)．
    pub degrade: Option<(Filter, Compression)>,
}

/// 仕立て方の指定．
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Recipe {
    /// **見かけの周期を測らずにこの値を使う** (`None` なら測る)．
    ///
    /// 拾ってきた画像の多くは格子を持たないので段 1 が読めない．そこで縮小の粗さを
    /// こちらが決める．**正解は段 3 の倍率なので，これでも循環参照にはならない** —
    /// 段 1 は「ドット絵の解像度をいくつにするか」を決めているだけである．
    pub force_period: Option<usize>,
    /// **元絵の解像度を復元してから拡大する** ([`recover_native_size`])．
    ///
    /// 非整数倍で拡大されて配られているドット絵素材はこれで元に戻せる．平均を採る
    /// [`downscale_mean`] と違い**画素をそのまま抜く**ので，元絵が厳密に戻る —
    /// 中身が本物のドット絵である正例を作れる唯一の道である．
    ///
    /// `force_period` とは併用しない．
    pub recover_native: bool,
    /// **入力がすでに元絵である** — 縮小しない．
    ///
    /// CC0 のドット絵素材 (Kenney ・Dungeon Crawl 等) は 16x16 ・32x32 の元絵そのもので
    /// 配られている．縮小を挟むと絵が壊れるだけなので，そのまま拡大へ回す．
    ///
    /// **拡大済みの絵を誤ってこれで渡すと正解が一意に決まらない** (元の 1 画素が
    /// $k \times k$ として残り，$s$ と $ks$ の格子が両方成立する) ．周期を測って
    /// 2 以上が読めたら拒否する．
    pub already_native: bool,
    /// 拡大倍率．
    pub scale: u32,
    /// 位相をずらすために切り落とす画素数．
    pub crop: (u32, u32),
    /// 縮小後に許す一辺 (短い方) の上限．
    pub native_max: u32,
    /// 劣化を掛けるなら補間法と圧縮．**`None` なら最近傍で拡大するだけ**．
    ///
    /// 最近傍のままだと補間も圧縮も無い「きれいな格子」になり，実運用の入力より
    /// はるかに易しい (自作レンダ 25 件が全滅しているのは劣化を通しているからである) ．
    /// 非整数倍リサイズは**掛けない** — 掛けると整数の格子が消えて正例にならない．
    pub degrade: Option<(Filter, Compression)>,
}

/// 1 枚を正例へ仕立てる．
pub fn ingest_one(
    path: &Path,
    recipe: &Recipe,
) -> Result<std::result::Result<(RgbaCanvas, Ingested), Refusal>> {
    let img = pxsmith_io::png::read_rgba(path)
        .with_context(|| format!("{} を読めない", path.display()))?;

    let (reduction, native) = if recipe.already_native {
        // 整数倍で拡大された絵を渡していないかを**厳密に**確かめる．
        // 正解が割れるのはここだけである ([`integer_block_size`] の doc を見ること)
        if let Some(k) = integer_block_size(&img) {
            return Ok(Err(Refusal::NotNative {
                native: (img.width() / k, img.height() / k),
            }));
        }
        (Reduction::AsIs, img.clone())
    } else if recipe.recover_native {
        let (Some(nw), Some(nh)) = (
            recover_native_size(&img, false, recipe.native_max),
            recover_native_size(&img, true, recipe.native_max),
        ) else {
            return Ok(Err(Refusal::NoNativeSize {
                max: recipe.native_max,
            }));
        };
        (Reduction::Recovered, sample_native(&img, (nw, nh)))
    } else {
        let (reduction, period) = match recipe.force_period {
            // 指定されていても測る — **約数を指定していないかを確かめるため**．
            // 読めない (格子が無い) 場合は指定をそのまま使えばよい
            Some(p) => {
                let p = p.max(2);
                if let Ok(d) = detect_period(&img)
                    && d != p
                    && d.is_multiple_of(p)
                {
                    return Ok(Err(Refusal::AmbiguousTruth {
                        forced: p,
                        detected: d,
                    }));
                }
                (Reduction::ForcedPeriod(p), p)
            }
            None => match detect_period(&img) {
                Ok(p) => (Reduction::Period(p), p),
                Err(r) => return Ok(Err(r)),
            },
        };
        (reduction, downscale_mean(&img, period))
    };

    let side = native.width().min(native.height());
    if !(NATIVE_MIN..=recipe.native_max).contains(&side) {
        return Ok(Err(Refusal::OutOfRange {
            native: side,
            period: match reduction {
                Reduction::Period(p) | Reduction::ForcedPeriod(p) => Some(p),
                Reduction::Recovered | Reduction::AsIs => None,
            },
            max: recipe.native_max,
        }));
    }

    // 出来上がりが大きすぎると推定に時間がかかるので倍率を抑える
    let longest = native.width().max(native.height());
    let scale = recipe.scale.min((OUTPUT_MAX / longest.max(1)).max(2));
    let out = match recipe.degrade {
        None => upscale(&native, scale, recipe.crop),
        Some((filter, compression)) => Degradation {
            scale,
            filter,
            // 整数の格子を残す唯一の水準．**ここを変えると正例でなくなる**
            resize: Resize::Keep,
            compression,
            crop: recipe.crop,
        }
        .apply(&native)?,
    };
    Ok(Ok((
        out,
        Ingested {
            reduction,
            original: (img.width(), img.height()),
            native: (native.width(), native.height()),
            scale,
            degrade: recipe.degrade,
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
    fn the_negative_crop_keeps_the_pixels_untouched() {
        // 負例は再標本化しない — 切り出した画素が元と 1 画素も違わないこと
        let src = sprite::synthesize(24);
        let big = blow_up(&src, 8);
        let side = 32;
        let out = center_crop(&big, side);
        assert_eq!((out.width(), out.height()), (side, side));
        let (ox, oy) = ((big.width() - side) / 2, (big.height() - side) / 2);
        for y in 0..side as i32 {
            for x in 0..side as i32 {
                assert_eq!(out.get(x, y), big.get(ox as i32 + x, oy as i32 + y));
            }
        }
    }

    #[test]
    fn a_small_image_is_not_padded_by_the_crop() {
        let src = sprite::synthesize(25);
        let out = center_crop(&src, 4096);
        assert_eq!((out.width(), out.height()), (src.width(), src.height()));
        assert_eq!(out.pixels(), src.pixels());
    }

    #[test]
    fn the_crop_shifts_the_phase_the_same_way_as_degrade() {
        // 正解の規約は評価データセットと揃える
        assert_eq!(truth_phase(6, (2, 3)), (4, 3));
        assert_eq!(truth_phase(6, (0, 0)), (0, 0));
        assert_eq!(truth_phase(6, (6, 6)), (0, 0));
    }

    fn recipe(scale: u32, crop: (u32, u32)) -> Recipe {
        Recipe {
            force_period: None,
            recover_native: false,
            already_native: false,
            scale,
            crop,
            native_max: NATIVE_MAX,
            degrade: None,
        }
    }

    /// 最近傍で `l` 画素へ非整数倍に拡大する (配布サイトがやっていることの再現)．
    fn stretch(img: &RgbaCanvas, lw: u32, lh: u32) -> RgbaCanvas {
        let (nw, nh) = (img.width(), img.height());
        let mut pixels = Vec::with_capacity((lw * lh) as usize);
        for y in 0..lh {
            for x in 0..lw {
                pixels.push(
                    img.get((x * nw / lw) as i32, (y * nh / lh) as i32)
                        .unwrap_or(Rgba8::TRANSPARENT),
                );
            }
        }
        RgbaCanvas::from_pixels(lw, lh, pixels).expect("画素数は lw*lh で作っている")
    }

    #[test]
    fn a_non_integer_stretch_can_be_undone_exactly() {
        // 非整数倍で拡大された絵から元絵が**厳密に**戻ること．
        // 実測 (DOT ILLUST) では 500/22 = 22.727 倍などで配られていた
        let src = (0..)
            .map(sprite::synthesize)
            .find(|c| c.width() >= 20 && c.height() >= 20)
            .expect("元絵がある");
        let big = stretch(&src, 500, 466);
        assert_eq!(
            recover_native_size(&big, false, NATIVE_MAX),
            Some(src.width())
        );
        assert_eq!(
            recover_native_size(&big, true, NATIVE_MAX),
            Some(src.height())
        );
        let back = sample_native(&big, (src.width(), src.height()));
        assert_eq!(back.pixels(), src.pixels(), "元絵に戻っていない");
    }

    #[test]
    fn an_interpolated_stretch_is_refused() {
        // 補間が掛かっていれば画素が動いているので復元できない — 黙って壊れた元絵を
        // 作らないこと
        let src = (0..)
            .map(sprite::synthesize)
            .find(|c| c.width() >= 20 && c.height() >= 20)
            .expect("元絵がある");
        let blurred = Degradation {
            scale: 24,
            filter: Filter::Bilinear,
            resize: Resize::Keep,
            compression: Compression::Png,
            crop: (0, 0),
        }
        .apply(&src)
        .unwrap();
        assert_eq!(recover_native_size(&blurred, false, NATIVE_MAX), None);
    }

    #[test]
    fn a_flat_image_has_no_recoverable_size() {
        // 変化が無いとどの N でも条件を満たす．読めたことにしない
        let flat = RgbaCanvas::filled(120, 120, Rgba8::rgb(10, 20, 30));
        assert_eq!(recover_native_size(&flat, false, NATIVE_MAX), None);
    }

    #[test]
    fn an_already_native_sprite_is_taken_as_is() {
        // CC0 素材は 16x16 ・32x32 の元絵そのままで配られている．縮小してはいけない
        use pxsmith_core::grid::{GridParams, estimate_grid};
        let src = (0..)
            .map(sprite::synthesize)
            .find(|c| c.width() >= 16 && c.height() >= 16)
            .expect("元絵がある");
        let (out, info) = ingest_one(
            &write_temp(&src, "native.png", "as-is"),
            &Recipe {
                already_native: true,
                ..recipe(8, (3, 1))
            },
        )
        .unwrap()
        .expect("拒否された");
        assert_eq!(info.reduction, Reduction::AsIs);
        assert_eq!(info.native, (src.width(), src.height()));
        has_grid(&out, 8, (3, 1));

        let e = estimate_grid(&out, &GridParams::default()).expect("推定できない");
        assert_eq!(e.scale, 8);
        assert_eq!((e.phase.x as u32, e.phase.y as u32), truth_phase(8, (3, 1)));
    }

    #[test]
    fn an_already_upscaled_image_is_refused_as_native() {
        // 拡大済みの絵を「元絵」として渡すと，s と ks の格子が両方成立して正解が割れる
        let src = (0..)
            .map(sprite::synthesize)
            .find(|c| c.width() >= 40 && c.height() >= 40)
            .expect("大きい元絵がある");
        let big = blow_up(&src, 6);
        assert_eq!(
            ingest_one(
                &write_temp(&big, "not_native.png", "as-is-refuse"),
                &Recipe {
                    already_native: true,
                    native_max: 512,
                    ..recipe(4, (0, 0))
                }
            )
            .unwrap(),
            Err(Refusal::NotNative {
                native: (src.width(), src.height())
            })
        );
    }

    #[test]
    fn a_small_sprite_is_not_mistaken_for_a_block_image() {
        // 小さい元絵を「拡大されたもの」と誤判定しないこと．
        //
        // ここに `detect_period` や `recover_native_size` を使うと落ちる — 前者は
        // 16 画素角で周期 2 ・3 しか調べられず当てずっぽうになり，後者は変化が少ない
        // 短い軸で偶然条件を満たす小さい N を拾う (実測: CC0 の 16x16 タイルが 5 件
        // 「2x2 のブロック」と，24 件「14x15 等からの拡大」と判定された)
        for seed in 0..40 {
            let src = sprite::synthesize(seed);
            assert_eq!(
                integer_block_size(&src),
                None,
                "元絵 {seed} ({}x{}) を拡大されたものと見なした",
                src.width(),
                src.height()
            );
        }
    }

    #[test]
    fn an_integer_upscale_is_detected_but_a_non_integer_one_is_not() {
        let src = (0..)
            .map(sprite::synthesize)
            .find(|c| c.width() >= 20 && c.height() >= 20)
            .expect("元絵がある");
        // 整数倍は正解が割れる — 見つけなければならない
        assert_eq!(integer_block_size(&blow_up(&src, 4)), Some(4));
        // 非整数倍は粗い側に整数の格子が無いので割れない — 見つけてはいけない
        let (w, h) = (src.width() * 7 / 2, src.height() * 7 / 2);
        assert_eq!(integer_block_size(&stretch(&src, w, h)), None);
    }

    #[test]
    fn a_recovered_native_becomes_a_positive() {
        // 非整数倍で配られた絵 → 元絵を戻す → こちらが決めた倍率で拡大 → 正例
        use pxsmith_core::grid::{GridParams, estimate_grid};
        let src = (0..)
            .map(sprite::synthesize)
            .find(|c| c.width() >= 20 && c.height() >= 20)
            .expect("元絵がある");
        let big = stretch(&src, 500, 466);
        let (out, info) = ingest_one(
            &write_temp(&big, "stretched.png", "recover"),
            &Recipe {
                recover_native: true,
                ..recipe(6, (2, 3))
            },
        )
        .unwrap()
        .expect("拒否された");
        assert_eq!(info.reduction, Reduction::Recovered);
        assert_eq!(info.native, (src.width(), src.height()));
        assert_eq!(info.original, (500, 466));
        has_grid(&out, 6, (2, 3));

        let e = estimate_grid(&out, &GridParams::default()).expect("推定できない");
        assert_eq!(e.scale, 6);
        assert_eq!((e.phase.x as u32, e.phase.y as u32), truth_phase(6, (2, 3)));
    }

    #[test]
    fn the_ingested_image_can_be_recovered_by_the_estimator() {
        // 仕立てた正例が実際に推定できること — これが通らないと正例と呼べない
        use pxsmith_core::grid::{GridParams, estimate_grid};
        // 大きめの元絵を選ぶ (周期を読むには数周期分の長さが要る)
        let src = (0..)
            .map(sprite::synthesize)
            .find(|c| c.width() >= 40 && c.height() >= 40)
            .expect("大きい元絵がある");
        let big = blow_up(&src, 8); // AI 出力の代わり (周期 8)
        let (out, info) = ingest_one(
            &write_temp(&big, "ingest_case.png", "recover"),
            &recipe(6, (2, 1)),
        )
        .unwrap()
        .expect("拒否された");
        assert_eq!(info.reduction, Reduction::Period(8));

        let e = estimate_grid(&out, &GridParams::default()).expect("推定できない");
        assert_eq!(e.scale, 6);
        assert_eq!((e.phase.x as u32, e.phase.y as u32), truth_phase(6, (2, 1)));
    }

    /// 主張した倍率と位相の格子が本当に入っているか．
    ///
    /// 最近傍で $s$ 倍した画像では，**ブロックの境目でない行はすべて 1 つ上と同一**に
    /// なる (境目は $(y + d_y) \bmod s = 0$ の行) ．推定器が読めるかどうかとは別に，
    /// 格子があること自体はこれで確かめられる．
    fn has_grid(img: &RgbaCanvas, scale: u32, crop: (u32, u32)) {
        let s = scale as i32;
        let (dx, dy) = (crop.0 as i32 % s, crop.1 as i32 % s);
        for y in 1..img.height() as i32 {
            if (y + dy) % s == 0 {
                continue;
            }
            for x in 0..img.width() as i32 {
                assert_eq!(img.get(x, y), img.get(x, y - 1), "行 {y} が上と違う");
            }
        }
        for x in 1..img.width() as i32 {
            if (x + dx) % s == 0 {
                continue;
            }
            for y in 0..img.height() as i32 {
                assert_eq!(img.get(x, y), img.get(x - 1, y), "列 {x} が左と違う");
            }
        }
    }

    #[test]
    fn a_forced_period_makes_a_positive_out_of_a_refused_image() {
        // 段 1 が読めない画像でも，縮小の粗さを決めれば正例になる．
        // **正解は段 3 の倍率なので循環しない**
        let mut rng = crate::rng::Rng::new(11);
        // 周期の無い画像 (滑らかな勾配 + 雑音) — detect_period は読めない
        let (w, h) = (240u32, 240u32);
        let pixels: Vec<Rgba8> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let n = rng.below(24) as u8;
                Rgba8::rgb(
                    (x * 255 / w) as u8,
                    (y * 255 / h) as u8,
                    n.saturating_add(80),
                )
            })
            .collect();
        let img = RgbaCanvas::from_pixels(w, h, pixels).unwrap();
        let path = write_temp(&img, "forced.png", "forced");
        assert_eq!(detect_period(&img), Err(Refusal::NoPeriod), "前提が崩れた");

        let (out, info) = ingest_one(
            &path,
            &Recipe {
                force_period: Some(8),
                ..recipe(5, (1, 2))
            },
        )
        .unwrap()
        .expect("拒否された");
        assert_eq!(info.reduction, Reduction::ForcedPeriod(8));
        assert_eq!(info.native, (30, 30));
        // 格子が本当に入っていること — 正例と呼べる根拠はこれである
        has_grid(&out, 5, (1, 2));

        // **推定器が読めるとは限らない．** この画像 (滑らかな勾配) は棄却される —
        // 平坦すぎて信頼度が立たないためで，まさにこの領域が測れていなかった．
        // 「正例を作れること」と「今の推定器が解けること」は別である
        use pxsmith_core::grid::{GridError, GridParams, estimate_grid};
        assert!(matches!(
            estimate_grid(&out, &GridParams::default()),
            Err(GridError::LowConfidence)
        ));
    }

    #[test]
    fn forcing_a_divisor_of_the_real_block_is_refused() {
        // 周期 8 の画像を 4 で縮小すると 2x2 のブロックが残り，s の格子と 2s の格子が
        // 両方成立する．**正解が一意に決まらないので拒否する** — 黙って s を正解として
        // 書くと，推定器が返す 2s を誤答として数えてしまう
        let src = (0..)
            .map(sprite::synthesize)
            .find(|c| c.width() >= 40 && c.height() >= 40)
            .expect("大きい元絵がある");
        let big = blow_up(&src, 8);
        let path = write_temp(&big, "divisor.png", "divisor");
        assert_eq!(
            ingest_one(
                &path,
                &Recipe {
                    force_period: Some(4),
                    native_max: 128,
                    ..recipe(5, (1, 2))
                }
            )
            .unwrap(),
            Err(Refusal::AmbiguousTruth {
                forced: 4,
                detected: 8
            })
        );
    }

    #[test]
    fn a_forced_period_positive_can_still_be_recovered_when_the_content_is_not_flat() {
        // 中身に階調があれば，指定した周期で作った正例も推定器が読める
        use pxsmith_core::grid::{GridParams, estimate_grid};
        let src = (0..)
            .map(sprite::synthesize)
            .find(|c| c.width() >= 40 && c.height() >= 40)
            .expect("大きい元絵がある");
        let big = blow_up(&src, 8);
        // 測れば 8 だが，あえて 12 を指定する (約数ではないのでブロックは残らない)
        let (out, info) = ingest_one(
            &write_temp(&big, "forced_ok.png", "forced-ok"),
            &Recipe {
                force_period: Some(12),
                ..recipe(5, (1, 2))
            },
        )
        .unwrap()
        .expect("拒否された");
        assert_eq!(info.reduction, Reduction::ForcedPeriod(12));
        has_grid(&out, 5, (1, 2));

        let e = estimate_grid(&out, &GridParams::default()).expect("推定できない");
        assert_eq!(e.scale, 5);
        assert_eq!((e.phase.x as u32, e.phase.y as u32), truth_phase(5, (1, 2)));
    }

    #[test]
    fn degrading_keeps_the_integer_grid() {
        // 劣化を掛けても正解 (倍率と位相) は変わらない — 非整数倍リサイズは掛けないため
        let src = sprite::synthesize(26);
        let big = blow_up(&src, 6);
        let path = write_temp(&big, "degraded.png", "degrade");
        for (filter, compression) in [
            (Filter::Bilinear, Compression::Png),
            (Filter::Bicubic, Compression::Jpeg80),
        ] {
            let (out, info) = ingest_one(
                &path,
                &Recipe {
                    degrade: Some((filter, compression)),
                    ..recipe(4, (1, 0))
                },
            )
            .unwrap()
            .expect("拒否された");
            assert_eq!(info.degrade, Some((filter, compression)));
            // 大きさは最近傍で作った場合と一致する (格子の周期が変わっていない証拠)
            let period = match info.reduction {
                Reduction::Period(p) | Reduction::ForcedPeriod(p) => p,
                Reduction::Recovered | Reduction::AsIs => unreachable!("周期で縮小している"),
            };
            let clean = upscale(&downscale_mean(&big, period), info.scale, (1, 0));
            assert_eq!((out.width(), out.height()), (clean.width(), clean.height()));
        }
    }

    /// 試験ごとにディレクトリを分ける — 共有すると `fs::write` の隙に別の試験が読む．
    fn write_temp(img: &RgbaCanvas, name: &str, case: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("pxforge-ingest-test-{case}"));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        pxsmith_io::png::write_rgba(&path, img).unwrap();
        path
    }
}
