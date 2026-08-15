//! **lint の負例を作る．**
//!
//! 「良い絵に現行の閾値を掛ける」で分かるのは**誤爆**だけである．ルールが本来の相手を
//! 捕まえるかは，**わざと壊した絵**でしか測れない．手描きの失敗例は用意できないので，
//! CC0 の実物のドット絵に**狙った欠陥を 1 つだけ**入れて作る (`degrade` と同じ考え方) ．
//!
//! - 元絵は良い絵なので，鳴ったルールは**入れた欠陥のせい**だと言える
//! - 生成は種で決まる (決定論性の規則 1)
//! - **1 枚に 1 種類の欠陥**しか入れない．混ぜると «どのルールが何を見たか» が分からない

use std::path::Path;

use anyhow::{Context, Result};
use pxsmith_core::canvas::RgbaCanvas;
use pxsmith_core::color::{Rgba8, oklab_of};

use crate::rng::Rng;

/// 入れる欠陥の種類．**ルール 1 つに 1 種類**を対応させる．
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Defect {
    /// ルール 3 — どこにも無い色の 1 画素を撒く．
    StrayPixels,
    /// ルール 10 — ディザの中で同じ色を長く続ける (塊化)．
    DitherClump,
    /// ルール 15 — 画面の大半をディザで埋める．
    DitherFlood,
    /// ルール 11 — 隣り合う面の明度差を潰す．
    FlatLightness,
    /// ルール 16 — 大面積を高彩度で塗る．
    LoudFill,
    /// ルール 17 — 明度が大きく離れた 2 色でディザを敷く．
    HarshDither,
    /// ルール 18 — 純黒を混ぜる．
    PureBlack,
    /// ルール 13 — 縁から中心へ一様に明るくする (pillow shading)．
    Pillow,
    /// ルール 8 — 色境界をぎざぎざにする (ジャギー)．
    Jaggy,
    /// ルール 14 — 中間色を敷き詰める (AA 過多)．
    AaFlood,
    /// ルール 4 — 縁取りの角を重ねる．
    OutlineCorner,
    /// ルール 6 — 影を光と同一色相の明度違いだけにする．
    MonoShadow,
    /// ルール 12 — 同じ形の段の列を並走させる (バンディング)．
    Banding,
}

impl Defect {
    pub const ALL: [Defect; 13] = [
        Defect::StrayPixels,
        Defect::DitherClump,
        Defect::DitherFlood,
        Defect::FlatLightness,
        Defect::LoudFill,
        Defect::HarshDither,
        Defect::PureBlack,
        Defect::Pillow,
        Defect::Jaggy,
        Defect::AaFlood,
        Defect::OutlineCorner,
        Defect::MonoShadow,
        Defect::Banding,
    ];

    /// 狙っているルール番号．
    pub fn rule(self) -> u8 {
        match self {
            Self::StrayPixels => 3,
            Self::DitherClump => 10,
            Self::DitherFlood => 15,
            Self::FlatLightness => 11,
            Self::LoudFill => 16,
            Self::HarshDither => 17,
            Self::PureBlack => 18,
            Self::Pillow => 13,
            Self::Jaggy => 8,
            Self::AaFlood => 14,
            Self::OutlineCorner => 4,
            Self::MonoShadow => 6,
            Self::Banding => 12,
        }
    }

    /// **敷ける面が要る欠陥か．** ディザ系は透明の穴があると検出に届かない．
    /// ジャギーも同じで，透明で段が途切れると «長い段の列» にならない．
    pub fn needs_solid_area(self) -> bool {
        matches!(
            self,
            Self::DitherClump | Self::DitherFlood | Self::HarshDither | Self::Jaggy | Self::Banding
        )
    }

