//! **ジャギー検出に «真値のある場面» を作って掛ける** (付録 C 要調査事項 #1)．
//!
//! # なぜ実素材だけでは閉じないか
//!
//! `pxsmith-calib jaggy` は CC0 の実物 61 枚で **24714 本中 146 件 (0.59%)** と測って
//! いるが，**あの測定には正解ラベルが無い**．付録 C #1 が問うているのは
//! «手描きの**意図的な**凹凸を違反と判定しないか» であって，件数ではない．
//! 実素材では «この 1 件のくぼみは作者が描きたかったのか，滑らせ損ねたのか» が
//! **誰にも分からない**ので，何件鳴っても誤検出率にはならない．
//! (`by_shape` の分類は代理でしかない — 形が単発だから意図的でない，とは言えない．)
//!
//! # そこで «全部が意図的» な絵を作る
//!
//! **円板と有理数の傾きの階段は，縁の凹凸が 1 つ残らず幾何で決まる**．
//! だから**清書に出た検出は，全件が定義から誤検出**である．
//!
//! これは設計書 6.4 が名指しで心配している場面でもある — 6.4 は
//! «理想的なラン長列は単谷形で，**谷そのものは理想形にも必ず存在する**» と書く．
//!
//! # 対照: 段を 1 画素だけ早く落とす
//!
//! 誤検出率だけでは «何も鳴らさない道具» が満点を取る (D114 で «動かさない» を
//! 対照に置いたのと同じ) ．負例は**設計書 6.4 が挙げる形そのもの** —
//! 段の境目を 1 つ手前へ動かして `[3, 3, 3]` を `[3, 2, 4]` にする．
//! 段が早く落ちて次で取り返す，これが «揃っていない階段» である．
//!
//! > [!warning] **負例が欠陥になっているかを機械で確かめる (この計画で 4 度目の罠)．**
//! > 最初は «縁の外へ 1 画素出っ張らせる» で負例を作ったが，**あれは原理的に
//! > 検出できない** — 出っ張りは向きの反転を作るので `split_monotone` が
//! > そこで区間を切り，`[1,3,3,3,…]` が `[1,3,3,2]` と `[3,3,3,…]` に割れて
//! > **どちらにも谷が残らない**．谷を数える規則に «谷を作らない負例» を
//! > 掛けて «捕捉 8%» と読むところだった．
//! > だから各負例について**実際に谷ができたか**を数え，
//! > [`Summary::defects_with_valley`] として必ず併記する．

use std::collections::BTreeMap;

use pxsmith_core::canvas::IndexedCanvas;
use pxsmith_core::geom::jaggy::analyze_canvas;
use pxsmith_core::geom::runs::{run_lengths, run_valleys};
use pxsmith_core::geom::{split_monotone, trace_contours};
use pxsmith_core::math::ivec2;
use rayon::prelude::*;

/// 背景 (透明) の添字．
pub(crate) const BG: u8 = 0;
/// 図形の添字．
pub(crate) const FG: u8 = 1;

/// 清書 1 枚の測定．
#[derive(Clone, Debug)]
pub struct Case {
    pub name: String,
    pub kind: &'static str,
    pub runs: usize,
    /// 清書に出た検出 — **全件が定義から誤検出**である．
    pub jaggies: usize,
    /// **`pxsmith smooth` が実際に動かした画素**．
    ///
    /// 検出が鳴るだけなら助言で済むが，`smooth` は画素を動かす —
    /// **正しく描いた線が書き換えられるかどうか**がここで決まる．
    pub moved: usize,
}

/// 負例 1 件の結果．
#[derive(Clone, Debug)]
pub struct Defect {
    pub name: String,
    /// **そもそも谷ができたか** — できていない負例で捕捉率を語らない．
    pub has_valley: bool,
    /// 崩した場所で鳴ったか．
    pub caught: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub clean: Vec<Case>,
    pub defects: Vec<Defect>,
    /// 出っ張り (原理的に谷にならない形) を別に数える．
    pub bumps: Vec<Defect>,
    /// **清書に出た谷の深さ** — 全件が偽である．
    pub clean_depths: BTreeMap<i32, usize>,
    /// **崩した場所で捕まえた谷の深さ** — 全件が真である．
    pub defect_depths: BTreeMap<i32, usize>,
    /// 清書に出た谷の «両隣の長さの差» — 全件が偽．
    pub clean_neighbour_gap: BTreeMap<u32, usize>,
    /// 崩した場所で捕まえた谷の «両隣の長さの差» — 全件が真．
    pub defect_neighbour_gap: BTreeMap<u32, usize>,
}

