//! 格子推定 (設計書 6.1) と局所格子推定 (G4)．
//!
//! # 定式化
//!
//! 真のスケール $s_*$ の**すべての約数**がセル内分散 0 を与えるため，「分散を
//! 最小化する」定式化は原理的に誤りである．正しい原理は「**分散が閾値以下になる
//! 最大の $s$**」(D28)．
//!
//! $$ \hat{s} = \max \{ s \mid \bar{V}(s, d_s) \le \varepsilon \land \mathrm{ReconErr}(s, d_s) \le (\delta, \tau) \} $$
//!
//! 各 $s$ の位相 $d_s$ を先に確定させてから判定する．$\hat{d} = d_{\hat{s}}$ は
//! $\hat{s}$ が決まった後にしか定義されないので，条件式の中では使わない．
//!
//! # 再構成検査
//!
//! 倍数側の過大推定 ($s = 2 s_*$) を排除する．**厳密一致は採らない** — 適用範囲に
//! JPEG 圧縮・bicubic / lanczos・非整数倍リサイズを含むため，厳密一致は原理的に
//! 成立せず評価データセットのほぼ全件が棄却される．画素色差の許容 $\delta$ と
//! 不一致画素率の許容 $\tau$ の 2 段で判定する．
//!
//! # 位相ずれ検査
//!
//! 再構成検査だけでは**非整数の周期を落とせない**．画像全体で 1 つの割合しか見ないので，
//! 補間による一様な滲みと，非整数倍リサイズによる距離に比例したずれが同じ数に潰れる．
//! 合成 500 件では，どちらを優先しても他方が落ちる反比例になり，閾値の選び直しでは
//! 抜けられなかった (`docs/investigations/grid-calibration.md`)．
//!
//! そこで画像を帯に切り，帯ごとに最も合う位相を求めて揃っているかを見る．**位相は
//! 絵の中身に左右されない** — 本物の格子なら何が描いてあろうとどの帯でも同じ値になり，
//! 偽物なら帯が進むほどずれる．実測では単一閾値で均衡正解率 95.9% (再構成検査だけなら
//! 76.6%) だった．
//!
//! **帯が薄いときは飛ばさず，帯を減らして測る．** 大きい $s$ ほど帯が薄くなるので，
//! 固定 4 本では $2 s_*$ の候補ほど検査が飛ぶ (実測で 268 件中 212 件が未検査) ．
//!
//! > **この検査がいま真の $s$ を落としている主因である．** 検証セットの格子あり
//! > 101 件のうち 42 件はここで落ちており，再構成検査はそこへ届いていない．
//! > 経緯と測って捨てた案は `docs/investigations/grid-calibration.md`．
//!
//! # 信頼度 (D63)
//!
//! $\hat{s}$ より**大きい** $s$ から倍数を除いた対照群に対する分離マージンを，画像全体の
//! 分散で正規化する．小さい $s$ を対照群へ入れてはいけない — $\bar{V}(s)$ は $s$ と
//! ともに単調に増えるので，最小値を必ず小さい $s$ が取り，マージンが負に潰れる．
//!
//! # 性能
//!
//! 位相探索は**積分画像 (summed-area table) で定数時間化する**．セル分散は
//! $\mathrm{Var} = E[x^2] - E[x]^2$ なので，チャネルごとに $\sum x$ と $\sum x^2$ の
//! 積分画像を 1 度作れば，任意の $(s, d_x, d_y)$ のセル統計が $O(1)$ で求まる．

use crate::canvas::RgbaCanvas;
use crate::color::{Rgba8, delta_e, oklab_of};
use crate::geom::mask::Field;
use crate::math::{IVec2, ivec2};

/// 推定に使う閾値．
///
/// 既定値は合成 500 件の**検証セット 300 件で校正した**もので，マクロ平均 (格子ありの
/// 完全一致率と格子なしの正棄却率の平均) が最大になる組である．
///
/// | 指標 | 値 |
/// | --- | --- |
/// | マクロ平均 | 88.1% |
/// | 格子あり 完全一致 | 83.2% |
/// | 格子なし 正しい棄却 | 93.0% |
///
/// > **まだ暫定である．** 実装計画書 M2 の目標は 95% で，届いていない．テストセット
/// > 200 件には 1 度も触れていない (目標を満たしてから 1 度だけ使う) ．実データ
/// > 20〜30 件も未調達なので，合成データと実運用対象の分布のずれは測れていない．
/// > 経緯は `docs/investigations/grid-calibration.md`．
#[derive(Copy, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GridParams {
    /// 探索するスケールの上限 $s_{\max}$．
    pub max_scale: u32,
    /// セル内平均分散の許容 $\varepsilon$ (画素値を $[0, 1]$ に正規化した値の分散)．
    pub epsilon: f32,
    /// 再構成の画素色差の許容 $\delta$ (OKLab の $\Delta E$)．
    pub delta: f32,
    /// 再構成の不一致画素率の許容 $\tau$．
    pub tau: f32,
    /// 位相ずれ検査で画像を切る帯の数 (縦横それぞれ)．0 で検査を飛ばす．
    pub phase_bands: usize,
    /// 帯どうしの位相のずれの許容．**$s$ に対する割合**で持つ．
    ///
    /// 曲線の食い違い ([`Self::phase_agreement`]) を入れる前は 0.25 だった．**単独で
    /// 緩めると正棄却が崩れる** (0.35 にすると 182 → 151 / 199) が，曲線が棄却を
    /// 引き受けるので緩められる．検証セットで $\theta \in [0.34, 0.38]$ ・許容
    /// $\in [0.15, 0.17]$ が同じ成績の平らな面になっており，その内側を採った．
    pub phase_tolerance: f32,
    /// 帯ごとの位相**曲線**の食い違いの許容．
    ///
    /// $$ \frac{1}{2} \sum_{\text{軸}} \frac{J - M}{A - M}, \quad
    ///    J = \min_p \sum_b C_b(p), \; M = \sum_b \min_p C_b(p), \;
    ///    A = \mathrm{mean}_p \sum_b C_b(p) $$
    ///
    /// 「**全帯に共通の位相を 1 つ選ぶと，帯ごとに best を選ぶのに比べてどれだけ損を
    /// するか**」を谷の深さで割ったものである．帯ずれ (argmin の食い違い) が
    /// $\lfloor s/2 \rfloor$ に張り付いて飽和するのに対し，こちらは谷が浅い帯が分子にも
    /// 分母にも小さくしか効かないので飽和しない — 実測でも通したい件の 90% 点 0.165 ・
    /// 落としたい件の中央 0.481 と重なりが薄い．
    ///
    /// **1.0 以上にすると事実上この検査は働かない** (割合は 1 を超えない)．
    pub phase_agreement: f32,
    /// 半セルずらしたときにセル内分散が最低これだけ悪化することを求める．
    ///
    /// **«セルの中が揃っているか» を見る関門は，滑らかな絵を止められない．** 検証
    /// セットの誤受理 14 件のうち 7 件は $\hat{s} = 2 \ldots 4$ で，帯ずれ 0 ・曲線の
    /// 食い違い 0 ・不一致率 0 と**すべての関門を余裕ゼロで通っていた**．位相を半セル
    /// ずらしても崩れないことがその証拠で，本物の格子なら必ず崩れる．
    ///
    /// 検証セットで比と信頼度の下限を同時に掃くと，1.20〜1.30 が平らな面である．
    /// **1.0 以下でこの検査を外せる．**
    pub phase_contrast_min: f32,
    /// **境界の峰を拾うときの非極大抑制の半径** — $s$ に対する割合 (D173)．
    ///
    /// 初期値は 0.5 ($s/2$) で «補間で境界が 2 画素に広がっても峰を 2 つ数えない»
    /// ために置いたものだったが，**1 通りしか試していなかった** (D72 が «残る的» と
    /// して名指しした) ．掃いたら平らな面が [0.60, 0.65] にあり，その中央を採った．
    /// **0.70 まで上げると A が 26 → 25 に落ちる．**
    pub peak_suppression: f32,
    /// **峰とみなすエネルギーの下限** — 画像の平均エネルギーに対する倍率 (D173)．
    ///
    /// 初期値は 1.0 (平均そのもの) で «絵の中身に依らない尺度が無いので画像自身の
    /// 平均を使う» としたものだったが，**1 通りしか試していなかった**．
    /// 平らな面は [1.2, 1.8] で，その中央を採った．**2.2 まで上げると B が 40 に戻る．**
    pub peak_floor: f32,
    /// 位相の検査が**測れない候補を棄却する**か．
    ///
    /// 帯が薄くて測れない候補は，これまで素通ししていた (「少ないセルから求めた位相は
    /// 落とす根拠にならない」) ．**測ると逆で**，検査を通せない候補を通すと $\hat{s}$ が
    /// そこへ流れる — 棄却すると完全一致 58 → 60 / 101 で，正棄却は 182 / 199 のまま
    /// 変わらなかった．D65 (帯を減らしてでも測る) と同じ向きの直しである．
    pub phase_require_measurable: bool,
    /// 帯ごとの位相を**副画素**で求める．
    ///
    /// 位相は整数に量子化されているが，分けたい量はその刻みと同じ大きさである
    /// (真の $s$ の帯ずれ 0〜4 画素に対し，非整数の周期は帯あたり 0.6〜1.6 画素) ．
    /// 最小点の周りに放物線を当てて刻みを細かくする．
    ///
    /// > **既定は `false`．** 掃引で確かめるまで既定は動かさない．
    pub phase_subpixel: bool,
    /// 許容の下限 (画素)．割合が小さい $s$ で 1 画素を割るのを防ぐ．
    ///
    /// 帯ごとの位相は補間 ・JPEG ・帯に入るセル数で **1 画素くらい揺れる**．実測した
    /// 真の $s$ の帯ずれは 0〜4 画素で $s$ にあまり依らない (雑音由来である) のに対し，
    /// 許容 $s \theta$ は $s = 4$ ・$\theta = 1/6$ で 0.67 画素しかなく，**1 画素でも
    /// 揺れたら落ちる**．検証セットでは真の $s$ の 42 / 101 がこの検査で落ちている．
    ///
    /// > **既定は 0 — つまり下限を掛けない．** 1 画素にすると完全一致は 46.5% →
    /// > 53.5% に上がるが，**格子が無い件の正棄却が 92.0% → 56.3% へ崩れる**
    /// > (マクロ 69.2% → 54.9%) ．非整数の周期はこの検査だけが落としているので，
    /// > 緩めた分がそのまま誤受理になる．測って捨てた値であり，`0` 以外にするなら
    /// > 非整数の周期を別の関門で止めてからにすること．
    pub phase_tolerance_floor: f32,
    /// 位相ずれ検査を行うために帯 1 本へ要るセル数．
    ///
    /// 下回ると検査そのものを行わない (候補は通る) ．**大きい $s$ ほど帯が薄くなるので，
    /// この値が大きいと過大推定の抜け穴になる** — 2 では候補の 23% で検査が働かず，
    /// $s = 12$ ・$16$ の過大推定がそこから漏れている．
    ///
    /// > **1 にすると評価データセットの成績は上がるが (88.1% → 90.1%) ，採らない．**
    /// > 帯あたり 1 セルでは位相が当てにならず，24 画素角のスプライトを 4 倍した画像で
    /// > 真のスケールを落とす (試験 `recovers_the_phase_when_the_image_is_cropped`) ．
    /// > 評価データセットの元絵は 16〜48 画素角なので**真のスケールでは常に 16 セル以上**
    /// > あり，この失敗を見られない — 掃引の数字が良くなるのは，データセットが
    /// > 見ていない場面を代償にしているからである．小さい絵を入れてから測り直す．
    pub phase_min_cells: usize,
    /// この値未満の信頼度は棄却する．[`Self::confidence_per_scale`] を立てると
    /// $\hat{s}$ で割った値が実際の下限になる．
    pub min_confidence: f32,
    /// 下限を $\hat{s}$ で割る．**信頼度の意味は $\hat{s}$ によって違う**ためである．
    ///
    /// 実測すると，同じ信頼度でも $\hat{s}$ が違えば正しさが違った．
    ///
    /// - **小さい $\hat{s}$ には甘すぎた** — 答えを返した件のうち $\hat{s} = 2$ が
    ///   98 件中 68 件を占め，その多くは «滑らかな絵に $2 \times 2$ の格子が見えた»
    ///   だけの縮退である (設計書 6.1 の退化ケース) ．一様な下限ではこの裾を落とせない
    /// - **大きい $\hat{s}$ には厳しすぎた** — 対照群は $\hat{s}$ より大きい $s$ なので，
    ///   $\hat{s}$ が大きいほど痩せ，マージンが構造的に小さくなる
    ///
    /// 検証セットでの効果 (下限を $\hat{s}$ で割る前後) ．**3 つの率が同時に上がる**．
    ///
    /// | | 一様 0.03 | $\hat{s} = 2$ だけ厳しく | 大きい $\hat{s}$ だけ緩く | 両方 |
    /// | --- | --- | --- | --- | --- |
    /// | マクロ平均 | 70.5% | 71.7% | 72.7% | **74.9%** |
    /// | 完全一致 | 51.5% | 51.5% | 57.4% | **58.4%** |
    /// | 正しい棄却 | 89.4% | 92.0% | 87.9% | **91.5%** |
    pub confidence_per_scale: bool,
    /// 境界の当てはめに使う差分の階数 (1 か 2)．**`0` でこの関門を外す．**
    ///
    /// 実測すると，真の $s$ の当てはめた間隔のずれは補間で大きく変わる．
    ///
    /// | 補間 | 1 階の \|傾き\| 中央 / 90% | 2 階の \|傾き\| 中央 / 90% |
    /// | --- | --- | --- |
    /// | nearest | 0.000 / 0.001 | 0.000 / 0.012 |
    /// | bilinear | 0.035 / 0.170 | **0.003 / 0.121** |
    /// | bicubic | 0.002 / 0.011 | 0.017 / 0.123 |
    /// | lanczos | 0.021 / 0.082 | 0.090 / 0.316 |
    ///
    /// **1 階は bilinear で外れる** — bilinear はセル中心の間を直線で結ぶので 1 階差分が
    /// 区間ごとに平らになり，峰の «頂点» が定まらない．2 階差分なら折れ点に山が立つ．
    /// 選択規則を回すと 2 階が勝つ (完全一致 76 対 73) ．
    pub edge_fit_order: u32,
    /// 当てはめた間隔のずれ $|(\hat{s}_{\mathrm{fit}} - s)/s|$ の許容．
    ///
    /// 非整数倍リサイズ (1.3 ・0.85) の実効周期は整数から 2〜6% ずれるので，
    /// 1.25% はその下に十分入る．検証セットで掃くと 0.0125 が膝で，
    /// **これを超えると正棄却が崩れ始める** (0.0125 で 191 / 199 ・0.02 で 183 ・
    /// 0.03 で 173) 一方，完全一致は 0.0125 で頭打ちになる (76 → 75) ．
    pub edge_fit_slope: f32,
    /// 直線からの残差 RMS ($s$ で正規化) の許容．
    ///
    /// **ほとんど効いていない** — 外す (1.0) のと 0.15 とで正棄却が 1 件違うだけである．
    /// 傾きの検査が先に落とすためで，残しているのは «峰が直線にまったく乗らない»
    /// という別の壊れ方への備えにすぎない．検証セットはその場面をほとんど見ていない．
    pub edge_fit_residual: f32,
    /// 肩代わりに要る境界の本数 (軸ごと)．
    ///
    /// 4 点あれば，切片と傾きの 2 つを決めた上で残差に 2 自由度が残る．検証セットでは
    /// **3 でも 5 でも完全一致が 2 件落ちる** (74 対 76) ので，数字そのものには
    /// それ以上の根拠が無い — 少なすぎれば当てずっぽうを肩代わりし，多すぎれば
    /// 小さい絵の本物を肩代わりできない，という両側の効き方だけが確かである．
    pub edge_fit_min_count: usize,
    /// **曲線の検査も肩代わりしてよい残差の上限** (`None` で肩代わりしない)．
    ///
    /// 境界の当てはめは既定では帯ずれだけを肩代わりする (D71) ．曲線は D68 で
    /// «帯ずれを緩めた分の棄却を引き受ける» ために入れた量なので，**同じ緩さで
    /// 手放すと取り戻した分だけ誤受理が戻る** — D71 でまるごと肩代わりさせたときは
    /// 実データの誤答が 2 → 4 ・`local/` の誤受理が 2 → 4 に増えた．
    ///
    /// **役目が違えば厳しさも変える**という案だった (D73) ．曲線で落ちていた真の $s$ の
    /// 当てはめは残差 0.001 〜 0.032 と [`Self::edge_fit_residual`] (0.15) より**一桁良い**
    /// ので，そこだけ 0.04 を課せば «良く乗っている» ものだけを通せる．
    ///
    /// **検証セットでは無償で目標に届く．**
    ///
    /// | 曲線を肩代わりする残差 | 完全一致 | 正棄却 | D | A | **B** |
    /// | --- | --- | --- | --- | --- | --- |
    /// | **`None` (既定)** | 73 | 192 | 9 | 26 / 26 | 39 / 50 |
    /// | 0.04 | **74** | **192** | **9** | 26 / 26 | **40 / 50 — 目標 80%** |
    /// | 0.15 (帯ずれと同じ緩さ．D71 で捨てた形) | 74 | 191 | 10 | 26 / 26 | 40 / 50 |
    ///
    /// > [!warning] **それでも既定は `None` である — 実データ枠が払う**
    /// > 同梱 148 件が 完全一致 60 → 61 ・誤答 2 → **3** ・誤受理 0 → **1**，
    /// > `local/` 92 件が 誤受理 2 → **4**．**正解 1 件と引き換えに誤りが 4 件**増える．
    /// > D66 の要件は «黙って誤答しないこと» の方なので採らない．
    /// >
    /// > 漏れる 4 件は $\hat{s} = 8$ ・$14$ で信頼度 0.021 〜 0.074 と**大きい $\hat{s}$ に
    /// > 偏っている** (下限は $\hat{s}$ で割るのでそこが最も弱い) ．**下限の形では塞げない** —
    /// > $\hat{s} = 14$ の 0.074 を止めるには一様下限 0.074 が要り，それは小さい $\hat{s}$ 側を
    /// > 壊す (D67) ．拾えた境界の «割合» でも分けられない — 真の $s$ の中央 0.63 に対し
    /// > 格子なしの候補は 0.72 と**逆を向いている** (偽の峰は小さい $s$ ほど多く立つ) ．
    ///
    /// 傾きの許容は [`Self::edge_fit_slope`] と同じ値を使う — 0.005 まで締めても検証
    /// セットの成績は 1 件も動かず，**分けているのは残差だけ**だからである．
    pub edge_fit_curve_residual: Option<f32>,
    /// $\varepsilon$ を**画像全体の分散に対する割合**として解釈する．
    ///
    /// $\varepsilon$ は分散の絶対値に対する閾値なので，**低コントラストの入力では
    /// すべての $s$ が楽に通り**，「閾値を満たす最大の $s$」が大きい方へ流れる．
    /// 実測で画像分散は素材によって 2 倍以上ちがう (合成スプライト 0.089 ・自作レンダ
    /// 0.050 ・生成 AI を縮小したもの 0.038) ．
    ///
    /// これを立てると判定が $\bar{V}(s) \le \varepsilon \bar{V}_{\mathrm{image}}$ に
    /// なり，尺度に依らなくなる．**既定は `false`** — 掃引で確かめるまで既定は動かさない．
    pub normalize_epsilon: bool,
}

