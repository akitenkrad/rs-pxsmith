//! **掃引を回さずに関門を掛け替える．** `recon` の CSV から `estimate_grid` の答えを
//! 再現し，選択規則を回した完全一致数で案を比べる．
//!
//! 測定の作法 (課題分析と戦略 8 節) のうち，ここが引き受けるのは 2 つである．
//!
//! 1. **他の関門を通した状態で測る** — $\varepsilon$ ・再構成 ・帯ずれ ・曲線 ・
//!    半セルずらしを全部掛けた上で，新しい関門を足したときの答えを出す
//! 2. **分離能で採らない** — $\hat{s} = \max \{ s \mid \text{関門を通る} \}$ を実際に
//!    回し，完全一致数 ・A ・B ・D で見る
//!
//! > [!warning] 再現は完全ではない
//! > `scale_candidates` が全 $s$ を素通しで評価するのに対し，`estimate_grid` は自己相関
//! > で候補を絞ってから全探索へ落ちる．**差分を見るには十分だが，最終の数字は掃引で
//! > 確かめること** (`--verify` で食い違いを数えられる) ．
//! >
//! > $\delta$ は掃けない — 不一致率が $\delta$ の下で計算済みだからである．
//! > $\delta$ を変えるなら `recon` から回し直す．

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::dataset::{Manifest, Split};
use crate::sweep::Outcome;

/// 境界の当てはめの掛け方．
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum EdgeMode {
    And,
    /// 位相の検査 (帯ずれ + 曲線) をまとめて肩代わりする．
    Or,
    /// **帯ずれだけを肩代わりし，曲線の検査は残す．推定器がこの形である (D71)．**
    ///
    /// D68 で «曲線が棄却を引き受けている» ことが分かっている (曲線を外すと正棄却が
    /// 183 → 151) ．B が落ちている 13 件の内訳は帯ずれ 7 ・曲線 3 なので，
    /// 肩代わりを帯ずれに限れば，取り戻す側の大半を得たまま棄却を手放さずに済む．
    #[default]
    OrDrift,
}

impl EdgeMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "and" => Some(Self::And),
            "or" => Some(Self::Or),
            "or-drift" => Some(Self::OrDrift),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::And => "and",
            Self::Or => "or",
            Self::OrDrift => "or-drift",
        }
    }
}

/// **境界の当てはめに «帯ずれ以外» を肩代わりさせる範囲** (D72 で測る)．
///
/// D71 は帯ずれだけを肩代わりさせた．真の $s$ を落としている関門は帯ずれ以外にも
/// あり ($\varepsilon$ 4 件 ・再構成 9 件 ・半セルずらし 4 件) ，そこを肩代わり
/// させるのは**まだ測っていない**．
///
/// > [!warning] 肩代わりは «通す» 側にしか働かない
/// > 関門を足す (AND) 形は 200 通り掃いて 1 つも現行を超えなかった (D71) ．
/// > 逆に肩代わりを広げると棄却を手放すので，**取り戻した完全一致と同じだけ
/// > 誤受理が戻る**危険がある — 検証セットだけで決めないこと．
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Rescue {
    pub epsilon: bool,
    pub recon: bool,
    pub contrast: bool,
    /// **曲線の検査を肩代わりする．**
    ///
    /// D71 は «まるごと肩代わり» (`EdgeMode::Or`) として測って捨てた — 検証セットは
    /// +2 件だが実データの誤答が 2 → 4 ・`local/` の誤受理が 2 → 4 に増えた．
    /// ただしそのとき使った当てはめの許容は**帯ずれ用と同じ緩い値**である
    /// (傾き 0.0125 ・残差 0.15) ．
    ///
    /// 曲線で落ちている B 4 件の当てはめは残差 0.001 〜 0.032 ・傾き 0.0044 以下と
    /// **一桁良い**ので，**役目ごとに厳しさを変える** ([`Gates::edge_curve_slope`]) と
    /// 分けられる見込みがある．**緩い許容のまま肩代わりさせないこと．**
    pub curve: bool,
}

impl Rescue {
    /// `none` ・`eps` ・`recon` ・`contrast` を `+` でつないだもの．
    pub fn parse(s: &str) -> Option<Self> {
        let mut out = Self::default();
        for token in s.split('+') {
            match token {
                "none" | "" => {}
                "eps" | "epsilon" => out.epsilon = true,
                "recon" => out.recon = true,
                "contrast" => out.contrast = true,
                "curve" => out.curve = true,
                _ => return None,
            }
        }
        Some(out)
    }

    pub fn label(self) -> String {
        let mut parts = Vec::new();
        if self.epsilon {
            parts.push("eps");
        }
        if self.recon {
            parts.push("recon");
        }
        if self.contrast {
            parts.push("contrast");
        }
        if self.curve {
            parts.push("curve");
        }
        if parts.is_empty() {
            "none".to_string()
        } else {
            parts.join("+")
        }
    }
}

/// 曲線の食い違いを軸ごとにまとめる形．
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum CurveAxis {
    /// 軸の平均 (現行)．
    #[default]
    Mean,
    /// 悪い方の軸 (帯ずれと同じ作法)．
    Max,
}

impl CurveAxis {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "mean" => Some(Self::Mean),
            "max" => Some(Self::Max),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mean => "mean",
            Self::Max => "max",
        }
    }
}