    /// **シルエットが要る欠陥か．** 縁取りは透明に対して描くものなので，
    /// 画面いっぱいのタイルには入れられない．
    pub fn needs_silhouette(self) -> bool {
        matches!(self, Self::OutlineCorner)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::StrayPixels => "stray",
            Self::DitherClump => "clump",
            Self::DitherFlood => "flood",
            Self::FlatLightness => "flat",
            Self::LoudFill => "loud",
            Self::HarshDither => "harsh",
            Self::PureBlack => "black",
            Self::Pillow => "pillow",
            Self::Jaggy => "jaggy",
            Self::AaFlood => "aaflood",
            Self::OutlineCorner => "corner",
            Self::MonoShadow => "mono",
            Self::Banding => "band",
        }
    }
}

/// 絵の中で最も広い不透明な色 (置き換えの相手にする)．
fn dominant(img: &RgbaCanvas) -> Option<Rgba8> {
    let mut counts: std::collections::BTreeMap<[u8; 4], usize> = std::collections::BTreeMap::new();
    for p in img.pixels() {
        if p.a == 0 {
            continue;
        }
        *counts.entry([p.r, p.g, p.b, p.a]).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(c, n)| (*n, std::cmp::Reverse(*c)))
        .map(|(c, _)| Rgba8 {
            r: c[0],
            g: c[1],
            b: c[2],
            a: c[3],
        })
}

/// 不透明な画素の座標を集める．
fn opaque_pixels(img: &RgbaCanvas) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    for y in 0..img.height() as i32 {
        for x in 0..img.width() as i32 {
            if img.get(x, y).is_some_and(|c| c.a != 0) {
                out.push((x, y));
            }
        }
    }
    out
}