impl Default for GridParams {
    fn default() -> Self {
        Self {
            max_scale: 16,
            // **画像分散に対する割合**である (normalize_epsilon = true)．
            // 検証セットで 0.1 / 0.2 / 0.3 / 0.5 を掃引し 0.2 が最良 (マクロ 88.8%)．
            // 絶対値のままだと 88.1% で，実データでは誤答が 4 件多い
            // 位相の検査を作り直した (D68) あとに 486 通りを掃引し直した値．
            // 関門を変えれば «通ってくる相手» が変わるので，閾値は**組で選び直す**．
            // ε ・δ ・τ ・θ ・曲線の許容 ・信頼度の下限を同時に見て，
            // (0.15, 0.15, 0.05, 0.35, 0.16, 0.10) がマクロ 79.2% で最良だった
            // (旧関門 ・旧閾値は 74.9%) ．実データ枠も同じ向きに動いている
            epsilon: 0.15,
            delta: 0.15,
            tau: 0.05,
            phase_bands: 4,
            // **実物の元絵へ差し替えた検証セットで測り直した値である．**
            // 1/8 で 67.5% ・1/6 で 69.2% ・1/5 で 69.0% ・**1/4 で 70.5%** ・1/3 で 69.0%
            // (マクロ平均．ε = 0.2 ・δ = 0.1 ・τ = 0.1 ・信頼度 0.03)
            // 境界の当てはめ (D71) が «真の $s$ を通す» 側を引き受けたので**締め直した**．
            // 検証セットで 0.25 ・0.28 ・0.30 が同じ成績の平らな面になり，その中央を
            // 採った (0.35 のままだと A が 26 → 24 に落ちる — 緩い帯ずれは
            // $2 s_*$ に «閾値を満たす最大の $s$» を渡してしまう)
            // **境界の峰の拾い方** (D173) ．D72 が «初期値のまま 1 通りしか試して
            // いない» と名指しした 2 つで，掃いたら B が 40 → 42 / 50 に届いた．
            //
            // | 抑制 | 0.55 | **0.60** | **0.65** | 0.70 |
            // | B | 41 | **42** | **42** | 42 だが A が 26 → 25 |
            //
            // | 下限 | 1.0 | **1.2** | **1.5** | **1.8** | 2.2 |
            // | B | 40 | **42** | **42** | **42** | 40 |
            //
            // **平らな面の中央を採った** (phase_tolerance を 0.28 にしたのと同じ作法) ．
            // 実データ枠は 同梱 完全一致 60 → 61 ・誤答 2 のまま ・`local/` は完全に
            // 同一 (誤受理 2 のまま) で，**どちらの枠も悪くならない**．
            peak_suppression: 0.625,
            peak_floor: 1.5,
            phase_tolerance: 0.28,
            // 検証セットで θ ・許容を同時に掃いて選んだ (平らな面の内側)．
            // 1.0 以上にすると曲線の検査は働かない．
            // **境界の当てはめ (D71) を入れて選び直した値である** — 肩代わりされた分
            // 曲線に掛かる相手が変わる．0.16 ・0.18 ・0.20 は検証セットで同じ成績
            // (73 / 101 ・191 / 199) で，その平らな面の中央を採った．実データ枠は
            // 0.18 以上で完全一致 58 → 60 と動き，誤答 ・誤受理は変わらない
            phase_agreement: 0.18,
            // 検証セットで信頼度の下限と同時に掃いて選び，**実データ枠で膝を確かめて**
            // 決めた値．1.14 以上にすると検証セットの正棄却は 3 件増えるが，実データの
            // AI 出力の正例が 23 → 19 / 28 件へ落ちる — 元絵が連続階調の «本物の格子» は
            // 合成データセットに 1 件も入っていないので (種は実物のドット絵) ，
            // **検証セットだけでは見えない失敗**である．1.0 以下でこの検査を外せる
            phase_contrast_min: 1.12,
            phase_require_measurable: true,
            // 副画素は最良同士で +0.5 ポイントにとどまる (70.7% → 71.2%)．
            // **既定にはしない** — 詳細は docs/investigations/grid-calibration.md
            phase_subpixel: false,
            // 検証セットで 0 ・1 ・2 を掃引した．**0 が最良** — 1 にすると完全一致は
            // 上がるが正棄却が崩れる (docs/investigations/grid-calibration.md)
            phase_tolerance_floor: 0.0,
            phase_min_cells: 2,
            // 運転点．**$\hat{s}$ で割って使う** (confidence_per_scale) ．
            // 検証セットで 0.05〜0.16 を掃引し 0.10 が最良 (マクロ 74.9%) ．
            // 0.06〜0.14 で 72.9〜74.9% と平らなので，尖った選び方ではない
            //
            // **0.10 → 0.095 (D72)．** 掃引の格子が 0.08 の次に 0.10 と粗く，間が
            // 測れていなかった — 下限は $\hat{s}$ で割るので，$\hat{s} = 2 \ldots 3$
            // では 0.005 の差が実際の下限の 0.002 の差になる．0.095 は検証セットの
            // 完全一致を 72 → 73 (B 38 → 39) にし，**正棄却 192 ・D 9 ・実データ枠
            // (同梱 148 件 ・`local/` 92 件) はどれも 1 件も動かない**．
            //
            // > [!warning] 0.09 まで下げると B は 40 / 50 (目標 80%) に届くが採らない
            // > 検証セットは完全一致 74 ・B 40 になる一方，`local/` 92 件で
            // > **誤答 0 → 1 ・誤受理 2 → 3** と後退する (同梱は不変) ．`local/` は
            // > `px conform` が実際に受け取る入力に最も近い枠であり，D66 の要件は
            // > «黙って誤答しないこと» の方である．**取り戻す 1 件 (信頼度 0.0305 ・
            // > $\hat{s} = 3$) と失う 1 件 (`local/009.png` ・信頼度 0.031 ・
            // > $\hat{s} = 3$) は 0.0005 しか違わず，この統計では分離できない．**
            min_confidence: 0.095,
            confidence_per_scale: true,
            // 検証セットで確かめてから立てた．**実データでは誤答が半分になり
            // (8 → 4 件) ，代償は無かった** — 負例の成績はどちらも同じである
            normalize_epsilon: true,
            // 2 階差分で拾う (1 階は bilinear で外れる)．検証セットで掃いて決めた膝
            edge_fit_order: 2,
            edge_fit_slope: 0.0125,
            edge_fit_residual: 0.15,
            edge_fit_min_count: 4,
            // **肩代わりしない．** 0.04 なら検証セットは無償で B 40 / 50 (目標 80%) に
            // 届くが，実データ枠が 正解 1 件に対し誤り 4 件を払う (D73．doc を読む) ．
            // `px-calib sweep|real --edge-fit-curve-residual` で掛け替えて比べられる
            edge_fit_curve_residual: None,
        }
    }
}

impl GridParams {
    /// 実際に効く信頼度の下限．[`Self::confidence_per_scale`] を立てると
    /// $\hat{s}$ で割る．
    pub fn confidence_floor(&self, scale: u32) -> f32 {
        if self.confidence_per_scale {
            self.min_confidence / scale.max(1) as f32
        } else {
            self.min_confidence
        }
    }

    /// 実際に使う $\varepsilon$．正規化を立てると画像分散に対する割合になる．
    ///
    /// 完全に平坦な画像では 0 になり，どの候補も通らない — 尺度が無いので当然である
    /// (信頼度も 0 になるので，どちらにせよ棄却される)．
    fn epsilon_for(&self, image_var: f32) -> f32 {
        if self.normalize_epsilon {
            self.epsilon * image_var
        } else {
            self.epsilon
        }
    }
}

/// 推定の結果．
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct GridEstimate {
    /// 推定したスケール $\hat{s}$．
    pub scale: u32,
    /// 位相 $\hat{d} = (d_x, d_y)$．
    pub phase: IVec2,
    /// 信頼度 $\mathrm{conf} \in [0, 1]$．
    pub confidence: f32,
    /// 採用した $s$ でのセル内平均分散．
    pub mean_variance: f32,
}

/// 推定に失敗する理由．
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GridError {
    /// 閾値を満たす $s$ が 1 つも無い．
    NotFound,
    /// 画像が小さすぎて格子を切れない．
    TooSmall,
    /// 信頼度が下限を下回った (棄却)．
    LowConfidence,
}

impl std::fmt::Display for GridError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "閾値を満たす格子が見つからない"),
            Self::TooSmall => write!(f, "画像が小さすぎて格子を切れない"),
            Self::LowConfidence => write!(f, "信頼度が下限を下回ったので棄却した"),
        }
    }
}

/// ある $s$ についての評価結果．信頼度の計算が全候補の $\bar{V}$ を要求するため，
/// 早期 return せずに全部集める．
#[derive(Copy, Clone, Debug)]
struct Candidate {
    scale: u32,
    mean_variance: f32,
    phase: IVec2,
}

/// 積分画像．チャネルごとに $\sum x$ と $\sum x^2$ を持つ．
///
/// 値は `u8` なので `u64` で厳密に積める．浮動小数で積むと大きな画像で
/// 桁落ちし，同じ入力でも積む順序で答えが変わりうる (設計書 6.15 規則 3)．
struct Integral {
    w: usize,
    h: usize,
    sum: [Vec<u64>; 3],
    sq: [Vec<u64>; 3],
}

impl Integral {
    fn new(img: &RgbaCanvas) -> Self {
        let (w, h) = (img.width() as usize, img.height() as usize);
        let stride = w + 1;
        let mut sum = [
            vec![0u64; stride * (h + 1)],
            vec![0u64; stride * (h + 1)],
            vec![0u64; stride * (h + 1)],
        ];
        let mut sq = [
            vec![0u64; stride * (h + 1)],
            vec![0u64; stride * (h + 1)],
            vec![0u64; stride * (h + 1)],
        ];

        for y in 0..h {
            let mut row = [0u64; 3];
            let mut row_sq = [0u64; 3];
            for x in 0..w {
                let c = img.pixels()[y * w + x];
                let v = [c.r as u64, c.g as u64, c.b as u64];
                let i = (y + 1) * stride + (x + 1);
                for (k, value) in v.iter().enumerate() {
                    row[k] += value;
                    row_sq[k] += value * value;
                    sum[k][i] = sum[k][i - stride] + row[k];
                    sq[k][i] = sq[k][i - stride] + row_sq[k];
                }
            }
        }
        Self { w, h, sum, sq }
    }

    fn rect(&self, table: &[u64], x0: usize, y0: usize, x1: usize, y1: usize) -> u64 {
        let stride = self.w + 1;
        table[y1 * stride + x1] + table[y0 * stride + x0]
            - table[y0 * stride + x1]
            - table[y1 * stride + x0]
    }

    /// 矩形内の分散 (3 チャネルの和)．画素値は $[0, 1]$ に正規化する．
    fn variance(&self, x0: usize, y0: usize, x1: usize, y1: usize) -> f64 {
        let n = ((x1 - x0) * (y1 - y0)) as f64;
        if n <= 0.0 {
            return 0.0;
        }
        let mut acc = 0.0;
        for k in 0..3 {
            let s = self.rect(&self.sum[k], x0, y0, x1, y1) as f64;
            let q = self.rect(&self.sq[k], x0, y0, x1, y1) as f64;
            let mean = s / n;
            // 丸め誤差で僅かに負になることがある
            acc += (q / n - mean * mean).max(0.0);
        }
        // 0..255 の分散を 0..1 の尺度へ
        acc / (255.0 * 255.0)
    }

    fn mean(&self, x0: usize, y0: usize, x1: usize, y1: usize) -> Rgba8 {
        let n = ((x1 - x0) * (y1 - y0)) as f64;
        if n <= 0.0 {
            return Rgba8::TRANSPARENT;
        }
        let mut c = [0u8; 3];
        for (k, slot) in c.iter_mut().enumerate() {
            let s = self.rect(&self.sum[k], x0, y0, x1, y1) as f64;
            *slot = (s / n).round().clamp(0.0, 255.0) as u8;
        }
        Rgba8::rgb(c[0], c[1], c[2])
    }
}

/// 完全なセルの並び．端の欠けたセルは含めない．
fn cells(
    w: usize,
    h: usize,
    s: usize,
    dx: usize,
    dy: usize,
) -> impl Iterator<Item = (usize, usize)> {
    let nx = if w > dx { (w - dx) / s } else { 0 };
    let ny = if h > dy { (h - dy) / s } else { 0 };
    (0..ny).flat_map(move |j| (0..nx).map(move |i| (dx + i * s, dy + j * s)))
}

/// セル内平均分散 $\bar{V}(s, d_x, d_y)$．完全なセルが無ければ `None`．
fn mean_cell_variance(it: &Integral, s: usize, dx: usize, dy: usize) -> Option<f32> {
    let mut acc = 0.0f64;
    let mut count = 0usize;
    for (x, y) in cells(it.w, it.h, s, dx, dy) {
        acc += it.variance(x, y, x + s, y + s);
        count += 1;
    }
    (count > 0).then(|| (acc / count as f64) as f32)
}

/// 平均分散が最小の位相．**同点は $(d_x, d_y)$ の辞書式順**で決める
/// (設計書 6.15 規則 2)．
fn best_phase(it: &Integral, s: usize) -> Option<(f32, IVec2)> {
    let mut best: Option<(f32, IVec2)> = None;
    for dy in 0..s {
        for dx in 0..s {
            let Some(v) = mean_cell_variance(it, s, dx, dy) else {
                continue;
            };
            let phase = ivec2(dx as i32, dy as i32);
            match best {
                // 走査順が辞書式なので，同点は先に見たものが残る
                Some((bv, _)) if bv <= v => {}
                _ => best = Some((v, phase)),
            }
        }
    }
    best
}

/// 矩形の中だけでセル内平均分散を測る．位相は**画像の原点を基準**に与える．
fn mean_cell_variance_in(
    it: &Integral,
    s: usize,
    rect: (usize, usize, usize, usize),
    dx: usize,
    dy: usize,
) -> Option<f32> {
    let (x0, y0, x1, y1) = rect;
    let mut acc = 0.0f64;
    let mut count = 0usize;
    let mut y = first_cell(y0, dy, s);
    while y + s <= y1 {
        let mut x = first_cell(x0, dx, s);
        while x + s <= x1 {
            acc += it.variance(x, y, x + s, y + s);
            count += 1;
            x += s;
        }
        y += s;
    }
    (count > 0).then(|| (acc / count as f64) as f32)
}

/// 帯の中で最初にセルが始まる座標．
///
/// **位相は画像の原点を基準に測る．** 帯の左端を基準にすると帯ごとに違う物差しで
/// 測ることになり，本物の格子でもずれが出て何も分からなくなる．
fn first_cell(band_start: usize, phase: usize, s: usize) -> usize {
    band_start + (phase + s - band_start % s) % s
}

/// 帯 1 つの中の，位相ごとのセル内平均分散．測れない位相は $+\infty$ にする．
///
/// **谷の形が要る場面がある**ので，最小値だけでなく曲線そのものを返す
/// ([`refine_phase`])．
fn phase_curve(
    it: &Integral,
    s: usize,
    rect: (usize, usize, usize, usize),
    fixed: usize,
    horizontal: bool,
) -> Vec<f32> {
    (0..s)
        .map(|p| {
            let (dx, dy) = if horizontal { (p, fixed) } else { (fixed, p) };
            mean_cell_variance_in(it, s, rect, dx, dy).unwrap_or(f32::INFINITY)
        })
        .collect()
}

/// 曲線の最小点．**同点は小さい方**を採る (設計書 6.15 規則 2)．測れなければ `None`．
fn argmin_phase(curve: &[f32]) -> Option<usize> {
    let mut best: Option<(f32, usize)> = None;
    for (p, &v) in curve.iter().enumerate() {
        // 測れない位相は候補にしない (元の実装が `None` を読み飛ばしていたのと同じ)
        if !v.is_finite() {
            continue;
        }
        match best {
            Some((bv, _)) if bv <= v => {}
            _ => best = Some((v, p)),
        }
    }
    best.map(|(_, p)| p)
}

/// 最小点の周りに放物線を当てて位相を**副画素**で求める．
///
/// 位相は整数に量子化されているが，**分けたい量はその刻みと同じ大きさである** —
/// 真の $s$ の帯ずれが 0〜4 画素なのに対し，落としたい非整数の周期は帯あたり
/// 0.6〜1.6 画素しか流れない．刻みが粗いままでは，締めれば真の $s$ が落ち，
/// 緩めれば非整数の周期が通る．
///
/// 3 点 $(p-1, p, p+1)$ の分散に放物線を当て，その頂点を採る．谷になっていない
/// (2 階差分が正でない) ときは整数のまま返す．
fn refine_phase(curve: &[f32], p: usize) -> f32 {
    let s = curve.len();
    if s < 3 {
        return p as f32;
    }
    let (l, c, r) = (curve[(p + s - 1) % s], curve[p], curve[(p + 1) % s]);
    let denom = l - 2.0 * c + r;
    if !l.is_finite() || !r.is_finite() || !c.is_finite() || denom <= 0.0 {
        return p as f32;
    }
    // 頂点は 3 点の中央から ±0.5 の外へは出ない
    p as f32 + (0.5 * (l - r) / denom).clamp(-0.5, 0.5)
}

/// 巡回的な最大距離 (副画素版)．
fn cyclic_spread_f(phases: &[f32], s: f32) -> f32 {
    let mut worst = 0.0f32;
    for (i, a) in phases.iter().enumerate() {
        for b in &phases[i + 1..] {
            let d = (a - b).abs();
            worst = worst.max(d.min(s - d));
        }
    }
    worst
}

/// 巡回的な最大距離．位相は $s$ で一周するので 0 と $s - 1$ は隣どうしである．
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