/// 当てはまりの «測り方»．**RMS は外れの峰 1 本で跳ね，添字の丸めが滑ると壊れる．**
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum EdgeStat {
    /// 残差の RMS (現行)．
    #[default]
    Rms,
    /// 残差の中央絶対値．外れの峰に鈍い．
    Median,
    /// **峰を $s$ で畳んだときの散らばり．添字を振らないので丸めの滑りに壊されない．**
    Folded,
}

impl EdgeStat {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "rms" => Some(Self::Rms),
            "median" => Some(Self::Median),
            "folded" => Some(Self::Folded),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rms => "rms",
            Self::Median => "median",
            Self::Folded => "folded",
        }
    }
}

/// 境界の当てはめ (階数 1 つ分)．
#[derive(Clone, Copy, Debug, Default)]
pub struct Edge {
    pub count: [usize; 2],
    pub coverage: [f32; 2],
    pub residual: [Option<f32>; 2],
    pub slope: [Option<f32>; 2],
    /// 残差の中央絶対値 (2 階のみ CSV にある)．
    pub median: [Option<f32>; 2],
    /// 畳んだ散らばり (2 階のみ CSV にある)．
    pub folded: [Option<f32>; 2],
}

impl Edge {
    /// 測り方を選んで軸ごとの値を返す．
    fn stat(&self, axis: usize, stat: EdgeStat) -> Option<f32> {
        match stat {
            EdgeStat::Rms => self.residual[axis],
            EdgeStat::Median => self.median[axis],
            EdgeStat::Folded => self.folded[axis],
        }
    }
}

/// 候補 1 つ．CSV の 1 行から関門に要る列だけ取り出したもの．
#[derive(Clone, Debug)]
pub struct Cand {
    pub scale: u32,
    pub v: f32,
    pub phase: (u32, u32),
    /// 不一致率 (`stats.overall`)．**$\delta$ はこの中に畳み込まれている．**
    pub recon: f32,
    /// 帯の数 2 ・3 ・4 それぞれで測った帯ごとの位相．
    pub bands: [Option<(Vec<usize>, Vec<usize>)>; 3],
    /// 適応帯で実際に使えた帯の数．0 なら測れない候補である．
    pub agree_bands: usize,
    pub joint: [f32; 2],
    pub separate: [f32; 2],
    pub level: [f32; 2],
    /// 半セルずらしたときの分散の比．
    pub contrast: [f32; 2],
    /// 境界の当てはめ (1 階 ・2 階)．
    pub edge: [Edge; 2],
}

/// 1 件分．
#[derive(Clone, Debug)]
pub struct Case {
    pub item_id: u32,
    pub image_var: f32,
    pub has_integer_grid: bool,
    pub truth_scale: u32,
    pub truth_phase: Option<(u32, u32)>,
    pub filter: String,
    pub max_scale: u32,
    pub cands: Vec<Cand>,
}

/// 関門の設定．**`GridParams` と同じ意味の値をここでも持つ** — CSV の上で掛け替える
/// ためのもので，推定の経路ではない．
#[derive(Copy, Clone, Debug)]
pub struct Gates {
    pub epsilon: f32,
    pub tau: f32,
    pub phase_tolerance: f32,
    pub phase_agreement: f32,
    pub phase_contrast_min: f32,
    pub require_measurable: bool,
    pub min_confidence: f32,
    pub confidence_per_scale: bool,
    /// 境界の当てはめを使う階数 (1 か 2)．`0` でこの関門を外す．
    pub edge_order: u32,
    /// 境界の当てはめの**掛け方**．
    ///
    /// - `And` — 位相の検査に**足す**．落とす側にしか働かない
    /// - `Or` — 位相の検査を**肩代わりする**．当てはまりが良ければ帯ずれが暴れていても
    ///   通す．**«真の $s$ を通す» 側だけを担当する**という設計はこちらである
    pub edge_mode: EdgeMode,
    /// 当てはめた間隔のずれ $|(\hat{s}_{\mathrm{fit}} - s)/s|$ の許容．
    pub edge_slope: f32,
    /// 残差 RMS ($s$ で正規化) の許容．
    pub edge_residual: f32,
    /// 拾えた境界の本数の下限 (軸ごと)．
    pub edge_min_count: usize,
    /// 期待される本数 (幅 / $s$) に対する割合の下限．**本数の下限は $s$ が大きいほど
    /// 厳しくなる** ので，割合でも掛けられるようにしておく．
    pub edge_min_coverage: f32,
    /// 境界が拾えない候補を棄却するか．**`Or` では効かない** — 肩代わりの側では
    /// «測れない» は常に «肩代わりしない» であり，旗で «無条件に通す» へ反転させて
    /// しまうと関門が消える．
    pub edge_require_measurable: bool,
    /// 境界の当てはめが**帯ずれ以外に**肩代わりする関門．
    pub rescue: Rescue,
    /// **曲線を肩代わりするときだけに使う，厳しい方の許容** (傾き)．
    ///
    /// 曲線は D68 で «棄却を引き受ける» ために入れた量なので，**帯ずれと同じ緩さで
    /// 手放すと取り戻した分だけ誤受理が戻る** (D71 で実測) ．役目が違えば厳しさも
    /// 変える — 帯ずれの肩代わりは «通す» 側だけを担うが，曲線の肩代わりは
    /// «落とす» 側を削るからである．
    pub edge_curve_slope: f32,
    /// 同上 (残差 RMS)．
    pub edge_curve_residual: f32,
    /// 同上 (境界の本数の下限)．
    pub edge_curve_min_count: usize,
    /// **当てはまりが «酷い» 候補を落とす床** (`None` で落とさない)．
    ///
    /// 境界の当てはめはこれまで**肩代わり (通す側) にしか使っていない**．
    /// [`EdgeMode::And`] は肩代わりの構造ごと置き換えてしまうので B が崩れる
    /// (25 / 50) — 欲しいのは «肩代わりは残したまま，直線にまるで乗らない候補だけ
    /// 落とす» 床である．
    ///
    /// 実測の裾 (2 階差分の残差 max(x, y)) ．
    ///
    /// | | 中央 | 90% | 最大 |
    /// | --- | --- | --- | --- |
    /// | 真の $s$ (通したい) | 0.073 | 0.280 | 0.543 |
    /// | 格子なしで通ってしまう候補 | 0.323 | 0.634 | 0.934 |
    ///
    /// **測れない候補は落とさない** — 平坦な絵で境界が拾えないことは «格子が無い»
    /// ことの根拠にならない (肩代わりの側と同じ規則) ．
    pub edge_drop_residual: Option<f32>,
    /// 床を «両軸とも酷いときだけ» 掛ける (既定) か，«どちらかが酷ければ» 掛けるか．
    pub edge_drop_both_axes: bool,
    /// 曲線の食い違いの正規化 — 分母を «谷の深さ $A - M$» から
    /// «$(A - M) + \lambda A$» へ寄せる．$\lambda = 0$ が現行 (谷の深さ) ，
    /// $\lambda \to \infty$ が «曲線の高さ» に当たる．
    ///
    /// D68 は 3 通り (谷の深さ ・曲線の高さ ・画像分散) しか測っておらず，
    /// **その間は探索していない**．
    pub curve_lambda: f32,
    /// 曲線の食い違いを軸ごとにまとめる形．
    pub curve_axis: CurveAxis,
    /// 当てはまりの測り方 (肩代わり ・床の両方に効く)．
    pub edge_stat: EdgeStat,
}

