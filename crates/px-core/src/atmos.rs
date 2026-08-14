//! 空気遠近法 (`px atmos`)．
//!
//! **設計書はこの機能を «空気遠近法» の 1 語と «`depth` からレイヤ速度係数を出力»
//! の 1 行でしか決めていない** (5 章の表の基盤の欄は «—»，4.4 の出力形式の表に
//! «多重スクロールメタ») ．したがってここも残像 (D126) ・生成過程 GIF (D134) と
//! 同じで，**他の判断から引ける形に落としたもの**である．引いた先を明記する．
//!
//! | 決めたこと | どこから引いたか |
//! | --- | --- |
//! | **空の色は宣言させる** | D89 — 絵だけからは «この絵の空» は決まらない．推定して当てはめるのは同語反復 |
//! | **寄せ具合も宣言させる** | 同上．厚みは «どれだけ遠いか» であって絵には書かれていない (D126 が `--ramp` を必須にしたのと同じ) |
//! | **色を作らない．元の色と空を結ぶ線の上にある色だけを使う** | D94 ・D124 — 並べ替えるだけの道具は色を作ってはいけない．«間» の定義は `px aa` (D83) と 1 つに揃える |
//! | **線の上に色が無ければ動かさずに数える** | D124 «無ければ作らない» と同じ形．**5 度目** |
//! | **速度係数は記録するだけで導出しない** | D92 ・D95 — 視差速度はゲーム側の選択であって絵からは決まらない |
//! | **潰れた色を報告する** | D101 — 削減率と同じで入力で決まる量なので校正しない．黙って平らにしない |
//!
//! # 入力を受け取れるのは L0 だけである (**D81 ・D119 と同じ形の 5 度目**)
//!
//! `depth` の欄を持っているのは L0 (`.px.toml`) だけで，`.aseprite` には対応する
//! 概念が無い (`px-io/src/document.rs` の `project_meta`) ．しかも L0 は
//! **1 ファイル 1 レイヤ** (D9) なので，`[meta] depth` は**ファイル 1 枚に 1 つ**
//! である．これは多重スクロールの単位そのものなので，`px atmos` は
//! **«奥行きを 1 つ持つ絵» を複数受け取る**形にしてある．
//!
//! `.aseprite` を渡すときは `--depth` で宣言する — 読み直しても残らないので，
//! 呼ぶ側が毎回言うことになる．
//!
//! # 何を «寄せる» と呼ぶか
//!
//! 書籍は «遠くのものには白青みをかけ空との色の差を少なくし，より近くのものは
//! 色味が濃くなるように描く» とする [^upc]．混色ができれば
//! $\mathrm{lerp}(c, s, t)$ でよいが，こちらは**固定パレットのインデックス
//! カラー**なので (D2 ・D4) ，できるのは**添字の置き換え**だけである．
//!
//! そこで «寄せた先» を [`nearest_toward`] で選ぶ — **$c$ と空を結ぶ線から
//! 許容以内にあり，空へ近づいている色**のうち $\mathrm{lerp}(c, s, t)$ に
//! 最も近いもの．許容は `px aa` ・`px anim subpixel` と同じ値を引いて使う
//! ([`crate::subpixel::DEFAULT_TOLERANCE`]) ．
//!
//! > [!warning] **«最も近い色» では線から外れる．**
//! > 制約なしの `Palette::nearest` は狙いに近い数字を出すが，**実素材では
//! > 寄せ具合 1.0 で 59.0% の色が線から許容より外れて落ちる** — それは
//! > «霞んだ» のではなく**色が変わった**ということである (下表) ．
//!
//! # 測った結果 (`px-calib atmos`)
//!
//! 実素材 61 枚 x 空 3 色 (`palettes/sweetie-16.hex` から取った晴天 ・曇天 ・
//! 夕方) ．**飛ばした件も数えてある** — 添字にできない絵 3 枚 (256 色超) ．
//!
//! **置き換え先がパレットに在る割合は 76.9%** (2259 色 x 空) ．`px aa` の
//! 81.3% (D83) ・サブピクセルの 53.6% (D124) と同じ量である．
//!
//! | 寄せ具合 | 動いた (線の上) | 線から外れた色 (制約なし) | 残った色 | 明度の幅 |
//! | --- | --- | --- | --- | --- |
//! | 0.15 | 76.9% | 0.2% | 65.9% | 0.71 |
//! | 0.30 | 76.9% | 3.9% | 55.4% | 0.65 |
//! | 0.60 | 76.9% | 20.7% | 38.2% | 0.42 |
//! | 1.00 | 76.9% | **59.0%** | 29.6% | 0.36 |
//!
//! **«在るかどうか» は寄せ具合に依らない** — サブピクセルの $f$ が候補の数を
//! 変えないのと同じ性質である (D124) ．変わるのは «どれを選ぶか» だけである．
//!
//! **明暗差は単調に落ちる** (1.00 → 0.36) ．書籍が言う «空との色の差を少なく
//! する» はこれで満たされる．**奥へ行くほど霞むという単調性も崩れない**
//! (実素材 61 枚 x 空 3 色 x 段 7 通りで逆転 0 件) ．
//!
//! ## 効かないのはパレットのせいであって規則のせいではない
//!
//! パレットに «霞ませた先» を足した真値を作ると，**規則は 99.8% で正解の添字を
//! 引く**．そのとき色は 96.8% (寄せ 0.60) 残る — 絵のままのパレットでは 38.2%
//! しか残らない．
//!
//! > [!warning] **絵のままのパレットに強く掛けると絵が平らになる．**
//! > 道具の誤りではなく «そのパレットにその霞は無い» という結果なので，
//! > [`AtmosReport`] が**潰れた色の数と «段が無かった» 色の数を分けて**返す．
//! > 直し方は «パレットに霞の段を足す» であって，道具が色を作ることではない．
//!
//! ## 許容を変えても折れ曲がりは無い
//!
//! | 許容 | 0.01 | 0.02 | **0.04** | 0.08 | 0.16 | 0.32 |
//! | --- | --- | --- | --- | --- | --- | --- |
//! | 置き換え先が在った | 58.2% | 68.4% | **76.9%** | 85.2% | 91.1% | 91.9% |
//!
//! **測定からは閾値が出てこない** (滑らかで，0.32 では制約なしと同じ 91.9% に
//! 飽和する) ．だから**別の判断から引く** — `px aa` (D83) ・`px anim subpixel`
//! (D124) と同じ «2 色の間» の定義を使う．道具の中で «間» の意味が 2 つある方が
//! 害が大きい (D110) ．
//!
//! # 3 値の [`Depth`] で足りる
//!
//! $t$ を 50 分割して «パレットが表せる段の数» を色ごとに数えた (2259 色 x 空)．
//!
//! | | 1 段 (効かない) | 2 段 | 3 段 | 4 段以上 | 中央 |
//! | --- | --- | --- | --- | --- | --- |
//! | 件数 | 521 | 418 | 246 | 1074 | **3** |
//!
//! **中央が 3 である．** 23.1% の色はそもそも 1 段も無く，3 段以上を表せるのは
//! 58.4% である．つまり**段数を縛っているのはパレットであって型ではない** —
//! [`Depth`] を連続値へ変えても表せる絵は増えないので，**型も L0 のスキーマ版も
//! 変えない**．
//!
//! [^upc]: ULTIMATE PIXEL CREW REPORT PAGE:038．