/// 位相ずれ検査 — 帯ごとに最も合う位相を求め，帯の間で揃っているかを見る．
///
/// **再構成検査だけでは非整数の周期を落とせない．** 周期 $s \cdot r$ ($r$ が非整数) の
/// 入力を整数 $q$ で近似すると，セル境界は 1 セルあたり $|q - s r|$ ずつずれていく．
/// ところが再構成検査は画像全体で 1 つの割合しか見ないので，このずれが「補間による
/// 一様な滲み」と同じ数に潰れてしまう．評価データセットではどちらを優先しても
/// もう一方が落ちる反比例になり，閾値の選び直しでは抜けられなかった．
///
/// 位相は**絵の中身に左右されない**ところが違う．格子が本物なら，何が描いてあろうと
/// どの帯でも同じ位相が最も合う．偽物なら帯が進むほどずれる．
///
/// 帯の数は閾値ではなく**検査の適用範囲**を決めている (D65) ．固定 4 本では大きい $s$
/// ほど帯が薄くなって検査が飛び，$2 s_*$ の候補の 212 / 268 が未検査だった — 過大推定は
/// ここから漏れていた．**飛ばさずに帯を 3 本 ・2 本と減らして測る．**
///
/// 帯ずれだけでは足りない理由は [`BandAgreement`] にある — argmin の食い違いは
/// 落としたい候補で $\lfloor s/2 \rfloor$ に張り付いており (巡回距離の上限) ，
/// **どこで切っても分かれない**．曲線どうしの比較は飽和しないので，帯ずれの許容を
/// 緩めた分の棄却をこちらが引き受ける．
///
/// | 検証セット 300 件 | 完全一致 | 正棄却 | マクロ |
/// | --- | --- | --- | --- |
/// | 帯ずれのみ $\theta = 0.25$ (D65) | 58 / 101 | 182 / 199 | 74.4% |
/// | + 測れなければ棄却 | 60 / 101 | 182 / 199 | 75.4% |
/// | + 曲線 ($\theta = 0.35$ ・許容 0.16) | **62 / 101** | **183 / 199** | **76.7%** |
/// | 曲線なしで $\theta = 0.35$ にしただけ | 62 / 101 | 151 / 199 | 68.6% |
///
/// 最下行が「曲線が棄却を引き受けている」ことの実測である (正棄却 151 → 183) ．
/// 位相の検査を**帯ずれと曲線に分けて**返す．`None` は «測れない» である．
///
/// 分けてあるのは，境界の当てはめ (D71) に**帯ずれだけを肩代わりさせる**ためである．
/// 曲線はまとめて肩代わりさせない — D68 で «曲線が棄却を引き受けている» ことが
/// 分かっており (外すと正棄却が 183 → 151 / 199) ，そこを手放すと，取り戻した
/// 完全一致と同じだけ誤受理が戻ってくる．
fn phase_parts(it: &Integral, s: usize, d: IVec2, check: DriftCheck) -> Option<(bool, bool)> {
    // 帯 0 ・1 は «検査を切る» 指定である．**測れなかった候補とは区別する**
    if check.bands < 2 {
        return Some((true, true));
    }
    // 測れない候補は**棄却する**．検査を通せない答えを返すほうが危ない
    // (実測で完全一致 58 → 60 ・正棄却は 182 のまま) ．
    let (_, curves) = adaptive_curves(it, s, d, check.bands, check.min_cells)?;
    let spread = drift_of(&curves, s, check.subpixel);
    let drift = spread <= (s as f32 * check.tolerance).max(check.floor);
    // 谷が無い = 位相を選ぶ根拠が無い．測れない候補と同じ扱いにする
    let curve = agreement_of(&curves)? <= check.agreement;
    Some((drift, curve))
}

/// 位相ずれ検査の設定．**[`GridParams`] から検査に要る分だけ取り出したもの．**
#[derive(Copy, Clone, Debug)]
struct DriftCheck {
    bands: usize,
    tolerance: f32,
    floor: f32,
    min_cells: usize,
    subpixel: bool,
    agreement: f32,
    require_measurable: bool,
}

impl From<&GridParams> for DriftCheck {
    fn from(p: &GridParams) -> Self {
        Self {
            bands: p.phase_bands,
            tolerance: p.phase_tolerance,
            floor: p.phase_tolerance_floor,
            min_cells: p.phase_min_cells,
            subpixel: p.phase_subpixel,
            agreement: p.phase_agreement,
            require_measurable: p.phase_require_measurable,
        }
    }
}

/// 帯ごとの位相の食い違い (画素)．検査を行わない場合は `None`．
///
/// **判定ではなく生の量を返す．** 閾値 $s \theta$ は $s$ に比例するので，小さい $s$ では
/// 1 画素未満になる — $s = 4$ ・$\theta = 1/6$ なら許容 0.67 画素であり，帯が 1 画素でも
/// 食い違えば落ちる．**補間や JPEG で 1 画素揺れるかどうか**が閾値の形と噛み合って
/// いるかは，判定の真偽ではなくこの量の分布を見ないと分からない．
fn drift_spread(
    it: &Integral,
    s: usize,
    d: IVec2,
    bands: usize,
    min_cells: usize,
) -> Option<usize> {
    let (by_x, by_y) = measure_bands(it, s, d, bands, min_cells)?;
    let ints = |v: &[(usize, f32)]| v.iter().map(|p| p.0).collect::<Vec<_>>();
    Some(cyclic_spread(&ints(&by_x), s).max(cyclic_spread(&ints(&by_y), s)))
}

/// 帯ごとの位相を整数と副画素の両方で測る．検査を行わない場合は `None`．
///
/// 帯を切る規則はここ 1 か所にまとめる — 判定 ・診断 ・副画素の 3 つで別々に書くと，
/// **帯の切り方が食い違ったまま数字だけ比べる**ことになる．
#[allow(clippy::type_complexity)]
fn measure_bands(
    it: &Integral,
    s: usize,
    d: IVec2,
    bands: usize,
    min_cells: usize,
) -> Option<(Vec<(usize, f32)>, Vec<(usize, f32)>)> {
    let curves = band_curves(it, s, d, bands, min_cells)?;
    let phases = |cs: &[Vec<f32>]| -> Option<Vec<(usize, f32)>> {
        cs.iter()
            .map(|c| argmin_phase(c).map(|p| (p, refine_phase(c, p))))
            .collect()
    };
    Some((phases(&curves[0])?, phases(&curves[1])?))
}

/// 帯ごとの位相曲線．`[x 方向の帯, y 方向の帯]`．
///
/// **帯の切り方はここ 1 か所にまとめる．** 帯ずれと曲線の食い違いは同じ帯 ・同じ
/// 曲線から出さないと，数字を並べて比べられない (どちらも同じ 1 回の走査で済む) ．
fn band_curves(
    it: &Integral,
    s: usize,
    d: IVec2,
    bands: usize,
    min_cells: usize,
) -> Option<[Vec<Vec<f32>>; 2]> {
    if bands < 2 {
        return None;
    }
    let (dx, dy) = (d.x.max(0) as usize, d.y.max(0) as usize);
    if it.w.saturating_sub(dx) / s < min_cells * bands
        || it.h.saturating_sub(dy) / s < min_cells * bands
    {
        return None;
    }

    let mut by_x = Vec::with_capacity(bands);
    let mut by_y = Vec::with_capacity(bands);
    for b in 0..bands {
        let (x0, x1) = (it.w * b / bands, it.w * (b + 1) / bands);
        let (y0, y1) = (it.h * b / bands, it.h * (b + 1) / bands);
        by_x.push(phase_curve(it, s, (x0, 0, x1, it.h), dy, true));
        by_y.push(phase_curve(it, s, (0, y0, it.w, y1), dx, false));
    }
    Some([by_x, by_y])
}

/// 適応帯 — 4 ・3 ・2 の順に，測れる本数で曲線を得る (D65)．
fn adaptive_curves(
    it: &Integral,
    s: usize,
    d: IVec2,
    bands: usize,
    min_cells: usize,
) -> Option<(usize, [Vec<Vec<f32>>; 2])> {
    (2..=bands).rev().find_map(|b| {
        let curves = band_curves(it, s, d, b, min_cells)?;
        // 位相が求まらない帯が 1 本でもあれば，その本数では測れないものとして扱う
        // (従来の `measure_bands` と同じ切り方にしておく)．**曲線は 1 度しか作らない**
        // — 判定は候補ごとに走るので，作り直すと推定の費用がそのまま倍になる
        curves
            .iter()
            .all(|cs| cs.iter().all(|c| argmin_phase(c).is_some()))
            .then_some((b, curves))
    })
}

/// 曲線から帯ずれ (帯ごとの argmin の食い違い) を出す．
fn drift_of(curves: &[Vec<Vec<f32>>; 2], s: usize, subpixel: bool) -> f32 {
    let axis = |cs: &Vec<Vec<f32>>| -> f32 {
        let ps: Vec<(usize, f32)> = cs
            .iter()
            .filter_map(|c| argmin_phase(c).map(|p| (p, refine_phase(c, p))))
            .collect();
        if subpixel {
            cyclic_spread_f(&ps.iter().map(|p| p.1).collect::<Vec<_>>(), s as f32)
        } else {
            cyclic_spread(&ps.iter().map(|p| p.0).collect::<Vec<_>>(), s) as f32
        }
    };
    // **軸は max のまま．** 平均にすると検証セットでは 2 件得をするが，片方の軸だけが
    // 非整数倍という入力を構造的に見逃す — 評価データセットのリサイズは等方なので
    // その失敗が測れない (D65 の «データセットが見ていない場面» と同じ形)
    axis(&curves[0]).max(axis(&curves[1]))
}

/// 曲線の食い違い — **共通の位相を 1 つ選ぶと，帯ごとに best を選ぶのに比べて
/// どれだけ損をするか**を谷の深さで正規化したもの．軸の平均を返す．
///
/// **谷が無い軸は棄権する** — 平らな曲線は「位相が合っていない」ことの証拠ではない．
/// 縦縞だけの絵では横方向の位相がどれも同点になるが，それは格子が偽物だからではない．
/// 両方の軸が平らなときだけ `None` (位相を選ぶ根拠がまったく無い) を返す．
fn agreement_of(curves: &[Vec<Vec<f32>>; 2]) -> Option<f32> {
    let mut acc = 0.0;
    let mut voted = 0;
    for cs in curves {
        let usable: Vec<&Vec<f32>> = cs
            .iter()
            .filter(|c| c.iter().all(|v| v.is_finite()))
            .collect();
        if usable.len() < 2 {
            continue;
        }
        let s = usable[0].len();
        let sum: Vec<f32> = (0..s)
            .map(|p| usable.iter().map(|c| c[p]).sum::<f32>())
            .collect();
        let joint = sum.iter().copied().fold(f32::INFINITY, f32::min);
        let separate: f32 = usable
            .iter()
            .map(|c| c.iter().copied().fold(f32::INFINITY, f32::min))
            .sum();
        let level = sum.iter().sum::<f32>() / s as f32;
        let depth = level - separate;
        if depth <= f32::EPSILON {
            continue;
        }
        acc += (joint - separate) / depth;
        voted += 1;
    }
    (voted > 0).then(|| acc / voted as f32)
}

/// 帯ごとの位相の食い違いを画素で返す (診断用)．検査を飛ばす場合は `None`．
pub fn phase_drift_spread(
    img: &RgbaCanvas,
    s: u32,
    phase: IVec2,
    bands: usize,
    min_cells: usize,
) -> Option<usize> {
    drift_spread(&Integral::new(img), s as usize, phase, bands, min_cells)
}

/// 帯ごとに最も合う位相そのもの (診断用)．検査を飛ばす場合は `None`．
///
/// **ずれの大きさだけでは $2 s_*$ と「雑音で揺れた真の $s$」を分けられない．**
/// 実測では真の $s$ の帯ずれが 0〜4 画素とばらつく一方，$2 s_*$ のずれは
/// **候補の半分ちょうど** — 帯ごとに「同じくらい正しい 2 つの位相」($d$ と $d + s/2$)
/// のどちらかを選ぶためである．**並びの形**を見るために生の値を返す．
pub fn band_phases(
    img: &RgbaCanvas,
    s: u32,
    phase: IVec2,
    bands: usize,
    min_cells: usize,
) -> Option<(Vec<usize>, Vec<usize>)> {
    let it = Integral::new(img);
    let (by_x, by_y) = measure_bands(&it, (s as usize).max(1), phase, bands, min_cells)?;
    Some((
        by_x.iter().map(|p| p.0).collect(),
        by_y.iter().map(|p| p.0).collect(),
    ))
}

/// 帯ごとの位相を**副画素**で返す (診断用)．検査を飛ばす場合は `None`．
///
/// 整数の位相では，通したい「補間で滲んだ本物の格子」(帯ずれ 0〜4 画素) と
/// 落としたい「非整数の周期」(帯あたり 0.6〜1.6 画素の流れ) が**量子化の刻みと同じ
/// 大きさで競っている**．刻みを細かくすれば分かれるか，を測るための口である．
pub fn band_phases_subpixel(
    img: &RgbaCanvas,
    s: u32,
    phase: IVec2,
    bands: usize,
    min_cells: usize,
) -> Option<(Vec<f32>, Vec<f32>)> {
    let it = Integral::new(img);
    let (by_x, by_y) = measure_bands(&it, (s as usize).max(1), phase, bands, min_cells)?;
    Some((
        by_x.iter().map(|p| p.1).collect(),
        by_y.iter().map(|p| p.1).collect(),
    ))
}

/// 帯ごとの位相曲線を**そのまま突き合わせた**ときの食い違い (診断用)．
///
/// **帯ごとの argmin は，谷が浅いと当てずっぽうになる．** 実測すると，落としたい候補の
/// 帯ずれは $s$ ごとにほぼ $\lfloor s/2 \rfloor$ — つまり**巡回距離の上限に張り付いて
/// いる**．argmin が無相関になれば必ずこの値が出るので，補間で滲んだ本物の格子も
/// 同じ値を取ってしまう．統計が飽和しているところに閾値を引いている以上，
/// **どこで切っても分かれない** (件ごとの閾値を許した上限で測っても
/// マクロ 74.4% → 75.7%) ．
///
/// そこで argmin を取らずに曲線どうしを比べる．「**全帯に共通の位相を 1 つ選ぶと，
/// 帯ごとに best を選ぶのに比べてどれだけ損をするか**」であれば，谷が浅い帯は
/// 分子にも分母にも小さくしか効かないので飽和しない．
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BandAgreement {
    /// 実際に使った帯の数．
    pub bands: usize,
    /// 共通の位相を選んだときの損 $J = \min_p \sum_b C_b(p)$ (`[x, y]`)．
    pub joint: [f32; 2],
    /// 帯ごとに best を選んだときの損 $M = \sum_b \min_p C_b(p)$ (`[x, y]`)．
    pub separate: [f32; 2],
    /// 位相を選ばなかったときの損 $A = \mathrm{mean}_p \sum_b C_b(p)$ (`[x, y]`)．
    /// **谷の深さの物差し**であり，正規化の分母に使う．
    pub level: [f32; 2],
}

/// 半セルずらしたときにセル内分散がどれだけ崩れるか (`[x, y]` の平均)．
///
/// **格子がそこに «在る» ことを確かめる量である．** ほかの関門はすべて «セルの中が
/// 揃っているか» を見るので，滑らかな絵では $s = 2$ のセルがどれも揃ってしまい，
/// 帯ずれ 0 ・曲線の食い違い 0 ・不一致率 0 で**全部を余裕ゼロで通る** (検証セットの
/// 誤受理 14 件のうち 7 件がこれ) ．位相を半セルずらすと，本物の格子はセル境界が
/// セルの真ん中へ来て崩れる一方，**滑らかな絵は何も変わらない** — 比が 1 に留まる．
///
/// 軸は**平均**を採る．$\min$ にすると «片方の軸だけ平坦な絵» (縦縞など) で比が 1 に
/// 張り付き，本物の格子を構造的に落とす — 曲線の検査で棄権させたのと同じ理由である．
/// 検証セットでの成績は $\min$ と同じ (どちらも比 1.14〜1.15 で最良) なので，
/// **数字で選べない以上は安全な側を採る**．
fn phase_contrast_ok(it: &Integral, s: usize, d: IVec2, min_ratio: f32) -> bool {
    if min_ratio <= 1.0 {
        return true;
    }
    let half = (s / 2).max(1);
    let (dx, dy) = (d.x.max(0) as usize % s, d.y.max(0) as usize % s);
    let Some(base) = mean_cell_variance(it, s, dx, dy) else {
        return true; // 測れない位相は落とす根拠にならない (ここは適用範囲の話である)
    };
    let mut acc = 0.0;
    for (sx, sy) in [((dx + half) % s, dy), (dx, (dy + half) % s)] {
        let Some(shifted) = mean_cell_variance(it, s, sx, sy) else {
            return true;
        };
        // 分母 0 は «完全な格子» — 崩れ方は無限大だが有限値で代表させる
        acc += if base <= f32::EPSILON {
            if shifted <= f32::EPSILON { 1.0 } else { 1.0e6 }
        } else {
            shifted / base
        };
    }
    acc / 2.0 >= min_ratio
}

/// 帯ごとの位相曲線の食い違いを測る (診断用)．帯が足りなければ `None`．
pub fn band_agreement(
    img: &RgbaCanvas,
    s: u32,
    phase: IVec2,
    bands: usize,
    min_cells: usize,
) -> Option<BandAgreement> {
    let it = Integral::new(img);
    let (b, curves) = adaptive_curves(&it, (s as usize).max(1), phase, bands, min_cells)?;

    let mut out = BandAgreement {
        bands: b,
        joint: [0.0; 2],
        separate: [0.0; 2],
        level: [0.0; 2],
    };
    for (axis, cs) in curves.iter().enumerate() {
        // 測れない位相がある帯は使わない (無限大を足すと全部が無限大になる)
        let usable: Vec<&Vec<f32>> = cs
            .iter()
            .filter(|c| c.iter().all(|v| v.is_finite()))
            .collect();
        if usable.len() < 2 {
            return None;
        }
        let n = usable[0].len();
        let sum: Vec<f32> = (0..n)
            .map(|p| usable.iter().map(|c| c[p]).sum::<f32>())
            .collect();
        out.joint[axis] = sum.iter().copied().fold(f32::INFINITY, f32::min);
        out.separate[axis] = usable
            .iter()
            .map(|c| c.iter().copied().fold(f32::INFINITY, f32::min))
            .sum();
        out.level[axis] = sum.iter().sum::<f32>() / n as f32;
    }
    Some(out)
}

/// 帯ごとの位相の食い違いを**副画素**で返す (診断用)．
pub fn phase_drift_spread_subpixel(
    img: &RgbaCanvas,
    s: u32,
    phase: IVec2,
    bands: usize,
    min_cells: usize,
) -> Option<f32> {
    let (by_x, by_y) = band_phases_subpixel(img, s, phase, bands, min_cells)?;
    let su = s.max(1) as f32;
    Some(cyclic_spread_f(&by_x, su).max(cyclic_spread_f(&by_y, su)))
}