/// 欠陥を 1 つ入れる．**元絵は壊さない** (複製に入れる)．
pub fn apply(src: &RgbaCanvas, defect: Defect, seed: u64) -> RgbaCanvas {
    let mut img = src.clone();
    let mut rng = Rng::new(seed);
    let pixels = opaque_pixels(src);
    if pixels.is_empty() {
        return img;
    }
    let base = dominant(src).unwrap_or(Rgba8::rgb(128, 128, 128));
    let lab = oklab_of(base);

    match defect {
        // どこにも無い色を 1 画素だけ置く．**色は 1 点ごとに変える** — 同じ色を
        // 2 度使うと «その色は他にもある» ことになりルール 3 の相手ではなくなる
        Defect::StrayPixels => {
            for i in 0..12u32 {
                let (x, y) = pixels[rng.below(pixels.len() as u32) as usize];
                let c = Rgba8::rgb(
                    (17 + i * 19) as u8,
                    (211 - i * 13) as u8,
                    (73 + i * 7) as u8,
                );
                img.set(x, y, c);
            }
        }
        // 市松のディザを敷き，その中で片方の色を横に長く続ける．
        //
        // **欠陥は検出窓より十分大きくする．** ディザの検出は 8 画素角の窓で
        // «2 色が 92% 以上» を求めるので，16x16 の絵に中央 8x8 だけ敷くと窓が
        // 領域の外へはみ出して 1 枚も見つからない — 実際 16x16 の 3 枚が
        // «ルールの見逃し» ではなく **生成の弱さ**で鳴っていなかった
        Defect::DitherClump => {
            let other = shifted(base, 0.18);
            let (w, h) = (img.width() as i32, img.height() as i32);
            // **窓の格子に揃える．** 検出は 8 画素角の窓を 8 画素刻みで置くので，
            // 領域が格子からずれていると窓が領域の外へはみ出し，16x16 では
            // 1 つも収まらない — «ルールの見逃し» に見えていた 2 枚はこれだった
            let margin = |n: i32| if n >= 32 { 8 } else { 0 };
            let (x0, y0) = (margin(w), margin(h));
            let (x1, y1) = (w - margin(w), h - margin(h));
            for y in y0..y1 {
                for x in x0..x1 {
                    if img.get(x, y).is_some_and(|c| c.a == 0) {
                        continue;
                    }
                    // 塊: 一定の行では片方の色だけを並べる
                    let clumped = (y - y0) % 5 == 0;
                    let use_other = if clumped { true } else { (x + y) % 2 == 0 };
                    img.set(x, y, if use_other { other } else { base });
                }
            }
        }
        // 画面のほとんどをディザで埋める
        Defect::DitherFlood => {
            let other = shifted(base, 0.12);
            for &(x, y) in &pixels {
                img.set(x, y, if (x + y) % 2 == 0 { base } else { other });
            }
        }
        // 隣り合う面の明度差を潰す (どの色も base とほぼ同じ明度にする)
        Defect::FlatLightness => {
            for &(x, y) in &pixels {
                let Some(c) = img.get(x, y) else { continue };
                let mut l = oklab_of(c);
                // 明度だけ base に寄せる — 色相 ・彩度は残す
                l.l = lab.l + (l.l - lab.l) * 0.05;
                let mut c2 = pxsmith_core::quantize::oklab_to_rgba(l);
                c2.a = c.a;
                img.set(x, y, c2);
            }
        }
        // 大面積を高彩度で塗る
        Defect::LoudFill => {
            let loud = Rgba8::rgb(0xff, 0x18, 0x08);
            let (w, h) = (img.width() as i32, img.height() as i32);
            for y in 0..h {
                for x in 0..w {
                    if y >= h / 3 && img.get(x, y).is_some_and(|c| c.a != 0) {
                        img.set(x, y, loud);
                    }
                }
            }
        }
        // 明度が大きく離れた 2 色でディザを敷く
        Defect::HarshDither => {
            let (dark, light) = (Rgba8::rgb(0x14, 0x12, 0x1a), Rgba8::rgb(0xf4, 0xf2, 0xe8));
            for &(x, y) in &pixels {
                img.set(x, y, if (x + y) % 2 == 0 { dark } else { light });
            }
        }
        // **教科書どおりの pillow shading．** 縁からの距離だけで明るさを決める —
        // 光源の向きを持たない同心状の陰影である．
        //
        // **絵が元から持っている色しか使わない** (明度順に並べて距離で引く) ．
        // 新しい色を作ると，鳴ったのがルール 13 なのか «パレットが変わったから» なのか
        // 分からなくなる．
        Defect::Pillow => {
            let mut palette: Vec<Rgba8> = {
                let mut c: Vec<Rgba8> = src.pixels().iter().copied().filter(|c| c.a != 0).collect();
                c.sort_unstable_by_key(|c| c.sort_key());
                c.dedup();
                c
            };
            palette.sort_by(|a, b| {
                oklab_of(*a)
                    .l
                    .total_cmp(&oklab_of(*b).l)
                    // 同じ明度は色で決める (決定論性の規則 2)
                    .then(a.sort_key().cmp(&b.sort_key()))
            });
            if palette.len() >= 2 {
                let mut mask = pxsmith_core::geom::Mask::new(img.width(), img.height());
                for &(x, y) in &pixels {
                    mask.set(pxsmith_core::math::ivec2(x, y), true);
                }
                let d = pxsmith_core::geom::signed_distance(&mask);
                let far = pixels
                    .iter()
                    .filter_map(|&(x, y)| d.copied(pxsmith_core::math::ivec2(x, y)))
                    .fold(0.0f32, f32::max)
                    .max(1.0);
                for &(x, y) in &pixels {
                    let at = pxsmith_core::math::ivec2(x, y);
                    let t = (d.copied(at).unwrap_or(0.0) / far).clamp(0.0, 1.0);
                    let i = ((t * palette.len() as f32).floor() as usize).min(palette.len() - 1);
                    let mut c = palette[i];
                    c.a = img.get(x, y).map(|c| c.a).unwrap_or(255);
                    img.set(x, y, c);
                }
            }
        }
        // **長い段の列の中に «1 画素の段» を混ぜる** (ルール 8 の教科書どおりの相手)．
        Defect::Jaggy => draw_jagged_steps(&mut img, base, &mut rng),
        // **中間色を敷き詰める** — 設計書 6.5 が «多すぎるより少ない方が良い» と
        // 言う，まさにその «多すぎる» 側である．
        Defect::AaFlood => {
            if let Some(out) = blur_every_boundary(src) {
                img = out;
            }
        }
        // **角の重なる縁取りを描く．** «斜めにも接していれば縁» とすると，段の角で
        // 縦の線と横の線が両方置かれて $2 \times 2$ になる — 角で «曲がる» のでは
        // なく «足す» と起きる失敗そのものである．
        //
        // 色は絵の中で最も暗い色を使う (新しい色を作らない) ．
        Defect::OutlineCorner => {
            let ink = {
                let mut colours: Vec<Rgba8> =
                    src.pixels().iter().copied().filter(|c| c.a != 0).collect();
                colours.sort_unstable_by_key(|c| c.sort_key());
                colours.dedup();
                colours
                    .into_iter()
                    // 同点は色で決める (決定論性の規則 2)
                    .min_by(|a, b| {
                        oklab_of(*a)
                            .l
                            .total_cmp(&oklab_of(*b).l)
                            .then(a.sort_key().cmp(&b.sort_key()))
                    })
                    .unwrap_or(Rgba8::rgb(0, 0, 0))
            };
            let (w, h) = (src.width() as i32, src.height() as i32);
            let clear = |x: i32, y: i32| {
                x < 0 || y < 0 || x >= w || y >= h || src.get(x, y).is_some_and(|c| c.a == 0)
            };
            for &(x, y) in &pixels {
                // **8 近傍**で見る (4 近傍なら 1 画素幅の正しい縁になる)
                if (-1..=1)
                    .any(|dy| (-1..=1).any(|dx| (dx != 0 || dy != 0) && clear(x + dx, y + dy)))
                {
                    let mut c = ink;
                    c.a = src.get(x, y).map(|c| c.a).unwrap_or(255);
                    img.set(x, y, c);
                }
            }
        }
        // **すべての色を «固有色の明度違い» にする．**
        //
        // 色相と彩度の «向き» を固有色に揃え，明度だけ元のまま残す — 影が光と
        // 同一色相の明度違いだけになる (設計書 7.3 のルール 6 そのもの) ．
        Defect::MonoShadow => {
            let hue = lab.b.atan2(lab.a);
            for &(x, y) in &pixels {
                let Some(c) = img.get(x, y) else { continue };
                let here = oklab_of(c);
                let chroma = here.chroma();
                let mut moved =
                    pxsmith_core::color::Oklab::new(here.l, chroma * hue.cos(), chroma * hue.sin());
                // 彩度が 0 だと色相が無い — 固有色の彩度を最低限持たせる
                if chroma < 0.03 {
                    moved.a = 0.05 * hue.cos();
                    moved.b = 0.05 * hue.sin();
                }
                let mut c2 = pxsmith_core::quantize::oklab_to_rgba(moved);
                c2.a = c.a;
                img.set(x, y, c2);
            }
        }
        // **同じ形の段の列を一定の間隔で並べる．**
        //
        // 傾き 2 の斜めの帯を 3 画素おきに敷く — どの帯の縁も同じラン長列を持ち，
        // 一定の隔たりで並走する (設計書 7.3 のルール 12 «同じ長さのランが並走») ．
        // 色は元からある 2 色だけを使う．
        Defect::Banding => {
            let other = shifted(base, 0.16);
            for &(x, y) in &pixels {
                let band = ((x * 2 + y).div_euclid(3)) % 2 == 0;
                img.set(x, y, if band { base } else { other });
            }
        }
        // 純黒を混ぜる
        Defect::PureBlack => {
            let (w, h) = (img.width() as i32, img.height() as i32);
            for y in 0..h {
                for x in 0..w {
                    if img.get(x, y).is_some_and(|c| c.a != 0) && (x + y) % 7 == 0 {
                        img.set(x, y, Rgba8::rgb(0, 0, 0));
                    }
                }
            }
        }
    }
    img
}