use serde::{Deserialize, Serialize};

use crate::canvas::IndexedCanvas;
use crate::color::{Oklab, Rgba8, distance_sq, oklab_of};
use crate::error::{CoreError, Result};
use crate::frame::{Depth, Frame, Surface};
use crate::palette::Palette;

/// 多重スクロールメタの形式版．
pub const SCROLL_FORMAT_VERSION: u32 = 1;

fn de(x: Oklab, y: Oklab) -> f32 {
    distance_sq(x, y, 1.0).sqrt()
}

fn lerp(a: Oklab, b: Oklab, t: f32) -> Oklab {
    Oklab::new(
        a.l + (b.l - a.l) * t,
        a.a + (b.a - a.a) * t,
        a.b + (b.b - a.b) * t,
    )
}

/// **$c$ と空を結ぶ線の上にあり，狙いに最も近いパレットの色**．
///
/// `px aa` の [`crate::aa::nearest_between`] と同じ条件だが，あちらは両端とも
/// パレットの添字である．こちらは**終点の空がパレットに無くてよい** — 空は
/// 宣言であって，その絵が使っている色とは限らない．
///
/// `skip` に入っている添字は候補にしない (画布が «透明として扱う» と宣言した
/// 添字．**パレットのアルファとは別物である**．D109) ．
pub fn nearest_toward(
    palette: &Palette,
    from: u8,
    sky: Oklab,
    target: Oklab,
    tolerance: f32,
    skip: Option<u8>,
) -> Option<u8> {
    let x = palette.lab_of(from)?;
    let span = de(x, sky);
    if span <= f32::EPSILON {
        return None;
    }
    // **狙いが自分なら動かさない．** ここが無いと寄せ具合 0 でも «置き換え先が
    // 見つかった» と数えてしまう
    if de(x, target) <= f32::EPSILON {
        return None;
    }
    let mut best: Option<(f32, u8)> = None;
    for (i, lab) in palette.lab().iter().enumerate() {
        let i = i as u8;
        if i == from || skip == Some(i) || palette.get(i).is_some_and(|c| c.a == 0) {
            continue;
        }
        let (da, db) = (de(*lab, x), de(*lab, sky));
        // 線から外れた分が許容以内で，かつ **空へ近づいている**
        if da + db - span > tolerance || db >= span {
            continue;
        }
        let off = de(*lab, target);
        // 同点は小さい添字 (決定論性の規則 2)
        if best.as_ref().is_none_or(|(b0, _)| off < *b0) {
            best = Some((off, i));
        }
    }
    best.map(|(_, i)| i)
}