/// セルを 4 分割したときに説明できる分散の割合 (診断用)．**倍数の抑止だけを担う量**．
///
/// $$ \mathrm{gain}(s, d) = 1 - \frac{\bar{V}_{4}(s, d)}{\bar{V}(s, d)} $$
///
/// $\bar{V}_4$ はセルを 4 つの象限に割ったときの分散の平均である．
///
/// - 真の $s$ — セルはもともと 1 色なので，割っても説明できる分散が無い．**0 に近い**
/// - $\hat{s} = 2 s_*$ — セルは真のセル 4 つでできているので，割ると**ほぼ全部**が
///   説明される．**1 に近い**
/// - 約数 $s_* / 2$ — セルは 1 色のままなので真の $s$ と同じ側に立つ．
///   **止める必要が無い** (「閾値を満たす最大の $s$」の規則が落とす)
///
/// 現行の再構成検査が「セル平均との色差が $\delta$ を超えた画素の割合」という
/// **絵の中身に依存する量**を見ているのに対し，これは同じセルの中で 2 度測った比なので
/// **補間の滲みが分子と分母で相殺する**．狙いは費用対効果 1 : 1.1 の解消である
/// (格子なしを 23 件落とすために真の $s$ を 26 件失っている) ．
pub fn split_gain(img: &RgbaCanvas, s: u32, phase: IVec2) -> f32 {
    let it = Integral::new(img);
    let su = (s as usize).max(1);
    let (dx, dy) = (phase.x.max(0) as usize, phase.y.max(0) as usize);
    // 1 画素のセルは割れない
    if su < 2 {
        return 0.0;
    }
    let half = su / 2;

    let (mut whole, mut split, mut count) = (0.0f64, 0.0f64, 0usize);
    for (x, y) in cells(it.w, it.h, su, dx, dy) {
        whole += it.variance(x, y, x + su, y + su);
        // 象限は面積で重みを付けずに平均する — 奇数の $s$ では大きさが揃わないが，
        // 倍数の候補は必ず偶数なので判定に効く場面では等分になる
        let q = [
            it.variance(x, y, x + half, y + half),
            it.variance(x + half, y, x + su, y + half),
            it.variance(x, y + half, x + half, y + su),
            it.variance(x + half, y + half, x + su, y + su),
        ];
        split += q.iter().sum::<f64>() / 4.0;
        count += 1;
    }
    if count == 0 || whole <= f64::EPSILON {
        return 0.0;
    }
    (1.0 - split / whole).clamp(0.0, 1.0) as f32
}

/// セルを 4 分割したときに**不一致率**がどれだけ下がるか (診断用)．
///
/// $$ 1 - \frac{\mathrm{rate}_{4}(s, d)}{\mathrm{rate}(s, d)} $$
///
/// [`split_gain`] の «生の分散» 版は補間の掛かった入力で外れた — 真のセルも勾配を
/// 持つので，割れば説明されてしまう (真の $s$ で 0.61〜0.73 ・$2 s_*$ で 0.59〜0.67) ．
/// **現行の再構成検査が強いのは $\delta$ の閾値がある**からで，滲み程度のずれは
/// 不一致に数えない．そこで**閾値を残したまま相対化する**．
///
/// 真の $s$ なら象限平均にしても不一致は大きく減らない (もともと 1 色) ．
/// $2 s_*$ なら象限が真のセルそのものなので不一致がほぼ消える．
pub fn split_recon_gain(img: &RgbaCanvas, s: u32, phase: IVec2, delta: f32) -> f32 {
    let it = Integral::new(img);
    let su = (s as usize).max(1);
    if su < 2 {
        return 0.0;
    }
    let (dx, dy) = (phase.x.max(0) as usize, phase.y.max(0) as usize);
    let half = su / 2;
    let (mut bad_cell, mut bad_quad, mut total) = (0usize, 0usize, 0usize);

    for (x, y) in cells(it.w, it.h, su, dx, dy) {
        let mean = oklab_of(it.mean(x, y, x + su, y + su));
        // 象限ごとの平均を先に求める
        let quad = [
            oklab_of(it.mean(x, y, x + half, y + half)),
            oklab_of(it.mean(x + half, y, x + su, y + half)),
            oklab_of(it.mean(x, y + half, x + half, y + su)),
            oklab_of(it.mean(x + half, y + half, x + su, y + su)),
        ];
        for j in 0..su {
            for i in 0..su {
                let Some(c) = img.get((x + i) as i32, (y + j) as i32) else {
                    continue;
                };
                let lab = oklab_of(c);
                total += 1;
                if delta_e(lab, mean) > delta {
                    bad_cell += 1;
                }
                let q = quad[usize::from(i >= half) + 2 * usize::from(j >= half)];
                if delta_e(lab, q) > delta {
                    bad_quad += 1;
                }
            }
        }
    }
    if total == 0 || bad_cell == 0 {
        return 0.0;
    }
    (1.0 - bad_quad as f32 / bad_cell as f32).clamp(0.0, 1.0)
}

/// 再構成の不一致画素率．完全なセルが 1 つも無ければ `None`．
fn recon_rate(img: &RgbaCanvas, it: &Integral, s: usize, d: IVec2, delta: f32) -> Option<f32> {
    let (dx, dy) = (d.x.max(0) as usize, d.y.max(0) as usize);
    let mut mismatched = 0usize;
    let mut total = 0usize;

    for (x, y) in cells(it.w, it.h, s, dx, dy) {
        let mean = oklab_of(it.mean(x, y, x + s, y + s));
        for j in 0..s {
            for i in 0..s {
                let Some(c) = img.get((x + i) as i32, (y + j) as i32) else {
                    continue;
                };
                total += 1;
                if delta_e(oklab_of(c), mean) > delta {
                    mismatched += 1;
                }
            }
        }
    }
    (total > 0).then(|| mismatched as f32 / total as f32)
}

/// 再構成誤差の判定 — 画素色差 $\delta$ を超える画素の割合が $\tau$ 以下か．
fn recon_ok(img: &RgbaCanvas, it: &Integral, s: usize, d: IVec2, delta: f32, tau: f32) -> bool {
    recon_rate(img, it, s, d, delta).is_some_and(|r| r <= tau)
}

/// 位相を半セルずらしたときに当てはまりがどれだけ崩れるか (診断用)．
///
/// **これは $s_*$ と $2 s_*$ を分けるために測る量である．**
///
/// 本物の格子を半セルずらすと，セルが境界をまたいで当てはまりが崩れる．ところが
/// $\hat{s} = 2 s_*$ を $s_* $ だけずらしても，**ずらした先がまた真のセル境界**なので
/// 崩れない — 真のセルを 4 つ束ねる組み方が変わるだけである．約数 $s_*/2$ は
/// 半セル ($s_*/4$) ずらせば崩れるので，本物と同じ側に立つ (約数は「閾値を満たす最大の
/// $s$」の規則が落とすので，止める必要が無い)．
///
/// 絵の中身に依らないだけでなく，**同じ画像 ・同じ $s$ で 2 度測った比**なので
/// 補間の滲みが分子と分母で相殺する．再構成検査の閾値が画像ごとに動いてしまう
/// (nearest の真の $s$ で 0.00 ・lanczos の真の $s$ で 0.13) 弱点をここで避けられる．
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PhaseContrast {
    /// $(V(s, d + s/2) - V(s, d)) / \bar{V}_{\mathrm{image}}$ (`[x 方向, y 方向]`)．
    pub variance_margin: [f32; 2],
    /// $V(s, d + s/2) / V(s, d)$ (`[x, y]`)．分母が 0 のときは大きな有限値を返す．
    pub variance_ratio: [f32; 2],
    /// 再構成の不一致率の比 (`[x, y]`)．
    pub recon_ratio: [f32; 2],
}

/// 半セルずらした位相での当てはまりを測る (診断用)．推定そのものには使わない．
pub fn phase_contrast(img: &RgbaCanvas, s: u32, phase: IVec2, delta: f32) -> PhaseContrast {
    let it = Integral::new(img);
    let su = (s as usize).max(1);
    let half = (su / 2).max(1);
    let (dx, dy) = (phase.x.max(0) as usize, phase.y.max(0) as usize);
    let image_var = image_variance(&it).max(f32::MIN_POSITIVE);

    let base_v = mean_cell_variance(&it, su, dx % su, dy % su);
    let base_r = recon_rate(img, &it, su, phase, delta);
    // 比の分母が 0 のときの代わり．**無限大を返さない** — CSV と比較の両方で扱いに困る
    let ratio = |shifted: f32, base: f32| -> f32 {
        if base <= f32::EPSILON {
            if shifted <= f32::EPSILON { 1.0 } else { 1.0e6 }
        } else {
            shifted / base
        }
    };

    let mut out = PhaseContrast {
        variance_margin: [0.0; 2],
        variance_ratio: [1.0; 2],
        recon_ratio: [1.0; 2],
    };
    for (axis, shift) in [
        ivec2(((dx + half) % su) as i32, dy as i32),
        ivec2(dx as i32, ((dy + half) % su) as i32),
    ]
    .into_iter()
    .enumerate()
    {
        let shifted_v = mean_cell_variance(&it, su, shift.x as usize, shift.y as usize);
        if let (Some(a), Some(b)) = (shifted_v, base_v) {
            out.variance_margin[axis] = (a - b) / image_var;
            out.variance_ratio[axis] = ratio(a, b);
        }
        if let (Some(a), Some(b)) = (recon_rate(img, &it, su, shift, delta), base_r) {
            out.recon_ratio[axis] = ratio(a, b);
        }
    }
    out
}

/// 再構成誤差の内訳 (診断用)．
///
/// 現行の判定は画像全体で「色差が $\delta$ を超えた画素の割合」を 1 つ見るだけである．
/// 実データで測ると**誤棄却の主犯がこの検査**で，落ちるのは補間が掛かった入力に限られる．
///
/// 補間の滲みは**セルの境界に集中する**はずで，中まで滲むわけではない．本物の格子なら
/// 内側は平坦なまま残り，偽物なら内側も一様に汚れる — そこが分かれるかを見るための型
/// である．推定そのものには使わない．
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ReconStats {
    /// 全画素の不一致率 (現行の判定が見ている量)．
    pub overall: f32,
    /// **セルの内側**だけの不一致率 ($s \ge 3$．$s = 2$ では内側が無いので `overall`)．
    pub interior: f32,
    /// セルの外周 1 画素だけの不一致率．
    pub border: f32,
    /// 色差の中央値 (閾値を通さない生の量)．
    pub median_delta_e: f32,
    /// 内側だけの色差の中央値．
    pub interior_median_delta_e: f32,
}

/// 再構成誤差を内側と境界に分けて測る (診断用)．
pub fn recon_stats(img: &RgbaCanvas, s: u32, phase: IVec2, delta: f32) -> ReconStats {
    let it = Integral::new(img);
    let (su, dx, dy) = (s as usize, phase.x.max(0) as usize, phase.y.max(0) as usize);
    let (mut all, mut inner) = (Vec::new(), Vec::new());
    let (mut all_bad, mut inner_bad, mut edge_bad, mut edge_n) = (0usize, 0usize, 0usize, 0usize);

    for (x, y) in cells(it.w, it.h, su, dx, dy) {
        let mean = oklab_of(it.mean(x, y, x + su, y + su));
        for j in 0..su {
            for i in 0..su {
                let Some(c) = img.get((x + i) as i32, (y + j) as i32) else {
                    continue;
                };
                let de = delta_e(oklab_of(c), mean);
                let bad = de > delta;
                all.push(de);
                all_bad += usize::from(bad);
                // 外周 1 画素かどうか．s <= 2 では内側が存在しない
                let is_edge = su <= 2 || i == 0 || j == 0 || i + 1 == su || j + 1 == su;
                if is_edge {
                    edge_n += 1;
                    edge_bad += usize::from(bad);
                } else {
                    inner.push(de);
                    inner_bad += usize::from(bad);
                }
            }
        }
    }

    let rate = |n: usize, total: usize| {
        if total == 0 {
            0.0
        } else {
            n as f32 / total as f32
        }
    };
    let median = |v: &mut Vec<f32>| {
        if v.is_empty() {
            return 0.0;
        }
        v.sort_by(f32::total_cmp);
        v[v.len() / 2]
    };
    let overall = rate(all_bad, all.len());
    ReconStats {
        overall,
        interior: if inner.is_empty() {
            overall
        } else {
            rate(inner_bad, inner.len())
        },
        border: rate(edge_bad, edge_n),
        median_delta_e: median(&mut all),
        interior_median_delta_e: if inner.is_empty() {
            median(&mut all)
        } else {
            median(&mut inner)
        },
    }
}

/// 差分エネルギーの折り畳み (診断用)．
///
/// **$s_*$ と $2 s_*$ を分ける情報はセル平均の残差に無い** — 再構成に基づく単一統計は
/// 均衡正解率 77% 前後で頭打ちだった (`docs/investigations/grid-calibration.md`) ．
/// セル平均は「セルの中がどれだけ平坦か」しか見ないが，$2 s_*$ のセルは平坦なまま
/// **中に境界を 1 本隠している**．そこを直接見る．
///
/// 列ごとの差分エネルギーを候補の周期で折り畳むと，本物の格子では山が 1 本だけ立つ．
/// $\hat{s} = m s_*$ なら山は $m$ 本になり，折り畳んだ形が周期 $s / m$ で繰り返す —
/// **絵に何が描いてあるかには依らない**性質で，位相ずれ検査 (D62) と同じ筋である．
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ProfileStats {
    /// 格子線の帯に乗った 1 階差分エネルギーの割合 (`[x, y]`)．
    ///
    /// 本物の格子なら段差は格子線に集まる．ただし補間で縁が引き伸ばされると
    /// セル全体へ散るので，**$s$ の約数でも同じ値になる** (約数の格子線は真の格子線を
    /// すべて含む) ．過大推定でだけ下がる量である．
    pub edge_share: [f32; 2],
    /// 折り畳んだ形が $s$ の約数周期で繰り返す度合い (1 階差分，`[x, y]`)．
    ///
    /// $s$ の真の約数 $q \ge 2$ すべてについて巡回相関を取り，その**最大**を採る．
    /// $\hat{s} = m s_*$ なら $q = s / m$ で 1 に近づき，本物の格子では 0 以下に落ちる．
    pub echo1: [f32; 2],
    /// 同じものを 2 階差分で測る (`[x, y]`)．
    ///
    /// 補間で拡大した画像の縁は段差ではなく**傾きの折れ**になる — bilinear なら値は
    /// セル中心の間を直線で結ぶので，1 階差分は区間ごとに一定になって折り畳むと平らに
    /// 潰れる．2 階差分なら折れ点に山が立つ．
    pub echo2: [f32; 2],
    /// 折り畳んだ形の起伏 = 標準偏差 / 平均 (1 階差分，`[x, y]`)．
    ///
    /// 平らな形の相関は当てにならない．**相関を信じてよいかどうか**をこれで見る．
    pub relief1: [f32; 2],
    /// 同じく 2 階差分の起伏 (`[x, y]`)．
    pub relief2: [f32; 2],
}

/// 列 (`along_x`) または行ごとの差分エネルギー．`order` は差分の階数 (1 か 2)．
///
/// 返り値は画像の幅 (または高さ) と同じ長さで，**定義できない端は `None`** である．
/// 端を 0 で埋めると折り畳みの特定の帯だけが薄まり，山が動いてしまう．
fn difference_energy(img: &RgbaCanvas, along_x: bool, order: u32) -> Vec<Option<f64>> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let (n, m) = if along_x { (w, h) } else { (h, w) };
    let at = |i: usize, j: usize| -> Rgba8 {
        if along_x {
            img.pixels()[j * w + i]
        } else {
            img.pixels()[i * w + j]
        }
    };

    let mut out = vec![None; n];
    let (lo, hi) = if order >= 2 { (1, n - 1) } else { (1, n) };
    for (i, slot) in out.iter_mut().enumerate().take(hi).skip(lo) {
        let mut acc = 0.0f64;
        for j in 0..m {
            let (a, b) = (at(i, j), at(i - 1, j));
            acc += if order >= 2 {
                let c = at(i + 1, j);
                let d =
                    |x: u8, y: u8, z: u8| (f64::from(z) - 2.0 * f64::from(x) + f64::from(y)).abs();
                d(a.r, b.r, c.r) + d(a.g, b.g, c.g) + d(a.b, b.b, c.b)
            } else {
                let d = |x: u8, y: u8| (f64::from(x) - f64::from(y)).abs();
                d(a.r, b.r) + d(a.g, b.g) + d(a.b, b.b)
            };
        }
        *slot = Some(acc);
    }
    out
}

/// 差分エネルギーを周期 $s$ で折り畳む．帯 $k$ は添字 $i \equiv d + k \pmod s$ の平均．
///
/// 帯 0 が格子線である ([`cells`] がセルを $d$ から始めるので，セルの境目の差分は
/// 添字 $d + i s$ に来る) ．
fn fold_profile(energy: &[Option<f64>], s: usize, phase: usize) -> Vec<f64> {
    let mut acc = vec![0.0f64; s];
    let mut count = vec![0usize; s];
    for (i, value) in energy.iter().enumerate() {
        let Some(v) = value else { continue };
        let k = (i + s - phase % s) % s;
        acc[k] += v;
        count[k] += 1;
    }
    acc.iter()
        .zip(&count)
        .map(|(a, n)| if *n == 0 { 0.0 } else { a / *n as f64 })
        .collect()
}

/// 折り畳んだ形が $s$ の真の約数周期で繰り返す度合い．**約数ごとの巡回相関の最大**．
///
/// $q = s / m$ だけ回して形が変わらないなら，山が $m$ 本並んでいる — すなわち
/// $\hat{s}$ は真の周期の $m$ 倍である．$q = 1$ は「形が平ら」という別のことなので
/// 見ない ($q \ge 2$ に限る) ．約数が無い $s$ (2 と素数) では 0 を返す．
fn periodic_echo(p: &[f64]) -> f32 {
    let s = p.len();
    let mean = p.iter().sum::<f64>() / s as f64;
    let var: f64 = p.iter().map(|v| (v - mean) * (v - mean)).sum();
    if var <= f64::EPSILON {
        return 0.0;
    }
    let mut best = f32::NEG_INFINITY;
    for q in 2..s {
        if !s.is_multiple_of(q) {
            continue;
        }
        let acc: f64 = (0..s)
            .map(|k| (p[k] - mean) * (p[(k + q) % s] - mean))
            .sum();
        best = best.max((acc / var) as f32);
    }
    if best.is_finite() { best } else { 0.0 }
}

/// 折り畳んだ形の起伏 (標準偏差 / 平均)．
fn relief(p: &[f64]) -> f32 {
    let s = p.len();
    let mean = p.iter().sum::<f64>() / s as f64;
    if mean <= f64::EPSILON {
        return 0.0;
    }
    let var: f64 = p.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / s as f64;
    (var.sqrt() / mean) as f32
}

/// 格子線の帯に乗った割合 $p_0 / \sum_k p_k$．
fn share_at_zero(p: &[f64]) -> f32 {
    let total: f64 = p.iter().sum();
    if total <= f64::EPSILON {
        return 0.0;
    }
    (p[0] / total) as f32
}

/// 差分エネルギーの折り畳みを測る (診断用)．推定そのものには使わない．
pub fn profile_stats(img: &RgbaCanvas, s: u32, phase: IVec2) -> ProfileStats {
    let su = (s as usize).max(1);
    let d = [phase.x.max(0) as usize, phase.y.max(0) as usize];
    let mut out = ProfileStats {
        edge_share: [0.0; 2],
        echo1: [0.0; 2],
        echo2: [0.0; 2],
        relief1: [0.0; 2],
        relief2: [0.0; 2],
    };
    for (axis, offset) in d.into_iter().enumerate() {
        let along_x = axis == 0;
        let p1 = fold_profile(&difference_energy(img, along_x, 1), su, offset);
        let p2 = fold_profile(&difference_energy(img, along_x, 2), su, offset);
        out.edge_share[axis] = share_at_zero(&p1);
        out.echo1[axis] = periodic_echo(&p1);
        out.echo2[axis] = periodic_echo(&p2);
        out.relief1[axis] = relief(&p1);
        out.relief2[axis] = relief(&p2);
    }
    out
}