impl Summary {
    pub fn clean_runs(&self) -> usize {
        self.clean.iter().map(|c| c.runs).sum()
    }
    pub fn clean_jaggies(&self) -> usize {
        self.clean.iter().map(|c| c.jaggies).sum()
    }
    pub fn clean_sheets_firing(&self) -> usize {
        self.clean.iter().filter(|c| c.jaggies > 0).count()
    }
    /// **`pxsmith smooth` が清書を書き換えた画素の合計** — 0 でなければ良い絵を壊している．
    pub fn clean_moved(&self) -> usize {
        self.clean.iter().map(|c| c.moved).sum()
    }
    pub fn clean_sheets_damaged(&self) -> usize {
        self.clean.iter().filter(|c| c.moved > 0).count()
    }
    /// **負例のうち実際に谷ができたもの** — 捕捉率の正しい分母．
    pub fn defects_with_valley(&self) -> usize {
        self.defects.iter().filter(|d| d.has_valley).count()
    }
    pub fn caught(&self) -> usize {
        self.defects.iter().filter(|d| d.caught).count()
    }
    pub fn bumps_with_valley(&self) -> usize {
        self.bumps.iter().filter(|d| d.has_valley).count()
    }
    pub fn bumps_caught(&self) -> usize {
        self.bumps.iter().filter(|d| d.caught).count()
    }
}

/// 列ごとの «上端の行» から画布を作る (その行から下を塗る)．
///
/// **階段を高さの列で持つと «段をずらす» が 1 行で書ける** — 画素を足し引き
/// する形だと，狙った欠陥になっているかが読めない．
pub(crate) fn from_heights(heights: &[u32], height_px: u32) -> IndexedCanvas {
    let w = heights.len() as u32;
    let mut c = IndexedCanvas::from_pixels(w, height_px, vec![BG; (w * height_px) as usize])
        .expect("画布を作れる")
        .with_transparent(Some(BG));
    for (x, &top) in heights.iter().enumerate() {
        for y in top..height_px {
            c.set_at(ivec2(x as i32, y as i32), FG);
        }
    }
    c
}

/// 理想の階段 — 傾き $a / b$．**走りの長さは傾きが決める**．
pub(crate) fn staircase_heights(a: u32, b: u32, w: u32) -> Vec<u32> {
    (0..w).map(|x| (a * x) / b).collect()
}

/// 階段が**画布の外へはみ出さない**高さ．
///
/// はみ出すと «塗る画素が 1 つも無い列» ができ，そこに画布の縁が縦の辺として
/// 現れる — **測っているのが階段ではなく «切り取られた形» になる**．
pub(crate) fn fitting_height(heights: &[u32]) -> u32 {
    heights.iter().copied().max().unwrap_or(0) + 6
}

/// **段の境目を 1 つ手前へ動かす** (設計書 6.4 が挙げる «揃っていない階段»)．
///
/// 段が上がる列 `x` を見つけ，その 1 つ手前の列を先に上げる．
/// 走りは «1 短い» と «1 長い» の対になり，**谷ができる**．
pub(crate) fn shift_one_step(heights: &[u32], which: usize) -> Option<(Vec<u32>, usize)> {
    let pool: Vec<usize> = (3..heights.len().saturating_sub(3))
        .filter(|&x| heights[x] > heights[x - 1] && heights[x - 1] == heights[x - 2])
        .collect();
    if pool.len() < 2 {
        return None;
    }
    let x = pool[(pool.len() / 3 + which) % pool.len()];
    let mut h = heights.to_vec();
    // 1 つ手前の列を «先に» 上げる = 段が 1 画素早く落ちる
    h[x - 1] = h[x];
    Some((h, x - 1))
}

/// **縁の外へ 1 画素出っ張らせる** — 比較用．谷にはならないはずの形である．
fn bump_one(heights: &[u32], which: usize) -> Option<(Vec<u32>, usize)> {
    let n = heights.len();
    if n < 8 {
        return None;
    }
    let x = n / 4 + which % (n / 2).max(1);
    if x >= n || heights[x] == 0 {
        return None;
    }
    let mut h = heights.to_vec();
    h[x] -= 1;
    Some((h, x))
}

/// **ラスタライズした円板** — 縁の凹凸はすべて幾何が決める．
pub(crate) fn disk(radius: u32) -> IndexedCanvas {
    let pad = 2;
    let size = radius * 2 + 1 + pad * 2;
    let mut c = IndexedCanvas::from_pixels(size, size, vec![BG; (size * size) as usize])
        .expect("画布を作れる")
        .with_transparent(Some(BG));
    let center = (radius + pad) as i32;
    let r2 = (radius * radius) as i32;
    for y in 0..size as i32 {
        for x in 0..size as i32 {
            let (dx, dy) = (x - center, y - center);
            if dx * dx + dy * dy <= r2 {
                c.set_at(ivec2(x, y), FG);
            }
        }
    }
    c
}