impl Default for Gates {
    /// 校正済みの既定値 (`GridParams::default()` と同じ運転点)．
    ///
    /// **数値を書き写さない．** 再現が本物と揃っていることが `replay` の値打ちなので，
    /// 既定を写経するとそこから黙って離れる (`pxsmith conform` で実際にやった) ．
    fn default() -> Self {
        let p = pxsmith_core::grid::GridParams::default();
        Self {
            epsilon: p.epsilon,
            tau: p.tau,
            phase_tolerance: p.phase_tolerance,
            phase_agreement: p.phase_agreement,
            phase_contrast_min: p.phase_contrast_min,
            require_measurable: p.phase_require_measurable,
            min_confidence: p.min_confidence,
            confidence_per_scale: p.confidence_per_scale,
            edge_order: p.edge_fit_order,
            edge_mode: EdgeMode::OrDrift,
            edge_slope: p.edge_fit_slope,
            edge_residual: p.edge_fit_residual,
            edge_min_count: p.edge_fit_min_count,
            edge_min_coverage: 0.0,
            edge_require_measurable: true,
            rescue: Rescue::default(),
            // 既定は帯ずれと同じ値 — こうしておけば `--edge-rescue curve` を
            // 単独で指定したときが D71 の «まるごと肩代わり» の再現になる
            edge_curve_slope: p.edge_fit_slope,
            edge_curve_residual: p.edge_fit_residual,
            edge_curve_min_count: p.edge_fit_min_count,
            edge_drop_residual: None,
            edge_drop_both_axes: true,
            curve_lambda: 0.0,
            curve_axis: CurveAxis::Mean,
            edge_stat: EdgeStat::Rms,
        }
    }
}

/// 巡回的な最大距離 (`pxsmith_core::grid` の同名関数と同じ規則)．
fn cyclic_spread(phases: &[usize], s: usize) -> usize {
    let mut worst = 0;
    for (i, a) in phases.iter().enumerate() {
        for b in &phases[i + 1..] {
            let d = a.abs_diff(*b);
            worst = worst.max(d.min(s - d));
        }
    }
    worst
}

impl Cand {
    /// 位相の検査を 3 つに分けて返す．`None` は «測れない» である．
    fn phase_parts(&self, g: &Gates) -> Option<(bool, bool)> {
        if self.agree_bands < 2 {
            return None;
        }
        let (bx, by) = self.bands[self.agree_bands - 2].as_ref()?;
        let s = self.scale as usize;
        let spread = cyclic_spread(bx, s).max(cyclic_spread(by, s)) as f32;
        let drift_ok = spread <= self.scale as f32 * g.phase_tolerance;

        // 曲線の食い違い — 谷が無い軸は棄権する
        let (mut acc, mut worst, mut voted) = (0.0, 0.0f32, 0);
        for axis in 0..2 {
            let depth = self.level[axis] - self.separate[axis];
            if depth <= f32::EPSILON {
                continue;
            }
            let norm = depth + g.curve_lambda * self.level[axis];
            if norm <= f32::EPSILON {
                continue;
            }
            let r = (self.joint[axis] - self.separate[axis]) / norm;
            acc += r;
            worst = worst.max(r);
            voted += 1;
        }
        if voted == 0 {
            return None;
        }
        let curve = match g.curve_axis {
            CurveAxis::Mean => acc / voted as f32,
            CurveAxis::Max => worst,
        };
        Some((drift_ok, curve <= g.phase_agreement))
    }

    /// 位相の検査 (帯ずれ + 曲線)．
    fn passes_phase(&self, g: &Gates) -> bool {
        match self.phase_parts(g) {
            None => !g.require_measurable,
            Some((drift, curve)) => drift && curve,
        }
    }

