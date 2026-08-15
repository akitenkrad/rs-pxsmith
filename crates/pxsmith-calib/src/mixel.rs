//! **局所格子推定の窓サイズを «真値のある場面» で測る** (付録 C 要調査事項 #4)．
//!
//! # 何が開いていたか
//!
//! 設計書 6.1 は局所適用 (G4) で «窓ごとに $\hat{s}$ を求め，ばらつきが閾値を
//! 超えたら非一様と判定する» と定めるが，**窓の一辺は決めていない**．
//! 付録 C #4 が «ミクセル検出の分解能» と呼んでいるのはこの値のことである．
//! 実装は窓 32 ・一致率 0.8 を既定に置いたまま M2 を閉じている
//! (`grid-calibration.md` も «$w_L$ ・窓サイズはこの掃引に含めていない» と書く)．
//!
//! # 窓を使う口は 2 つあり，**入力の格子が違う**
//!
//! | 口 | 入力 | 窓が見るもの |
//! | --- | --- | --- |
//! | `pxsmith conform --window` | 拡大された絵 (格子 $s \geq 2$) | 全体が同じ $s$ か |
//! | `pxsmith lint` ルール 9 | **PNG** (`lint_grid` からしか呼ばれない) | 場所によって格子が違わないか |
//!
//! **ルール 9 は L0 (`.px.toml`) の経路には掛かっていない** — `lint_frame` は
//! `lint_grid` を通らないので，ルール 9 が見るのは `pxsmith lint <png>` だけである．
//! そして PNG で来るものには**等倍のドット絵がそのまま含まれる** (`testdata/` の
//! 実素材 64 枚がまさにそれで，16x16 と 32x32 である) ．
//!
//! D37 は «同一の推定器を共有する» と定めており，共有そのものは正しい．
//! **共有できないのは «どんな入力に掛かるか» である** — `conform` の入力は必ず
//! 拡大された絵だが，`lint` の入力は等倍の絵でありうる．
//!
//! # 真値のある場面
//!
//! 種 (`testdata/grid-eval/seeds/`) は**読み込み時に «整数倍に拡大された絵では
//! ない» ことを確かめてある** (`sprite::load_seeds`)．だから種を $s$ 倍に拡大して
//! 敷き詰めた画布は**格子がどこでも $s$** であり，2 つの倍率で敷き分けた画布は
//! **定義からミクセル**である．どちらも «作り方から分かる正解» を持つ．
//!
//! > [!warning] **背景と種は 2 つの領域で共通にする．**
//! > 領域ごとに違う色を敷くと «色が変わる場所» という別の手掛かりができ，
//! > 何を測ったのか分からなくなる．**違うのは画素の大きさだけ**にする．
//!
//! # 負例が欠陥になっているかを機械で確かめる (D163 の作法)
//!
//! ミクセルの場面には «少数派の領域に窓がいくつ載るか» という上限がある．
//! 少数派に載る窓が投票の 20% を割れば，一致率は 0.8 を下回れない —
//! **窓をどう選んでも捕まらない**．だから各場面について
//! [`Obs::minority_windows`] を数え，捕捉率の分母から外して報告する．

use std::collections::BTreeMap;

use pxsmith_core::grid::{GridParams, local_grid, uniformity};
use pxsmith_core::math::ivec2;
use pxsmith_core::{Rgba8, RgbaCanvas};
use rayon::prelude::*;

use crate::sprite::{Seed, flatten};

/// 敷く背景．**全場面で 1 色に固定する** — 領域の違いを画素の大きさだけにするため．
const BACKGROUND: Rgba8 = Rgba8::rgb(96, 92, 84);

/// 掃く窓の一辺．
pub const WINDOWS: [u32; 8] = [6, 8, 12, 16, 24, 32, 48, 64];

/// 拡大側 (`conform` の入力) で測る倍率．
pub const SCALES: [u32; 5] = [2, 3, 4, 6, 8];