/// セル境界の位置に直線を当てたときの当てはまり (診断用)．推定には使わない．
///
/// **測る対象を «位相» から «セル境界そのものの位置» へ移した量である．**
///
/// 帯ごとの位相は «セル内平均分散を最小にする位相» を離散の $s$ 通りから選ぶが，
/// 補間で谷が浅くなると 1 画素ずれた位相と区別が付かなくなる — 落としたい候補の
/// 帯ずれは $\lfloor s/2 \rfloor$ に張り付き (巡回距離の上限) ，**滲んだ本物の格子も
/// 同じ値を取る**．失敗は argmin の雑音であって，同じ量の上ではどこで切っても動かない．
///
/// 差分エネルギーの極大は事情が違う．
///
/// | | 帯の位相 | 境界の位置 |
/// | --- | --- | --- |
/// | 標本数 | 帯の数 = 4 | 境界の数 = 幅 / $s$ (数十) |
/// | 1 標本 | 浅い谷の argmin (離散) | 鋭い峰の頂点 (副画素) |
/// | 補間の効き方 | 谷が浅くなり argmin が飛ぶ | **峰が広がるだけで頂点は動かない** |
/// | 位置の情報 | 巡回距離に潰す (上限 $s/2$ で飽和) | 直線の当てはめでそのまま使う |
///
/// **対称な暈けは峰の位置を動かさない** — ここが «補間で滲んだ本物の格子» に効く
/// 見込みの根拠である．非整数の周期なら，当てはめた間隔が $s$ から離れる．
///
/// > [!note] 折り畳み ([`ProfileStats`]) とは別物である
/// > あちらは $s$ で畳んで**位置の情報を捨てて**いた．ここでは畳まずに $b_k$ の
/// > **並び**を使う．
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct EdgeFit {
    /// 拾えた境界の数 (`[x, y]`)．
    pub count: [usize; 2],
    /// 期待される本数 (幅 / $s$) に対する割合 (`[x, y]`)．
    ///
    /// **平坦な絵では境界が拾えない．** 本物の格子でも全部の境目で色が変わるわけでは
    /// ないので 1 にはならないが，これが小さい候補は «測れない» 側である．
    pub coverage: [f32; 2],
    /// 当てた直線からの残差の RMS を $s$ で割ったもの (`[x, y]`)．測れなければ `None`．
    pub residual: [Option<f32>; 2],
    /// 当てはめた間隔と $s$ のずれ $(\hat{s}_{\mathrm{fit}} - s) / s$ (`[x, y]`)．
    pub slope: [Option<f32>; 2],
    /// **残差の中央絶対値** ($s$ で正規化．`[x, y]`)．
    ///
    /// RMS は**外れの峰 1 本で跳ねる** — 絵の中身が作る偽の峰が 1 つ混じるだけで
    /// 二乗が効く．実データの正例で残差が 0.9 まで伸びるのはこれが主因と見て，
    /// 外れに鈍い形も測れるようにしておく．
    pub residual_median: [Option<f32>; 2],
    /// **峰を $s$ で畳んだときの散らばり** (円周上の中央絶対偏差．$s$ で正規化．`[x, y]`)．
    ///
    /// 直線を当てるには峰へ添字 $k$ を振る必要があり，間隔が $1.5 s$ 付近だと丸めが
    /// 滑って**以降の添字がすべてずれる**．畳んでしまえば添字が要らない — 本物の
    /// 格子なら峰は $s$ を法として 1 点に集まり，偽の峰は散る．
    ///
    /// > [!note] 折り畳み (`fold_profile`) とは別物である
    /// > あれはエネルギーそのものを畳んで**位置情報を捨てて**いた．ここで畳むのは
    /// > **拾った峰の位置**であり，散らばりを見るために畳んでいる．
    pub residual_folded: [Option<f32>; 2],
}

/// 差分エネルギーの極大点を副画素で拾う．
///
/// 非極大抑制の窓は $s/2$ — 補間で境界が 2 画素に広がっても峰を 2 つ数えないためである．
/// 下限は平均エネルギー (絵の中身に依らない尺度が無いので，画像自身の平均を使う)．
/// **同点の平坦部は左端を採る** (設計書 6.15 規則 2) ．bilinear の 1 階差分は
/// セル中心の間で平らになるが，左端を採れば間隔は $s$ のままで，ずれは切片が吸収する．
fn energy_peaks(energy: &[Option<f64>], s: usize, suppression: f32, floor_scale: f32) -> Vec<f32> {
    let r = ((s as f32 * suppression).round() as usize).max(1);
    let defined: Vec<f64> = energy.iter().flatten().copied().collect();
    if defined.is_empty() {
        return Vec::new();
    }
    let floor = defined.iter().sum::<f64>() / defined.len() as f64 * f64::from(floor_scale);

    let mut out = Vec::new();
    for (i, slot) in energy.iter().enumerate() {
        let Some(v) = *slot else { continue };
        // **平坦なところに峰は無い．** 等号を含めると，どこも同じエネルギー
        // (平坦な絵 ・一様な勾配) でも先頭が «峰» として拾われてしまう
        if v <= floor {
            continue;
        }
        let lo = i.saturating_sub(r);
        let hi = (i + r).min(energy.len() - 1);
        let peak = (lo..=hi).all(|j| match energy[j] {
            Some(u) if j < i => u < v,
            Some(u) if j > i => u <= v,
            _ => true,
        });
        if peak {
            out.push(refine_peak(energy, i));
        }
    }
    out
}

/// 峰の周りに放物線を当てて位置を**副画素**にする．谷になっていなければ整数のまま．
fn refine_peak(energy: &[Option<f64>], i: usize) -> f32 {
    if i == 0 || i + 1 >= energy.len() {
        return i as f32;
    }
    let (Some(l), Some(c), Some(r)) = (energy[i - 1], energy[i], energy[i + 1]) else {
        return i as f32;
    };
    let denom = l - 2.0 * c + r;
    if denom >= 0.0 {
        return i as f32;
    }
    (i as f64 + (0.5 * (l - r) / denom).clamp(-0.5, 0.5)) as f32
}

/// 境界の並びに直線 $b_k = a + k b$ を当てる．返すのは (残差の RMS, 間隔 $b$)．
///
/// **添字 $k$ は隣どうしの間隔から積む．** 位置から直接 $\mathrm{round}((p - d)/s)$ と
/// 求めると，非整数の周期でずれが溜まって途中で添字が 1 つ飛び，«間隔» と «残差» の
/// 両方に中途半端に現れる (周期 5.3 を $s = 5$ で読むと 9 本目で半分を超える) ．
/// 間隔から積めば，1 本の飛びは 1 か所の判定にしか効かないので，
/// **«間隔が 5.3 である» という 1 つの読み方**に落ちる．
///
/// 間隔は最低 1 セルとする．非極大抑制の窓が $s/2$ なので峰は $s/2$ より近づかず，
/// 0 セル (同じ境界を 2 度数える) は起こらない — 起きたとすれば絵の側の縞である．
/// 直線 $b_k = a + k s$ の当てはまり．
struct SpacingFit {
    /// 残差の RMS (画素)．
    rms: f32,
    /// 残差の中央絶対値 (画素)．**外れの峰 1 本で跳ねない．**
    median: f32,
    /// 当てはめた間隔 (画素)．
    spacing: f32,
}

fn fit_spacing(peaks: &[f32], s: f32) -> Option<SpacingFit> {
    if peaks.len() < 3 {
        return None;
    }
    let mut ks = Vec::with_capacity(peaks.len());
    let mut k = 0.0f32;
    ks.push(k);
    for w in peaks.windows(2) {
        k += (((w[1] - w[0]) / s).round()).max(1.0);
        ks.push(k);
    }

    let n = peaks.len() as f32;
    let mk = ks.iter().sum::<f32>() / n;
    let mp = peaks.iter().sum::<f32>() / n;
    let sxx: f32 = ks.iter().map(|k| (k - mk) * (k - mk)).sum();
    if sxx <= f32::EPSILON {
        return None;
    }
    let b: f32 = ks
        .iter()
        .zip(peaks)
        .map(|(k, p)| (k - mk) * (p - mp))
        .sum::<f32>()
        / sxx;
    if !b.is_finite() || b <= 0.0 {
        return None;
    }
    let a = mp - b * mk;
    let mut errors: Vec<f32> = ks
        .iter()
        .zip(peaks)
        .map(|(k, p)| (p - (a + b * k)).abs())
        .collect();
    let rss: f32 = errors.iter().map(|e| e * e).sum();
    errors.sort_by(f32::total_cmp);
    Some(SpacingFit {
        rms: (rss / n).sqrt(),
        median: errors[errors.len() / 2],
        spacing: b,
    })
}

/// セル境界の位置に直線を当てて測る (診断用)．`order` は差分の階数 (1 か 2)．
///
/// 階数を選べるようにしてあるのは，**補間で «境界» の現れ方が変わる**ためである．
/// nearest なら 1 階差分が境界で尖るが，bilinear はセル中心の間を直線で結ぶので
/// 1 階差分が区間ごとに平らになり，尖るのは 2 階差分の方である．どちらで拾うのが
/// 良いかは測ってから決める．
pub fn edge_fit(img: &RgbaCanvas, s: u32, order: u32, params: &GridParams) -> EdgeFit {
    edge_fit_of(&axis_energies(img, order), s, params)
}

/// 軸ごとの差分エネルギー．**$s$ に依らないので候補ごとに作り直さない．**
///
/// 作り直すと候補の数だけ画像を走査することになる (再構成検査と同じ費用が
/// もう 1 つ増える) ．帯の曲線を 1 か所にまとめたのと同じ理由である．
fn axis_energies(img: &RgbaCanvas, order: u32) -> [Vec<Option<f64>>; 2] {
    [
        difference_energy(img, true, order),
        difference_energy(img, false, order),
    ]
}

/// 用意した差分エネルギーから当てはめる．
fn edge_fit_of(energies: &[Vec<Option<f64>>; 2], s: u32, params: &GridParams) -> EdgeFit {
    let su = (s as usize).max(1);
    let mut out = EdgeFit {
        count: [0; 2],
        coverage: [0.0; 2],
        residual: [None; 2],
        slope: [None; 2],
        residual_median: [None; 2],
        residual_folded: [None; 2],
    };
    for (axis, energy) in energies.iter().enumerate() {
        let peaks = energy_peaks(energy, su, params.peak_suppression, params.peak_floor);
        out.count[axis] = peaks.len();
        out.coverage[axis] = peaks.len() as f32 / (energy.len() / su).max(1) as f32;
        if let Some(fit) = fit_spacing(&peaks, su as f32) {
            out.residual[axis] = Some(fit.rms / su as f32);
            out.residual_median[axis] = Some(fit.median / su as f32);
            out.slope[axis] = Some((fit.spacing - su as f32) / su as f32);
        }
        out.residual_folded[axis] = folded_spread(&peaks, su as f32).map(|v| v / su as f32);
    }
    out
}

/// **峰を $s$ で畳んだときの円周上の散らばり (中央絶対偏差)．**
///
/// 添字を振らないので，間隔の丸めが滑っても壊れない．中央値で測るので外れの峰にも鈍い．
fn folded_spread(peaks: &[f32], s: f32) -> Option<f32> {
    if peaks.len() < 3 || s <= 0.0 {
        return None;
    }
    let phases: Vec<f32> = peaks.iter().map(|p| p.rem_euclid(s)).collect();
    // 円周上の中心は «どこを起点に測るか» で変わるので，峰そのものを起点に総当たりする
    // (標本は数十なので費用は問題にならない)
    let mut best = f32::INFINITY;
    for &origin in &phases {
        let mut d: Vec<f32> = phases
            .iter()
            .map(|p| {
                let raw = (p - origin).rem_euclid(s);
                raw.min(s - raw)
            })
            .collect();
        d.sort_by(f32::total_cmp);
        let mad = d[d.len() / 2];
        best = best.min(mad);
    }
    best.is_finite().then_some(best)
}

/// **境界の当てはめが «真の $s$ である» と言えるか (D71)．**
///
/// これは位相の検査を**肩代わりする**関門である — 通す側にしか働かない．
/// 帯ずれと曲線が落とした候補でも，境界の位置が $b_k = d + k s$ の直線に乗るなら通す．
///
/// > **落とす側を兼ねさせない．** $2 s_*$ の抑止は再構成検査と半セルずらし (D69) の
/// > 仕事であり，こちらに背負わせると D68 で飽和した量と同じ罠に入る．
///
/// 測れない候補 (境界が足りない ・直線を当てられない) は**肩代わりしない**．
/// 平坦な絵で境界が拾えないことは «格子がある» ことの根拠にならない．
fn edge_fit_ok(energies: &[Vec<Option<f64>>; 2], s: u32, params: &GridParams) -> bool {
    edge_fit_within(energies, s, params, params.edge_fit_residual)
}

/// **曲線の検査も肩代わりしてよいか (D73)．**
///
/// 帯ずれの肩代わりより**厳しい残差**を課す ([`GridParams::edge_fit_curve_residual`]) ．
/// 曲線は «棄却を引き受ける» ために入れた量なので，帯ずれと同じ緩さで手放すと
/// 取り戻した完全一致と同じだけ誤受理が戻る．
fn edge_fit_rescues_curve(energies: &[Vec<Option<f64>>; 2], s: u32, params: &GridParams) -> bool {
    params
        .edge_fit_curve_residual
        .is_some_and(|r| edge_fit_within(energies, s, params, r))
}

fn edge_fit_within(
    energies: &[Vec<Option<f64>>; 2],
    s: u32,
    params: &GridParams,
    residual_max: f32,
) -> bool {
    if params.edge_fit_order == 0 {
        return false;
    }
    let fit = edge_fit_of(energies, s, params);
    (0..2).all(|axis| {
        fit.count[axis] >= params.edge_fit_min_count
            && matches!(fit.slope[axis], Some(v) if v.abs() <= params.edge_fit_slope)
            && matches!(fit.residual[axis], Some(v) if v <= residual_max)
    })
}

fn divisors_and_multiples(s: u32, max: u32) -> impl Fn(u32) -> bool {
    move |t: u32| t != 0 && (s.is_multiple_of(t) || (t.is_multiple_of(s) && t <= max))
}

/// 画像全体の分散 $\bar{V}_{\mathrm{image}}$．信頼度の正規化に使う．
fn image_variance(it: &Integral) -> f32 {
    it.variance(0, 0, it.w, it.h) as f32
}

/// 対照群 — $\hat{s}$ より**大きい** $s$ から，倍数を除いたもの (D63)．
///
/// $\bar{V}(s)$ は $s$ とともに単調に増える (セルが大きいほど中に色が混ざる) ．合成
/// 500 件で測ると $s = 2$ で 0.0006 ・$s = 16$ で 0.0204 と 30 倍以上ちがう．
/// そのため**小さい $s$ を対照群へ入れると，最小値は必ずその小さい $s$ が取る** —
/// 正解した件の 52% でマージンが負になり，信頼度が 0 へ潰れていた．
///
/// $\hat{s}$ は「閾値を満たす最大の $s$」なので，問うべきは「1 つ上の $s$ がどれだけ
/// 悪いか」である．小さい $s$ は条件を満たして当たり前で，何の情報も持たない．
/// 倍数を除くのは従来どおり — 分散が同等に小さく，マージンを不当に縮めるためである．
///
/// 実測での分離能は，現行の定義が 70.8% に対しこの定義で 92.2% (均衡正解率)．
fn control_group(hat: u32, all: &[Candidate], max: u32) -> Vec<&Candidate> {
    let excluded = divisors_and_multiples(hat, max);
    all.iter()
        .filter(|c| c.scale > hat && !excluded(c.scale))
        .collect()
}

fn confidence(hat: &Candidate, all: &[Candidate], image_var: f32, max: u32) -> f32 {
    let group = control_group(hat.scale, all, max);
    // 退化ケース: 対照群なし / 画像が完全に平坦 — どちらも本質的に曖昧なので棄却
    if group.is_empty() || image_var <= 0.0 {
        return 0.0;
    }
    let min_other = group
        .iter()
        .map(|c| c.mean_variance)
        .fold(f32::INFINITY, f32::min);
    ((min_other - hat.mean_variance) / image_var).clamp(0.0, 1.0)
}

/// 与えた候補集合を評価する．
fn evaluate(
    img: &RgbaCanvas,
    it: &Integral,
    scales: &[u32],
    params: &GridParams,
) -> (Option<Candidate>, Vec<Candidate>) {
    let mut all = Vec::new();
    // 早期 return しない — 信頼度が全候補の分散を要求する
    for &s in scales {
        let Some((v, phase)) = best_phase(it, s as usize) else {
            continue;
        };
        all.push(Candidate {
            scale: s,
            mean_variance: v,
            phase,
        });
    }

    let epsilon = params.epsilon_for(image_variance(it));
    // 差分エネルギーは $s$ に依らない．候補ごとに作り直さない
    let energies = axis_energies(img, params.edge_fit_order);
    let accepted: Vec<&Candidate> = all
        .iter()
        .filter(|c| c.mean_variance <= epsilon)
        .filter(|c| recon_ok(img, it, c.scale as usize, c.phase, params.delta, params.tau))
        // 非整数の周期を落とす．再構成検査と違い，絵の中身に左右されない．
        // **境界の当てはめが肩代わりできる** (D71) — 帯ごとの argmin は谷が浅いと
        // 当てずっぽうになるが，境界の並びは標本が数十あり，対称な暈けで動かない
        .filter(|c| {
            let check: DriftCheck = params.into();
            match phase_parts(it, c.scale as usize, c.phase, check) {
                None => !check.require_measurable,
                Some((drift, curve)) => {
                    // **曲線も肩代わりできる — ただし厳しい残差で** (D73)
                    let curve = curve || edge_fit_rescues_curve(&energies, c.scale, params);
                    curve && (drift || edge_fit_ok(&energies, c.scale, params))
                }
            }
        })
        // 格子がそこに «在る» ことを確かめる (滑らかな絵はここで落ちる)
        .filter(|c| phase_contrast_ok(it, c.scale as usize, c.phase, params.phase_contrast_min))
        .collect();

    // 閾値を満たす最大の s．集合の最大値なので同点は起きない
    let hat = accepted.into_iter().max_by_key(|c| c.scale).copied();
    (hat, all)
}

/// 候補 1 つの評価結果 (診断用)．
///
/// どの検査で落ちたのか，対照群の中でどれと競っているのかを外から見るための型である．
/// 推定そのものには使わない．
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ScaleCandidate {
    pub scale: u32,
    /// セル内平均分散 $\bar{V}(s, d_s)$．
    pub mean_variance: f32,
    /// その $s$ で最も合う位相．
    pub phase: IVec2,
    pub passes_epsilon: bool,
    pub passes_recon: bool,
    pub passes_phase: bool,
}