/// 円板の «上端» を高さの列にする (段をずらす操作を掛けるため)．
pub(crate) fn disk_heights(radius: u32) -> Vec<u32> {
    let pad = 2;
    let size = radius * 2 + 1 + pad * 2;
    let center = (radius + pad) as f64;
    let r2 = (radius * radius) as f64;
    (0..size)
        .map(|x| {
            let dx = x as f64 - center;
            let inside = r2 - dx * dx;
            if inside < 0.0 {
                size
            } else {
                (center - inside.sqrt()).ceil().max(0.0) as u32
            }
        })
        .collect()
}

/// 高さの列を 1 か所だけ崩す関数 (段のずらし / 出っ張り)．
type Breaker = fn(&[u32], usize) -> Option<(Vec<u32>, usize)>;

/// **谷ができたか** — 負例が欠陥になっているかの機械的な確認．
fn has_valley(canvas: &IndexedCanvas) -> bool {
    let mask = canvas.mask_of(FG);
    trace_contours(&mask).iter().any(|contour| {
        split_monotone(contour).iter().any(|chain| {
            let r = run_lengths(chain);
            r.len() >= 3 && !run_valleys(&r).is_empty()
        })
    })
}

/// 検出が列 `x` の近くを動かすか (**崩した場所で鳴ったか**)．
fn caught_near(canvas: &IndexedCanvas, x: usize, max_move: u32) -> bool {
    analyze_canvas(canvas, max_move)
        .jaggies
        .iter()
        .any(|j| j.pixels.iter().any(|q| (q.x - x as i32).abs() <= 2))
}

/// 谷の «深さ» と «両隣の差» を数える．
///
/// **偽 (清書) と真 (崩した場所) で形が分かれるかを見るため**に取る —
/// 分かれるなら `smooth` が動かない条件をそこから書ける (D168)．
///
/// 両隣の差は「目標 (両隣の小さい方) − 谷の長さ」ではなく，
/// **両隣どうしの差** ($|r_{i-1} - r_{i+1}|$) である．理想の digitization では
/// 隣り合う走りの長さは 1 しか違わないので，ここが開いていれば «並びが崩れている»．
fn depth_of(canvas: &IndexedCanvas, max_move: u32, near: Option<usize>) -> Vec<(i32, u32)> {
    let report = analyze_canvas(canvas, max_move);
    let mut out = Vec::new();
    for chain in trace_contours(&canvas.mask_of(FG)) {
        for part in split_monotone(&chain) {
            let r = run_lengths(&part);
            for i in run_valleys(&r) {
                if i == 0 || i + 1 >= r.len() {
                    continue;
                }
                let gap = r[i - 1].abs_diff(r[i + 1]);
                let target = r[i - 1].min(r[i + 1]) as i32;
                out.push((target - r[i] as i32, gap));
            }
        }
    }
    // 崩した場所を指定されているときは，そこで鳴った件だけに絞る
    if let Some(x) = near {
        let hit = report
            .jaggies
            .iter()
            .any(|j| j.pixels.iter().any(|q| (q.x - x as i32).abs() <= 2));
        if !hit {
            return Vec::new();
        }
    }
    out
}

const SLOPES: [(u32, u32); 20] = [
    // 走りが 1 種類だけになる傾き (単位分数)
    (1, 1),
    (1, 2),
    (1, 3),
    (1, 4),
    (1, 5),
    // 走りが 2 種類になる傾き．**短い走りが孤立するかどうかで分かれる**
    (2, 3),
    (2, 5),
    (2, 7),
    (3, 4),
    (3, 5),
    (3, 7),
    (3, 8),
    (4, 5),
    (4, 7),
    (4, 9),
    (4, 11),
    (5, 6),
    (5, 8),
    (5, 9),
    (5, 14),
];