    /// 半セルずらしたときの崩れ方．
    fn passes_contrast(&self, g: &Gates) -> bool {
        g.phase_contrast_min <= 1.0
            || (self.contrast[0] + self.contrast[1]) / 2.0 >= g.phase_contrast_min
            || (g.rescue.contrast && self.edge_fits(g))
    }

    /// **境界の当てはめそのもの** (肩代わりの資格)．測れない候補は `false` である —
    /// 肩代わりの側では «測れない» は常に «肩代わりしない» でなければならない．
    fn edge_fits(&self, g: &Gates) -> bool {
        self.edge_fits_within(g, g.edge_slope, g.edge_residual, g.edge_min_count)
    }

    /// **曲線を肩代わりするときの資格** — 厳しい方の許容で見る．
    fn edge_fits_strictly(&self, g: &Gates) -> bool {
        self.edge_fits_within(
            g,
            g.edge_curve_slope,
            g.edge_curve_residual,
            g.edge_curve_min_count,
        )
    }

    fn edge_fits_within(&self, g: &Gates, slope_max: f32, residual_max: f32, min: usize) -> bool {
        if g.edge_order == 0 {
            return false;
        }
        let e = &self.edge[(g.edge_order - 1) as usize];
        let mut worst_slope = 0.0f32;
        let mut worst_residual = 0.0f32;
        for axis in 0..2 {
            if e.count[axis] < min || e.coverage[axis] < g.edge_min_coverage {
                return false;
            }
            let (Some(slope), Some(residual)) = (e.slope[axis], e.stat(axis, g.edge_stat)) else {
                return false;
            };
            worst_slope = worst_slope.max(slope.abs());
            worst_residual = worst_residual.max(residual);
        }
        worst_slope <= slope_max && worst_residual <= residual_max
    }

    /// **境界の当てはめ (この案)．** 軸は `max` に揃える (帯ずれと同じ作法)．
    fn passes_edge(&self, g: &Gates) -> bool {
        if g.edge_order == 0 {
            return true;
        }
        if self.edge_fits(g) {
            return true;
        }
        // 測れない候補の行き先．`Or` では «肩代わりしない» で固定する
        !self.edge_measurable(g) && g.edge_mode == EdgeMode::And && !g.edge_require_measurable
    }

    /// 境界が拾えたか (当てはまりの良し悪しは見ない)．
    fn edge_measurable(&self, g: &Gates) -> bool {
        if g.edge_order == 0 {
            return false;
        }
        let e = &self.edge[(g.edge_order - 1) as usize];
        (0..2).all(|axis| {
            e.count[axis] >= g.edge_min_count
                && e.coverage[axis] >= g.edge_min_coverage
                && e.slope[axis].is_some()
                && e.stat(axis, g.edge_stat).is_some()
        })
    }

    /// 落ちた関門を**すべて**並べる．最初の 1 つだけ数えると犯人を取り違える．
    pub fn failed_gates(&self, g: &Gates, image_var: f32) -> Vec<&'static str> {
        let mut out = Vec::new();
        if !self.passes_epsilon(g, image_var) {
            out.push("epsilon");
        }
        if !self.passes_recon(g) {
            out.push("再構成");
        }
        if !self.passes_geometry(g) {
            out.push(if g.edge_order == 0 {
                "位相"
            } else {
                "位相と境界"
            });
            // 位相の内訳 — 帯ずれと曲線は役目が違うので分けて数える (D68 ・D71)
            match self.phase_parts(g) {
                None => out.push("  └ 測れない"),
                Some((drift, curve)) => {
                    if !curve {
                        out.push("  └ 曲線");
                    }
                    if !drift {
                        out.push("  └ 帯ずれ (境界も肩代わりせず)");
                    }
                }
            }
        }
        if !self.passes_contrast(g) {
            out.push("半セルずらし");
        }
        out
    }

    fn passes_epsilon(&self, g: &Gates, image_var: f32) -> bool {
        self.v <= g.epsilon * image_var || (g.rescue.epsilon && self.edge_fits(g))
    }

    fn passes_recon(&self, g: &Gates) -> bool {
        self.recon <= g.tau || (g.rescue.recon && self.edge_fits(g))
    }

    /// **当てはまりが酷い候補を落とす床．** 測れない候補は落とさない．
    fn passes_edge_floor(&self, g: &Gates) -> bool {
        let Some(max) = g.edge_drop_residual else {
            return true;
        };
        if g.edge_order == 0 || !self.edge_measurable(g) {
            return true;
        }
        let e = &self.edge[(g.edge_order - 1) as usize];
        // **両軸とも酷いときだけ落とす．** 片方の軸だけ酷い件は実データの正例に
        // 実在する (真の $s$ で 0.715 / 0.278 ・0.607 / 0.134 ・0.823 / 0.605) ．
        // 絵の中身が一方向に強い縞を持つと，格子が正しくてもその軸の峰が乱れる —
        // **格子は等方でも当てはまりは等方ではない．**
        if g.edge_drop_both_axes {
            !(0..2).all(|axis| e.stat(axis, g.edge_stat).is_some_and(|v| v > max))
        } else {
            (0..2).all(|axis| e.stat(axis, g.edge_stat).is_some_and(|v| v <= max))
        }
    }

    /// 位相の検査と境界の当てはめの合わせ方．
    fn passes_geometry(&self, g: &Gates) -> bool {
        if !self.passes_edge_floor(g) {
            return false;
        }
        match (g.edge_order, g.edge_mode) {
            (0, _) => self.passes_phase(g),
            (_, EdgeMode::And) => self.passes_phase(g) && self.passes_edge(g),
            (_, EdgeMode::Or) => self.passes_phase(g) || self.passes_edge(g),
            (_, EdgeMode::OrDrift) => match self.phase_parts(g) {
                // 測れない候補は肩代わりしない (曲線が課せないので «残す» 側が無い)
                None => !g.require_measurable,
                Some((drift, curve)) => {
                    // **曲線の肩代わりは «厳しい方» の許容で見る** (D71 は帯ずれと
                    // 同じ緩さで手放して誤受理を戻した) ．既定は肩代わりしない
                    let curve = curve || (g.rescue.curve && self.edge_fits_strictly(g));
                    curve && (drift || self.passes_edge(g))
                }
            },
        }
    }

    fn passes(&self, g: &Gates, image_var: f32) -> bool {
        self.passes_epsilon(g, image_var)
            && self.passes_recon(g)
            && self.passes_geometry(g)
            && self.passes_contrast(g)
    }
}