impl ScaleCandidate {
    pub fn accepted(&self) -> bool {
        self.passes_epsilon && self.passes_recon && self.passes_phase
    }
}

/// すべての $s$ を評価して返す (診断用)．返り値は候補と画像全体の分散
/// $\bar{V}_{\mathrm{image}}$．
///
/// [`estimate_grid`] は絞り込みと全探索フォールバックを経るが，こちらは常に
/// $2 \ldots s_{\max}$ を素通しで評価する．**校正で「何と何が競っているのか」を
/// 見るための口**であって，推定の経路ではない．
pub fn scale_candidates(img: &RgbaCanvas, params: &GridParams) -> (Vec<ScaleCandidate>, f32) {
    let max = params
        .max_scale
        .min(img.width().max(1))
        .min(img.height().max(1));
    let it = Integral::new(img);
    let image_var = image_variance(&it);
    let energies = axis_energies(img, params.edge_fit_order);

    let out = (2..=max)
        .filter_map(|s| {
            let (v, phase) = best_phase(&it, s as usize)?;
            Some(ScaleCandidate {
                scale: s,
                mean_variance: v,
                phase,
                passes_epsilon: v <= params.epsilon_for(image_var),
                passes_recon: recon_ok(img, &it, s as usize, phase, params.delta, params.tau),
                passes_phase: {
                    let check: DriftCheck = params.into();
                    match phase_parts(&it, s as usize, phase, check) {
                        None => !check.require_measurable,
                        Some((drift, curve)) => {
                            (curve || edge_fit_rescues_curve(&energies, s, params))
                                && (drift || edge_fit_ok(&energies, s, params))
                        }
                    }
                } && phase_contrast_ok(
                    &it,
                    s as usize,
                    phase,
                    params.phase_contrast_min,
                ),
            })
        })
        .collect();
    (out, image_var)
}

/// 与えた $\hat{s}$ に対する対照群の添字 (診断用)．[`control_group`] と同じ規則．
///
/// 信頼度の分子は「対照群の最小分散 - $\bar{V}(\hat{s})$」なので，**誰が最小なのか**が
/// 分からないと式の当否を論じられない．
pub fn control_group_of(hat: u32, candidates: &[ScaleCandidate], max_scale: u32) -> Vec<u32> {
    let excluded = divisors_and_multiples(hat, max_scale);
    candidates
        .iter()
        .map(|c| c.scale)
        .filter(|s| *s > hat && !excluded(*s))
        .collect()
}

/// 格子を推定する (設計書 6.1)．
pub fn estimate_grid(
    img: &RgbaCanvas,
    params: &GridParams,
) -> std::result::Result<GridEstimate, GridError> {
    let max = params
        .max_scale
        .min(img.width().max(1))
        .min(img.height().max(1));
    if max < 2 {
        return Err(GridError::TooSmall);
    }
    let it = Integral::new(img);

    // 候補で絞ってから全探索へ落ちる 2 段構成．候補を取りこぼしたときに
    // 黙って誤答しないよう，フォールバックを必ず持つ
    let narrowed = candidate_scales(img, max);
    let (mut hat, mut all) = evaluate(img, &it, &narrowed, params);
    let needs_fallback = match &hat {
        None => true,
        Some(c) => control_group(c.scale, &all, max).is_empty(),
    };
    if needs_fallback {
        let full: Vec<u32> = (2..=max).collect();
        let (h2, a2) = evaluate(img, &it, &full, params);
        hat = h2;
        all = a2;
    }

    let hat = hat.ok_or(GridError::NotFound)?;
    let conf = confidence(&hat, &all, image_variance(&it), max);
    if conf < params.confidence_floor(hat.scale) {
        return Err(GridError::LowConfidence);
    }
    Ok(GridEstimate {
        scale: hat.scale,
        phase: hat.phase,
        confidence: conf,
        mean_variance: hat.mean_variance,
    })
}

/// 自己相関からスケールの候補を絞る．
///
/// 列ごとの差分エネルギーは格子の境界で山になる．その自己相関のピーク位置が
/// 周期，すなわちスケールの候補になる．**取りこぼしうるので，これだけに
/// 頼ってはいけない** — 呼び出し側は必ず全探索へ落ちられるようにする．
pub fn candidate_scales(img: &RgbaCanvas, max_scale: u32) -> Vec<u32> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    if w < 2 || h < 2 {
        return (2..=max_scale).collect();
    }

    let edge_energy = |along_x: bool| -> Vec<f64> {
        let n = if along_x { w } else { h };
        let m = if along_x { h } else { w };
        let mut out = vec![0.0f64; n];
        for (i, slot) in out.iter_mut().enumerate().skip(1) {
            let mut acc = 0.0;
            for j in 0..m {
                let (a, b) = if along_x {
                    (img.pixels()[j * w + i], img.pixels()[j * w + i - 1])
                } else {
                    (img.pixels()[i * w + j], img.pixels()[(i - 1) * w + j])
                };
                acc += (a.r as f64 - b.r as f64).abs()
                    + (a.g as f64 - b.g as f64).abs()
                    + (a.b as f64 - b.b as f64).abs();
            }
            *slot = acc;
        }
        out
    };

    let mut votes = vec![0.0f64; (max_scale + 1) as usize];
    for signal in [edge_energy(true), edge_energy(false)] {
        let n = signal.len();
        let mean = signal.iter().sum::<f64>() / n as f64;
        let centered: Vec<f64> = signal.iter().map(|v| v - mean).collect();
        let norm: f64 = centered.iter().map(|v| v * v).sum();
        if norm <= f64::EPSILON {
            continue;
        }
        for lag in 2..=max_scale as usize {
            if lag >= n {
                break;
            }
            let acc: f64 = (lag..n).map(|i| centered[i] * centered[i - lag]).sum();
            votes[lag] += acc / norm;
        }
    }

    let mut candidates: Vec<u32> = (2..=max_scale)
        .filter(|&s| votes[s as usize] > 0.0)
        .collect();
    // 真のスケールの約数もセル内分散 0 を与えるので，候補に含めておく
    let peaks = candidates.clone();
    for s in peaks {
        for d in 2..=s {
            if s % d == 0 && !candidates.contains(&d) {
                candidates.push(d);
            }
        }
    }
    candidates.sort_unstable();
    candidates.dedup();
    if candidates.is_empty() {
        (2..=max_scale).collect()
    } else {
        candidates
    }
}

/// **窓が $\hat{s}$ を当てるのに要るセルの数** (D164)．
///
/// 付録 C 要調査事項 #4 で測った値である．真値のある場面 (種を $s$ 倍に拡大して
/// 敷き詰めた画布) で窓を 1 画素刻みに掃くと，**$s = 2, 3, 4, 6, 8$ のすべてで
/// 下限がちょうど $4s$** になる — 窓の一辺にセルが 4 つ入れば当たり，
/// 3 つでは当たらない．
///
/// **これは推定器の «帯» の本数ではない．** 帯を 4 本から 2 本に減らしても
/// 下限は $4s$ のまま動かず，位相ずれ検査ごと外すと $2.2s \sim 3.0s$ まで
/// 下がる．つまり «4» は検査の刻みではなく，**位相を帯どうしで突き合わせる
/// のに要るセルの数**である (D62 ・D68 が入れた検査の代償)．
///
/// 測る口は `px-calib mixel`．
pub const MIN_CELLS_PER_WINDOW: u32 = 4;

/// 倍率 $s$ を局所推定で当てるのに要る窓の一辺 (D164)．
///
/// **窓を先に決めると，見える $s$ の上限が決まる** — 既定の 32 では
/// $s \leq 8$ までしか当たらない．
pub fn min_window_for(scale: u32) -> u32 {
    scale * MIN_CELLS_PER_WINDOW
}

/// 局所格子推定 (G4)．窓ごとの $\hat{s}$．
///
/// ミクセル検出 (lint 9) と非一様格子の棄却が共有する (D37)．
///
/// **信頼度 0 の窓は投票しない．** 平坦な窓ではすべての $s$ が閾値を満たすので
/// 「窓に収まる最大の $s$」が選ばれてしまう．これは本質的に曖昧なケースであり
/// (設計書 6.1 の退化ケース)，票に混ぜると一致率が下がって非一様と誤判定される．
///
/// > [!warning] **投票しない窓には «測れなかった» と «格子が無い» の両方が入る**
/// > (D164)．前者は平坦な窓 ・窓が $4s$ に足りない窓で，後者は等倍のドット絵
/// > そのものである．[`uniformity`] は**投票した窓だけ**で一致率を出すので，
/// > 等倍の絵に 2 倍の領域が混ざった «書籍の言うミクセル» では**票が 2 倍側に
/// > しか立たず，一致率は必ず 1.0 になる**．窓をどう選んでも鳴らない．
/// > 数え上げであって閾値の問題ではない．
pub fn local_grid(img: &RgbaCanvas, window: u32, params: &GridParams) -> Field<Option<u32>> {
    let step = window.max(2);
    let nx = img.width().div_ceil(step).max(1);
    let ny = img.height().div_ceil(step).max(1);
    let mut out: Field<Option<u32>> = Field::filled(nx, ny, None);

    for wy in 0..ny {
        for wx in 0..nx {
            let x0 = wx * step;
            let y0 = wy * step;
            let w = step.min(img.width().saturating_sub(x0));
            let h = step.min(img.height().saturating_sub(y0));
            if w < 4 || h < 4 {
                continue;
            }
            let mut sub = RgbaCanvas::filled(w, h, Rgba8::TRANSPARENT);
            for y in 0..h as i32 {
                for x in 0..w as i32 {
                    if let Some(c) = img.get(x0 as i32 + x, y0 as i32 + y) {
                        sub.set(x, y, c);
                    }
                }
            }
            let local = GridParams {
                max_scale: params.max_scale.min(w.min(h) / 2).max(2),
                ..*params
            };
            let value = estimate_grid(&sub, &local)
                .ok()
                .filter(|e| e.confidence > 0.0)
                .map(|e| e.scale);
            out.set(ivec2(wx as i32, wy as i32), value);
        }
    }
    out
}

/// **窓 1 つの厳密な升判定** — ルール 9 が使う (D172)．
///
/// # なぜ統計的推定器と別なのか (D37 の改訂)
///
/// [`local_grid`] は «測れなかった窓» と «格子が無い窓» をどちらも «票なし» に
/// するので，**等倍の絵に 2 倍が混ざった «書籍の言うミクセル» を窓をどう選んでも
/// 検出できない** (D164 が数え上げで示した) ．分けられるのは厳密な判定だけで，
/// **平らな窓はどの $k$ でも条件を満たす**から «測れなかった» と名乗れる．
///
/// D37 は «ミクセル検出と非一様格子の棄却は同一の推定器を共有する» と定めていたが，
/// **入力が違うので道具も分かれる** (D172) — `px lint` が受け取るのは劣化していない
/// 等倍の PNG なので厳密判定が使えるが，`px conform` が受け取るのは JPEG や補間を
/// 通った絵なので使えない．
///
/// > [!warning] **拡大素材に掛けてはいけない．** 実測で $s$ 倍に敷き詰めた絵の
/// > **30 枚中 23 枚が誤爆する** — 絵が平らな場所では $2s$ の升でも揃うので，
/// > 窓ごとに違う $k$ が立つ．`grid-calibration.md` が繰り返し測った
/// > «$2s_*$ への転落» と同じ現象である．
///
/// # 位相は画像の原点に取る
///
/// 升の格子は画像全体で 1 つなので，**窓ごとに位相を取り直してはいけない**．
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellVerdict {
    /// **どの $k$ でも条件を満たす** — この窓は格子について何も言っていない．
    ///
    /// **«測れなかった» であって «格子 1 だった» ではない** — 混ぜると
    /// 実素材の背景がすべて 1 に投票する．
    Flat,
    /// $k$ が決まった窓．**等倍の絵はここで 1 になる** — 統計的推定器が
    /// 票を立てられなかったのはまさにここである．
    Pinned(u32),
}

/// 厳密判定で見る升の上限 (D172)．
///
/// 上げるほど平らな窓が増える (大きい升ほど条件が緩い) ので無制限にはしない．
pub const MIXEL_MAX_K: u32 = 16;

/// **ルール 9 の窓** (D172 の実測値)．
///
/// | 窓 | 書籍のミクセル | 同じ絵の清書 | 実素材 64 枚 |
/// | ---: | ---: | ---: | ---: |
/// | 8 | 19 / 36 | **5 / 18 誤爆** | **5 / 64 誤爆** |
/// | **16** | **11 / 36** | **0 / 18** | **0 / 32 (検査できた枚数)** |
/// | 32 | 0 / 36 | 0 / 18 | 検査できた絵が無い |
///
/// **16 が «誤爆 0» の上限である**．8 まで下げると検出は上がるが，
/// 正しく描いた絵をミクセルと呼ぶ．
pub const MIXEL_WINDOW: u32 = 16;

/// 窓 1 つを厳密に判定する．
pub fn judge_cell_window(img: &RgbaCanvas, x0: u32, y0: u32, side: u32, max_k: u32) -> CellVerdict {
    // **平らな窓は先に外す** — どの $k$ でも通るので格子を 1 つも縛っていない
    let first = img.get(x0 as i32, y0 as i32);
    if (y0..y0 + side).all(|y| (x0..x0 + side).all(|x| img.get(x as i32, y as i32) == first)) {
        return CellVerdict::Flat;
    }
    let holds = |k: u32| -> bool {
        let k = k as i32;
        (y0..y0 + side).all(|y| {
            (x0..x0 + side).all(|x| {
                let (x, y) = (x as i32, y as i32);
                img.get(x, y) == img.get(x - x % k, y - y % k)
            })
        })
    };
    // 大きい升で揃っていればその約数でも揃うので，**上から採る**
    CellVerdict::Pinned((1..=max_k.min(side)).rev().find(|&k| holds(k)).unwrap_or(1))
}

/// **画像を窓で敷き詰めて厳密判定を集める** (D172)．
///
/// 返り値は `(決まった $k$ ごとの窓数, 平らだった窓の数)`．
/// **平らな窓は数えるだけで投票させない** — «測れなかった» を «格子 1» に
/// 混ぜると，背景の広い絵がすべて 1 に投票してしまう．
pub fn exact_grid_votes(
    img: &RgbaCanvas,
    window: u32,
    max_k: u32,
) -> (std::collections::BTreeMap<u32, usize>, usize) {
    let mut by_k: std::collections::BTreeMap<u32, usize> = std::collections::BTreeMap::new();
    let mut flat = 0usize;
    if window == 0 {
        return (by_k, flat);
    }
    let mut y = 0;
    while y + window <= img.height() {
        let mut x = 0;
        while x + window <= img.width() {
            match judge_cell_window(img, x, y, window, max_k) {
                CellVerdict::Flat => flat += 1,
                CellVerdict::Pinned(k) => *by_k.entry(k).or_default() += 1,
            }
            x += window;
        }
        y += window;
    }
    (by_k, flat)
}

/// **票がミクセルを示すか** (D172)．
///
/// 書籍の言うミクセルは «**等倍の絵**に拡大された部分が混ざる» ことである
/// (Pixel Logic PAGE:021) ．だから «升が 2 通りある» だけでは足りず，
/// **等倍 ($k = 1$) が混ざっていること**を要求する．
///
/// > [!warning] **この絞りが無いと一様に拡大した絵が誤爆する** — 絵が平らな
/// > 場所では $2s$ の升でも揃うので $s$ と $2s$ が並び立つ (実測 30 枚中 23 枚) ．
/// > `px lint` は渡された PNG が等倍か拡大かを知らないので，
/// > **絞りを規則の側に置く**しかない．一様に拡大された絵の «格子が場所により
/// > 違う» はルール 2 ・`px conform` の持ち場である．
///
/// **判定はここ 1 か所にある** — 測る口と道具が違うものを見てはいけない (D110)．
pub fn votes_show_mixel(by_k: &std::collections::BTreeMap<u32, usize>) -> bool {
    by_k.len() >= 2 && by_k.contains_key(&1)
}

/// 局所推定のばらつき．非一様格子の判定に使う．
///
/// 返り値は `(最頻のスケール, 一致した窓の割合)`．窓が 1 つも推定できなければ
/// `None`．**割合が閾値を下回ったら非一様として棄却する** (D29)．
pub fn uniformity(local: &Field<Option<u32>>) -> Option<(u32, f32)> {
    let values: Vec<u32> = local.data().iter().filter_map(|v| *v).collect();
    if values.is_empty() {
        return None;
    }
    let mut counts: std::collections::BTreeMap<u32, usize> = std::collections::BTreeMap::new();
    for v in &values {
        *counts.entry(*v).or_default() += 1;
    }
    // 同数のときは小さいスケールを採る (BTreeMap の走査順で決まる)
    let (best, count) = counts
        .iter()
        .max_by_key(|(scale, count)| (**count, std::cmp::Reverse(**scale)))
        .map(|(s, c)| (*s, *c))?;
    Some((best, count as f32 / values.len() as f32))
}