/// **ぎざぎざの段を «描く»** — 長い段の列の中に 1 画素の段を混ぜる (設計書 6.4)．
///
/// 使うのは絵が元から持っている 2 色だけで，**新しい色は作らない** — 鳴ったのが
/// ジャギーのせいか色のせいか分からなくなるのを避ける．
///
/// > [!warning] **既にある境界を突いても負例にならなかった．**
/// > 先に 3 通り測った — どれも良い絵の分布に埋もれるか，1 件も当たらない．
/// >
/// > | 壊し方 | 負例の谷の率 | 良い絵 (中央 0.0027 ・最大 0.060) |
/// > | --- | --- | --- |
/// > | 境界画素の 40% を隣の色と入れ替える | 0 〜 0.0097 | 埋もれる |
/// > | 25% ・互いに 3 画素離して突く | 0 〜 0.019 | 埋もれる |
/// > | **ラン長列を見て «両隣以下の段» を 1 画素削る** | **1 件も削れない** | — |
/// >
/// > 1 つ目 ・2 つ目は**刻みすぎ**である．ラン長がすべて 1 になると
/// > $r_{i-1} > r_i < r_{i+1}$ (厳密な不等号) が成り立たず，**谷そのものが消える**．
/// > 3 つ目が 1 件も当たらないのは素材の側の事情で，**種 64 枚の単調区間は
/// > ラン 2 本以下がほとんど**である (例: 16x16 のタイルで 18 区間に 34 ラン) ．
/// > 削れる «3 ラン以上の区間» が存在しない．
/// >
/// > つまり**この素材には «長い段の列» が無い**ので，突いて作ることはできない．
/// > 段の列ごと描く．
///
/// 描くのは**段幅が揃っていない階段**である (設計書 6.4 の «ランをより長いランで
/// 挟まない» に真っ向から反する形) ．**検出器の出力は 1 度も参照しない** (参照すると
/// «検出器が鳴るもの» が負例の定義になり，捕捉率が自明に 100% になる) ．
///
/// > [!note] **段を 1 つだけ短くした階段では足りなかった．**
/// > 段幅 $[S, S, S, 1, S, S, S]$ の階段は谷が 1 つで，**良い絵の 34 / 61 枚と
/// > 同じ密度**である (1 区間あたり 1 谷) ．«絵の中にジャギーが 1 つある» は良い絵に
/// > も普通にあることなので，負例は «縁がぎざぎざである» 方でなければならない．
fn draw_jagged_steps(img: &mut RgbaCanvas, base: Rgba8, rng: &mut Rng) {
    let (w, h) = (img.width() as i32, img.height() as i32);
    if w < 8 || h < 8 {
        return;
    }
    // 段の «下» になる色 — 元からある色のうち base と明度が最も離れたもの
    let other = {
        let mut colours: Vec<Rgba8> = img.pixels().iter().copied().filter(|c| c.a != 0).collect();
        colours.sort_unstable_by_key(|c| c.sort_key());
        colours.dedup();
        let l0 = oklab_of(base).l;
        colours
            .into_iter()
            // 同点は色で決める (決定論性の規則 2)
            .max_by(|a, b| {
                let (da, db) = ((oklab_of(*a).l - l0).abs(), (oklab_of(*b).l - l0).abs());
                da.total_cmp(&db).then(b.sort_key().cmp(&a.sort_key()))
            })
            .unwrap_or_else(|| shifted(base, 0.3))
    };
    // 下半分を other で塗り (段の «地»)，その上端に base の階段を描く
    let top = h / 2;
    for y in top..h {
        for x in 0..w {
            if img.get(x, y).is_some_and(|c| c.a != 0) {
                img.set(x, y, other);
            }
        }
    }
    // **段幅を毎段ばらばらにする** ($1 \ldots S$) ．揃った階段はジャギーではない
    let s = (w / 5).max(3);
    let mut x = 0i32;
    let mut step = 0i32;
    while x < w && top + step + 1 < h {
        let width = 1 + rng.below(s as u32) as i32;
        for dx in 0..width {
            for y in top..=(top + step) {
                if x + dx < w && img.get(x + dx, y).is_some_and(|c| c.a != 0) {
                    img.set(x + dx, y, base);
                }
            }
        }
        x += width;
        step += 1;
    }
}