/// 拡大側の画布の一辺．
const BIG: u32 = 192;

/// L0 側 (lint ルール 9) の画布の一辺の候補．**設計書 4.1 の助言上限 48 に合わせる**．
pub const L0_SIZES: [u32; 3] = [16, 32, 48];

/// 場面の正解．**作り方から決まる**．
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Truth {
    /// 画布のどこでも格子が同じ (`Some(s)`)．格子 1 の L0 は `None`．
    Uniform(Option<u32>),
    /// ミクセル — 少数派の領域が画布の何割か (千分率で持つ)．
    Mixel { minority_permille: u32 },
}

impl Truth {
    pub fn is_mixel(self) -> bool {
        matches!(self, Truth::Mixel { .. })
    }
}

/// 測る場面 1 枚．
pub struct Scene {
    pub name: String,
    pub group: &'static str,
    pub canvas: RgbaCanvas,
    pub truth: Truth,
    /// 少数派の領域 (ミクセルのときだけ) — `(x0, x1)`．
    pub minority_x: Option<(u32, u32)>,
}

/// 窓 1 通りぶんの観測．
#[derive(Clone, Copy, Debug)]
pub struct Obs {
    pub window: u32,
    /// 窓の総数 (`local_grid` が並べた升の数)．
    pub cells: usize,
    /// **推定できた窓** — 一致率の分母である．
    pub voting: usize,
    /// 少数派の領域に載った投票窓の数．**捕捉できる見込みの上限を決める**．
    pub minority_windows: usize,
    pub modal: Option<u32>,
    pub ratio: Option<f32>,
    /// **最頻の次に多い票**と，その数．
    ///
    /// 一様な絵で鳴ったときに «何と間違えたのか» を読むために持つ．
    /// 約数への票なら退化 (どの $s$ でもセル内が揃う窓) であって，
    /// **別の格子を見たわけではない**．
    pub dissent: Option<(u32, usize)>,
    /// ルール 9 ・`conform` の棄却が鳴るか．
    pub fires: bool,
}

impl Obs {
    /// **一致率が閾値を下回りうるか** — 少数派に窓が足りていれば真．
    ///
    /// 一致率は «最頻の窓 / 投票した窓» なので，少数派が投票の
    /// $1 - \theta$ を割ると**どんな窓の置き方でも 0.8 を下回れない**．
    /// 数え上げであって閾値の問題ではない．
    pub fn can_fire(&self, threshold: f32) -> bool {
        if self.voting == 0 {
            return false;
        }
        self.minority_windows as f32 / self.voting as f32 > 1.0 - threshold
    }
}

/// 場面 1 枚の全窓ぶん．
pub struct Case {
    pub name: String,
    pub group: &'static str,
    pub truth: Truth,
    pub obs: Vec<Obs>,
}

impl Case {
    /// 窓 1 通りぶんを引く (**不変条件を試験で押さえるため**)．
    #[cfg(test)]
    pub fn at(&self, window: u32) -> Option<&Obs> {
        self.obs.iter().find(|o| o.window == window)
    }
}

// --- 場面を作る ---

/// 最近傍で整数倍に拡大する．**色を作らない**ので格子が厳密に残る．
fn upscale(src: &RgbaCanvas, s: u32) -> RgbaCanvas {
    let mut out = RgbaCanvas::filled(src.width() * s, src.height() * s, BACKGROUND);
    for y in 0..src.height() as i32 {
        for x in 0..src.width() as i32 {
            let Some(c) = src.get(x, y) else { continue };
            for dy in 0..s as i32 {
                for dx in 0..s as i32 {
                    out.set(x * s as i32 + dx, y * s as i32 + dy, c);
                }
            }
        }
    }
    out
}

