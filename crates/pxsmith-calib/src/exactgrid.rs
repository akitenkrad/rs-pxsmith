//! **ルール 9 の道具を «厳密なブロック判定» に替えたときの上限を測る** (D164 の続き)．
//!
//! # なぜ実装より先に測るか
//!
//! D164 は «ルール 9 は書籍の言うミクセルを窓をどう選んでも検出できない» を
//! 数え上げで示した — **等倍の領域は «推定できない» として投票しない**ので，
//! 一致率はいつも 2 倍側の全会一致 (1.0000) になる．そして直すには
//! **«格子が無い» を «格子 1» として投票させる**しかなく，統計的な推定器は
//! «測れなかった» と «測ったら 1 だった» を分けられない．
//!
//! 分けられるのは**厳密な判定** — «この窓は $k$ 画素の升で完全に揃っているか» —
//! であり，これは [`crate::ingest::integer_block_size`] が画像全体に対して
//! 既にしていることである．だが道具を替えると
//! **D37 «ミクセル検出と非一様格子の棄却は同一の推定器を共有する»** に触る．
//!
//! **だから替える前に上限を測る** (D170 と同じ手) ．替えて本当に鳴るのか，
//! 実素材で誤爆しないのか．**採れない結論なら D37 を触らずに済む．**
//!
//! # 厳密判定が «測れなかった» を分けられる理由
//!
//! 窓の中身が**平らなら，どの $k$ でも条件を満たす** — その窓は格子について
//! 何も言っていない (`Verdict::Flat`) ．中身に模様があれば $k$ は 1 つに決まる
//! (`Verdict::Pinned`) ．統計的推定器はこの 2 つをどちらも «票なし» にするが，
//! 厳密判定は分けられる．**これが D164 の言う «分けられるのは厳密な検査» である．**
//!
//! # 位相は画像の原点に合わせる
//!
//! 升の格子は**画像全体で 1 つ**なので，窓ごとに位相を取り直してはいけない．
//! [`crate::ingest::integer_block_size`] と同じく `x - x % k` で先頭を取る —
//! ただし**窓の中の座標ではなく画像の絶対座標**で取る．

use std::collections::BTreeMap;

use pxsmith_core::grid::{exact_grid_votes, votes_show_mixel};

use crate::mixel::{Scene, Truth};

pub use pxsmith_core::grid::MIXEL_MAX_K as MAX_K;

/// 場面 1 枚 ・窓 1 通りぶんの観測．
#[derive(Clone, Debug)]
pub struct ExactObs {
    pub window: u32,
    /// 並べた窓の総数．
    pub windows: usize,
    /// **$k$ が決まった窓** — これが判定の分母である．
    pub pinned: usize,
    /// **平らで何も言えなかった窓** — «測れなかった» 側．
    pub flat: usize,
    /// 決まった $k$ ごとの窓の数．
    pub by_k: BTreeMap<u32, usize>,
    /// **格子が 2 通り以上あるか** = ミクセルと判定するか．
    pub fires: bool,
}

/// 場面 1 枚ぶん．
#[derive(Clone, Debug)]
pub struct ExactCase {
    pub name: String,
    pub group: &'static str,
    pub truth: Truth,
    pub obs: Vec<ExactObs>,
}

/// 測る窓 (統計側と同じ並び)．
pub const WINDOWS: [u32; 4] = [8, 16, 32, 48];

/// 場面 1 枚を全窓で判定する．
pub fn observe(scene: &Scene, max_k: u32) -> ExactCase {
    let img = &scene.canvas;
    let obs = WINDOWS
        .iter()
        .map(|&window| {
            // **判定は pxsmith-core が持つ** — 道具の本体を 2 か所に置かない (D110)
            let (by_k, flat) = exact_grid_votes(img, window, max_k);
            let pinned: usize = by_k.values().sum();
            ExactObs {
                window,
                windows: pinned + flat,
                pinned,
                flat,
                fires: votes_show_mixel(&by_k),
                by_k,
            }
        })
        .collect();
    ExactCase {
        name: scene.name.clone(),
        group: scene.group,
        truth: scene.truth,
        obs,
    }
}

/// まとめ．
#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub cases: Vec<ExactCase>,
}

impl Summary {
    /// 窓ごとに «正解が混在の場面で鳴った数 / 一様の場面で鳴った数» を数える．
    pub fn tally(&self) -> BTreeMap<u32, (usize, usize, usize, usize)> {
        let mut m: BTreeMap<u32, (usize, usize, usize, usize)> = BTreeMap::new();
        for c in &self.cases {
            for o in &c.obs {
                let e = m.entry(o.window).or_default();
                if c.truth.is_mixel() {
                    e.1 += 1;
                    if o.fires {
                        e.0 += 1;
                    }
                } else {
                    e.3 += 1;
                    if o.fires {
                        e.2 += 1;
                    }
                }
            }
        }
        m
    }
}

impl Summary {
    /// **群ごとに «鳴った枚数 / 枚数»** — 群名を手で当てにいかない．
    ///
    /// 抜き出す群を文字列で当てると取り違える (実際に «等倍» で
    /// **鳴ってはいけない側**を掴んだ) ．全部並べる．
    pub fn by_group(&self, window: u32) -> BTreeMap<(&'static str, bool), (usize, usize)> {
        let mut m: BTreeMap<(&'static str, bool), (usize, usize)> = BTreeMap::new();
        for c in &self.cases {
            let Some(o) = c.obs.iter().find(|o| o.window == window) else {
                continue;
            };
            let e = m.entry((c.group, c.truth.is_mixel())).or_default();
            e.1 += 1;
            if o.fires {
                e.0 += 1;
            }
        }
        m
    }
}

/// 合成した場面すべてに掛ける．
pub fn run(seeds: &[crate::sprite::Seed], sheets: usize, max_k: u32) -> Summary {
    Summary {
        cases: crate::mixel::scenes(seeds, sheets)
            .iter()
            .map(|s| observe(s, max_k))
            .collect(),
    }
}

/// **実素材そのもの** (等倍のドット絵) に掛ける — 誤爆の測定．
///
/// ここに出た «ミクセル» は**全件が誤検出**である (種は等倍だと確かめてある．
/// `sprite::load_seeds`) ．
pub fn corpus(seeds: &[crate::sprite::Seed], max_k: u32) -> Vec<(String, ExactObs)> {
    let mut out = Vec::new();
    for seed in seeds {
        let scene = Scene {
            name: seed.name.clone(),
            group: "実素材",
            canvas: seed.art.clone(),
            truth: Truth::Uniform(Some(1)),
            minority_x: None,
        };
        for o in observe(&scene, max_k).obs {
            if o.windows > 0 {
                out.push((seed.name.clone(), o));
            }
        }
    }
    out
}