/// **すべての色境界に中間色を敷く** (ルール 14 の相手を作る)．
///
/// 隣に別の面がある画素を，その 2 色の中点の色へ置き換える — AI の出力や
/// 補間つき縮小が作る «縁がぼやけた絵» そのものである．
///
/// > [!note] **`pxsmith aa` を過剰な設定で掛ける形では足りなかった．**
/// > 段の下限 2 ・外郭あり ・4 巡で掛けても，中間色の割合は 3.9 〜 10.2% にしか
/// > ならず，**良い絵の 90% 点 (11.2%) を下回った**．`pxsmith aa` は «角» にしか置かない
/// > ので，どれだけ回しても縁が埋まらない．過剰な AA は «縁を全部塗る» ことである．
fn blur_every_boundary(src: &RgbaCanvas) -> Option<RgbaCanvas> {
    let mut out = src.clone();
    let (w, h) = (src.width() as i32, src.height() as i32);
    let mut touched = 0usize;
    for y in 0..h {
        for x in 0..w {
            let Some(here) = src.get(x, y) else { continue };
            if here.a == 0 {
                continue;
            }
            // 4 近傍で最も «遠い» 別の不透明色を相手に選ぶ (同点は色で決める)
            let mut best: Option<(f32, Rgba8)> = None;
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let Some(c) = src.get(x + dx, y + dy) else {
                    continue;
                };
                if c.a == 0 || (c.r, c.g, c.b) == (here.r, here.g, here.b) {
                    continue;
                }
                let d = pxsmith_core::color::distance_sq(oklab_of(here), oklab_of(c), 1.0);
                if best
                    .as_ref()
                    .is_none_or(|(bd, bc)| d > *bd || (d == *bd && c.sort_key() < bc.sort_key()))
                {
                    best = Some((d, c));
                }
            }
            let Some((_, other)) = best else { continue };
            let (la, lb) = (oklab_of(here), oklab_of(other));
            let mid = pxsmith_core::color::Oklab::new(
                (la.l + lb.l) * 0.5,
                (la.a + lb.a) * 0.5,
                (la.b + lb.b) * 0.5,
            );
            let mut c = pxsmith_core::quantize::oklab_to_rgba(mid);
            c.a = here.a;
            out.set(x, y, c);
            touched += 1;
        }
    }
    (touched > 0).then_some(out)
}