/// **谷ができる傾きかどうかを式で言う** (測定ではなく数え上げから出る)．
///
/// 傾き $a / b$ の階段は，1 周期 ($x$ が $b$ 進む間) に走りが $a$ 本並ぶ．
/// 長さは $\lceil b/a \rceil$ (長い) と $\lfloor b/a \rfloor$ (短い) の 2 種類で，
/// **長い走りが $b \bmod a$ 本，短い走りが $a - (b \bmod a)$ 本**である．
///
/// 谷とは «両隣より短い走り» なので，**短い走りが 2 本続くと谷にならない**
/// (隣が同じ長さになる) ．走りの並びは可能なかぎり均等に散るので，
/// **短い走りが長い走りより多いときだけ隣り合う**．したがって
///
/// $$\text{谷ができる} \iff a - (b \bmod a) \leq b \bmod a \iff 2\,(b \bmod a) \geq a$$
///
/// $a = 1$ なら $b \bmod a = 0$ で式は偽 — 走りが 1 種類しかないので正しい．
///
/// > [!note] **最初 $b \bmod a = a - 1$ (短い走りが 1 本だけ) と書いて外した．**
/// > 5 / 8 は短い走りが 2 本あるのに谷ができる — **1 本である必要は無く，
/// > 隣り合わなければよい**．数え上げの条件を 1 段強く書きすぎていた．
pub fn predicts_valley(a: u32, b: u32) -> bool {
    2 * (b % a) >= a
}

/// 清書の群．
pub(crate) fn clean_scenes() -> Vec<(String, &'static str, IndexedCanvas)> {
    let mut out = Vec::new();
    for r in 3u32..=40 {
        out.push((format!("disk-r{r:02}"), "円板", disk(r)));
    }
    for (a, b) in SLOPES {
        let h = staircase_heights(a, b, 64);
        let n = fitting_height(&h);
        // **式が «鳴る» と言う傾きと言わない傾きを分けて数える** — 混ぜると
        // «直線でも 12% 鳴る» という 1 つの数字になり，原因が消える
        let kind = if predicts_valley(a, b) {
            "階段 (短い走りが孤立)"
        } else {
            "階段 (短い走りが隣接)"
        };
        out.push((format!("stair-{a}_{b}"), kind, from_heights(&h, n)));
    }
    out
}

/// 負例の元になる高さの列 (階段と円板の上端)．
pub(crate) fn defect_bases() -> Vec<(String, Vec<u32>, u32)> {
    let mut out = Vec::new();
    for (a, b) in SLOPES {
        let h = staircase_heights(a, b, 64);
        let n = fitting_height(&h);
        out.push((format!("stair-{a}_{b}"), h, n));
    }
    for r in [8u32, 12, 16, 20, 24, 28, 32, 36, 40] {
        out.push((format!("diskcap-r{r:02}"), disk_heights(r), r * 2 + 5));
    }
    out
}