/// 領域 `[x0, x1)` を倍率 `scale` の絵で敷き詰める．
///
/// **タイルの原点は `x0`** にする．`x0` は倍率の倍数に取るので，セルが
/// 途中で切れることはない．種は順に回す — 1 枚を敷き詰めると画布が
/// タイルの周期でも周期的になり，**格子と紛らわしい周期を自分で作ってしまう**．
fn fill(dst: &mut RgbaCanvas, seeds: &[Seed], pick: &mut usize, scale: u32, x0: u32, x1: u32) {
    if x0 >= x1 {
        return;
    }
    let mut y = 0u32;
    while y < dst.height() {
        let mut x = x0;
        let mut row_h = 0u32;
        while x < x1 {
            let art = flatten(&seeds[*pick % seeds.len()].art, BACKGROUND);
            *pick += 1;
            let tile = upscale(&art, scale);
            row_h = row_h.max(tile.height());
            for ty in 0..tile.height() as i32 {
                for tx in 0..tile.width() as i32 {
                    let (px, py) = (x as i32 + tx, y as i32 + ty);
                    if (px as u32) >= x1 {
                        break;
                    }
                    if let Some(c) = tile.get(tx, ty) {
                        dst.set(px, py, c);
                    }
                }
            }
            x += tile.width();
        }
        y += row_h.max(1);
    }
}

/// 一様な場面 — 画布のどこでも格子 `scale`．
fn uniform_scene(seeds: &[Seed], start: usize, scale: u32, side: u32) -> Scene {
    let mut canvas = RgbaCanvas::filled(side, side, BACKGROUND);
    let mut pick = start;
    fill(&mut canvas, seeds, &mut pick, scale, 0, side);
    Scene {
        name: format!("uniform-s{scale}-{start:02}"),
        group: "一様 (拡大側)",
        canvas,
        truth: Truth::Uniform(Some(scale)),
        minority_x: None,
    }
}

/// ミクセルの場面 — 左を `major`，右の `permille` を `minor` で敷く．
///
/// 境目は**両方の倍率の倍数**に丸める．丸めないとセルが途中で切れ，
/// «ミクセルだから鳴った» のか «切れたセルで鳴った» のかが分からない．
fn mixel_scene(
    seeds: &[Seed],
    start: usize,
    major: u32,
    minor: u32,
    permille: u32,
    side: u32,
    group: &'static str,
) -> Option<Scene> {
    let step = lcm(major, minor);
    let raw = side * permille / 1000;
    let width = (raw / step) * step;
    if width == 0 || width >= side {
        return None;
    }
    let x0 = side - width;

    let mut canvas = RgbaCanvas::filled(side, side, BACKGROUND);
    let mut pick = start;
    fill(&mut canvas, seeds, &mut pick, major, 0, x0);
    fill(&mut canvas, seeds, &mut pick, minor, x0, side);
    Some(Scene {
        name: format!("mixel-{major}x{minor}-p{permille:03}-{start:02}"),
        group,
        canvas,
        truth: Truth::Mixel {
            minority_permille: width * 1000 / side,
        },
        minority_x: Some((x0, side)),
    })
}

/// L0 の場面 — 画布は等倍のドット絵．`patch` があるとそこだけ 2 倍で描かれる．
///
/// **これが書籍の言うミクセルである** — 等倍の絵の中に，2 倍の大きさで
/// 描かれた部分が混ざっている状態を指す．
fn l0_scene(seeds: &[Seed], start: usize, side: u32, patch: Option<(u32, u32)>) -> Scene {
    let mut canvas = RgbaCanvas::filled(side, side, BACKGROUND);
    let mut pick = start;
    fill(&mut canvas, seeds, &mut pick, 1, 0, side);
    match patch {
        None => Scene {
            name: format!("l0-{side}-clean-{start:02}"),
            group: "L0 の等倍 (鳴ってはいけない)",
            canvas,
            truth: Truth::Uniform(None),
            minority_x: None,
        },
        Some((scale, permille)) => {
            let step = scale;
            let width = ((side * permille / 1000) / step) * step;
            let x0 = side.saturating_sub(width);
            fill(&mut canvas, seeds, &mut pick, scale, x0, side);
            Scene {
                name: format!("l0-{side}-x{scale}-p{permille:03}-{start:02}"),
                group: "L0 にミクセル (鳴るべき)",
                canvas,
                truth: Truth::Mixel {
                    minority_permille: width * 1000 / side,
                },
                minority_x: Some((x0, side)),
            }
        }
    }
}