/// 奥行きごとの寄せ具合．
///
/// **既定はすべて 0** — つまり «何もしない»．厚みは絵に書かれていないので
/// 呼ぶ側が宣言する (D89 ・D126) ．手前を 0 にするのは既定ではなく**定義**で
/// ある — 書籍の «より近くのものは色味が濃くなるように描く» の基準がそこである．
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct HazeTable {
    pub foreground: f32,
    pub midground: f32,
    pub background: f32,
}

impl HazeTable {
    pub fn amount(self, depth: Depth) -> f32 {
        match depth {
            Depth::Foreground => self.foreground,
            Depth::Midground => self.midground,
            Depth::Background => self.background,
        }
    }

    /// **奥へ行くほど濃くなっているか．** 逆なら空気遠近法ではない．
    pub fn is_monotone(self) -> bool {
        self.foreground <= self.midground && self.midground <= self.background
    }

    /// 宣言が 1 つも無い (全部 0) か．
    pub fn is_empty(self) -> bool {
        self.foreground == 0.0 && self.midground == 0.0 && self.background == 0.0
    }
}

/// 空気遠近法の設定．
#[derive(Copy, Clone, Debug)]
pub struct AtmosOptions {
    /// **宣言された空の色**．絵からは決まらない (D89)．
    pub sky: Rgba8,
    pub haze: HazeTable,
    /// «線の上» と認める遠回りの許容．`px aa` (D83) と同じ値を引く．
    pub tolerance: f32,
}

impl AtmosOptions {
    /// «線の上» と認める遠回りの許容の既定．**`px aa` (D83) ・`px anim subpixel`
    /// (D124) と同じ «2 色の間» の定義を引いている** — 道具の中で «間» の意味が
    /// 2 つあると必ず食い違う (D110)．
    pub const DEFAULT_TOLERANCE: f32 = crate::subpixel::DEFAULT_TOLERANCE;
}

impl Default for AtmosOptions {
    fn default() -> Self {
        Self {
            sky: Rgba8::rgb(255, 255, 255),
            haze: HazeTable::default(),
            tolerance: Self::DEFAULT_TOLERANCE,
        }
    }
}