/// 明度を少しずらした色 (ディザの相方に使う)．
///
/// **明るい方へ寄せられないときは暗い方へ寄せる．** 明度を上げるだけだと，元が
/// すでに明るい絵で丸めた後に**同じ色**になり，ディザではなく塗り潰しになる —
/// 実際にそれで 2 枚がルール 10 に鳴っていなかった (鳴っていたのは «大面積の
/// 高彩度色» の方) ．
fn shifted(base: Rgba8, dl: f32) -> Rgba8 {
    let lab = oklab_of(base);
    for delta in [dl, -dl, dl * 2.0, -dl * 2.0] {
        let mut moved = lab;
        moved.l = (lab.l + delta).clamp(0.0, 1.0);
        let mut c = pxsmith_core::quantize::oklab_to_rgba(moved);
        c.a = base.a;
        if (c.r, c.g, c.b) != (base.r, base.g, base.b) {
            return c;
        }
    }
    // どうしても動かない (完全な白か黒) ときは反対側の端へ振る
    let mut c = if lab.l > 0.5 {
        Rgba8::rgb(0x20, 0x20, 0x28)
    } else {
        Rgba8::rgb(0xe0, 0xe0, 0xd8)
    };
    c.a = base.a;
    c
}

/// 種のディレクトリから負例を書き出す．
pub fn generate(seeds: &Path, out: &Path, per_defect: usize, seed: u64) -> Result<usize> {
    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(seeds)
        .with_context(|| format!("{} を読めない", seeds.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")))
        .collect();
    paths.sort();
    anyhow::ensure!(!paths.is_empty(), "{} に PNG が無い", seeds.display());
    std::fs::create_dir_all(out)?;

    // 透けている割合を先に測る．**ディザ系の欠陥は «敷ける面» が要る** — 検出は
    // 8 画素角の窓で «2 色が 92% 以上» を求めるので，透明で穴が空くと見つからない．
    // 16x16 の絵で 2 枚が «ルールの見逃し» ではなくこれで鳴っていなかった
    let mut art: Vec<(std::path::PathBuf, RgbaCanvas, f32)> = Vec::new();
    for path in &paths {
        let img = pxsmith_io::png::read_rgba(path)
            .with_context(|| format!("{} を読めない", path.display()))?;
        let opaque = img.pixels().iter().filter(|c| c.a != 0).count() as f32;
        let ratio = opaque / (img.width() * img.height()).max(1) as f32;
        art.push((path.clone(), img, ratio));
    }
    let solid: Vec<usize> = (0..art.len()).filter(|&i| art[i].2 >= 0.98).collect();
    anyhow::ensure!(!solid.is_empty(), "ディザを敷ける絵が 1 枚も無い");
    // 縁取りの欠陥には**シルエットのある絵**が要る (画面いっぱいのタイルには縁が無い)
    let cut_out: Vec<usize> = (0..art.len())
        .filter(|&i| (0.1..0.9).contains(&art[i].2))
        .collect();
    anyhow::ensure!(!cut_out.is_empty(), "シルエットのある絵が 1 枚も無い");

    let mut written = 0;
    for defect in Defect::ALL {
        for i in 0..per_defect {
            // 種ごとに違う絵を使う．**添字で選ぶので毎回同じ絵になる**
            let pick = defect.rule() as usize * 13 + i * 7;
            let index = if defect.needs_solid_area() {
                solid[pick % solid.len()]
            } else if defect.needs_silhouette() {
                cut_out[pick % cut_out.len()]
            } else {
                pick % art.len()
            };
            let (path, src, _) = &art[index];
            let _ = path;
            let img = apply(src, defect, seed ^ (defect.rule() as u64) << 8 ^ i as u64);
            let name = format!("{}-{i:02}.png", defect.as_str());
            pxsmith_io::png::write_rgba(out.join(&name), &img)
                .with_context(|| format!("{name} を書けない"))?;
            written += 1;
        }
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn art() -> RgbaCanvas {
        let mut img = RgbaCanvas::filled(16, 16, Rgba8::TRANSPARENT);
        for y in 0..16 {
            for x in 0..16 {
                let v = ((x * 31 + y * 17) % 5) as u8;
                img.set(x, y, Rgba8::rgb(40 + v * 40, 60 + v * 30, 120 - v * 20));
            }
        }
        img
    }

    #[test]
    fn a_defect_changes_the_art_and_keeps_the_size() {
        for defect in Defect::ALL {
            let src = art();
            let out = apply(&src, defect, 7);
            assert_eq!((out.width(), out.height()), (src.width(), src.height()));
            assert_ne!(
                out.pixels(),
                src.pixels(),
                "{defect:?} で何も変わっていない"
            );
        }
    }

    #[test]
    fn the_same_seed_gives_the_same_art() {
        let src = art();
        let a = apply(&src, Defect::StrayPixels, 3);
        let b = apply(&src, Defect::StrayPixels, 3);
        assert_eq!(a.pixels(), b.pixels());
    }

    /// 迷子の画素は**それぞれ違う色**でなければならない — 同じ色を 2 度使うと
    /// «その色は他にもある» ことになり，ルール 3 の相手ではなくなる．
    #[test]
    fn stray_pixels_do_not_share_a_colour() {
        let src = art();
        let out = apply(&src, Defect::StrayPixels, 5);
        let mut added: Vec<[u8; 4]> = Vec::new();
        for (a, b) in out.pixels().iter().zip(src.pixels()) {
            if a != b {
                added.push([a.r, a.g, a.b, a.a]);
            }
        }
        let mut uniq = added.clone();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(added.len(), uniq.len(), "同じ色の迷子が複数ある");
    }
}