/// 格子に沿って最頻色へ縮小する (`px conform` の中核)．
///
/// **平均ではなく最頻色を採る**．平均だと境界のセルで元のパレットに無い色が
/// できてしまい，インデックスカラーへ戻せなくなる．
pub fn downscale_modal(img: &RgbaCanvas, scale: u32, phase: IVec2) -> RgbaCanvas {
    let s = scale.max(1) as usize;
    let (w, h) = (img.width() as usize, img.height() as usize);
    let (dx, dy) = (phase.x.max(0) as usize, phase.y.max(0) as usize);
    let nx = if w > dx { (w - dx) / s } else { 0 };
    let ny = if h > dy { (h - dy) / s } else { 0 };

    let mut out = RgbaCanvas::filled(nx as u32, ny as u32, Rgba8::TRANSPARENT);
    for j in 0..ny {
        for i in 0..nx {
            let mut counts: std::collections::BTreeMap<(u8, u8, u8, u8), usize> =
                std::collections::BTreeMap::new();
            for y in 0..s {
                for x in 0..s {
                    if let Some(c) = img.get((dx + i * s + x) as i32, (dy + j * s + y) as i32) {
                        *counts.entry((c.r, c.g, c.b, c.a)).or_default() += 1;
                    }
                }
            }
            // 同数のときは色の並び順で決める (決定論性，設計書 6.15 規則 2)
            if let Some(((r, g, b, a), _)) = counts.into_iter().max_by_key(|(k, v)| (*v, *k)) {
                out.set(i as i32, j as i32, Rgba8::new(r, g, b, a));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {

    /// 市松の絵を作る (升 `cell` 画素)．
    fn checker(side: u32, cell: u32) -> RgbaCanvas {
        let mut c = RgbaCanvas::filled(side, side, Rgba8::new(0, 0, 0, 255));
        for y in 0..side as i32 {
            for x in 0..side as i32 {
                let on = ((x / cell as i32) + (y / cell as i32)) % 2 == 0;
                let v = if on { 255 } else { 0 };
                c.set(x, y, Rgba8::new(v, v, v, 255));
            }
        }
        c
    }

    /// **等倍の模様は $k = 1$ に決まる** — 統計的推定器が «票なし» にする場面である．
    ///
    /// 壊れると: «格子が無い» を «格子 1» として投票させられず，D164 の
    /// 行き止まり (書籍のミクセルを検出できない) がそのまま残る．
    #[test]
    fn a_native_resolution_pattern_pins_the_grid_at_one() {
        assert_eq!(
            judge_cell_window(&checker(32, 1), 0, 0, 16, MIXEL_MAX_K),
            CellVerdict::Pinned(1)
        );
    }

    /// **2 倍に拡大した模様は $k = 2$ に決まる．**
    #[test]
    fn a_doubled_pattern_pins_the_grid_at_two() {
        assert_eq!(
            judge_cell_window(&checker(32, 2), 0, 0, 16, MIXEL_MAX_K),
            CellVerdict::Pinned(2)
        );
    }

    /// **平らな窓は «測れなかった» であって «格子 1» ではない** — ここが要である．
    ///
    /// 壊れると: 実素材の背景がすべて «格子 1» に投票し，2 倍で描かれた絵が
    /// ミクセルとして誤爆する．
    #[test]
    fn a_flat_window_says_nothing_rather_than_voting_for_one() {
        let img = RgbaCanvas::filled(32, 32, Rgba8::new(7, 7, 7, 255));
        assert_eq!(
            judge_cell_window(&img, 0, 0, 16, MIXEL_MAX_K),
            CellVerdict::Flat
        );
    }

    /// **位相は画像の原点に取る** — 窓ごとに取り直すと，升の途中から始まる窓で
    /// $k$ を見失う．
    ///
    /// 壊れると: ずれた窓が «等倍» に見え，2 倍の絵がミクセルとして誤爆する．
    #[test]
    fn the_cell_phase_comes_from_the_image_not_the_window() {
        assert_eq!(
            judge_cell_window(&checker(64, 2), 3, 3, 16, MIXEL_MAX_K),
            CellVerdict::Pinned(2)
        );
    }

    /// **一様に拡大した絵をミクセルと呼ばない** — 等倍が混ざっていることを要求する．
    ///
    /// 絵が平らな場所では $2s$ の升でも揃うので，$s$ 倍に拡大しただけの絵でも
    /// 升は 2 通りになる (実測 30 枚中 23 枚) ．**書籍の言うミクセルは «等倍の絵に
    /// 拡大が混ざる» ことなので，$k = 1$ が無ければ鳴らさない．**
    ///
    /// 壊れると: 一様に拡大した絵が軒並み blocking になる．
    #[test]
    fn two_cell_sizes_without_native_art_are_not_a_mixel() {
        let by_k = std::collections::BTreeMap::from([(4u32, 10usize), (8, 3)]);
        assert!(
            !votes_show_mixel(&by_k),
            "拡大しただけの絵をミクセルと呼んだ"
        );
        let with_native = std::collections::BTreeMap::from([(1u32, 10usize), (2, 3)]);
        assert!(votes_show_mixel(&with_native), "書籍のミクセルを見逃した");
    }

    /// **等倍の絵に 2 倍を混ぜると升が 2 通りになる** — 書籍の言うミクセルである．
    ///
    /// 統計的推定器はこの場面で票が 2 倍側にしか立たず，一致率が必ず 1.0 になる
    /// (D164) ．**厳密判定はここで初めて 2 通りを見る** (D172)．
    ///
    /// 壊れると: ルール 9 が書籍のミクセルを見逃す状態へ戻る．
    #[test]
    fn native_art_with_a_doubled_patch_shows_two_cell_sizes() {
        let mut img = checker(64, 1);
        // 右半分だけ 2 倍で描き直す
        for y in 0..64i32 {
            for x in 32..64i32 {
                let on = ((x / 2) + (y / 2)) % 2 == 0;
                let v = if on { 255 } else { 0 };
                img.set(x, y, Rgba8::new(v, v, v, 255));
            }
        }
        let (by_k, _flat) = exact_grid_votes(&img, MIXEL_WINDOW, MIXEL_MAX_K);
        assert!(by_k.contains_key(&1), "等倍側を見ていない: {by_k:?}");
        assert!(by_k.contains_key(&2), "2 倍側を見ていない: {by_k:?}");
        assert!(
            votes_show_mixel(&by_k),
            "ミクセルと判定しなかった: {by_k:?}"
        );
    }

    use super::*;

    /// 添字の並びを `scale` 倍に拡大した画像を作る．`phase` だけずらして切る．
    fn upscaled(pattern: &[&str], colors: &[Rgba8], scale: u32, phase: (u32, u32)) -> RgbaCanvas {
        let ph = pattern.len() as u32;
        let pw = pattern[0].len() as u32;
        let full_w = pw * scale;
        let full_h = ph * scale;
        let (dx, dy) = phase;
        let w = full_w - dx;
        let h = full_h - dy;

        let mut out = RgbaCanvas::filled(w, h, Rgba8::TRANSPARENT);
        for y in 0..h {
            for x in 0..w {
                let sx = ((x + dx) / scale) as usize;
                let sy = ((y + dy) / scale) as usize;
                let ch = pattern[sy].as_bytes()[sx];
                let index = (ch - b'0') as usize;
                out.set(x as i32, y as i32, colors[index]);
            }
        }
        out
    }

    /// 位相の検査を «帯ずれも曲線も課す» 形で回す (境界の当てはめは使わない)．
    ///
    /// 推定器は帯ずれの方だけを境界の当てはめに肩代わりさせるので，production には
    /// この形の呼び出しが無い．**検査そのものの振る舞いはここで固定する．**
    fn phase_check_ok(it: &Integral, s: usize, d: IVec2, check: DriftCheck) -> bool {
        match phase_parts(it, s, d, check) {
            None => !check.require_measurable,
            Some((drift, curve)) => drift && curve,
        }
    }

    /// 位相ずれ検査の設定．**下限 0 ・整数の位相**が既定の形である．
    fn check(bands: usize, tolerance: f32) -> DriftCheck {
        DriftCheck {
            bands,
            tolerance,
            floor: 0.0,
            min_cells: 2,
            subpixel: false,
            agreement: GridParams::default().phase_agreement,
            require_measurable: true,
        }
    }

    fn palette() -> Vec<Rgba8> {
        vec![
            Rgba8::rgb(0x1a, 0x1c, 0x2c),
            Rgba8::rgb(0xb1, 0x3e, 0x53),
            Rgba8::rgb(0xff, 0xcd, 0x75),
            Rgba8::rgb(0x38, 0xb7, 0x64),
        ]
    }

    const PATTERN: [&str; 6] = ["001122", "011223", "112233", "223300", "233001", "330011"];

    /// 12 x 12 の広めの模様．帯に切って測るには 1 帯あたり 2 セル以上が要る．
    const WIDE: [&str; 12] = [
        "001122330011",
        "011223300112",
        "112233001122",
        "122330011223",
        "223300112233",
        "233001122330",
        "330011223300",
        "300112233001",
        "001122330011",
        "011223300112",
        "112233001122",
        "122330011223",
    ];

    /// 非整数倍で最近傍リサンプルした画像．周期 `period` は整数にならない．
    ///
    /// これが位相ずれ検査の相手である — 周期 7.8 を 8 と読むと，セル境界が
    /// 1 セルあたり 0.2 画素ずつずれていく．
    fn resampled(pattern: &[&str], colors: &[Rgba8], period: f32) -> RgbaCanvas {
        let pw = pattern[0].len() as f32;
        let ph = pattern.len() as f32;
        let w = (pw * period) as u32;
        let h = (ph * period) as u32;
        let mut out = RgbaCanvas::filled(w, h, Rgba8::TRANSPARENT);
        for y in 0..h {
            for x in 0..w {
                let sx = ((x as f32 / period) as usize).min(pattern[0].len() - 1);
                let sy = ((y as f32 / period) as usize).min(pattern.len() - 1);
                let index = (pattern[sy].as_bytes()[sx] - b'0') as usize;
                out.set(x as i32, y as i32, colors[index]);
            }
        }
        out
    }

    /// 帯ごとの位相曲線から，正規化した食い違いを出す (試験用)．
    fn penalty(img: &RgbaCanvas, s: usize, d: IVec2) -> Option<f32> {
        let it = Integral::new(img);
        let (_, curves) = adaptive_curves(&it, s, d, 4, 2)?;
        agreement_of(&curves)
    }

    /// 非整数の周期で刻まれた絵には，**崩れる格子が無い**．
    ///
    /// ほかの関門はすべて «セルの中が揃っているか» を見るので，こういう絵の小さい $s$ は
    /// **余裕ゼロで全部を通る** — 走りの中に収まったセルはどれも 1 色だからである．
    /// 検証セットの誤受理 14 件のうち 7 件がこの形で，帯ずれ 0 ・曲線の食い違い 0 ・
    /// 不一致率 0 だった．位相を半セルずらしても崩れないことがその証拠になる．
    ///
    /// ここで作れる 93 画素角では他の関門が先に落とすので，**この試験が押さえるのは
    /// 統計の振る舞いだけ**である (実測は $s = 2$ で比 1.10 ・$s = 3$ で 1.07) ．
    /// «ほかの関門を全部通る» のは補間と圧縮が乗った実物での話で，評価データセット側で
    /// 測ってある．
    #[test]
    fn a_run_of_flat_cells_has_no_grid_to_break() {
        let img = resampled(&WIDE, &palette(), 7.8);
        let it = Integral::new(&img);
        let min_ratio = GridParams::default().phase_contrast_min;
        for s in [2usize, 3] {
            let d = best_phase(&it, s).expect("位相はある").1;
            assert!(
                !phase_contrast_ok(&it, s, d, min_ratio),
                "崩れる格子が無いのに «格子がある» と認めている (s = {s})"
            );
        }
    }

    /// 本物の格子は半セルずらすと崩れる — セル境界がセルの真ん中へ来る．
    #[test]
    fn a_true_grid_breaks_when_the_phase_is_shifted_half_a_cell() {
        for scale in [4u32, 6, 8] {
            let img = upscaled(&WIDE, &palette(), scale, (0, 0));
            let it = Integral::new(&img);
            assert!(
                phase_contrast_ok(
                    &it,
                    scale as usize,
                    ivec2(0, 0),
                    GridParams::default().phase_contrast_min
                ),
                "{scale} 倍の本物の格子を落としている"
            );
        }
    }

    /// 本物の格子は**帯をまたいで 1 つの位相で説明できる**．
    #[test]
    fn a_true_grid_is_explained_by_one_shared_phase() {
        for scale in [4u32, 6, 8] {
            let img = upscaled(&WIDE, &palette(), scale, (0, 0));
            let p = penalty(&img, scale as usize, ivec2(0, 0)).expect("測れる");
            assert!(p < 0.05, "{scale} 倍で共通の位相が損をしている ({p})");
        }
    }

    /// 非整数の周期は，どの位相を共通に選んでも損をする．
    #[test]
    fn a_non_integer_period_has_no_shared_phase() {
        let img = resampled(&WIDE, &palette(), 7.8);
        let it = Integral::new(&img);
        let d = best_phase(&it, 8).expect("位相はある").1;
        let p = penalty(&img, 8, d).expect("測れる");
        assert!(
            p > GridParams::default().phase_agreement,
            "非整数の周期なのに共通の位相で説明できている ({p})"
        );
    }

    /// **曲線の検査が棄却を引き受けている．**
    ///
    /// 帯ずれの許容を $\theta = 0.35$ まで緩めると，この非整数の周期は帯ずれだけでは
    /// 止まらない．止めているのは曲線の食い違いである — 検証セットでも，曲線を外して
    /// $\theta$ だけ緩めると正棄却が 183 → 151 / 199 に崩れる．
    #[test]
    fn the_curve_check_is_what_rejects_when_the_drift_tolerance_is_loose() {
        let img = resampled(&WIDE, &palette(), 7.8);
        let it = Integral::new(&img);
        let d = best_phase(&it, 8).expect("位相はある").1;
        let loose = DriftCheck {
            agreement: 1.01, // 曲線の検査を外す
            ..check(4, 0.35)
        };
        assert!(
            phase_check_ok(&it, 8, d, loose),
            "帯ずれだけでは止まらない前提が崩れている (この試験の意味が無くなる)"
        );
        assert!(
            !phase_check_ok(&it, 8, d, check(4, 0.35)),
            "曲線の検査が棄却を引き受けていない"
        );
    }

    #[test]
    fn a_true_grid_fits_the_same_phase_in_every_band() {
        for scale in [4u32, 6, 8] {
            for phase in [(0u32, 0u32), (2, 3)] {
                let img = upscaled(&WIDE, &palette(), scale, phase);
                let it = Integral::new(&img);
                let d = ivec2(
                    ((scale - phase.0) % scale) as i32,
                    ((scale - phase.1) % scale) as i32,
                );
                assert!(
                    phase_check_ok(&it, scale as usize, d, check(4, 0.0)),
                    "{scale} 倍 ・位相 {phase:?} で帯ごとに位相が違う"
                );
            }
        }
    }

    #[test]
    fn a_non_integer_period_fails_the_phase_check() {
        // 周期 7.8 を 8 と読ませる．1 セルあたり 0.2 画素ずつずれる
        let img = resampled(&WIDE, &palette(), 7.8);
        let it = Integral::new(&img);
        let d = best_phase(&it, 8).expect("位相はある").1;
        assert!(
            !phase_check_ok(
                &it,
                8,
                d,
                DriftCheck {
                    floor: 1.0,
                    ..check(4, 1.0 / 6.0)
                }
            ),
            "非整数の周期を通してしまった"
        );
    }

    /// 帯が薄いときは**飛ばさずに帯を減らして測る**．飛ばすと過大推定が素通りする．
    #[test]
    fn thin_bands_are_measured_with_fewer_bands_instead_of_being_skipped() {
        // 24 x 24 に s = 4 なら 6 セル．8 本には足りないが 2 本 (4 セル) なら測れる
        let img = upscaled(&PATTERN, &palette(), 4, (0, 0));
        let it = Integral::new(&img);
        assert_eq!(
            drift_spread(&it, 4, ivec2(0, 0), 8, 2),
            None,
            "8 本では測れないはずである"
        );
        let (bands, curves) =
            adaptive_curves(&it, 4, ivec2(0, 0), 8, 2).expect("帯を減らせば測れるのに飛ばしている");
        assert_eq!(bands, 3, "測れる範囲でいちばん多い本数を使う");
        assert_eq!(drift_of(&curves, 4, false), 0.0);
        // 本物の格子なので，測った結果は当然通る
        assert!(phase_check_ok(&it, 4, ivec2(0, 0), check(8, 0.0)));
    }

    /// セルが 2 本ぶんに足りない候補は**棄却する**．
    ///
    /// 以前は素通ししていた (「少ないセルから求めた位相は落とす根拠にならない」) が，
    /// 測ると逆で，検査を通せない候補を通すと $\hat{s}$ がそこへ流れる —
    /// 棄却で完全一致 58 → 60 / 101，正棄却は 182 / 199 のまま動かなかった．
    #[test]
    fn a_candidate_too_thin_to_measure_is_rejected() {
        // 12 x 12 に s = 4 なら 3 セル．2 本 x 2 セル = 4 セルに届かない
        let img = upscaled(&["012", "120", "201"], &palette(), 4, (0, 0));
        let it = Integral::new(&img);
        assert!(adaptive_curves(&it, 4, ivec2(0, 0), 4, 2).is_none());
        assert!(!phase_check_ok(&it, 4, ivec2(0, 0), check(4, 0.0)));
        // 検査を切っている (帯 < 2) 場合と混同しない
        assert!(phase_check_ok(
            &it,
            4,
            ivec2(0, 0),
            DriftCheck {
                require_measurable: false,
                ..check(4, 0.0)
            }
        ));
    }

    #[test]
    fn the_phase_check_can_be_turned_off() {
        let img = resampled(&WIDE, &palette(), 7.8);
        let it = Integral::new(&img);
        let d = best_phase(&it, 8).expect("位相はある").1;
        assert!(
            phase_check_ok(&it, 8, d, check(0, 0.0)),
            "帯 0 で検査が働いている"
        );
        assert!(
            phase_check_ok(&it, 8, d, check(1, 0.0)),
            "帯 1 では比べようがない"
        );
    }

    #[test]
    fn phases_wrap_around_the_scale() {
        assert_eq!(cyclic_spread(&[0, 7], 8), 1, "0 と s-1 は隣どうし");
        assert_eq!(cyclic_spread(&[0, 4], 8), 4);
        assert_eq!(cyclic_spread(&[2, 2, 2, 2], 8), 0);
    }

    #[test]
    fn a_band_measures_its_phase_from_the_image_origin() {
        // 帯の左端を基準にすると帯ごとに違う物差しになる
        assert_eq!(first_cell(0, 3, 8), 3);
        assert_eq!(first_cell(8, 3, 8), 11);
        assert_eq!(first_cell(10, 3, 8), 11);
        assert_eq!(first_cell(11, 3, 8), 11);
        assert_eq!(first_cell(12, 3, 8), 19);
    }

    /// 折り畳みの要点 — $2 s_*$ では山が 2 本になる．
    #[test]
    fn a_doubled_candidate_repeats_the_folded_profile() {
        let img = upscaled(&WIDE, &palette(), 4, (0, 0));
        let it = Integral::new(&img);
        let echo = |s: u32| {
            let d = best_phase(&it, s as usize).expect("位相はある").1;
            let p = profile_stats(&img, s, d);
            p.echo1[0].max(p.echo1[1])
        };
        assert!(echo(4) < 0.5, "真の s で山が 2 本立っている ({})", echo(4));
        assert!(
            echo(8) > 0.8,
            "2 倍の候補で山が 1 本に見えている ({})",
            echo(8)
        );
    }

    /// 位相ずらしの要点 — 本物は半セルずらすと崩れ，$2 s_*$ は崩れない．
    #[test]
    fn shifting_half_a_cell_breaks_a_true_grid_but_not_a_doubled_one() {
        let img = upscaled(&WIDE, &palette(), 4, (0, 0));
        let it = Integral::new(&img);
        let margin = |s: u32| {
            let d = best_phase(&it, s as usize).expect("位相はある").1;
            let c = phase_contrast(&img, s, d, 0.1);
            c.variance_margin[0].min(c.variance_margin[1])
        };
        assert!(
            margin(4) > 0.1,
            "真の s が半セルずらしで崩れない ({})",
            margin(4)
        );
        assert!(
            margin(8).abs() < 0.02,
            "2 倍の候補が半セルずらしで崩れている ({})",
            margin(8)
        );
    }

    /// 段差は格子線に集まる — 折り畳みの帯 0 がセルの境目である．
    #[test]
    fn the_folded_profile_puts_the_boundary_in_the_first_band() {
        for phase in [(0u32, 0u32), (2, 3)] {
            let img = upscaled(&WIDE, &palette(), 4, phase);
            let d = ivec2(((4 - phase.0) % 4) as i32, ((4 - phase.1) % 4) as i32);
            let p = profile_stats(&img, 4, d);
            for axis in 0..2 {
                assert!(
                    p.edge_share[axis] > 0.9,
                    "位相 {phase:?} 軸 {axis} で段差が格子線に乗っていない ({})",
                    p.edge_share[axis]
                );
            }
        }
    }

    /// 副画素の当てはめは，谷が対称なら整数の位置を動かさない．
    #[test]
    fn a_symmetric_valley_keeps_the_integer_phase() {
        let curve = [1.0f32, 0.5, 0.0, 0.5];
        assert!((refine_phase(&curve, 2) - 2.0).abs() < 1e-6);
    }

    /// 谷が傾いていれば，低い側へ半画素まで寄る．
    #[test]
    fn a_lopsided_valley_moves_toward_the_lower_side() {
        // 左が低い → 頂点は p より手前 (小さい側) へ寄る
        let curve = [0.2f32, 0.0, 0.8, 1.0];
        let refined = refine_phase(&curve, 1);
        assert!((0.5..1.0).contains(&refined), "副画素の位相 {refined}");
        // 山になっている点は動かさない (2 階差分が正でない)．
        // **位相軸は巡回する**ので，端が低いだけでは「谷でない」ことにならない
        assert_eq!(refine_phase(&[0.0, 1.0, 0.5, 1.0], 1), 1.0);
    }

    /// 副画素でも巡回する — 0 と $s - \epsilon$ は隣どうしである．
    #[test]
    fn subpixel_phases_wrap_around_the_scale() {
        assert!((cyclic_spread_f(&[0.0, 7.5], 8.0) - 0.5).abs() < 1e-6);
        assert!((cyclic_spread_f(&[0.0, 4.0], 8.0) - 4.0).abs() < 1e-6);
    }

    /// 本物の格子なら副画素で測っても帯ごとに揃う．
    #[test]
    fn a_true_grid_agrees_across_bands_in_subpixel_too() {
        for scale in [4u32, 6, 8] {
            let img = upscaled(&WIDE, &palette(), scale, (0, 0));
            let spread = phase_drift_spread_subpixel(&img, scale, ivec2(0, 0), 4, 2)
                .expect("測れるはずである");
            assert!(spread < 0.5, "{scale} 倍で副画素の帯ずれが {spread}");
        }
    }

    /// 4 分割の要点 — 真の $s$ は割っても説明されず，$2 s_*$ は割ると説明される．
    #[test]
    fn splitting_a_cell_explains_a_doubled_candidate_but_not_a_true_one() {
        let img = upscaled(&WIDE, &palette(), 4, (0, 0));
        assert!(
            split_gain(&img, 4, ivec2(0, 0)) < 0.05,
            "真の s が割ると説明されている ({})",
            split_gain(&img, 4, ivec2(0, 0))
        );
        assert!(
            split_gain(&img, 8, ivec2(0, 0)) > 0.9,
            "2 倍の候補が割っても説明されない ({})",
            split_gain(&img, 8, ivec2(0, 0))
        );
        // 約数は真の s と同じ側に立つ (止める必要が無い)
        assert!(split_gain(&img, 2, ivec2(0, 0)) < 0.05);
    }

    #[test]
    fn a_flat_profile_has_no_echo() {
        assert_eq!(periodic_echo(&[1.0, 1.0, 1.0, 1.0]), 0.0);
        // 約数が無ければ比べようがない
        assert_eq!(periodic_echo(&[3.0, 0.0, 0.0]), 0.0);
        assert_eq!(periodic_echo(&[2.0, 0.0]), 0.0);
    }

    #[test]
    fn folding_counts_every_defined_sample_once() {
        let energy = vec![None, Some(1.0), Some(2.0), Some(3.0), Some(4.0)];
        // 位相 1 なら添字 1 が帯 0 に来る
        assert_eq!(fold_profile(&energy, 2, 1), vec![2.0, 3.0]);
    }

    #[test]
    fn the_diagnostic_view_agrees_with_the_estimator() {
        // 診断用の口が推定と食い違うと，校正で見ているものが別物になる
        let img = upscaled(&WIDE, &palette(), 6, (2, 3));
        let params = GridParams::default();
        let (cands, image_var) = scale_candidates(&img, &params);
        let e = estimate_grid(&img, &params).unwrap();

        let hat = cands
            .iter()
            .filter(|c| c.accepted())
            .max_by_key(|c| c.scale)
            .expect("受け入れられた候補がある");
        assert_eq!(hat.scale, e.scale);
        assert_eq!(hat.phase, e.phase);
        assert!((hat.mean_variance - e.mean_variance).abs() < 1e-6);
        assert!(image_var > 0.0);
    }

    #[test]
    fn the_control_group_is_larger_non_multiples_only() {
        let img = upscaled(&WIDE, &palette(), 4, (0, 0));
        let (cands, _) = scale_candidates(&img, &GridParams::default());
        let group = control_group_of(4, &cands, 16);

        assert!(group.iter().all(|s| *s > 4), "ŝ 以下の s が残っている");
        for s in [8u32, 12, 16] {
            assert!(!group.contains(&s), "倍数 {s} が残っている");
        }
        assert!(group.contains(&5) && group.contains(&7));
    }

    #[test]
    fn a_clean_upscale_is_confident() {
        let img = upscaled(&WIDE, &palette(), 4, (0, 0));
        let e = estimate_grid(&img, &GridParams::default()).unwrap();
        assert!(e.confidence > 0.0, "きれいな格子なのに信頼度が 0 である");
    }

    #[test]
    fn recovers_the_scale_of_a_clean_upscale() {
        for scale in [2u32, 3, 4, 6, 8] {
            let img = upscaled(&PATTERN, &palette(), scale, (0, 0));
            let e = estimate_grid(&img, &GridParams::default())
                .unwrap_or_else(|err| panic!("{scale} 倍で失敗: {err}"));
            assert_eq!(e.scale, scale, "{scale} 倍を {} と推定した", e.scale);
            assert_eq!(e.phase, ivec2(0, 0));
        }
    }

    #[test]
    fn recovers_the_phase_when_the_image_is_cropped() {
        for (dx, dy) in [(1u32, 0u32), (0, 1), (2, 3), (3, 3)] {
            let img = upscaled(&PATTERN, &palette(), 4, (dx, dy));
            let e = estimate_grid(&img, &GridParams::default()).unwrap();
            assert_eq!(e.scale, 4, "位相 ({dx},{dy}) でスケールを外した");
            // 位相は「切り落とした分だけ格子がずれる」
            let expect = ivec2(((4 - dx) % 4) as i32, ((4 - dy) % 4) as i32);
            assert_eq!(e.phase, expect, "位相 ({dx},{dy})");
        }
    }

    /// D28 の要点 — 約数を選んではいけない．
    #[test]
    fn picks_the_largest_scale_not_a_divisor() {
        let img = upscaled(&PATTERN, &palette(), 8, (0, 0));
        let e = estimate_grid(&img, &GridParams::default()).unwrap();
        assert_eq!(e.scale, 8, "約数 (2 や 4) を選んでいる");
    }

    /// 再構成検査の要点 — 倍数側の過大推定を排除する．
    #[test]
    fn reconstruction_check_rejects_overestimation() {
        let img = upscaled(&PATTERN, &palette(), 3, (0, 0));
        // 6 は 3 の倍数だが，隣り合うセルの色が違うので再構成が合わない
        let e = estimate_grid(&img, &GridParams::default()).unwrap();
        assert_eq!(e.scale, 3);
    }

    #[test]
    fn confidence_is_zero_for_a_flat_image() {
        let img = RgbaCanvas::filled(32, 32, Rgba8::rgb(10, 20, 30));
        // 平坦な画像はすべての s が閾値を満たす退化ケース (設計書 6.1) なので，
        // 自信を持って答えてはいけない．棄却されるか，信頼度 0 で返るかのどちらかである．
        // 既定の min_confidence が 0 より大きいので通常は LowConfidence になる
        match estimate_grid(&img, &GridParams::default()) {
            Ok(e) => assert_eq!(e.confidence, 0.0, "平坦な画像に信頼度が付いている"),
            Err(GridError::NotFound | GridError::LowConfidence) => {}
            Err(e) => panic!("想定外: {e}"),
        }
    }

    #[test]
    fn a_flat_image_is_rejected_when_a_minimum_confidence_is_set() {
        let img = RgbaCanvas::filled(32, 32, Rgba8::rgb(10, 20, 30));
        let params = GridParams {
            min_confidence: 0.01,
            ..GridParams::default()
        };
        assert!(matches!(
            estimate_grid(&img, &params),
            Err(GridError::LowConfidence) | Err(GridError::NotFound)
        ));
    }

    #[test]
    fn confidence_is_positive_for_a_clear_grid() {
        let img = upscaled(&PATTERN, &palette(), 4, (0, 0));
        let e = estimate_grid(&img, &GridParams::default()).unwrap();
        assert!(e.confidence > 0.0, "はっきりした格子に信頼度が付かない");
    }

    #[test]
    fn noise_beyond_epsilon_is_not_accepted_as_a_grid() {
        // 全画素がばらばら — 格子は無い
        let mut img = RgbaCanvas::filled(32, 32, Rgba8::TRANSPARENT);
        let mut state = 12345u32;
        for y in 0..32 {
            for x in 0..32 {
                state = state.wrapping_mul(1103515245).wrapping_add(12345);
                let v = (state >> 16) as u8;
                img.set(x, y, Rgba8::rgb(v, v.wrapping_mul(3), v.wrapping_mul(7)));
            }
        }
        assert!(estimate_grid(&img, &GridParams::default()).is_err());
    }

    #[test]
    fn tiny_images_are_rejected() {
        let img = RgbaCanvas::filled(1, 1, Rgba8::rgb(0, 0, 0));
        assert_eq!(
            estimate_grid(&img, &GridParams::default()),
            Err(GridError::TooSmall)
        );
    }

    #[test]
    fn phase_ties_are_broken_lexicographically() {
        // 縦縞だけの画像は y 方向の位相が全て同点になる
        let mut img = RgbaCanvas::filled(16, 16, Rgba8::TRANSPARENT);
        for y in 0..16 {
            for x in 0..16 {
                let c = if (x / 4) % 2 == 0 {
                    Rgba8::rgb(0, 0, 0)
                } else {
                    Rgba8::rgb(255, 255, 255)
                };
                img.set(x, y, c);
            }
        }
        let a = estimate_grid(&img, &GridParams::default()).unwrap();
        let b = estimate_grid(&img, &GridParams::default()).unwrap();
        assert_eq!(a, b, "同じ入力で結果が揺れている");
        assert_eq!(a.phase.y, 0, "同点なのに辞書式で最小になっていない");
    }

    #[test]
    fn candidate_scales_include_the_true_scale() {
        for scale in [2u32, 3, 4, 6, 8] {
            let img = upscaled(&PATTERN, &palette(), scale, (0, 0));
            let candidates = candidate_scales(&img, 16);
            assert!(
                candidates.contains(&scale),
                "{scale} 倍が候補に入っていない: {candidates:?}"
            );
        }
    }

    #[test]
    fn downscale_modal_recovers_the_original_pattern() {
        let colors = palette();
        let img = upscaled(&PATTERN, &colors, 5, (0, 0));
        let small = downscale_modal(&img, 5, ivec2(0, 0));
        assert_eq!((small.width(), small.height()), (6, 6));
        for (y, row) in PATTERN.iter().enumerate() {
            for (x, ch) in row.bytes().enumerate() {
                let expect = colors[(ch - b'0') as usize];
                assert_eq!(small.get(x as i32, y as i32), Some(expect), "({x},{y})");
            }
        }
    }

    #[test]
    fn downscale_modal_is_deterministic() {
        let img = upscaled(&PATTERN, &palette(), 4, (0, 0));
        assert_eq!(
            downscale_modal(&img, 4, ivec2(0, 0)),
            downscale_modal(&img, 4, ivec2(0, 0))
        );
    }

    #[test]
    fn local_grid_agrees_with_the_global_estimate_on_a_uniform_image() {
        let big: Vec<String> = (0..24)
            .map(|y| {
                (0..24)
                    .map(|x| char::from(b'0' + ((x + y) % 4) as u8))
                    .collect()
            })
            .collect();
        let refs: Vec<&str> = big.iter().map(|s| s.as_str()).collect();
        let img = upscaled(&refs, &palette(), 4, (0, 0));

        let local = local_grid(&img, 32, &GridParams::default());
        let (scale, ratio) = uniformity(&local).expect("局所推定が 1 つも取れない");
        assert_eq!(scale, 4);
        assert!(ratio > 0.5, "窓ごとの一致率が低すぎる: {ratio}");
    }

    /// 平坦な窓は票を投じない．窓に収まる最大の $s$ を選んでしまうため．
    #[test]
    fn flat_windows_do_not_vote() {
        let img = RgbaCanvas::filled(64, 64, Rgba8::rgb(10, 20, 30));
        let local = local_grid(&img, 32, &GridParams::default());
        assert!(
            local.data().iter().all(|v| v.is_none()),
            "平坦な画像から局所推定が出ている: {:?}",
            local.data()
        );
        assert_eq!(uniformity(&local), None);
    }

    #[test]
    fn uniformity_is_none_without_any_estimate() {
        let empty: Field<Option<u32>> = Field::filled(2, 2, None);
        assert_eq!(uniformity(&empty), None);
    }

    /// 本物の格子なら，境界の並びに当てた直線の間隔が $s$ と一致する．
    #[test]
    fn the_fitted_spacing_matches_a_real_grid() {
        let img = upscaled(&WIDE, &palette(), 6, (0, 0));
        let fit = edge_fit(&img, 6, 1, &GridParams::default());
        for axis in 0..2 {
            let slope = fit.slope[axis].expect("境界を拾えていない");
            assert!(slope.abs() < 0.02, "軸 {axis} の間隔がずれている: {slope}");
            let residual = fit.residual[axis].expect("境界を拾えていない");
            assert!(residual < 0.05, "軸 {axis} の残差が大きい: {residual}");
        }
    }

    /// **非整数の周期は «間隔のずれ» として出る．** 残差ではないところが要点で，
    /// 添字付けを繰り返すと «5 画素の格子が途中で 1 本飛んだ» ではなく
    /// «間隔が 5.3» という 1 つの読み方に収束する．
    #[test]
    fn a_non_integer_period_shows_up_as_a_spacing_error() {
        let peaks: Vec<f32> = (0..20).map(|k| k as f32 * 5.3).collect();
        let fit = fit_spacing(&peaks, 5.0).expect("当てはまらない");
        assert!((fit.spacing - 5.3).abs() < 0.01, "間隔 {}", fit.spacing);
        assert!(fit.rms < 0.01, "残差 {}", fit.rms);
    }

    /// 真の $s$ の約数は**そのまま直線に乗る**．止めるのは «閾値を満たす最大の $s$» の
    /// 規則であって，この量の仕事ではない — **この検査は «真の $s$ を通す» 側だけを
    /// 担当する** (2 倍の抑止は再構成検査と半セルずらしのままである) ．
    #[test]
    fn a_divisor_still_fits_the_line() {
        let peaks: Vec<f32> = (0..12).map(|k| k as f32 * 6.0).collect();
        let fit = fit_spacing(&peaks, 3.0).expect("当てはまらない");
        assert!(fit.rms < 0.01, "残差 {}", fit.rms);
        assert!((fit.spacing - 3.0).abs() < 0.01, "間隔 {}", fit.spacing);
    }

    /// **外れの峰 1 本は «残差の測り方» では吸収できない — 畳むしかない．**
    ///
    /// 偽の峰が 1 本入るとそこで添字の積み上げが 1 つずれ，**以降の点がすべて直線から
    /// 外れる**．だから RMS だけでなく中央絶対値も跳ねる (0 → 0.77) — 外れに鈍い統計を
    /// 選んでも，**添字を振る限り直らない**．畳んだ散らばりは添字を要らないので動かない．
    ///
    /// 実データの正例で残差が 0.9 まで伸びるのはこの形であり，畳んだ形の裾が素材に
    /// 依らない (合成 0.373 ・同梱 0.326 ・`local/` 0.311) 理由でもある．
    #[test]
    fn one_stray_peak_breaks_the_indexing_and_only_folding_survives() {
        let mut peaks: Vec<f32> = (0..12).map(|k| k as f32 * 5.0).collect();
        let clean = fit_spacing(&peaks, 5.0).expect("当てはまらない");
        let clean_folded = folded_spread(&peaks, 5.0).expect("畳めない");
        peaks.insert(6, 27.5); // セルの真ん中に偽の峰を 1 本
        peaks.sort_by(f32::total_cmp);
        let dirty = fit_spacing(&peaks, 5.0).expect("当てはまらない");
        let dirty_folded = folded_spread(&peaks, 5.0).expect("畳めない");

        assert!(
            dirty.rms > clean.rms + 0.3,
            "RMS {} → {}",
            clean.rms,
            dirty.rms
        );
        assert!(
            dirty.median > clean.median + 0.3,
            "中央絶対値も跳ねる {} → {}",
            clean.median,
            dirty.median
        );
        assert!(
            dirty_folded < clean_folded + 0.1,
            "畳んだ散らばりは動かない {clean_folded} → {dirty_folded}"
        );
    }

    /// 畳んだ散らばりは**添字を振らない**ので，間隔の丸めが滑る並びでも壊れない．
    #[test]
    fn the_folded_spread_needs_no_indexing() {
        // 5 画素周期だが途中で 2 本ぶん飛ぶ (丸めが滑る形)
        let peaks: Vec<f32> = [0.0, 5.0, 10.0, 20.0, 25.0, 35.0, 40.0].into();
        assert!(folded_spread(&peaks, 5.0).expect("畳めない") < 0.01);
    }

    /// 平坦な画像では境界が 1 本も立たない — **測れない候補**である．
    #[test]
    fn a_flat_image_has_no_boundaries() {
        let img = RgbaCanvas::filled(64, 64, Rgba8::rgb(10, 20, 30));
        let fit = edge_fit(&img, 4, 1, &GridParams::default());
        assert_eq!(fit.count, [0, 0]);
        assert_eq!(fit.residual, [None, None]);
    }

    /// 峰の頂点は**対称な暈けで動かない** — これがこの量を採る理由である．
    #[test]
    fn a_symmetric_blur_keeps_the_peak_in_place() {
        let sharp = [0.0, 1.0, 5.0, 1.0, 0.0].map(Some);
        let blurred = [0.5, 2.0, 4.0, 2.0, 0.5].map(Some);
        assert_eq!(refine_peak(&sharp, 2), refine_peak(&blurred, 2));
        assert_eq!(refine_peak(&sharp, 2), 2.0);
    }

    /// 非極大抑制の窓は $s/2$ — 2 画素に広がった峰を 2 本と数えない．
    #[test]
    fn a_widened_peak_is_counted_once() {
        let mut energy = vec![Some(0.0); 24];
        for k in 1..6 {
            energy[k * 4] = Some(9.0);
            energy[k * 4 + 1] = Some(8.0);
        }
        let peaks = energy_peaks(&energy, 4, 0.5, 1.0);
        assert_eq!(peaks.len(), 5, "峰を数え違えている: {peaks:?}");
    }
}