pub fn run(max_move: u32) -> Summary {
    let clean: Vec<Case> = clean_scenes()
        .par_iter()
        .map(|(name, kind, canvas)| {
            let report = analyze_canvas(canvas, max_move);
            // **鳴るだけか，書き換えるかを分ける** — 助言と破壊は別の話である
            let mut smoothed = canvas.clone();
            let opts = pxsmith_core::smooth::SmoothOptions {
                max_move,
                ..pxsmith_core::smooth::SmoothOptions::default()
            };
            let moved = pxsmith_core::smooth::smooth_canvas(&mut smoothed, &opts).moved;
            Case {
                name: name.clone(),
                kind,
                runs: report.runs,
                jaggies: report.jaggies.len(),
                moved,
            }
        })
        .collect();

    let bases = defect_bases();
    let make = |f: Breaker| -> Vec<Defect> {
        bases
            .par_iter()
            .flat_map(|(name, heights, h)| {
                (0..3)
                    .filter_map(|k| {
                        let (broken, x) = f(heights, k * 5)?;
                        let canvas = from_heights(&broken, *h);
                        Some(Defect {
                            name: format!("{name}#{k}"),
                            has_valley: has_valley(&canvas),
                            caught: caught_near(&canvas, x, max_move),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    };

    // **偽と真で谷の形が分かれるかを測る** (D168)
    let mut clean_depths: BTreeMap<i32, usize> = BTreeMap::new();
    let mut clean_neighbour_gap: BTreeMap<u32, usize> = BTreeMap::new();
    for (_, _, canvas) in clean_scenes() {
        for (d, gap) in depth_of(&canvas, max_move, None) {
            *clean_depths.entry(d).or_default() += 1;
            *clean_neighbour_gap.entry(gap).or_default() += 1;
        }
    }
    let mut defect_depths: BTreeMap<i32, usize> = BTreeMap::new();
    let mut defect_neighbour_gap: BTreeMap<u32, usize> = BTreeMap::new();
    for (name, heights, h) in defect_bases() {
        let _ = &name;
        for k in 0..3 {
            let Some((broken, x)) = shift_one_step(&heights, k * 5) else {
                continue;
            };
            let canvas = from_heights(&broken, h);
            for (d, gap) in depth_of(&canvas, max_move, Some(x)) {
                *defect_depths.entry(d).or_default() += 1;
                *defect_neighbour_gap.entry(gap).or_default() += 1;
            }
        }
    }

    Summary {
        clean,
        defects: make(shift_one_step),
        bumps: make(bump_one),
        clean_depths,
        defect_depths,
        clean_neighbour_gap,
        defect_neighbour_gap,
    }
}

/// 清書を種類ごとにまとめる．
pub fn by_kind(summary: &Summary) -> BTreeMap<&'static str, (usize, usize, usize)> {
    let mut m: BTreeMap<&'static str, (usize, usize, usize)> = BTreeMap::new();
    for c in &summary.clean {
        let e = m.entry(c.kind).or_default();
        e.0 += 1;
        e.1 += c.runs;
        e.2 += c.jaggies;
    }
    m
}

pub const HEADER: &str = "name,kind,runs,jaggies";

#[cfg(test)]
mod tests {
    use super::*;

    /// **壊れると: 場面が図形になっておらず，測っても意味が無い．**
    #[test]
    fn the_scenes_are_the_shapes_they_claim_to_be() {
        let d = disk(10);
        assert!(
            !d.is_transparent_at(ivec2(12, 12)),
            "円板の中心が空いている"
        );
        assert!(d.is_transparent_at(ivec2(1, 1)), "円板の角が塗られている");

        let h = staircase_heights(1, 3, 32);
        let s = from_heights(&h, fitting_height(&h));
        assert!(!s.is_transparent_at(ivec2(0, 0)), "階段の左上が空いている");
        assert!(
            s.is_transparent_at(ivec2(31, 0)),
            "階段の右上が塗られている"
        );
    }

    /// **単位分数の傾き (1/b) は走りが揃うので谷ができない．**
    ///
    /// 誤検出率を測るときの «下限» にあたる場面である．
    #[test]
    fn a_unit_fraction_staircase_has_no_valley() {
        for (a, b) in SLOPES.iter().copied().filter(|(a, _)| *a == 1) {
            let h = staircase_heights(a, b, 64);
            let c = from_heights(&h, fitting_height(&h));
            assert!(!has_valley(&c), "理想の階段 {a}/{b} に谷がある");
        }
    }

    /// **これが付録 C #1 の答えである — どの傾きが誤検出になるかは式で決まる．**
    ///
    /// 正しく描いた階段のうち $b \equiv -1 \pmod a$ のものは，**短い走りが
    /// 1 周期に 1 本だけ**なので必ず両隣より短くなり，**定義上すべて谷**に
    /// なる．幾何が決めた刻みであってジャギーではない．
    ///
    /// **閾値ではなく数え上げなので，掃引しても動かない** (D92 と同じ側)．
    ///
    /// **壊れると: 誤検出の範囲が変わったのに «前からこうだった» と読む．**
    #[test]
    fn which_intentional_slopes_get_flagged_follows_a_formula() {
        for (a, b) in SLOPES {
            let h = staircase_heights(a, b, 64);
            let c = from_heights(&h, fitting_height(&h));
            assert_eq!(
                has_valley(&c),
                predicts_valley(a, b),
                "傾き {a}/{b}: 式は {} と言うが実際は {}",
                predicts_valley(a, b),
                has_valley(&c)
            );
        }
    }

    /// **壊れると: 谷を作らない負例で捕捉率を測ってしまう** (この計画で 4 度目の罠)．
    #[test]
    fn shifting_one_step_actually_creates_a_valley() {
        let mut made = 0;
        for (a, b) in SLOPES {
            let h = staircase_heights(a, b, 64);
            if let Some((broken, _)) = shift_one_step(&h, 0) {
                assert_ne!(broken, h, "崩れていない");
                if has_valley(&from_heights(&broken, fitting_height(&h))) {
                    made += 1;
                }
            }
        }
        assert!(made >= 6, "谷ができた負例が {made} 件しかない");
    }

    /// **壊れると: «出っ張りは谷にならない» という測定結果を取り違える．**
    ///
    /// 出っ張りは向きの反転を作るので `split_monotone` が区間を切り，
    /// 谷が残らない — **原理的に検出できない形**である．
    #[test]
    fn an_outward_bump_does_not_create_a_valley() {
        let h = staircase_heights(1, 3, 64);
        let (bumped, _) = bump_one(&h, 0).expect("出っ張らせられる");
        assert!(
            !has_valley(&from_heights(&bumped, fitting_height(&h))),
            "出っ張りが谷を作っている — この測定の前提が変わった"
        );
    }
}