fn lcm(a: u32, b: u32) -> u32 {
    a / gcd(a, b) * b
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

// --- 測る ---

/// 1 枚に窓を全通り掛ける．
fn observe(scene: &Scene, params: &GridParams, threshold: f32) -> Case {
    let obs = WINDOWS
        .iter()
        .map(|&window| {
            let field = local_grid(&scene.canvas, window, params);
            let step = window.max(2);
            let mut minority_windows = 0usize;
            let mut voting = 0usize;
            let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
            for wy in 0..field.height() as i32 {
                for wx in 0..field.width() as i32 {
                    let Some(Some(vote)) = field.get(ivec2(wx, wy)) else {
                        continue;
                    };
                    voting += 1;
                    *counts.entry(*vote).or_default() += 1;
                    if let Some((x0, x1)) = scene.minority_x {
                        // **窓の中心がどちらの領域にあるか**で数える．またぐ窓は
                        // どちらの正解も持たないので，中心の側に寄せる
                        let cx = wx as u32 * step + step / 2;
                        if cx >= x0 && cx < x1 {
                            minority_windows += 1;
                        }
                    }
                }
            }
            let (modal, ratio) = match uniformity(&field) {
                Some((m, r)) => (Some(m), Some(r)),
                None => (None, None),
            };
            let dissent = counts
                .iter()
                .filter(|(s, _)| Some(**s) != modal)
                .max_by_key(|(s, c)| (**c, std::cmp::Reverse(**s)))
                .map(|(s, c)| (*s, *c));
            Obs {
                window,
                cells: field.data().len(),
                voting,
                minority_windows,
                modal,
                ratio,
                dissent,
                fires: ratio.is_some_and(|r| r < threshold),
            }
        })
        .collect();
    Case {
        name: scene.name.clone(),
        group: scene.group,
        truth: scene.truth,
        obs,
    }
}

pub struct Summary {
    pub cases: Vec<Case>,
    pub threshold: f32,
}

/// 場面をすべて組む．
pub(crate) fn scenes(seeds: &[Seed], sheets: usize) -> Vec<Scene> {
    let mut out = Vec::new();
    for k in 0..sheets {
        let start = k * 7;

        // 1. 一様 — 誤爆を測る
        for s in SCALES {
            out.push(uniform_scene(seeds, start, s, BIG));
        }

        // 2. ミクセル (半々) — 倍率の組を変えて捕捉を測る
        for (a, b) in [(2u32, 3u32), (2, 4), (3, 4), (2, 6), (4, 8), (3, 6)] {
            if let Some(s) = mixel_scene(seeds, start, a, b, 500, BIG, "ミクセル (半々)") {
                out.push(s);
            }
        }

        // 3. ミクセル (少数派の割合を掃く) — **分解能はここで決まる**
        for p in [50u32, 100, 150, 200, 250, 300, 400] {
            if let Some(s) = mixel_scene(seeds, start, 4, 2, p, BIG, "ミクセル (面積掃引)")
            {
                out.push(s);
            }
        }

        // 4. L0 の等倍 — lint ルール 9 が鳴ってはいけない側
        for side in L0_SIZES {
            out.push(l0_scene(seeds, start, side, None));
        }

        // 5. L0 にミクセル — 書籍の言うミクセル
        for side in L0_SIZES {
            for scale in [2u32, 3] {
                out.push(l0_scene(seeds, start, side, Some((scale, 400))));
            }
        }
    }
    out
}

pub fn run(seeds: &[Seed], sheets: usize, threshold: f32, params: &GridParams) -> Summary {
    let scenes = scenes(seeds, sheets);
    let cases = scenes
        .par_iter()
        .map(|s| observe(s, params, threshold))
        .collect();
    Summary { cases, threshold }
}

// --- まとめ ---

/// 群 x 窓ごとの成績．
#[derive(Clone, Copy, Debug, Default)]
pub struct Tally {
    pub sheets: usize,
    /// 鳴った枚数．
    pub fired: usize,
    /// 一様な場面で最頻が正解と一致した枚数．
    pub modal_ok: usize,
    /// 投票した窓が 1 つも無かった枚数．
    pub silent: usize,
    /// **窓が 1 つしか無かった枚数** — 一致率が定義から 1.0 になり，構造的に鳴らない．
    pub single_window: usize,
    /// 少数派に窓が足りていた枚数 (ミクセルの群だけ意味を持つ)．
    pub can_fire: usize,
    /// 少数派に窓が足りていて，かつ鳴った枚数．
    pub caught: usize,
}

pub fn tally(summary: &Summary) -> BTreeMap<(&'static str, u32), Tally> {
    let mut m: BTreeMap<(&'static str, u32), Tally> = BTreeMap::new();
    for case in &summary.cases {
        for o in &case.obs {
            let e = m.entry((case.group, o.window)).or_default();
            e.sheets += 1;
            if o.fires {
                e.fired += 1;
            }
            if o.voting == 0 {
                e.silent += 1;
            }
            if o.voting == 1 {
                e.single_window += 1;
            }
            if let Truth::Uniform(Some(s)) = case.truth
                && o.modal == Some(s)
            {
                e.modal_ok += 1;
            }
            if case.truth.is_mixel() && o.can_fire(summary.threshold) {
                e.can_fire += 1;
                if o.fires {
                    e.caught += 1;
                }
            }
        }
    }
    m
}

/// 面積掃引の群だけを «少数派の割合 x 窓» で並べる (**分解能の表**)．
pub fn by_fraction(summary: &Summary) -> BTreeMap<(u32, u32), Tally> {
    let mut m: BTreeMap<(u32, u32), Tally> = BTreeMap::new();
    for case in &summary.cases {
        let Truth::Mixel { minority_permille } = case.truth else {
            continue;
        };
        if case.group != "ミクセル (面積掃引)" {
            continue;
        }
        for o in &case.obs {
            let e = m.entry((minority_permille, o.window)).or_default();
            e.sheets += 1;
            if o.fires {
                e.fired += 1;
            }
            if o.can_fire(summary.threshold) {
                e.can_fire += 1;
                if o.fires {
                    e.caught += 1;
                }
            }
        }
    }
    m
}

/// 一様な場面だけを «倍率 x 窓» で並べる (**窓と $s$ の関係の表**)．
pub fn by_scale(summary: &Summary) -> BTreeMap<(u32, u32), Tally> {
    let mut m: BTreeMap<(u32, u32), Tally> = BTreeMap::new();
    for case in &summary.cases {
        let Truth::Uniform(Some(s)) = case.truth else {
            continue;
        };
        for o in &case.obs {
            let e = m.entry((s, o.window)).or_default();
            e.sheets += 1;
            if o.fires {
                e.fired += 1;
            }
            if o.modal == Some(s) {
                e.modal_ok += 1;
            }
            if o.voting == 0 {
                e.silent += 1;
            }
        }
    }
    m
}

/// **場面を PNG で書き出す** — 端から端まで CLI で通すため．
///
/// 測る口の中だけで測ると «道具が言うこと» と «CLI がすること» の差が見えない
/// (D138 ・D151 ・D161 と 4 度踏んでいる)．`pxsmith lint` ・`pxsmith conform` に
/// そのまま食わせられる形で数枚出す．
pub fn dump(seeds: &[Seed], dir: &std::path::Path) -> anyhow::Result<Vec<String>> {
    std::fs::create_dir_all(dir)?;
    let mut scenes = vec![
        uniform_scene(seeds, 0, 4, BIG),
        l0_scene(seeds, 0, 32, Some((2, 400))),
        l0_scene(seeds, 0, 32, None),
    ];
    scenes.extend(mixel_scene(seeds, 0, 4, 2, 500, BIG, "ミクセル (半々)"));
    scenes.extend(mixel_scene(seeds, 0, 4, 2, 100, BIG, "ミクセル (面積掃引)"));

    let mut names = Vec::new();
    for s in &scenes {
        let path = dir.join(format!("{}.png", s.name));
        pxsmith_io::png::write_rgba(&path, &s.canvas)?;
        names.push(path.display().to_string());
    }
    Ok(names)
}

// --- 窓と倍率の関係を刻んで測る ---

/// 倍率 $s$ に対し «最頻が正解になる最小の窓» ．
#[derive(Clone, Copy, Debug)]
pub struct Law {
    pub scale: u32,
    /// 位相ずれ検査の帯の数 (0 で検査を飛ばす)．**«4 の出どころ» を切り分けるため**．
    pub bands: usize,
    /// 全枚数で最頻が正解になった最小の窓．届かなければ `None`．
    pub min_window: Option<u32>,
}

/// **窓の下限が $s$ の何倍かを刻んで測る．**
///
/// [`WINDOWS`] は粗い等比の列なので «ちょうど $4s$» に見えても刻みの都合かも
/// しれない．$2s$ から $6s$ まで 1 画素ずつ掃いて下限を出す．
///
/// > [!note] **帯の数を変えて測るのは «4 の出どころ» を決めるためである．**
/// > 局所推定は帯ごとの位相を見る検査を持ち，その帯は既定で 4 本ある
/// > (`GridParams::phase_bands`)．下限が帯の数で動くならこれは推定器の
/// > 都合であって格子の性質ではない — **どちらなのかで «窓 ≥ 4s» を
/// > 手引きに書いてよいかが変わる**．
pub fn law(seeds: &[Seed], sheets: usize, params: &GridParams) -> Vec<Law> {
    let mut out = Vec::new();
    for bands in [params.phase_bands, 2, 0] {
        for scale in SCALES {
            let p = GridParams {
                phase_bands: bands,
                ..*params
            };
            let min_window = (2 * scale..=6 * scale).find(|&w| {
                (0..sheets).all(|k| {
                    let scene = uniform_scene(seeds, k * 7, scale, BIG);
                    let field = local_grid(&scene.canvas, w, &p);
                    uniformity(&field).map(|(m, _)| m) == Some(scale)
                })
            });
            out.push(Law {
                scale,
                bands,
                min_window,
            });
        }
    }
    out
}

// --- 実素材そのものに掛ける ---

/// 種 1 枚 (等倍のドット絵そのもの) に窓を掛けた結果．
#[derive(Clone, Copy, Debug, Default)]
pub struct CorpusTally {
    pub window: u32,
    pub sheets: usize,
    /// **投票した窓が 2 つ以上あった枚数** — これが 0 なら一致率は定義から 1.0 で，
    /// ルール 9 は**入力の側から鳴りようがない**．
    pub two_or_more: usize,
    /// 窓の升が 2 つ以上並んだ枚数 (投票したかは別)．
    pub cells_two_or_more: usize,
    pub fired: usize,
}

/// **実素材 (L0 と同じ等倍のドット絵) にそのまま掛ける．**
///
/// 合成した場面ではなく，`pxsmith lint` が実際に受け取る形で測る．
pub fn corpus(seeds: &[Seed], threshold: f32, params: &GridParams) -> Vec<CorpusTally> {
    WINDOWS
        .par_iter()
        .map(|&window| {
            let mut t = CorpusTally {
                window,
                ..Default::default()
            };
            for seed in seeds {
                let img = flatten(&seed.art, BACKGROUND);
                let field = local_grid(&img, window, params);
                let voting = field.data().iter().filter(|v| v.is_some()).count();
                t.sheets += 1;
                if voting >= 2 {
                    t.two_or_more += 1;
                }
                if field.data().len() >= 2 {
                    t.cells_two_or_more += 1;
                }
                if uniformity(&field).is_some_and(|(_, r)| r < threshold) {
                    t.fired += 1;
                }
            }
            t
        })
        .collect()
}

pub const HEADER: &str =
    "name,group,truth,window,cells,voting,minority_windows,modal,ratio,dissent,dissent_count,fires";

pub fn rows(summary: &Summary) -> Vec<String> {
    let mut out = Vec::new();
    for case in &summary.cases {
        let truth = match case.truth {
            Truth::Uniform(Some(s)) => format!("uniform{s}"),
            Truth::Uniform(None) => "uniform1".to_string(),
            Truth::Mixel { minority_permille } => format!("mixel{minority_permille}"),
        };
        for o in &case.obs {
            out.push(format!(
                "{},{},{},{},{},{},{},{},{},{},{},{}",
                case.name,
                case.group,
                truth,
                o.window,
                o.cells,
                o.voting,
                o.minority_windows,
                o.modal.map(|m| m.to_string()).unwrap_or_default(),
                o.ratio.map(|r| format!("{r:.4}")).unwrap_or_default(),
                o.dissent.map(|(s, _)| s.to_string()).unwrap_or_default(),
                o.dissent.map(|(_, c)| c.to_string()).unwrap_or_default(),
                o.fires as u8,
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeds() -> Vec<Seed> {
        crate::sprite::load_seeds(std::path::Path::new("../../testdata/grid-eval/seeds"))
            .expect("種を読める")
    }

    /// **壊れると: 場面が主張どおりの格子を持たず，測っても意味が無い．**
    ///
    /// 一様な場面は «$s$ 倍に拡大した絵» なので，$s$ 画素の升の中は 1 色である．
    #[test]
    fn a_uniform_scene_really_has_cells_of_its_scale() {
        let s = uniform_scene(&seeds(), 0, 4, 64);
        for cy in 0..16 {
            for cx in 0..16 {
                let base = s.canvas.get(cx * 4, cy * 4).expect("画布の中");
                for dy in 0..4 {
                    for dx in 0..4 {
                        assert_eq!(
                            s.canvas.get(cx * 4 + dx, cy * 4 + dy),
                            Some(base),
                            "升 ({cx}, {cy}) の中が 1 色でない"
                        );
                    }
                }
            }
        }
    }

    /// **壊れると: «ミクセル» の場面が実は一様で，捕捉率が測れない．**
    ///
    /// 少数派の領域の升は少数派の倍率でなければならない．
    #[test]
    fn a_mixel_scene_really_has_two_block_sizes() {
        let s = mixel_scene(&seeds(), 0, 4, 2, 500, 96, "t").expect("場面を作れる");
        let (x0, _) = s.minority_x.expect("少数派の領域がある");
        // 左 (4 倍側) は 4 画素の升が揃う
        let left = s.canvas.get(0, 0).expect("画布の中");
        for dx in 0..4 {
            assert_eq!(s.canvas.get(dx, 0), Some(left), "多数派の升が 4 画素でない");
        }
        // 少数派の領域が実際に置かれている
        assert!(x0 > 0 && x0 < 96, "少数派の領域が画布の外にある: {x0}");
        assert_eq!(x0 % 4, 0, "境目が両方の倍率の倍数になっていない");
    }

    /// **壊れると: «少数派が薄いから捕まらない» を «窓が悪いから» と読む．**
    ///
    /// 一致率は «最頻の窓 / 投票した窓» なので，少数派が投票の $1 - \theta$ を
    /// 割ると**どの窓でも 0.8 を下回れない**．閾値ではなく数え上げである．
    #[test]
    fn a_minority_smaller_than_the_threshold_gap_can_never_fire() {
        let o = Obs {
            window: 16,
            cells: 100,
            voting: 100,
            minority_windows: 15,
            modal: Some(4),
            ratio: Some(0.85),
            dissent: Some((2, 15)),
            fires: false,
        };
        assert!(!o.can_fire(0.8), "15 / 100 で鳴りうると言っている");
        let o = Obs {
            minority_windows: 25,
            ..o
        };
        assert!(o.can_fire(0.8), "25 / 100 で鳴りえないと言っている");
    }

    /// **これが付録 C #4 の答えの片方である — 等倍の領域は票を立てない．**
    ///
    /// 書籍の言うミクセル (等倍の絵に 2 倍で描いた部分が混ざる) では，
    /// **投票するのは 2 倍の側だけ**なので一致率は必ず 1.0 になる．
    /// 窓を大きくしても小さくしても変わらない — 候補に 1 が無いからである．
    ///
    /// **壊れると: «鳴らないのは窓が悪いから» と読んで窓を掃引しはじめる．**
    #[test]
    fn a_one_times_region_casts_no_vote_so_the_ratio_stays_one() {
        let seeds = seeds();
        let scene = l0_scene(&seeds, 0, 128, Some((2, 400)));
        for window in [8u32, 12, 16, 24, 32] {
            let field = local_grid(&scene.canvas, window, &GridParams::default());
            let votes: Vec<u32> = field.data().iter().filter_map(|v| *v).collect();
            assert!(
                votes.iter().all(|&v| v == 2),
                "窓 {window}: 2 以外の票がある {votes:?} — 等倍側が投票しはじめた"
            );
            if let Some((_, ratio)) = uniformity(&field) {
                assert_eq!(
                    ratio, 1.0,
                    "窓 {window}: 一致率が 1.0 でない — この測定の前提が変わった"
                );
            }
        }
    }

    /// **これが付録 C #4 の答えのもう片方である — 窓の一辺にセルが 4 つ要る．**
    ///
    /// **壊れると: 窓を倍率の 4 倍未満に取り，«一様だと確かめた» が空約束になる．**
    #[test]
    fn the_window_floor_is_four_cells_per_side() {
        let seeds = seeds();
        let laws = law(&seeds, 1, &GridParams::default());
        for scale in [2u32, 3, 4] {
            let l = laws
                .iter()
                .find(|l| l.scale == scale && l.bands == GridParams::default().phase_bands)
                .expect("掃引している");
            assert_eq!(
                l.min_window,
                Some(scale * pxsmith_core::grid::MIN_CELLS_PER_WINDOW),
                "倍率 {scale} の窓の下限が 4 セルぶんでない"
            );
        }
    }

    /// **壊れると: 窓が 1 つしか無い画布で «一様だと確かめた» と読む．**
    ///
    /// 窓が 1 つなら一致率は定義から 1.0 になる — 検査したのではなく，
    /// **検査できる形になっていない**．L0 の画布 (16 〜 48 画素) で
    /// 既定の窓 32 を使うとこれが起きる．
    #[test]
    fn one_window_makes_the_agreement_ratio_meaningless() {
        let s = l0_scene(&seeds(), 0, 32, Some((2, 400)));
        let case = observe(&s, &GridParams::default(), 0.8);
        let o = case.at(32).expect("窓 32 を測っている");
        assert!(
            o.cells <= 1,
            "32 画素の画布に窓 32 が {} 升も並んでいる",
            o.cells
        );
        assert!(!o.fires, "窓が 1 つしか無いのに鳴っている");
    }
}