/// 掛けた結果の素性．
#[derive(Clone, Debug, Default)]
pub struct AtmosReport {
    pub amount: f32,
    /// 使っていた不透明な添字の数．
    pub colors: usize,
    /// 置き換えた色の数．
    pub moved: usize,
    /// **線の上に色が無くて動かさなかった色の数．作らない** (D124)．
    ///
    /// > [!warning] **«霞ませなかった» と混ぜない．**
    /// > 寄せ具合が 0 のときは狙いが自分自身なので «段が無い» のではなく
    /// > **«段を要求していない»** である．端から端まで通したら手前の絵が
    /// > «段が無くて動かさなかった 8» と出た — D104 ««測れない» の理由も
    /// > 分ける» と同じ誤りなので，寄せ具合 0 では数えない．
    pub no_step: usize,
    /// 置き換えで消えた色の数 — 相異なる添字が減った分．**明暗差が落ちるとは
    /// こういうことであり，失敗ではない**．黙らずに数える．
    pub collapsed: usize,
    /// 書き換えた画素の数．
    pub pixels: usize,
    /// 使っている色の明度の幅 (前, 後)．**空気遠近法は明暗差を落とす**．
    pub spread: (f32, f32),
}

impl AtmosReport {
    /// **1 色も動かなかったか．**
    ///
    /// パレットに霞の段が無ければこうなる — 道具の誤りではなく «このパレットに
    /// その霞は無い» という結果なので，**黙らずにそう言う** (D126 の
    /// `invisible()` と同じ役目)．
    pub fn ineffective(&self) -> bool {
        self.moved == 0
    }

    /// 明暗差が落ちた割合 (1.0 なら変わっていない)．
    pub fn spread_ratio(&self) -> f32 {
        if self.spread.0 <= f32::EPSILON {
            1.0
        } else {
            self.spread.1 / self.spread.0
        }
    }
}

/// 空気遠近法を掛ける．
///
/// **色は 1 つも作らない．** パレットは 1 項目も変えずに返す — 変えるのは
/// 画素が指す添字だけである (D94 の不変条件をこの道具にも掛ける)．
pub fn atmos(
    frames: &[Frame],
    depth: Depth,
    opts: &AtmosOptions,
) -> Result<(Vec<Frame>, AtmosReport)> {
    if frames.is_empty() {
        return Err(CoreError::AtmosNoFrames);
    }
    if !(0.0..=1.0).contains(&opts.haze.amount(depth)) {
        return Err(CoreError::AtmosAmountOutOfRange {
            amount: opts.haze.amount(depth),
        });
    }
    if !opts.haze.is_monotone() {
        return Err(CoreError::AtmosNotMonotone {
            foreground: opts.haze.foreground,
            midground: opts.haze.midground,
            background: opts.haze.background,
        });
    }

    let t = opts.haze.amount(depth);
    let palette = frames[0].palette.clone();
    let sky = oklab_of(opts.sky);

    // 画布が «透明として扱う» と宣言した添字は動かさない．**パレットのアルファ
    // とは別物である** (D109) ので，両方を見る
    let declared_transparent = frames
        .iter()
        .flat_map(|f| f.layers.iter())
        .filter_map(|l| l.surface.as_indexed().and_then(|c| c.transparent()))
        .next();

    let mut used = [0usize; 256];
    for f in frames {
        for l in &f.layers {
            let Some(c) = l.surface.as_indexed() else {
                continue;
            };
            for &i in c.pixels() {
                used[i as usize] += 1;
            }
        }
    }

    let movable = |i: u8| -> bool {
        used[i as usize] > 0
            && declared_transparent != Some(i)
            && palette.get(i).is_some_and(|c| c.a != 0)
    };

    let mut map: Vec<u8> = (0..=255u8).collect();
    let mut report = AtmosReport {
        amount: t,
        ..Default::default()
    };
    let (mut before, mut after) = (Vec::new(), Vec::new());

    for i in 0..=255u8 {
        if !movable(i) {
            continue;
        }
        report.colors += 1;
        let Some(lab) = palette.lab_of(i) else {
            continue;
        };
        before.push(lab.l);

        // **寄せ具合 0 は «段が無い» ではない．** 狙いが自分自身なので，
        // そもそも段を要求していない (D104 ««測れない» の理由も分ける»)
        if t <= f32::EPSILON {
            after.push(lab.l);
            continue;
        }

        let target = lerp(lab, sky, t);
        match nearest_toward(
            &palette,
            i,
            sky,
            target,
            opts.tolerance,
            declared_transparent,
        ) {
            Some(to) => {
                map[i as usize] = to;
                report.moved += 1;
                report.pixels += used[i as usize];
                after.push(palette.lab_of(to).unwrap_or(lab).l);
            }
            None => {
                report.no_step += 1;
                after.push(lab.l);
            }
        }
    }

    // 潰れた色 — 置き換えた先が重なった分
    let distinct: std::collections::BTreeSet<u8> = (0..=255u8)
        .filter(|i| movable(*i))
        .map(|i| map[i as usize])
        .collect();
    report.collapsed = report.colors.saturating_sub(distinct.len());

    let range = |v: &[f32]| -> f32 {
        if v.is_empty() {
            return 0.0;
        }
        let (lo, hi) = v
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), x| (lo.min(*x), hi.max(*x)));
        hi - lo
    };
    report.spread = (range(&before), range(&after));

    let mut out = Vec::with_capacity(frames.len());
    for f in frames {
        let mut next = f.clone();
        for l in &mut next.layers {
            if let Surface::Indexed(c) = &mut l.surface {
                remap_keeping_transparency(c, &map);
            }
        }
        out.push(next);
    }
    Ok((out, report))
}