impl Case {
    /// 選択規則を回して $\hat{s}$ を選び，信頼度まで当てる．
    pub fn decide(&self, g: &Gates) -> Option<&Cand> {
        let hat = self
            .cands
            .iter()
            .filter(|c| c.passes(g, self.image_var))
            .max_by_key(|c| c.scale)?;

        // 対照群 — $\hat{s}$ より大きく，約数でも倍数でもない候補 (D63)
        let excluded = |t: u32| {
            t != 0
                && (hat.scale.is_multiple_of(t)
                    || (t.is_multiple_of(hat.scale) && t <= self.max_scale))
        };
        let min_other = self
            .cands
            .iter()
            .filter(|c| c.scale > hat.scale && !excluded(c.scale))
            .map(|c| c.v)
            .fold(f32::INFINITY, f32::min);
        if !min_other.is_finite() || self.image_var <= 0.0 {
            return None; // 対照群なし = 信頼度 0．下限が正なら棄却される
        }
        let conf = ((min_other - hat.v) / self.image_var).clamp(0.0, 1.0);
        let floor = if g.confidence_per_scale {
            g.min_confidence / hat.scale.max(1) as f32
        } else {
            g.min_confidence
        };
        (conf >= floor).then_some(hat)
    }

    /// **関門を通ったのに取り逃した理由．** 真の $s$ が全関門を通っている件で
    /// 完全一致にならなかったとき，犯人は選択規則か信頼度である — 関門をいくら
    /// 掛け替えても動かないので，分けて数える．
    pub fn missed_reason(&self, g: &Gates) -> Option<String> {
        if !self.has_integer_grid {
            return None;
        }
        let truth = self.cands.iter().find(|c| c.scale == self.truth_scale)?;
        if !truth.passes(g, self.image_var) {
            return None; // 関門で落ちた件はここでは扱わない
        }
        match self.decide(g) {
            None => {
                // 関門は通ったが信頼度で落ちた．どちらの側かまで書く
                let hat = self
                    .cands
                    .iter()
                    .filter(|c| c.passes(g, self.image_var))
                    .max_by_key(|c| c.scale)?;
                Some(format!(
                    "信頼度 (選ばれた s = {}．真の s = {})",
                    hat.scale, self.truth_scale
                ))
            }
            Some(c) if c.scale != self.truth_scale => Some(format!(
                "選択規則 (関門を通る最大の s = {} を採った．真の s = {})",
                c.scale, self.truth_scale
            )),
            Some(c) if self.truth_phase != Some(c.phase) => Some(format!(
                "位相 (s = {} は当たったが {:?} 対 {:?})",
                c.scale, c.phase, self.truth_phase
            )),
            Some(_) => None,
        }
    }

    /// 件の信頼度と下限 (診断用)．
    pub fn confidence_at(&self, g: &Gates) -> Option<(f32, f32, u32)> {
        let hat = self
            .cands
            .iter()
            .filter(|c| c.passes(g, self.image_var))
            .max_by_key(|c| c.scale)?;
        let excluded = |t: u32| {
            t != 0
                && (hat.scale.is_multiple_of(t)
                    || (t.is_multiple_of(hat.scale) && t <= self.max_scale))
        };
        let min_other = self
            .cands
            .iter()
            .filter(|c| c.scale > hat.scale && !excluded(c.scale))
            .map(|c| c.v)
            .fold(f32::INFINITY, f32::min);
        let conf = if min_other.is_finite() && self.image_var > 0.0 {
            ((min_other - hat.v) / self.image_var).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let floor = if g.confidence_per_scale {
            g.min_confidence / hat.scale.max(1) as f32
        } else {
            g.min_confidence
        };
        Some((conf, floor, hat.scale))
    }

    pub fn outcome(&self, g: &Gates) -> Outcome {
        match self.decide(g) {
            None => {
                if self.has_integer_grid {
                    Outcome::Rejected
                } else {
                    Outcome::CorrectRejection
                }
            }
            Some(c) => {
                if !self.has_integer_grid || c.scale != self.truth_scale {
                    Outcome::Wrong
                } else if self.truth_phase == Some(c.phase) {
                    Outcome::Exact
                } else {
                    Outcome::ScaleOnly
                }
            }
        }
    }

    /// D66 の区分 (`A` / `B` / `C`)．格子が無い件は `None`．
    pub fn tier(&self) -> Option<char> {
        if !self.has_integer_grid {
            return None;
        }
        match self.filter.as_str() {
            "nearest" => Some('A'),
            "bilinear" | "bicubic" => Some('B'),
            _ => Some('C'),
        }
    }
}

/// 採点．**率で語らない** — 件数のまま持つ．
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Score {
    pub grid_n: usize,
    pub nogrid_n: usize,
    pub exact: usize,
    pub scale_only: usize,
    pub rejected: usize,
    pub correct_rejection: usize,
    /// 黙って誤答した件 (格子ありの誤答 + 格子なしの誤受理)．**D66 の D**．
    pub wrong: usize,
    /// 区分ごとの完全一致 / 件数．
    pub tier: [(usize, usize); 3],
}

impl Score {
    pub fn macro_rate(&self) -> f32 {
        let a = self.exact as f32 / self.grid_n.max(1) as f32;
        let b = self.correct_rejection as f32 / self.nogrid_n.max(1) as f32;
        (a + b) / 2.0
    }