/// 添字を貼り替える．**画布が宣言した透明添字は動かさない．**
fn remap_keeping_transparency(canvas: &mut IndexedCanvas, map: &[u8]) {
    let transparent = canvas.transparent();
    for p in canvas.pixels_mut() {
        if transparent == Some(*p) {
            continue;
        }
        *p = map[*p as usize];
    }
}

// ------------------------------------------------------- 多重スクロールメタ

/// 1 レイヤぶんの記録．
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScrollLayer {
    /// 出力したファイルの名前．
    pub file: String,
    pub depth: String,
    /// 掛けた寄せ具合．
    pub haze: f32,
    /// **宣言された**視差速度係数．無ければ書かない．
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<f32>,
}

/// 多重スクロールメタ (設計書 4.4 «`depth` からレイヤ速度係数を出力»)．
///
/// > [!warning] **速度係数は導出しない．**
/// > 視差の速さは «そのゲームでどれくらい奥に見せたいか» であって，絵からも
/// > `depth` の 3 値からも決まらない．物理から引くなら距離と消散係数が要るが，
/// > どちらも宣言でしかない．**宣言を記録するだけにする** — D92 «数え上げで
/// > 決まることは校正しない» ・D95 «仮定の少ない方を既定にする» と同じ側である．
/// > 宣言が無ければ `speed` を書かず，[`ScrollDoc::undeclared`] がその数を返す．
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScrollDoc {
    pub format: u32,
    /// 宣言された空の色 (`RRGGBB`)．
    pub sky: String,
    pub layers: Vec<ScrollLayer>,
}

impl ScrollDoc {
    pub fn new(sky: Rgba8, layers: Vec<ScrollLayer>) -> Self {
        Self {
            format: SCROLL_FORMAT_VERSION,
            sky: sky.to_hex_string(),
            layers,
        }
    }