    pub fn line(&self) -> String {
        let [(ae, an), (be, bn), (ce, cn)] = self.tier;
        format!(
            "マクロ {:.1}%  完全一致 {}/{}  正棄却 {}/{}  D {}  A {ae}/{an}  B {be}/{bn}  C {ce}/{cn}",
            self.macro_rate() * 100.0,
            self.exact,
            self.grid_n,
            self.correct_rejection,
            self.nogrid_n,
            self.wrong,
        )
    }
}

pub fn score(cases: &[Case], g: &Gates) -> Score {
    let mut s = Score::default();
    for case in cases {
        if case.has_integer_grid {
            s.grid_n += 1;
        } else {
            s.nogrid_n += 1;
        }
        let outcome = case.outcome(g);
        match outcome {
            Outcome::Exact => s.exact += 1,
            Outcome::ScaleOnly => s.scale_only += 1,
            Outcome::Rejected => s.rejected += 1,
            Outcome::CorrectRejection => s.correct_rejection += 1,
            Outcome::Wrong => s.wrong += 1,
        }
        if let Some(t) = case.tier() {
            let slot = &mut s.tier[(t as u8 - b'A') as usize];
            slot.1 += 1;
            slot.0 += usize::from(outcome == Outcome::Exact);
        }
    }
    s
}

/// `|` でつないだ整数列を読む．空欄は `None`．
fn parse_bands(a: &str, b: &str) -> Option<(Vec<usize>, Vec<usize>)> {
    if a.is_empty() || b.is_empty() {
        return None;
    }
    let one =
        |s: &str| -> Option<Vec<usize>> { s.split('|').map(|v| v.parse::<usize>().ok()).collect() };
    Some((one(a)?, one(b)?))
}

/// CSV と目録を突き合わせて件ごとにまとめる．
///
/// **正解の位相は目録から採る** — CSV には候補ごとの位相しか無く，完全一致は
/// 位相まで含めた一致だからである．
pub fn load(csv: &Path, manifest: &Manifest, only: Option<Split>) -> Result<Vec<Case>> {
    let text =
        std::fs::read_to_string(csv).with_context(|| format!("{} を読めない", csv.display()))?;
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().context("CSV が空である")?.split(',').collect();
    let col = |name: &str| -> Result<usize> {
        header
            .iter()
            .position(|h| *h == name)
            .with_context(|| format!("CSV に列 {name} が無い．recon から取り直すこと"))
    };
    let idx: BTreeMap<&str, usize> = [
        "item_id",
        "scale",
        "truth_scale",
        "has_integer_grid",
        "filter",
        "overall",
        "v",
        "image_var",
        "dx",
        "dy",
        "vratio_x",
        "vratio_y",
        "agree_bands",
        "jx",
        "jy",
        "mx",
        "my",
        "ax",
        "ay",
        "bx2",
        "by2",
        "bx3",
        "by3",
        "bx4",
        "by4",
        "e1nx",
        "e1ny",
        "e1cx",
        "e1cy",
        "e1rx",
        "e1ry",
        "e1sx",
        "e1sy",
        "e2nx",
        "e2ny",
        "e2cx",
        "e2cy",
        "e2rx",
        "e2ry",
        "e2sx",
        "e2sy",
        "e2mx",
        "e2my",
        "e2fx",
        "e2fy",
    ]
    .into_iter()
    .map(|n| col(n).map(|i| (n, i)))
    .collect::<Result<_>>()?;

    let by_id: BTreeMap<u32, &crate::dataset::Item> =
        manifest.items.iter().map(|i| (i.id, i)).collect();

    let mut cases: BTreeMap<u32, Case> = BTreeMap::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split(',').collect();
        let get = |name: &str| f[idx[name]];
        let num = |name: &str| -> f32 { get(name).parse().unwrap_or(0.0) };
        let opt = |name: &str| -> Option<f32> { get(name).parse().ok() };
        // 頑健な測り方は 2 階の列しか無い．**無い列は «測れない» として読む** —
        // 1 階を選んだときに落ちるより，肩代わりが働かない方が診断しやすい
        let opt_if_present =
            |name: &str| -> Option<f32> { idx.get(name).and_then(|i| f[*i].parse().ok()) };

        let item_id: u32 = get("item_id").parse().context("item_id を読めない")?;
        let Some(item) = by_id.get(&item_id) else {
            bail!("目録に無い件が CSV にある: {item_id}")
        };
        if only.is_some_and(|s| item.split != s) {
            continue;
        }

        let edge = |p: &str| Edge {
            count: [
                get(&format!("{p}nx")).parse().unwrap_or(0),
                get(&format!("{p}ny")).parse().unwrap_or(0),
            ],
            coverage: [num(&format!("{p}cx")), num(&format!("{p}cy"))],
            residual: [opt(&format!("{p}rx")), opt(&format!("{p}ry"))],
            slope: [opt(&format!("{p}sx")), opt(&format!("{p}sy"))],
            // 頑健な測り方は 2 階だけ CSV にある (既定が 2 階なので足りる)
            median: [
                opt_if_present(&format!("{p}mx")),
                opt_if_present(&format!("{p}my")),
            ],
            folded: [
                opt_if_present(&format!("{p}fx")),
                opt_if_present(&format!("{p}fy")),
            ],
        };

        let cand = Cand {
            scale: get("scale").parse().context("scale を読めない")?,
            v: num("v"),
            phase: (
                get("dx").parse().unwrap_or(0),
                get("dy").parse().unwrap_or(0),
            ),
            recon: num("overall"),
            bands: [
                parse_bands(get("bx2"), get("by2")),
                parse_bands(get("bx3"), get("by3")),
                parse_bands(get("bx4"), get("by4")),
            ],
            agree_bands: get("agree_bands").parse().unwrap_or(0),
            joint: [num("jx"), num("jy")],
            separate: [num("mx"), num("my")],
            level: [num("ax"), num("ay")],
            contrast: [num("vratio_x"), num("vratio_y")],
            edge: [edge("e1"), edge("e2")],
        };

        let case = cases.entry(item_id).or_insert_with(|| Case {
            item_id,
            image_var: num("image_var"),
            has_integer_grid: item.has_integer_grid(),
            truth_scale: item.truth_scale,
            truth_phase: item.truth_phase,
            filter: item.degradation.filter.as_str().to_string(),
            max_scale: 16,
            cands: Vec::new(),
        });
        case.cands.push(cand);
    }