    /// 速度係数が宣言されていないレイヤの数．
    pub fn undeclared(&self) -> usize {
        self.layers.iter().filter(|l| l.speed.is_none()).count()
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).map_err(|e| CoreError::ScrollWrite {
            message: e.to_string(),
        })
    }

    pub fn from_json(text: &str) -> Result<Self> {
        let doc: Self = serde_json::from_str(text).map_err(|e| CoreError::ScrollRead {
            message: e.to_string(),
        })?;
        if doc.format != SCROLL_FORMAT_VERSION {
            return Err(CoreError::ScrollVersion {
                found: doc.format,
                expected: SCROLL_FORMAT_VERSION,
            });
        }
        // **奥行きの綴りを黙って読み飛ばさない** — 知らない値が入っていたら
        // 使う側が勝手に既定へ倒すので，ここで落とす
        for l in &doc.layers {
            if Depth::parse(&l.depth).is_none() {
                return Err(CoreError::ScrollUnknownDepth {
                    depth: l.depth.clone(),
                });
            }
        }
        Ok(doc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::IndexedCanvas;
    use crate::frame::{Layer, LayerMeta};
    use crate::math::uvec2;

    fn palette(hexes: &[&str]) -> Palette {
        Palette::new(
            hexes
                .iter()
                .map(|h| Rgba8::from_hex_str(h).unwrap())
                .collect(),
        )
        .unwrap()
    }

    fn frame(palette: Palette, pixels: Vec<u8>, transparent: Option<u8>) -> Frame {
        let canvas = IndexedCanvas::from_pixels(pixels.len() as u32, 1, pixels)
            .unwrap()
            .with_transparent(transparent);
        let mut f = Frame::new(uvec2(canvas.width(), 1), palette);
        f.layers.push(Layer::new(
            LayerMeta::named("art"),
            Surface::Indexed(canvas),
        ));
        f
    }

    fn haze(t: f32) -> AtmosOptions {
        AtmosOptions {
            sky: Rgba8::from_hex_str("41a6f6").unwrap(),
            haze: HazeTable {
                foreground: 0.0,
                midground: t,
                background: t,
            },
            ..Default::default()
        }
    }

    /// **壊れると: 霞ませたつもりでパレットが増える (D94 の不変条件)．**
    #[test]
    fn atmos_never_creates_a_colour() {
        let p = palette(&["1a1c2c", "3b5dc9", "41a6f6", "b13e53"]);
        let f = frame(p.clone(), vec![0, 1, 2, 3], None);
        let (out, _) = atmos(&[f], Depth::Background, &haze(0.5)).unwrap();
        assert_eq!(out[0].palette.entries(), p.entries());
    }

    /// **壊れると: 線から外れた色 (赤) を «霞» として選ぶ．**
    #[test]
    fn a_colour_with_nothing_on_the_line_is_left_alone_and_counted() {
        // 暗い青と赤しか無い．赤は «暗い青 → 空» の線の上に無い
        let p = palette(&["1a1c2c", "b13e53"]);
        let f = frame(p, vec![0, 1], None);
        let (out, report) = atmos(&[f], Depth::Background, &haze(0.5)).unwrap();
        assert_eq!(report.moved, 0);
        assert_eq!(report.no_step, 2);
        assert!(report.ineffective(), "1 色も動かなければそう言う");
        assert_eq!(
            out[0].layers[0].surface.as_indexed().unwrap().pixels(),
            &[0, 1],
            "動かさないなら 1 画素も変わらない"
        );
    }

    /// **壊れると: 線の上にある色を見落として «効かない» と言う．**
    #[test]
    fn a_colour_on_the_line_moves_towards_the_sky() {
        let p = palette(&["1a1c2c", "3b5dc9", "41a6f6"]);
        let f = frame(p.clone(), vec![0], None);
        let (out, report) = atmos(&[f], Depth::Background, &haze(0.5)).unwrap();
        assert_eq!(report.moved, 1);
        let got = out[0].layers[0].surface.as_indexed().unwrap().pixels()[0];
        let sky = oklab_of(Rgba8::from_hex_str("41a6f6").unwrap());
        assert!(
            de(p.lab_of(got).unwrap(), sky) < de(p.lab_of(0).unwrap(), sky),
            "空へ近づいていない"
        );
    }

    /// **壊れると: 手前の絵まで霞む．**
    #[test]
    fn the_foreground_is_the_reference_and_does_not_move() {
        let p = palette(&["1a1c2c", "3b5dc9", "41a6f6"]);
        let f = frame(p, vec![0], None);
        let (_, report) = atmos(&[f], Depth::Foreground, &haze(0.5)).unwrap();
        assert_eq!(report.amount, 0.0);
        assert_eq!(report.moved, 0);
    }

    /// **壊れると: «霞ませなかった» が «パレットに段が無い» として報告される．**
    ///
    /// **端から端まで CLI で通して見つけた** — 手前の絵が «段が無くて動かさ
    /// なかった 8» と出た (D104 と同じ形)．
    #[test]
    fn asking_for_no_haze_is_not_the_same_as_the_palette_having_no_step() {
        let p = palette(&["1a1c2c", "3b5dc9", "41a6f6"]);
        let f = frame(p, vec![0, 1, 2], None);
        let (_, report) = atmos(&[f], Depth::Foreground, &haze(0.5)).unwrap();
        assert_eq!(report.amount, 0.0);
        assert_eq!(report.moved, 0);
        assert_eq!(
            report.no_step, 0,
            "寄せ具合 0 は «段を要求していない» のであって «段が無い» のではない"
        );
        assert_eq!(report.collapsed, 0);
    }

    /// **壊れると: 奥の方が濃い «逆の空気遠近法» を黙って書き出す．**
    #[test]
    fn a_table_that_gets_clearer_with_distance_is_rejected() {
        let p = palette(&["1a1c2c", "3b5dc9", "41a6f6"]);
        let f = frame(p, vec![0], None);
        let opts = AtmosOptions {
            sky: Rgba8::from_hex_str("41a6f6").unwrap(),
            haze: HazeTable {
                foreground: 0.6,
                midground: 0.3,
                background: 0.1,
            },
            ..Default::default()
        };
        assert!(atmos(&[f], Depth::Background, &opts).is_err());
    }

    /// **壊れると: 透明が霞んで背景が現れる．**
    ///
    /// 画布が宣言する透明添字は**パレットのアルファではない** (D109)．
    #[test]
    fn the_declared_transparent_index_is_never_hazed() {
        // 添字 0 は実色だが «透明として扱う» と宣言されている
        let p = palette(&["2b2b3f", "1a1c2c", "3b5dc9", "41a6f6"]);
        let f = frame(p, vec![0, 1], Some(0));
        let (out, _) = atmos(&[f], Depth::Background, &haze(0.5)).unwrap();
        assert_eq!(
            out[0].layers[0].surface.as_indexed().unwrap().pixels()[0],
            0,
            "透明と宣言された添字は動かさない"
        );
    }

    /// **壊れると: 明暗差が «増える» のに空気遠近法だと言い張る．**
    #[test]
    fn hazing_never_widens_the_lightness_spread() {
        let p = palette(&["1a1c2c", "3b5dc9", "41a6f6", "73eff7"]);
        let f = frame(p, vec![0, 1, 2, 3], None);
        let (_, report) = atmos(&[f], Depth::Background, &haze(0.6)).unwrap();
        assert!(
            report.spread.1 <= report.spread.0 + 1e-6,
            "明度の幅が広がっている: {:?}",
            report.spread
        );
    }

    /// **壊れると: 潰れた色を数えず «全部そのまま» に見える．**
    #[test]
    fn colours_that_land_on_the_same_index_are_counted_as_collapsed() {
        let p = palette(&["1a1c2c", "22243a", "3b5dc9", "41a6f6"]);
        let f = frame(p, vec![0, 1, 2, 3], None);
        let (_, report) = atmos(&[f], Depth::Background, &haze(1.0)).unwrap();
        assert!(
            report.collapsed > 0,
            "寄せ具合 1.0 なら複数の色が同じ添字へ落ちるはず: {report:?}"
        );
        assert!(report.collapsed < report.colors);
    }

    /// **壊れると: 版が違うメタを黙って読む (D110 と同じ形)．**
    #[test]
    fn a_scroll_document_from_another_version_is_an_error() {
        let doc = ScrollDoc::new(
            Rgba8::from_hex_str("41a6f6").unwrap(),
            vec![ScrollLayer {
                file: "bg.px.toml".into(),
                depth: "background".into(),
                haze: 0.6,
                speed: Some(0.25),
            }],
        );
        let text = doc
            .to_json()
            .unwrap()
            .replace("\"format\": 1", "\"format\": 2");
        assert!(ScrollDoc::from_json(&text).is_err());
    }

    /// **壊れると: 知らない奥行きの綴りが既定へ倒れる．**
    #[test]
    fn an_unknown_depth_in_the_meta_is_an_error() {
        let text =
            r#"{"format":1,"sky":"41a6f6","layers":[{"file":"a","depth":"far","haze":0.5}]}"#;
        assert!(ScrollDoc::from_json(text).is_err());
    }

    /// **壊れると: 宣言していない速度係数が «0» として書かれる．**
    #[test]
    fn an_undeclared_speed_is_absent_rather_than_zero() {
        let doc = ScrollDoc::new(
            Rgba8::from_hex_str("41a6f6").unwrap(),
            vec![ScrollLayer {
                file: "bg.px.toml".into(),
                depth: "background".into(),
                haze: 0.6,
                speed: None,
            }],
        );
        let text = doc.to_json().unwrap();
        assert!(!text.contains("speed"), "書いていないものを 0 で埋めない");
        assert_eq!(doc.undeclared(), 1);
        assert_eq!(ScrollDoc::from_json(&text).unwrap(), doc);
    }
}