    Ok(cases.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(scale: u32, v: f32) -> Cand {
        Cand {
            scale,
            v,
            phase: (0, 0),
            recon: 0.0,
            bands: [None, None, None],
            agree_bands: 0,
            joint: [0.0; 2],
            separate: [0.0; 2],
            level: [1.0; 2],
            contrast: [2.0; 2],
            edge: [Edge::default(); 2],
        }
    }

    /// **対照群を必ず 1 つ入れる．** $\hat{s}$ より大きく約数でも倍数でもない候補が
    /// 無いと信頼度が 0 になり，どんな答えも棄却される (D63) ．
    fn case(mut cands: Vec<Cand>) -> Case {
        cands.push(cand(6, 1.0));
        Case {
            item_id: 0,
            image_var: 1.0,
            has_integer_grid: true,
            truth_scale: 4,
            truth_phase: Some((0, 0)),
            filter: "nearest".to_string(),
            max_scale: 16,
            cands,
        }
    }

    fn open() -> Gates {
        Gates {
            require_measurable: false,
            phase_contrast_min: 1.0,
            min_confidence: 0.0,
            ..Gates::default()
        }
    }

    /// 選択規則は「関門を通る**最大**の $s$」である．
    #[test]
    fn the_largest_passing_scale_wins() {
        let c = case(vec![cand(2, 0.0), cand(4, 0.0), cand(8, 1.0)]);
        assert_eq!(c.decide(&open()).map(|c| c.scale), Some(4));
        assert_eq!(c.outcome(&open()), Outcome::Exact);
    }

    /// 境界が測れない候補は，**肩代わりもされないが落とされもしない** (`OrDrift`)．
    #[test]
    fn an_unmeasurable_candidate_is_left_to_the_phase_check() {
        let c = case(vec![cand(4, 0.0)]);
        assert_eq!(c.outcome(&open()), Outcome::Exact);
    }

    /// 足す (`And`) 形なら，間隔がずれている候補は落ちる．
    /// **測れない候補の扱いは旗で決める** — ただし `And` に限る．
    #[test]
    fn the_edge_gate_drops_a_wrong_spacing_when_it_is_added() {
        let mut c = cand(4, 0.0);
        c.edge[0] = Edge {
            count: [10, 10],
            coverage: [1.0; 2],
            residual: [Some(0.0); 2],
            slope: [Some(0.2); 2],
            ..Edge::default()
        };
        let g = Gates {
            edge_order: 1,
            edge_mode: EdgeMode::And,
            edge_slope: 0.05,
            edge_residual: 1.0,
            edge_require_measurable: true,
            ..open()
        };
        assert_eq!(case(vec![c]).outcome(&g), Outcome::Rejected);

        let empty = case(vec![cand(4, 0.0)]);
        assert_eq!(
            empty.outcome(&g),
            Outcome::Rejected,
            "測れない候補は棄却する"
        );
        assert_eq!(
            empty.outcome(&Gates {
                edge_require_measurable: false,
                ..g
            }),
            Outcome::Exact
        );
    }

    /// **肩代わり (`OrDrift`) は帯ずれだけを引き受け，曲線の検査は残す．**
    ///
    /// 帯ずれが暴れていても直線に乗れば通るが，曲線が食い違っていれば通らない．
    #[test]
    fn the_rescue_covers_the_drift_but_not_the_curve() {
        let fit = Edge {
            count: [10, 10],
            coverage: [1.0; 2],
            residual: [Some(0.0); 2],
            slope: [Some(0.0); 2],
            ..Edge::default()
        };
        // 帯ずれは最大 (位相 0 と 2 で $s = 4$) ・曲線は完全に一致 ($J = M$)
        let mut c = cand(4, 0.0);
        c.edge[1] = fit;
        c.agree_bands = 2;
        c.bands[0] = Some((vec![0, 2], vec![0, 2]));
        c.joint = [1.0; 2];
        c.separate = [1.0; 2];
        c.level = [2.0; 2];
        let g = Gates {
            phase_tolerance: 0.25,
            require_measurable: true,
            ..open()
        };
        assert_eq!(
            case(vec![c.clone()]).outcome(&g),
            Outcome::Exact,
            "直線に乗るなら帯ずれは肩代わりされる"
        );

        // 曲線が食い違えば肩代わりされない
        c.joint = [1.5; 2];
        assert_eq!(case(vec![c]).outcome(&g), Outcome::Rejected);
    }

    /// **肩代わりの相手は帯ずれに限らない (D72)．** 再構成検査で落ちる候補も，
    /// 境界が直線に乗っていれば通せる — 効くかどうかは別として，測れる形にしておく．
    #[test]
    fn the_rescue_can_be_pointed_at_the_reconstruction_check() {
        let mut c = cand(4, 0.0);
        c.recon = 0.5; // τ = 0.05 を大きく超える
        c.edge[1] = Edge {
            count: [10, 10],
            coverage: [1.0; 2],
            residual: [Some(0.0); 2],
            slope: [Some(0.0); 2],
            ..Edge::default()
        };
        let g = open();
        assert_eq!(case(vec![c.clone()]).outcome(&g), Outcome::Rejected);
        assert_eq!(
            case(vec![c]).outcome(&Gates {
                rescue: Rescue {
                    recon: true,
                    ..Rescue::default()
                },
                ..g
            }),
            Outcome::Exact,
        );
    }

    /// **曲線の肩代わりは «厳しい方» の許容で見る (D73)．**
    ///
    /// 帯ずれと同じ緩さ (残差 0.15) で手放すと，D71 で捨てた «まるごと肩代わり» に
    /// 戻ってしまう — 実データの誤受理が返ってくる形である．
    #[test]
    fn the_curve_rescue_uses_the_stricter_tolerance() {
        let mut c = cand(4, 0.0);
        c.agree_bands = 2;
        c.bands[0] = Some((vec![0, 0], vec![0, 0])); // 帯ずれは無い
        c.joint = [1.5; 2]; // 曲線は食い違う (0.5 > 0.18)
        c.separate = [1.0; 2];
        c.level = [2.0; 2];
        // 当てはめは «そこそこ» — 帯ずれ用 (0.15) は通るが曲線用 (0.04) は通らない
        c.edge[1] = Edge {
            count: [10, 10],
            coverage: [1.0; 2],
            residual: [Some(0.08); 2],
            slope: [Some(0.0); 2],
            ..Edge::default()
        };
        let g = Gates {
            phase_agreement: 0.18,
            require_measurable: true,
            rescue: Rescue {
                curve: true,
                ..Rescue::default()
            },
            edge_curve_residual: 0.04,
            ..open()
        };
        assert_eq!(case(vec![c.clone()]).outcome(&g), Outcome::Rejected);

        // 同じ候補でも «良く乗っていれば» 肩代わりされる
        c.edge[1].residual = [Some(0.01); 2];
        assert_eq!(case(vec![c]).outcome(&g), Outcome::Exact);
    }

    /// **$\lambda = 0$ は現行そのものである．** 正規化を足しても既定の答えが動かない
    /// ことを固定しておかないと，«再現が本物と揃っている» という値打ちが黙って消える．
    #[test]
    fn the_curve_normalisation_falls_back_to_the_valley_depth() {
        let mut c = cand(4, 0.0);
        c.agree_bands = 2;
        c.bands[0] = Some((vec![0, 0], vec![0, 0]));
        c.joint = [1.5; 2];
        c.separate = [1.0; 2];
        c.level = [2.0; 2];
        // 谷の深さ 1.0 ・食い違い 0.5 → 許容 0.4 では落ちる
        let g = Gates {
            phase_agreement: 0.4,
            ..open()
        };
        assert_eq!(case(vec![c.clone()]).outcome(&g), Outcome::Rejected);
        // 分母を $(A - M) + \lambda A$ = 1.0 + 0.5 x 2.0 = 2.0 にすると 0.25 で通る
        assert_eq!(
            case(vec![c]).outcome(&Gates {
                curve_lambda: 0.5,
                ..g
            }),
            Outcome::Exact,
        );
    }

    /// 軸のまとめ方は `max` で**締まる**側へ動く (帯ずれと同じ作法)．
    #[test]
    fn taking_the_worse_axis_tightens_the_curve_check() {
        let mut c = cand(4, 0.0);
        c.agree_bands = 2;
        c.bands[0] = Some((vec![0, 0], vec![0, 0]));
        c.joint = [1.0, 1.6]; // 軸ごとに 0.0 と 0.6．平均 0.3 ・最大 0.6
        c.separate = [1.0; 2];
        c.level = [2.0; 2];
        let g = Gates {
            phase_agreement: 0.4,
            ..open()
        };
        assert_eq!(case(vec![c.clone()]).outcome(&g), Outcome::Exact);
        assert_eq!(
            case(vec![c]).outcome(&Gates {
                curve_axis: CurveAxis::Max,
                ..g
            }),
            Outcome::Rejected,
        );
    }

    /// 格子が無い件に答えを返したら「黙って誤答した」である．
    #[test]
    fn answering_a_resized_case_counts_as_wrong() {
        let mut c = case(vec![cand(4, 0.0)]);
        c.has_integer_grid = false;
        assert_eq!(c.outcome(&open()), Outcome::Wrong);
        assert_eq!(score(&[c], &open()).wrong, 1);
    }
}
