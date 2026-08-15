//! 色・格子・ディザ系の 11 ルール (設計書 7.3)．
//!
//! **閾値はすべて暫定である．** ルールは検査対象を作るフェーズで実装すると決めた
//! (実装計画書 M2 の注記) が，閾値の決定には `testdata/lint-cases/` の正例・負例が
//! 要る．ここに書いてある値は「合成した例で意図どおり動く」までしか根拠がない．

use std::collections::BTreeMap;

use pxsmith_core::canvas::{IndexedCanvas, RgbaCanvas};
use pxsmith_core::clean::{DenoiseOptions, detect_dither_noise};
use pxsmith_core::color::{Oklab, oklab_of};
use pxsmith_core::frame::{Frame, Surface};
use pxsmith_core::geom::distance::signed_distance;
use pxsmith_core::geom::regions::{RegionMap, label_regions};
use pxsmith_core::grid::{GridParams, exact_grid_votes, votes_show_mixel};
use pxsmith_core::math::{IRect, IVec2, Vec2, ivec2};
use pxsmith_core::palette::Palette;
use serde::{Deserialize, Serialize};

use crate::{Report, Violation, rule};

/// 閾値．**評価データセットと `testdata/lint-cases/` で校正するまで暫定値**．
#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub struct LintConfig {
    /// ルール 3 — この画素数未満の連結成分を孤立とみなす．
    pub isolated_max_area: u32,
    /// **ルール 19** — 周囲長を外接矩形の周囲長で割った値の上限．
    ///
    /// **$P^2 / A$ ではない** — あちらは細さを測ってしまい，良い絵の 93.8% が鳴る．
    pub max_boundary_excess: f32,
    /// **ルール 19** — この画素数未満の領域は見ない．
    pub shape_noise_min_area: u32,
    /// **ルール 20 ・21** — 接触を見るときに両方の領域に求める画素数の下限．
    ///
    /// **市松のディザは 1 画素の領域が斜めに接し続けている**ので，下限が無いと
    /// ディザの絵で鳴りっぱなしになる (下限 1 なら良い絵の 83.6% が鳴る) ．
    ///
    /// 掃引して 16 (= 4x4) を採った．**これより小さいものは «部品» ではなく
    /// 質感である**．負例は «角に四角を置く» (ルール 20) と «斜めに接する 2 領域を
    /// 同じ色にする» (ルール 21．書籍の «髪とヘッドバンドの同化») ．
    ///
    /// | 下限 | 20 良い絵 | 20 捕捉 | 21 良い絵 | 21 捕捉 |
    /// | --- | --- | --- | --- | --- |
    /// | 8 | 34.4% | 7 / 7 | 37.7% | 6 / 6 |
    /// | **16** | **9.8%** | **6 / 7** | **18.0%** | **6 / 6** |
    /// | 32 | 1.6% | 1 / 7 | 4.9% | 0 / 6 |
    ///
    /// **負例は 7 件と 6 件しかない** — 捕捉率はその程度の根拠しかないと読むこと．
    pub min_touch_area: u32,
    /// **ルール 23** — 重心を揃えて 2 度以上出入りした画素の，面積に対する割合の上限．
    pub wobble_ratio: f32,
    /// **ルール 24** — 重心を戻したときにディザ領域で入れ替わってよい画素の割合．
    pub moving_dither_ratio: f32,
    /// **ルール 24** — ディザ領域を探す窓の一辺．
    pub moving_dither_window: u32,
    /// **ルール 25** — 新しくできた列に残っていてよいドット数の下限．
    ///
    /// **`pxsmith anim subpixel` の下限から引く** — 直す側と検査する側で別の数を持つと
    /// 自分の出力が自分の検査に落ちる (D110) ．
    pub min_new_run: u32,
    /// **ルール 27** — 伸び縮みで許す体積の誤差．
    ///
    /// **`pxsmith anim squash` の実測から引く** — 画素が整数なので体積は保存しきれない
    /// (D123) ．
    pub volume_error: f32,
    /// ルール 2 — «この絵は格子を名乗っているか» の判定．セル内平均分散が画像分散の
    /// この割合以下なら格子らしいとみなす．
    ///
    /// $\varepsilon$ (0.15) をそのまま使うと**原寸のドット絵まで «格子らしい» に
    /// なる** — 平坦な面が多い 16x16 のタイルは $2 \times 2$ でも分散が小さい．
    /// 本物の拡大なら分散はほぼ 0 なので，**桁で切る**．
    ///
    /// 3 つの集合で掃いて決めた (原寸の CC0 64 枚 ・きれいな拡大 101 枚 ・崩れた格子
    /// 199 枚) ．
    ///
    /// | 値 | 原寸で誤爆 | きれいな拡大で誤爆 | 崩れた格子を捕捉 |
    /// | --- | --- | --- | --- |
    /// | 0.01 | 4.7% | 12.9% | 31.2% |
    /// | **0.05** | **9.4%** | 26.7% | **67.3%** |
    /// | 0.15 | 25.0% | 30.7% | 89.9% |
    /// | 0.30 | 73.4% | 30.7% | 96.0% |
    ///
    /// **きれいな拡大での誤爆は 30.7% で飽和する** — 推定器自身がそれだけ棄却する
    /// ためで，このルールは `conform` の再現率を超えられない．blocking なので
    /// 誤爆を重く見て 0.05 を採った．
    pub grid_like_ratio: f32,
    /// ルール 2 ・9 — 格子推定の閾値．
    pub grid: GridParams,
    /// **ルール 9 — 厳密な升判定の窓の一辺** (D172)．
    ///
    /// 実測で «誤爆 0» の上限が 16 だった．8 まで下げると書籍のミクセルの検出は
    /// 19 / 36 へ上がるが，正しく描いた絵を 5 / 64 でミクセルと呼ぶ．
    pub mixel_window: u32,
    /// **ルール 9 — 見る升の上限** (D172)．
    pub mixel_max_k: u32,
    /// ルール 4 — シルエットの縁のうち何割を同じ色が占めていれば «縁取り» か．
    ///
    /// **暫定値である．** 0.6 は «縁取りらしい色を 1 色に絞る» ために置いただけで，
    /// 掃引していない (掃いたのは下の 2 つ) ．
    pub min_outline_share: f32,
    /// ルール 4 — 縁取りの色に許す «内側» の割合 (線であることの条件)．
    ///
    /// 正例は CC0 の実物 64 枚 ・負例は «斜めにも接していれば縁» として描いた
    /// 縁取り 8 枚である (`pxsmith-calib lint --outline-interior`) ．
    ///
    /// | 内側の割合 | 良い絵で鳴る | 負例を捕捉 |
    /// | --- | --- | --- |
    /// | 0.02 | 0 / 64 | 0 / 8 |
    /// | **0.05 (採用)** | **2 / 64** | **3 / 8** |
    /// | 0.08 | 2 / 64 | 3 / 8 |
    /// | 0.10 | 4 / 64 | 3 / 8 |
    /// | 0.20 | 13 / 64 | 5 / 8 |
    ///
    /// 0.05 と 0.08 は同じ成績で，**運転点は平らな面の上にある** (刃の上ではない) ．
    ///
    /// > [!warning] **«内側が 1 画素も無い» では厳しすぎる．**
    /// > 太った縁取り — まさに検出したい相手 — は角が埋まって内側を持つ．
    ///
    /// > [!note] **捕捉 3 / 8 は自己整合性と引き換えである．**
    /// > 下の [`LintConfig::min_outline_overlap_share`] が無ければ 6 / 8 だが，
    /// > **`pxsmith outline --style black` の出力が自分の検査に落ちる**．D58 の優先順位に
    /// > 従い道具を正としてルールを緩めた．
    pub max_outline_interior: f32,
    /// ルール 4 — 重なりが何か所以上あれば違反とするか．
    ///
    /// 1 から 2 へ上げると誤爆が 5 枚 → 2 枚に減り，捕捉は動かない (**無償**) ．
    /// 幅 2 画素の脚や太い縁は狙って描かれるもので，どれも 1 〜 4 か所だった．
    pub min_outline_overlaps: usize,
    /// ルール 4 — 重なりに含まれる画素が縁取りの何割を超えたら «太い縁» とみなすか．
    ///
    /// **これが無いと `pxsmith aa` の出力が落ちる．** `kenney_tile_0101` は幅 2 画素の
    /// 枠を持つタイルで，AA が内側の画素を塗り替えると «内側の割合» が 5% を下回り，
    /// 枠がまるごと «重なった縁取り» になった (blocking が 0 → 20) ．
    /// **ずっと 2 画素幅の縁取りは様式であって失敗ではない．**
    pub max_outline_overlap_share: f32,
    /// ルール 4 — 重なりが縁取りの何割に満たなければ «段の折り返し» とみなすか．
    ///
    /// **これが無いと `pxsmith outline` の出力が落ちる．** 急な階段 (1 列で 2 行下がる
    /// 縁) を内側へ 1 画素で縁取ると環の画素が積み重なり，`crawl_salamander` の
    /// `black` で 3 か所の $2 \times 2$ ができた．**角を «足して» 描いた縁取りは
    /// 縁の全体に重なりが出る**ので，割合で分ける．
    ///
    /// | 下限 | 良い絵で鳴る | 負例を捕捉 | `pxsmith outline` の自己整合性 |
    /// | --- | --- | --- | --- |
    /// | 0.05 | 2 / 64 | 5 / 8 | **落ちる** |
    /// | **0.06 (採用)** | **2 / 64** | **3 / 8** | 通る |
    /// | 0.10 | 2 / 64 | 2 / 8 | 通る |
    pub min_outline_overlap_share: f32,
    /// ルール 6 — 隣接する 2 面を «光と影» と読む明度差の下限．
    ///
    /// **暫定値である．** 陰影の隣り合う段 (ランプの 1 段) ではなく «光面と影面» を
    /// 拾うために置いている．正例で決め直すこと．
    pub shadow_lightness_delta: f32,
    /// ルール 6 — «同一色相» とみなす色相差 (度)．
    ///
    /// **設計書のルール 6 は «同一色相の明度違いのみ» である** — 影の色相が光の
    /// 色相と*同じ*ときを言う．[`pxsmith_core::ramp::MIN_HUE_SEPARATION`] (35 度) は
    /// **作る側が狙う分離量**であって，検査が要求してよい下限ではない．
    /// 35 度で切ると良い絵 64 枚のうち **41 枚 (64.1%) が blocking** になる —
    /// 実物のドット絵は色相をずらさない陰影を普通に使う．
    ///
    /// 正例 (CC0 の実物 64 枚) と負例 (色相を固有色に揃えた 8 枚) で掃いた．
    ///
    /// | 色相差 | 良い絵で鳴る | 負例を捕捉 |
    /// | --- | --- | --- |
    /// | 0.5 | 20 / 64 | — |
    /// | 1 | 22 / 64 | 6 / 8 |
    /// | **3 (採用)** | **25 / 64** | **7 / 8** |
    /// | 5 | 27 / 64 | 7 / 8 |
    /// | 35 | 41 / 64 | — |
    ///
    /// **0.5 度まで下げても 20 / 64 が残る** — 文字どおり同一色相 (丸めの揺れより
    /// 小さい) の光 / 影の組を持つ良い絵がそれだけある．**分けられないので
    /// advisory に置く** (D86) ．3 度としてあるのは，暗く彩度の低い色では 8 ビットへ
    /// 丸めるだけで色相が数度動くためで，**様式の目標ではなく丸めの雑音の水準**である．
    pub min_shadow_hue_gap: f32,
    /// ルール 6 — 色相を問える彩度の下限．
    ///
    /// これを下回る色は灰色とみなして見逃す．**灰色だけの陰影は失敗ではない．**
    pub min_shadow_chroma: f32,
    /// ルール 7 — **宣言された光源**．`None` なら検査しない (既定)．
    ///
    /// 自動ミラーや反転タイル同値判定のように «光源方向を知っている» 経路だけが
    /// 立てる．絵だけからは矛盾を言えない (上のルール 7 の警告) ．
    pub light: Option<pxsmith_core::ramp::LightSource>,
    /// ルール 7 — 明度勾配と光源方向の一致度がこれを下回れば違反とする．
    ///
    /// **`pxsmith shade` の出力とその左右反転で決めた** (`pxsmith-calib flip`) ．光源が
    /// 分かっている絵を作れるので，正例 ・負例の両方を自前で用意できる．
    ///
    /// | 群 (平行光源の 4 プリセット x 実素材 64 枚) | 最小 | 中央 | 最大 |
    /// | --- | --- | --- | --- |
    /// | そのまま (正しい向き) | **0.714** | 0.975 | 1.000 |
    /// | 左右反転 (自動ミラーの失敗) | $-0.472$ | 0.061 | **0.386** |
    ///
    /// **群が重ならない．** 隙間 (0.386 〜 0.714) の中央を採る — 0.55 で
    /// **誤爆 0 / 256 ・捕捉 256 / 256** である．
    ///
    /// > [!note] 左右反転しても一致度は $-1$ にならない (中央 0.061) ．
    /// > 既定の光源は斜め ($(-0.6, 0.8)$) なので，左右反転で変わるのは $x$ 成分
    /// > だけである．**«逆を向いたら鳴らす» ($< 0$) では 95 / 256 しか捕まらない** —
    /// > «合っていないと鳴らす» でなければならない．
    pub min_shading_agreement: f32,
    /// ルール 7 — これ未満の画素数では勾配を測らない．
    pub shading_min_pixels: u32,
    /// ルール 12 — 並走とみなす直交方向の隔たり (画素)．
    ///
    /// **暫定値である．** 3 画素は «帯 1 本ぶんの幅» を目安に置いただけで，
    /// 掃引していない (掃いたのは下の本数) ．
    pub band_max_gap: u32,
    /// ルール 12 — そろって一致するラン長の本数．
    ///
    /// 正例 (CC0 の実物 64 枚) と負例 (傾き 2 の帯を 3 画素おきに敷いた 8 枚) で
    /// 掃いた．
    ///
    /// | 本数 | 良い絵で鳴る | 負例を捕捉 |
    /// | --- | --- | --- |
    /// | 4 | 25 / 64 | 8 / 8 |
    /// | 5 | 10 / 64 | 8 / 8 |
    /// | 6 | 8 / 64 | 8 / 8 |
    /// | **8 (採用)** | **3 / 64** | **8 / 8** |
    /// | 10 | 2 / 64 | **2 / 8** |
    /// | 24 | 0 / 64 | 0 / 8 |
    ///
    /// **8 が膝である** — そこまでは誤爆だけが減り，10 で捕捉が崩れる．
    /// 短い一致 (4 〜 6 本) は良い絵にいくらでもある — 段の列が少し揃うのは
    /// 普通のことで，«縞に見える» のは長く続いたときである．
    pub band_min_runs: usize,
    /// ルール 8 — 谷を «直せる» とみなす画素の移動上限 (設計書 6.4 の $\delta_{\max}$)．
    ///
    /// **`pxsmith smooth` と同じ値を使う** — 報告の «直せる» が実際に直せることと一致
    /// しなければ意味が無い．数値は書き写さず [`pxsmith_core::geom::jaggy::DEFAULT_MAX_MOVE`]
    /// から引く．
    pub jaggy_max_move: u32,
    /// ルール 10 — ディザ領域内の同色連結成分がこの長さを超えたら塊化．
    pub dither_clump: u32,
    /// ルール 11 — 隣接領域の $\Delta L$ の下限．
    pub min_lightness_delta: f32,
    /// ルール 11 — この面積未満の領域は隣接判定から外す (AA を拾わないため)．
    pub min_region_area: u32,
    /// ルール 3 — **これより近い色の 1 画素は «迷子» ではなく «段» とみなす**．
    ///
    /// ランプの宣言 (`adjacent_in_a_ramp`) はファイルに残らない — `.aseprite` にも
    /// `.hex` にも欄が無いので，`pxsmith shade` → `pxsmith lint` を CLI で繋いだ時点で消える．
    /// **色差なら残る**ので，同じことを色差で見る．
    ///
    /// Oklab の色距離 ($w_L = 1$) を 3 群で測った (D81) ．
    ///
    /// | 群 | 件数 | 最小 | 中央 | 最大 |
    /// | --- | --- | --- | --- | --- |
    /// | 良い絵で鳴っている 1 画素 | 2 | 0.097 | — | 0.347 |
    /// | **負例 (撒いた迷子)** | 13 | **0.254** | 0.389 | 0.683 |
    /// | **`pxsmith shade` の 1 画素ハイライト** | 8 | 0.102 | 0.176 | **0.178** |
    ///
    /// **群が重ならない** (陰影の段は 0.178 以下 ・撒いた迷子は 0.254 以上) ．
    /// 隙間の中央を採る．良い絵の 2 件のうち 1 件 (0.097) も除外されるので，
    /// **誤爆は 2 枚 → 1 枚**になる．
    ///
    /// > [!note] **D70 が捨てた «色差で絞る» とは向きが逆である．**
    /// > D70 が測ったのは «近い色だけを迷子とみなす» 案で，**派手な色の迷子を
    /// > 見逃す**ので捨てた．ここは逆に «近い色を除外する» ので，派手な迷子は
    /// > 今までどおり鳴る．
    pub stray_min_distance: f32,
    /// ルール 13 — 距離場と明度の相関の上限．
    ///
    /// `pxsmith-calib pillow` で 3 群を測って決めた (D77) ．**正例が先** (D70) ．
    ///
    /// | 閾値 | 正例で誤爆 (良い絵 61) | 負例を捕捉 (pillow 6) | **`pxsmith shade` が鳴る (320)** |
    /// | --- | --- | --- | --- |
    /// | 0.60 | 8.2% | 100% | **0%** |
    /// | 0.75 | 4.9% | 100% | **0%** |
    /// | **0.85** | **1.6%** | **100%** | **0%** |
    /// | 0.90 | 0.0% | 83.3% | 0% |
    ///
    /// > [!note] **負例は 8 枚作って 6 枚しか測れていない** — 見逃しではない．
    /// > 残りの 2 枚は元絵に半透明の画素があり，**アルファ 2 値の不変条件 (D4) で
    /// > パレットにできない**ので lint 自体が掛からない．同じ理由で正例も 64 → 61 枚
    /// > である．«鳴らない» と «測れない» を混ぜないこと (D70) ．
    ///
    /// **群がほとんど重ならない** (正例の最大 0.871 ・負例の最小 0.881) ．
    /// ただし**その隙間は 0.01 しかなく，負例は 6 件しかない** — 中点 (0.876) を
    /// 採るのは 2 点への当てはめでしかないので，誤爆側に余裕のある 0.85 を採る．
    ///
    /// > [!note] **`pxsmith shade` の出力は 320 通りすべてで負の相関だった** (最大 $-0.020$) ．
    /// > 設計書は «正しく陰影付けされた凸形状でも相当に相関する» と予告していたが，
    /// > 実測では逆を向く — $\langle n, \ell \rangle$ で決める明るさは «縁からの距離»
    /// > ではなく «縁の向き» に従うので，光の縁と影の縁がどちらも $d \approx 0$ で
    /// > 両極端に来て相関が潰れる．**D58 の心配は実測では起きない．**
    pub max_pillow_correlation: f32,
    /// ルール 13 — これ未満のシルエットでは相関を測らない．
    ///
    /// **暫定値である．** 16x16 のタイル (256 画素) が最小の素材なので，その 1/4 を
    /// 下限に置いただけで，小さい形での振れ方は測っていない．
    pub pillow_min_pixels: u32,
    /// ルール 14 — 不透明画素に占める中間色の割合の上限．
    ///
    /// 中間色の数え方は [`pxsmith_core::aa::intermediate_pixels`] である．**3 群で測った**
    /// (`pxsmith-calib aa`) — 正例が先 (D70) ，そして**自作の出力が自作の検査に落ちない**
    /// こと (D58 ・D77 と同じ作法) ．
    ///
    /// | 群 | 中央 | 90% | 95% | 最大 |
    /// | --- | --- | --- | --- | --- |
    /// | 良い絵 (CC0 61 枚) | 0.086 | 0.273 | 0.337 | 0.593 |
    /// | `pxsmith aa` を掛けた後 (同 61 枚) | 0.106 | 0.287 | 0.348 | 0.606 |
    /// | **`pxsmith shade` の出力 (64 枚 x 5 プリセット)** | 0.210 | 0.383 | 0.399 | **0.527** |
    /// | 負例 (縁を全部ぼかした 8 枚) | 0.153 | 0.218 | 0.623 | 0.623 |
    ///
    /// > [!warning] **陰影の «段» はそのまま中間色として数えられる．**
    /// > ランプの中間の段は端の 2 色の間にあり，端より狭く使われる — 定義に
    /// > そのまま当てはまる．**0.35 (良い絵の 95% 点のすぐ上) に置くと
    /// > `pxsmith shade` の出力が鳴った** (端から端まで CLI で通して 38.8%) ．
    /// > D58 の優先順位に従い，**`pxsmith shade` の最大 (0.527) の上**に置く．
    ///
    /// 0.55 での成績は **良い絵 1 / 64 ・`pxsmith shade` 0 / 320 ・負例 1 / 8** である．
    ///
    /// > [!note] **群は元から分かれていない．** 特徴量を 4 通り試した．
    /// >
    /// > | 試した特徴量 | 結果 |
    /// > | --- | --- |
    /// > | `pxsmith clean --remove-aa` が外す画素 | **AA が多いほど減る** (16 画素の上限で枠から外れる) |
    /// > | [`pxsmith_core::aa::strip_aa`] の画素マスク | 良い絵で中央 0.175 ・最大 0.670 と緩すぎる |
    /// > | 中点にあるだけ («より広く使われている» を外す) | 良い絵で中央 0.488 ・最大 0.987 |
    /// > | **中点にあり両端がより広い** (採用) | 上の表 |
    /// >
    /// > 負例も «`pxsmith aa` を過剰な設定で 4 巡» では 0.039 〜 0.102 にしかならず，
    /// > 良い絵を下回った (`pxsmith aa` は角にしか置かないので縁が埋まらない) ．
    /// > **種の corpus 自体が既にアンチエイリアス済み**で，Dungeon Crawl のタイルは
    /// > 中間色を 101 色持つものまである (Kenney の原寸タイルは中央 0.059 ・
    /// > 最大 0.234，Crawl は中央 0.194 ・最大 0.606 と出所で分かれる) ．
    /// > **分かれないので advisory のまま，誤爆側に大きく余裕を取っている．**
    pub max_intermediate_ratio: f32,
    /// ルール 14 — 中間色とみなす色距離の許容 (2 色の中点からの距離)．
    ///
    /// **`pxsmith clean --remove-aa` と同じ値を使う** — 数える側と外す側で «中間色» の
    /// 定義がずれると，鳴った AA を自分の道具で外せなくなる (D83) ．
    pub intermediate_tolerance: f32,
    /// ルール 15 — 画面に占めるディザ領域の割合の上限．
    pub max_dither_ratio: f32,
    /// ルール 16 — 「大面積」とみなす画面比．
    pub large_area_ratio: f32,
    /// ルール 16 — 大面積で許す彩度と明度の上限．
    pub max_large_chroma: f32,
    pub max_large_lightness: f32,
    /// ルール 17 — 隣接 2 色の $\Delta L$ がこれを超えるディザは高コントラスト．
    pub high_contrast_delta: f32,
    /// ルール 18 — 純黒とみなす明度と彩度の上限．
    pub pure_black_lightness: f32,
    pub pure_black_chroma: f32,
}

impl Default for LintConfig {
    fn default() -> Self {
        Self {
            isolated_max_area: 2,
            // **暫定値．`pxsmith-calib lintseq` で掃いて決める**
            // 掃引で決めた (誤爆 21.3% ・捕捉 100%．付録 C 要調査事項 #2)
            max_boundary_excess: 1.20,
            shape_noise_min_area: 8,
            // 掃引で決めた．**16 は 4x4** — これより小さいものは «部品» ではなく
            // 質感である (ルール 20: 誤爆 9.8% ・捕捉 6/7．ルール 21: 18.0% ・6/6)
            min_touch_area: 16,
            wobble_ratio: 0.01,
            moving_dither_ratio: 0.10,
            // **静止画のルール 10 ・15 より小さい．** 8 画素の窓は 16 〜 32 画素の
            // スプライトの内側にほとんど収まらず，捕捉が 8.6% しか出ない (窓 4 なら 80.0%)
            moving_dither_window: 4,
            // **直す側から引く** (書き写さない)
            min_new_run: pxsmith_core::subpixel::DEFAULT_MIN_RUN,
            volume_error: 0.05,
            grid_like_ratio: 0.05,
            grid: GridParams::default(),
            mixel_window: pxsmith_core::grid::MIXEL_WINDOW,
            mixel_max_k: pxsmith_core::grid::MIXEL_MAX_K,
            min_outline_share: 0.6,
            max_outline_interior: 0.05,
            min_outline_overlaps: 2,
            max_outline_overlap_share: 0.7,
            min_outline_overlap_share: 0.06,
            light: None,
            min_shading_agreement: 0.55,
            shading_min_pixels: 64,
            shadow_lightness_delta: 0.15,
            min_shadow_hue_gap: 3.0,
            min_shadow_chroma: 0.02,
            band_max_gap: 3,
            band_min_runs: 8,
            jaggy_max_move: pxsmith_core::geom::jaggy::DEFAULT_MAX_MOVE,
            dither_clump: 6,
            min_lightness_delta: 0.06,
            min_region_area: 4,
            stray_min_distance: 0.22,
            max_pillow_correlation: 0.85,
            pillow_min_pixels: 64,
            max_intermediate_ratio: 0.55,
            intermediate_tolerance: pxsmith_core::clean::AaOptions::default().tolerance,
            max_dither_ratio: 0.35,
            large_area_ratio: 0.25,
            max_large_chroma: 0.16,
            max_large_lightness: 0.85,
            high_contrast_delta: 0.35,
            pure_black_lightness: 0.06,
            pure_black_chroma: 0.01,
        }
    }
}

/// フレームを検査する．
///
/// レイヤごとにインデックスカラーの面を検査し，パレット側のルールは 1 度だけ見る．
///
/// **`keyframe` のルールは `kind` が key / breakdown のフレームにしか掛けない**
/// (設計書 7.1 ・D47) ．中間フレームは一瞬しか映らないのでジャギーや AA を
/// 気にしない — これが無いと `pxsmith anim` の出力が自らの lint に大量に落ちる．
pub fn lint_frame(frame: &Frame, cfg: &LintConfig) -> Report {
    let mut report = Report::default();
    report.extend(lint_palette(&frame.palette, cfg));

    for layer in &frame.layers {
        if let Surface::Indexed(canvas) = &layer.surface {
            report.extend(lint_canvas_scoped(
                canvas,
                &frame.palette,
                cfg,
                frame.kind.is_keyframe(),
            ));
        }
    }
    report.sorted()
}

/// 1 枚のキャンバスを検査する．
///
/// **単体の静止画は keyframe として扱う** — 設計書 7.1 の `keyframe` は «中間
/// フレームを外す» ための区分であり，フレーム列に属さない 1 枚は中間フレームでは
/// ない．フレームの `kind` を見て分けたいときは [`lint_canvas_scoped`] を使う．
pub fn lint_canvas(canvas: &IndexedCanvas, palette: &Palette, cfg: &LintConfig) -> Report {
    lint_canvas_scoped(canvas, palette, cfg, true)
}

/// スコープを指定して 1 枚のキャンバスを検査する．
pub fn lint_canvas_scoped(
    canvas: &IndexedCanvas,
    palette: &Palette,
    cfg: &LintConfig,
    keyframe: bool,
) -> Report {
    let mut report = Report::default();
    let regions = label_regions(canvas);

    rule_1_palette_escape(canvas, palette, &mut report);
    rule_3_isolated(&regions, canvas, palette, cfg, &mut report);
    rule_10_dither_clumping(canvas, cfg, &mut report);
    rule_11_lightness_delta(&regions, palette, canvas, cfg, &mut report);
    rule_6_monochrome_shadow(&regions, palette, canvas, cfg, &mut report);
    rule_7_flipped_shading(canvas, palette, cfg, &mut report);
    rule_13_pillow_shading(canvas, palette, cfg, &mut report);
    rule_15_dither_ratio(canvas, cfg, &mut report);
    rule_16_large_saturated(&regions, palette, canvas, cfg, &mut report);
    rule_17_high_contrast_dither(canvas, palette, cfg, &mut report);
    rule_21_same_colour_neighbours(&regions, canvas, cfg, &mut report);
    if keyframe {
        rule_4_outline_corners(canvas, cfg, &mut report);
        rule_8_jaggies(canvas, cfg, &mut report);
        rule_12_banding(canvas, cfg, &mut report);
        rule_14_too_much_aa(canvas, palette, cfg, &mut report);
        rule_19_shape_noise(canvas, cfg, &mut report);
        rule_20_tangent(&regions, canvas, cfg, &mut report);
    }
    report.sorted()
}

/// パレットだけを見るルール (5 ・18)．
pub fn lint_palette(palette: &Palette, cfg: &LintConfig) -> Report {
    let mut report = Report::default();
    rule_5_chroma_curve(palette, &mut report);
    rule_18_pure_black(palette, cfg, &mut report);
    report
}

/// 格子を見るルール (2 ・9)．RGBA の入力にしか意味がないので入口を分ける．
pub fn lint_grid(img: &RgbaCanvas, cfg: &LintConfig) -> Report {
    let mut report = Report::default();
    rule_2_broken_grid(img, cfg, &mut report);
    rule_9_mixels(img, cfg, &mut report);
    report.sorted()
}

// --- ルール 1: パレット逸脱 ---

fn rule_1_palette_escape(canvas: &IndexedCanvas, palette: &Palette, report: &mut Report) {
    let r = rule(1).expect("ルール 1 は定義済み");
    let mut seen: Vec<u8> = canvas.pixels().to_vec();
    seen.sort_unstable();
    seen.dedup();

    for index in seen {
        if palette.get(index).is_none() {
            let at = canvas
                .bounds()
                .iter()
                .find(|p| canvas.get_at(*p) == Some(index));
            let mut v = Violation::new(
                r,
                format!("添字 {index} がパレット ({} 色) の範囲外", palette.len()),
            );
            if let Some(p) = at {
                v = v.at(p);
            }
            report.push(v);
        }
    }
}

// --- ルール 2: 格子崩れ ---

/// ルール 2 — **格子«らしさ»があるのに格子として通らない**ときだけ鳴らす．
///
/// 以前は `estimate_grid` が失敗しただけで違反にしていたが，**原寸のドット絵には
/// $s \ge 2$ の格子が «無いのが正しい»** (1 画素 = 1 セル) ．CC0 の実物のドット絵
/// 61 枚のうち **58 枚がこれで blocking になっていた** — 良い絵を丸ごと落としていた
/// ことになる．
///
/// そこで «$\varepsilon$ を満たす候補が 1 つでもあるか» を先に見る．原寸の絵は
/// $s \ge 2$ のどの候補もセル内分散が大きいので，そもそも格子を名乗っていない．
/// 候補があるのに推定が通らない絵だけが «格子崩れ» である．
fn rule_2_broken_grid(img: &RgbaCanvas, cfg: &LintConfig, report: &mut Report) {
    let r = rule(2).expect("ルール 2 は定義済み");
    let (candidates, image_var) = pxsmith_core::grid::scale_candidates(img, &cfg.grid);
    let grid_like = candidates
        .iter()
        .any(|c| c.scale >= 2 && c.mean_variance <= cfg.grid_like_ratio * image_var);
    if grid_like && pxsmith_core::grid::estimate_grid(img, &cfg.grid).is_err() {
        report.push(Violation::new(
            r,
            format!(
                "格子らしい候補 (セル内分散が画像分散の {:.0}% 以下) はあるが，格子として通らない",
                cfg.grid_like_ratio * 100.0
            ),
        ));
    }
}

// --- ルール 3: 孤立ピクセル ---

/// ルール 3 — **その色が絵の中で他に使われていない**小さい領域だけを孤立とする．
///
/// さらに **«単色に囲まれている»** ことを求める．狙って置いた点 (目のハイライトなど) は
/// 面の境目や模様の中に置かれるので，隣に複数の色が来る．迷子は平らな面にぽつんと
/// 落ちるので隣が 1 色になる．
///
/// | | 良い絵 64 枚 | 負例 (迷子を撒いた) 8 枚 |
/// | --- | --- | --- |
/// | 一意の色の 1 画素がある | 16 枚 | 8 枚 |
/// | **うち単色に囲まれている** | **2 枚** | **6 枚** |
///
/// blocking なので誤爆を重く見て，捕捉 8 / 8 → 6 / 8 と引き換えに誤爆を
/// 14 枚 → 2 枚へ落とす．
///
/// 周囲との色差で絞る案も測ったが (**近い色だけ迷子とみなす**) ，違反は 66 → 45 件に
/// しか減らないうえ «派手な色の迷子» を見逃す．採らない．
///
/// 面積だけで判定すると**ドット絵の質感を «孤立» と呼ぶ**．CC0 の実物のタイルで
/// 測ると，石畳で 655 / 1024 画素 ・血糊で 428 / 1024 画素が «孤立» になった —
/// 模様そのものである (61 枚中 55 枚が blocking，違反 5689 件) ．
///
/// 質感は**同じ色が他の場所にも現れる**ところが «迷子の 1 画素» と違う．そこで
/// «その添字がこの領域にしか無い» ことを条件に足す．
///
/// > [!warning] **ランプの隣の段は «迷子» ではない — 陰影の最終段である** (M3)．
/// > `pxsmith shade` の出力 320 通りに掛けたら 8 件が blocking になった．中身を見ると
/// > どれも**光へ正対した 1 画素**で，光ランプの最上段が周りの 1 段下に囲まれている
/// > 形だった (`crawl_urand_fencer` の (27, 19) など) ．
/// >
/// > «光へ正対した面が最も明るい段になる» は陰影の根幹であり (`Shading::index` は
/// > そのために上端を閉じている) ，1 画素のハイライトはドット絵の常道でもある．
/// > **パレットが «この 2 色は同じランプの隣り合う段である» と宣言しているなら，
/// > それは狙って置いた点である．**
/// >
/// > 良い絵 ・負例の成績は 1 件も動かない — PNG を添字にしただけのパレットは
/// > ランプを宣言しないので，この除外は**自分で作ったランプにしか効かない**．
fn rule_3_isolated(
    regions: &RegionMap,
    canvas: &IndexedCanvas,
    palette: &Palette,
    cfg: &LintConfig,
    report: &mut Report,
) {
    let r = rule(3).expect("ルール 3 は定義済み");
    let mut used: BTreeMap<u8, u32> = BTreeMap::new();
    for &i in canvas.pixels() {
        *used.entry(i).or_default() += 1;
    }
    for region in regions.regions() {
        if region.area >= cfg.isolated_max_area || canvas.transparent() == Some(region.index) {
            continue;
        }
        // 同じ色が他にもあるなら，それは質感であって迷子ではない
        if used.get(&region.index).copied().unwrap_or(0) > region.area {
            continue;
        }
        // 隣に複数の色が来るなら，模様や面の境目に置かれた点である
        let around = neighbour_indices(regions, canvas, region);
        let [only] = around[..] else {
            continue;
        };
        // 同じランプの隣り合う段なら陰影の段である (上の警告)．
        // **宣言はファイルに残らない**ので，色差でも同じことを見る (D81)
        if adjacent_in_a_ramp(palette, region.index, only)
            || is_a_shading_step(palette, region.index, only, cfg)
        {
            continue;
        }
        report.push(
            Violation::new(
                r,
                format!(
                    "{} 画素の孤立した領域 (添字 {}．この色は他に無く，単色に囲まれている)",
                    region.area, region.index
                ),
            )
            .at(ivec2(region.bbox.x, region.bbox.y))
            .area(region.bbox),
        );
    }
}

/// **2 つの添字が同じランプの隣り合う段か．**
///
/// パレットが宣言したランプだけを見る — 明度が近いことでは代用できない
/// («たまたま似た色» と «同じ傾斜の隣の段» は別のものである) ．
fn adjacent_in_a_ramp(palette: &Palette, a: u8, b: u8) -> bool {
    palette.ramps().iter().any(|ramp| {
        let e = ramp.entries();
        let (Some(i), Some(j)) = (
            e.iter().position(|&x| x == a),
            e.iter().position(|&x| x == b),
        ) else {
            return false;
        };
        i.abs_diff(j) == 1
    })
}

/// **周囲との色差が小さければ «段» である** (宣言が残らない経路のための同義の検査)．
///
/// 派手な色の迷子は距離が大きいので今までどおり鳴る (D70 が捨てた案とは向きが逆) ．
fn is_a_shading_step(palette: &Palette, a: u8, b: u8, cfg: &LintConfig) -> bool {
    let (Some(x), Some(y)) = (palette.lab_of(a), palette.lab_of(b)) else {
        return false;
    };
    pxsmith_core::color::distance_sq(x, y, 1.0).sqrt() < cfg.stray_min_distance
}

/// 領域に接している添字を集める (孤立判定の «周囲») ．
fn neighbour_indices(
    regions: &RegionMap,
    canvas: &IndexedCanvas,
    region: &pxsmith_core::geom::regions::Region,
) -> Vec<u8> {
    let mut out = Vec::new();
    let transparent = canvas.transparent();
    for p in region.bbox.iter() {
        if regions.at(p).map(|r| r.id) != Some(region.id) {
            continue;
        }
        for d in [ivec2(1, 0), ivec2(-1, 0), ivec2(0, 1), ivec2(0, -1)] {
            if let Some(n) = regions.at(p + d)
                && n.id != region.id
                && Some(n.index) != transparent
                && !out.contains(&n.index)
            {
                out.push(n.index);
            }
        }
    }
    out
}

// --- ルール 5: 彩度カーブ異常 ---

fn rule_5_chroma_curve(palette: &Palette, report: &mut Report) {
    let r = rule(5).expect("ルール 5 は定義済み");
    for (i, ramp) in palette.ramps().iter().enumerate() {
        let labs: Vec<Oklab> = ramp
            .entries()
            .iter()
            .filter_map(|&e| palette.lab_of(e))
            .collect();
        if labs.len() < 3 {
            continue;
        }
        let chromas: Vec<f32> = labs.iter().map(|l| l.chroma()).collect();
        let rising = chromas.windows(2).all(|w| w[0] <= w[1]);
        let falling = chromas.windows(2).all(|w| w[0] >= w[1]);
        if rising || falling {
            report.push(Violation::new(
                r,
                format!(
                    "ランプ {i} の彩度が明度に対し単調 ({})．中間で最大になる形が自然",
                    if rising { "増加" } else { "減少" }
                ),
            ));
        }
    }
}

// --- ルール 9: ミクセル ---

/// **ルール 9 が検査できたか** — «鳴らなかった» と «検査していない» を分ける
/// (D77 ・D104 ・D142 と同じ作法．D164 で足し，D172 で厳密判定に合わせ直した)．
///
/// ルール 9 は**窓ごとに升の大きさを厳密に判定し，2 通り以上あれば鳴る**．
/// だから窓が 2 つ以上決まらなければ鳴りようがない — 黙って通すと
/// «検査して問題が無かった» と読まれる．
#[derive(Clone, Debug)]
pub struct MixelCoverage {
    /// 並べた窓の数．
    pub windows: usize,
    /// **升が決まった窓の数** — 判定の分母である．
    pub pinned: usize,
    /// **平らで何も言えなかった窓** — «測れなかった» 側 (D172)．
    pub flat: usize,
    /// 使った窓の一辺．
    pub window: u32,
    /// 画布の大きさ．
    pub size: (u32, u32),
}

impl MixelCoverage {
    /// 升が 2 通り以上になりうるか．
    pub fn checked(&self) -> bool {
        self.pinned >= 2
    }

    /// **この検査が見られる一番小さい混入** — 窓より小さい拡大は原理的に見えない．
    ///
    /// 升は窓ごとに決まるので，**窓 1 つがまるごと拡大側に入っていないと
    /// その升は立たない**．分解能の下限であって閾値ではない (D172)．
    pub fn resolution(&self) -> u32 {
        self.window
    }

    /// 検査できなかった理由．**検査できたなら `None`**．
    pub fn why_not(&self) -> Option<String> {
        if self.checked() {
            return None;
        }
        let (w, h) = self.size;
        if self.windows < 2 {
            return Some(format!(
                "窓 {} がこの画布 ({w}x{h}) に 1 つしか並ばない",
                self.window
            ));
        }
        Some(format!(
            "{} 窓のうち升が決まったのは {} つで，残り {} 窓は平らで何も言えない \
             (平らな窓はどの升でも揃うので «測れなかった» に数える)",
            self.windows, self.pinned, self.flat
        ))
    }
}

/// ルール 9 の検査範囲を測る (D164 ・D172)．
pub fn mixel_coverage(img: &RgbaCanvas, cfg: &LintConfig) -> MixelCoverage {
    let (by_k, flat) = exact_grid_votes(img, cfg.mixel_window, cfg.mixel_max_k);
    let pinned: usize = by_k.values().sum();
    MixelCoverage {
        windows: pinned + flat,
        pinned,
        flat,
        window: cfg.mixel_window,
        size: (img.width(), img.height()),
    }
}

fn rule_9_mixels(img: &RgbaCanvas, cfg: &LintConfig, report: &mut Report) {
    let r = rule(9).expect("ルール 9 は定義済み");
    // **厳密な升判定を使う** (D172) — 統計的推定器では «等倍の絵に 2 倍が混ざる»
    // という書籍の言うミクセルを窓をどう選んでも検出できない (D164)．
    // **平らな窓は投票しない** — «測れなかった» を «格子 1» に混ぜてはいけない
    let (by_k, _flat) = exact_grid_votes(img, cfg.mixel_window, cfg.mixel_max_k);
    // **等倍が混ざっていることを要求する** (D172 の端から端まで通して出た) ．
    //
    // 書籍の言うミクセルは «**等倍の絵**に拡大された部分が混ざる» ことである
    // (Pixel Logic PAGE:021) ．升が 2 通りあるだけで鳴らすと，
    // **一様に拡大した絵が誤爆する** — 絵が平らな場所では $2s$ の升でも揃うので
    // $s$ と $2s$ が並び立つ (実測 30 枚中 23 枚) ．`pxsmith lint` は渡された PNG が
    // 等倍か拡大かを知らないので，**絞りを規則の側に置く**しかない．
    //
    // 一様に拡大された絵に対する «格子が場所により違う» の判定は
    // ルール 2 ・`pxsmith conform` の持ち場である (D37 の分担はここで保たれる) ．
    if !votes_show_mixel(&by_k) {
        return;
    }
    let mut sizes: Vec<String> = by_k
        .iter()
        .map(|(k, n)| format!("{k} 画素の升が {n} 窓"))
        .collect();
    sizes.sort();
    report.push(Violation::new(
        r,
        format!(
            "升の大きさが場所により異なる ({}) — 等倍の絵に拡大された部分が\
             混ざっている (ミクセル)",
            sizes.join(" ・")
        ),
    ));
}

// --- ルール 4: アウトライン角の重なり ---

/// **シルエットの縁を占めている色** — その絵の «縁取り» を 1 色だけ返す．
///
/// 縁 (透明な隣か画像の外に接する不透明画素) のうち同じ色が `min_outline_share`
/// 以上を占め，**かつその色に «内側» が 1 画素も無い** (線である) ときだけ返す．
///
/// > [!warning] **«内側の無い色» を縁取りと呼んではいけない．**
/// > 最初はそう定義して測ったところ，良い絵 64 枚のうち **47 枚 (73.4%) が
/// > blocking** になった — 密な質感のタイルはどの色にも «4 近傍が全部自分» の画素が
/// > 無く，$2 \times 2$ の塊はいくらでもある．**縁取りは «シルエットを囲んでいる»
/// > ことで定義する** — 全面が不透明なタイルには縁が無いので，そもそも掛からない．
fn outline_index(canvas: &IndexedCanvas, min_share: f32, max_interior: f32) -> Option<u8> {
    let transparent = canvas.transparent();
    let opaque = |p: IVec2| canvas.get_at(p).is_some_and(|i| transparent != Some(i));
    let mut ring: BTreeMap<u8, usize> = BTreeMap::new();
    let mut total = 0usize;
    for p in canvas.bounds().iter() {
        if !opaque(p) {
            continue;
        }
        // **画像の外は «外» に数えない．** 縁取りは透明に対して描くものである —
        // 画面いっぱいのタイル (実測で 61 枚中 26 枚) は縁が無いので掛からない．
        // 数えると，端に並んだ幅 2 画素の帯が «縁取り» になり，タイル 1 枚で
        // 30 件の違反が出た
        let on_ring = [ivec2(1, 0), ivec2(-1, 0), ivec2(0, 1), ivec2(0, -1)]
            .iter()
            .any(|d| {
                canvas
                    .get_at(p + *d)
                    .is_some_and(|i| transparent == Some(i))
            });
        if !on_ring {
            continue;
        }
        total += 1;
        if let Some(i) = canvas.get_at(p) {
            *ring.entry(i).or_default() += 1;
        }
    }
    if total == 0 {
        return None;
    }
    // 同点は小さい添字 (決定論性の規則 2)
    let (index, n) = ring
        .into_iter()
        .max_by_key(|(i, n)| (*n, std::cmp::Reverse(*i)))?;
    if (n as f32 / total as f32) < min_share {
        return None;
    }
    // **縁を占めているだけでは足りない．線であること (内側がほとんど無いこと) も
    // 要る．** 縁取りの無い絵では «体の色» が縁を占めるので，これが無いと面をまるごと
    // 縁取りと呼ぶ (実測で良い絵 35 / 64 枚 ・違反 2068 件) ．
    //
    // **«内側が 1 画素も無い» では厳しすぎる．** 太った縁取り — まさに検出したい
    // 相手 — は角が埋まって内側を持つので，そこで弾くと**負例が 8 枚中 1 枚しか
    // 鳴らなかった**．割合で見る．
    let (mut area, mut interior) = (0usize, 0usize);
    for p in canvas.bounds().iter() {
        if canvas.get_at(p) != Some(index) {
            continue;
        }
        area += 1;
        if [ivec2(1, 0), ivec2(-1, 0), ivec2(0, 1), ivec2(0, -1)]
            .iter()
            .all(|d| canvas.get_at(p + *d) == Some(index))
        {
            interior += 1;
        }
    }
    (area > 0 && (interior as f32 / area as f32) <= max_interior).then_some(index)
}

/// ルール 4 — **縁取りの角が重なっている** (設計書 7.3)．
///
/// 縁取りは 1 画素幅の線なので $2 \times 2$ の塊を作らない．作っているなら，そこは
/// **線が二重に乗った角**である — 角で «曲がる» のではなく «足す» と起きる．
///
/// 対象は [`outline_index`] が返す «シルエットを囲んでいる色» だけである．
/// 全面が不透明なタイルには縁が無いので掛からない．
fn rule_4_outline_corners(canvas: &IndexedCanvas, cfg: &LintConfig, report: &mut Report) {
    let r = rule(4).expect("ルール 4 は定義済み");
    let Some(index) = outline_index(canvas, cfg.min_outline_share, cfg.max_outline_interior) else {
        return;
    };
    let transparent = canvas.transparent();
    // **縁に接している塊だけを見る．** 縁取りの色は絵の内側にも線として使われる —
    // 実測では象の脚の間を仕切る幅 2 画素の黒線が 11 件すべての正体だった．
    // 縁取りの «角» はシルエットの上にしかない
    let on_ring = |p: IVec2| {
        [ivec2(1, 0), ivec2(-1, 0), ivec2(0, 1), ivec2(0, -1)]
            .iter()
            .any(|d| {
                canvas
                    .get_at(p + *d)
                    .is_some_and(|i| transparent == Some(i))
            })
    };
    // **細い部分の «両側から» 来た縁は角の重なりではない．**
    //
    // 幅 3 画素の腕を内側へ 1 画素ずつ縁取ると，左右の縁が隣り合って $2 \times 2$ に
    // なる — これは `pxsmith outline` の正しい出力であり (D84 の «内側に描く») ，
    // 角を足した結果ではない．D58 の優先順位に従い**道具を正としてルールを直す**．
    //
    // 見分け方は «透明がどちら側にあるか» である．角なら透明は隣り合う 2 方向に
    // しか無いが，細い部分では**向かい合う 2 方向**にある．
    let opposite_sides = |p: IVec2| {
        let square = [ivec2(0, 0), ivec2(1, 0), ivec2(0, 1), ivec2(1, 1)];
        let (mut left, mut right, mut up, mut down) = (false, false, false, false);
        for s in square {
            for d in [ivec2(1, 0), ivec2(-1, 0), ivec2(0, 1), ivec2(0, -1)] {
                if canvas
                    .get_at(p + s + d)
                    .is_some_and(|i| transparent == Some(i))
                {
                    match (d.x, d.y) {
                        (1, 0) => right = true,
                        (-1, 0) => left = true,
                        (0, 1) => down = true,
                        _ => up = true,
                    }
                }
            }
        }
        (left && right) || (up && down)
    };

    let mut found: Vec<IVec2> = Vec::new();
    for p in canvas.bounds().iter() {
        let square = [ivec2(0, 0), ivec2(1, 0), ivec2(0, 1), ivec2(1, 1)];
        if square.iter().all(|d| canvas.get_at(p + *d) == Some(index))
            && square.iter().any(|d| on_ring(p + *d))
            && !opposite_sides(p)
        {
            found.push(p);
        }
    }
    // **1 か所だけの重なりは違反にしない．** 幅 2 画素の脚や «太い縁» は狙って
    // 描かれる — 良い絵で鳴った 5 枚のうち 4 枚がそれで，どれも 1 〜 4 か所だった．
    // 角を «足して» 描いた縁取りは絵の全周で重なる (負例は 2 〜 26 か所) ．
    if found.len() < cfg.min_outline_overlaps {
        return;
    }
    // **«ずっと 2 画素幅の縁取り» は角の重なりではない．**
    // 太い縁取りは様式であって失敗ではない — 角«だけ» が二重なのが失敗である．
    // 重なりに含まれる画素が縁取りの大半を占めるなら，それは «太い縁» である．
    let mut in_block: std::collections::BTreeSet<(i32, i32)> = Default::default();
    for p in &found {
        for d in [ivec2(0, 0), ivec2(1, 0), ivec2(0, 1), ivec2(1, 1)] {
            in_block.insert((p.x + d.x, p.y + d.y));
        }
    }
    let area = canvas
        .pixels()
        .iter()
        .filter(|i| **i == index)
        .count()
        .max(1);
    let share = in_block.len() as f32 / area as f32;
    // **縁取りのごく一部だけの重なりは，縁が段を折り返しただけである．**
    // 急な階段 (1 列で 2 行下がるような縁) を内側へ 1 画素で縁取ると，環の画素が
    // 積み重なって $2 \times 2$ になる — `pxsmith outline --style black` の出力に実際に
    // 3 か所あった．角を «足して» 描いた縁取りは縁の全体に重なりが出る．
    if !(cfg.min_outline_overlap_share..=cfg.max_outline_overlap_share).contains(&share) {
        return;
    }
    for p in found {
        report.push(
            Violation::new(
                r,
                format!("添字 {index} の縁取りが 2x2 に重なっている (角で線が二重になっている)"),
            )
            .at(p)
            .area(IRect::new(p.x, p.y, 2, 2)),
        );
    }
}

// --- ルール 6: 単色影 ---

/// OKLab の色相 (度)．彩度が無い色では意味を持たない．
fn hue_of(c: Oklab) -> f32 {
    c.b.atan2(c.a).to_degrees()
}

/// 2 つの色相の差 (0 〜 180 度)．
fn hue_gap(a: f32, b: f32) -> f32 {
    let d = (a - b).abs() % 360.0;
    if d > 180.0 { 360.0 - d } else { d }
}

/// ルール 6 — **影面が光面と同一色相の明度違いだけ** (設計書 7.3)．
///
/// 隣り合う面のうち «光と影» と読める組 (明度が `shadow_lightness_delta` 以上
/// 離れている組) について色相の差を見る．差が
/// [`pxsmith_core::ramp::MIN_HUE_SEPARATION`] 未満なら単色影である．
///
/// **どちらの色も彩度が要る．** 彩度の無い灰色には色相が無く，
/// **灰色だけの陰影は様式であって失敗ではない** — 色相を «ずらしていない» のでは
/// なく «ずらす先が無い» ．
fn rule_6_monochrome_shadow(
    regions: &RegionMap,
    palette: &Palette,
    canvas: &IndexedCanvas,
    cfg: &LintConfig,
    report: &mut Report,
) {
    let r = rule(6).expect("ルール 6 は定義済み");
    let mut reported: Vec<(u8, u8)> = Vec::new();

    for region in regions.regions() {
        if region.area < cfg.min_region_area || canvas.transparent() == Some(region.index) {
            continue;
        }
        let Some(a) = palette.lab_of(region.index) else {
            continue;
        };
        for &id in &region.neighbors {
            let other = &regions.regions()[id as usize];
            if other.area < cfg.min_region_area
                || canvas.transparent() == Some(other.index)
                || other.index == region.index
            {
                continue;
            }
            let Some(b) = palette.lab_of(other.index) else {
                continue;
            };
            let pair = (region.index.min(other.index), region.index.max(other.index));
            if reported.contains(&pair) {
                continue;
            }
            // «光と影» と読める組だけを見る
            if (a.l - b.l).abs() < cfg.shadow_lightness_delta {
                continue;
            }
            // 灰色には色相が無い
            if a.chroma() < cfg.min_shadow_chroma || b.chroma() < cfg.min_shadow_chroma {
                continue;
            }
            let gap = hue_gap(hue_of(a), hue_of(b));
            if gap < cfg.min_shadow_hue_gap {
                reported.push(pair);
                report.push(
                    Violation::new(
                        r,
                        format!(
                            "隣接する添字 {} と {} が明度違いだけ (色相差 {gap:.1} 度 < {:.1} 度)．\
                             影は色相をずらすこと",
                            pair.0, pair.1, cfg.min_shadow_hue_gap
                        ),
                    )
                    .area(region.bbox),
                );
            }
        }
    }
}

// --- ルール 7: 反転同値の陰影不整合 ---

/// ルール 7 — **明度勾配の向きが光源方向と矛盾している** (設計書 7.3)．
///
/// 自動ミラーや反転タイル同値判定は，**陰影を持つ素材では光源方向を反転させる**
/// (設計書 6.7 ・6.8) ．反転した絵は «光源が右上» と言いながら左上が明るい，という
/// 形になる — それをここで捕まえる．
///
/// > [!warning] **光源が宣言されていなければ検査しない** ([`LintConfig::light`] は
/// > 既定 `None`) ．絵だけを見て «光源方向» は決まらない — 決めるとすれば絵から
/// > 推定することになるが，それは**推定した向きと絵が合っているかを見る**という
/// > 同語反復である．矛盾を言えるのは «宣言された向き» があるときだけである．
///
/// 明度勾配は**シルエットの内側**で中心差分を取り，全画素で平均する．平均した
/// 向きが $\ell$ (面から光源へ向かう向き) と逆を向いていたら違反とする．
fn rule_7_flipped_shading(
    canvas: &IndexedCanvas,
    palette: &Palette,
    cfg: &LintConfig,
    report: &mut Report,
) {
    let r = rule(7).expect("ルール 7 は定義済み");
    let Some(source) = cfg.light else { return };
    let Some(agreement) = shading_agreement_with(canvas, palette, source, cfg.shading_min_pixels)
    else {
        return;
    };
    if agreement < cfg.min_shading_agreement {
        report.push(Violation::new(
            r,
            format!(
                "明度勾配が光源方向と合っていない (一致度 {agreement:.2} < {:.2})．\
                 反転したまま陰影を持ち込んでいないか",
                cfg.min_shading_agreement
            ),
        ));
    }
}

/// **平均した明度勾配が $\ell$ とどれだけ合っているか** ($-1$ 〜 $1$)．
///
/// 閾値を測るための口でもある (`pxsmith-calib flip`) ．
pub fn shading_agreement(
    canvas: &IndexedCanvas,
    palette: &Palette,
    source: pxsmith_core::ramp::LightSource,
) -> Option<f32> {
    shading_agreement_with(
        canvas,
        palette,
        source,
        LintConfig::default().shading_min_pixels,
    )
}

fn shading_agreement_with(
    canvas: &IndexedCanvas,
    palette: &Palette,
    source: pxsmith_core::ramp::LightSource,
    min_pixels: u32,
) -> Option<f32> {
    // **平行光源だけを見る．** 点 ・線 ・面の光源では $\ell$ が画素ごとに違うので，
    // 絵全体で 1 つに平均した勾配と突き合わせても意味を持たない —
    // 実測でも `pxsmith shade --preset night` (点光源) は 64 枚すべてで一致度が
    // $-0.54$ 〜 $-1.0$ になり，平行光源の 4 プリセット (最小 $0.714$) と
    // まったく別の分布になった．
    let pxsmith_core::ramp::LightSource::Directional { .. } = source else {
        return None;
    };
    let l = pxsmith_core::outline::light_direction(source);

    let lightness = |p: IVec2| -> Option<f32> {
        let i = canvas.get_at(p)?;
        if canvas.transparent() == Some(i) {
            return None;
        }
        let c = palette.get(i)?;
        (c.a != 0).then(|| palette.lab_of(i).map(|x| x.l))?
    };

    // f64 で積む．**足す順序で答えが変わらないように**走査順に固定する (規則 3)
    let (mut gx, mut gy, mut n) = (0.0f64, 0.0f64, 0usize);
    for p in canvas.bounds().iter() {
        if lightness(p).is_none() {
            continue;
        }
        let (Some(east), Some(west), Some(south), Some(north)) = (
            lightness(p + ivec2(1, 0)),
            lightness(p + ivec2(-1, 0)),
            lightness(p + ivec2(0, 1)),
            lightness(p + ivec2(0, -1)),
        ) else {
            continue;
        };
        gx += (east - west) as f64 * 0.5;
        gy += (south - north) as f64 * 0.5;
        n += 1;
    }
    if n < min_pixels as usize {
        return None;
    }
    let norm = (gx * gx + gy * gy).sqrt();
    // 勾配が平均して消えている絵 (平坦 ・対称) には向きが無い
    if norm <= f64::EPSILON {
        return None;
    }
    Some((gx / norm) as f32 * l.x + (gy / norm) as f32 * l.y)
}

/// **ルール 7 が勾配を取れる画素の数．**
///
/// [`shading_agreement`] が `None` を返す理由は 2 つあり，**混ぜてはいけない**．
///
/// | 理由 | 見分け方 |
/// | --- | --- |
/// | 標本が足りない (タイルが小さい ・細い) | この関数が `shading_min_pixels` 未満を返す |
/// | **向きが無い** (平坦か左右対称で勾配が打ち消し合う) | 標本は足りているのに `None` |
///
/// 2 つ目は**見逃しではない** — 左右対称な絵は左右反転しても同じ絵なので，
/// 光源と矛盾のしようがない．autotile は象限を鏡像で組むので，
/// **47 枚のうち左右対称になるものがここに落ちる**．
pub fn shading_sample_count(canvas: &IndexedCanvas, palette: &Palette) -> usize {
    let lightness = |p: IVec2| -> Option<f32> {
        let i = canvas.get_at(p)?;
        if canvas.transparent() == Some(i) {
            return None;
        }
        let c = palette.get(i)?;
        (c.a != 0).then(|| palette.lab_of(i).map(|x| x.l))?
    };
    let mut n = 0usize;
    for p in canvas.bounds().iter() {
        if lightness(p).is_none() {
            continue;
        }
        let (Some(_), Some(_), Some(_), Some(_)) = (
            lightness(p + ivec2(1, 0)),
            lightness(p + ivec2(-1, 0)),
            lightness(p + ivec2(0, 1)),
            lightness(p + ivec2(0, -1)),
        ) else {
            continue;
        };
        n += 1;
    }
    n
}

/// **絵が «どちらから照らされているように見えるか»** — 平均した明度勾配の向き．
///
/// > [!warning] **これで «その絵» を検査してはいけない．** 推定した向きと絵が
/// > 合っているかを見るのは同語反復である (D89．ルール 7 が光源の宣言を要求する
/// > 理由がこれである) ．
/// >
/// > 使ってよいのは**別の絵を検査するとき**である — 元の絵から向きを取り，
/// > その向きの下で**反転した絵**を検査するのは同語反復ではない．方向展開の
/// > 測定 (`pxsmith-calib direction`) はこの形で使う．
pub fn mean_lightness_direction(canvas: &IndexedCanvas, palette: &Palette) -> Option<Vec2> {
    mean_lightness_direction_with(canvas, palette, LintConfig::default().shading_min_pixels)
}

fn mean_lightness_direction_with(
    canvas: &IndexedCanvas,
    palette: &Palette,
    min_pixels: u32,
) -> Option<Vec2> {
    let lightness = |p: IVec2| -> Option<f32> {
        let i = canvas.get_at(p)?;
        if canvas.transparent() == Some(i) {
            return None;
        }
        let c = palette.get(i)?;
        (c.a != 0).then(|| palette.lab_of(i).map(|x| x.l))?
    };
    let (mut gx, mut gy, mut n) = (0.0f64, 0.0f64, 0usize);
    for p in canvas.bounds().iter() {
        if lightness(p).is_none() {
            continue;
        }
        let (Some(east), Some(west), Some(south), Some(north)) = (
            lightness(p + ivec2(1, 0)),
            lightness(p + ivec2(-1, 0)),
            lightness(p + ivec2(0, 1)),
            lightness(p + ivec2(0, -1)),
        ) else {
            continue;
        };
        gx += (east - west) as f64 * 0.5;
        gy += (south - north) as f64 * 0.5;
        n += 1;
    }
    if n < min_pixels as usize {
        return None;
    }
    let norm = (gx * gx + gy * gy).sqrt();
    if norm <= f64::EPSILON {
        return None;
    }
    Some(Vec2 {
        x: (gx / norm) as f32,
        y: (gy / norm) as f32,
    })
}

// --- ルール 8: ジャギー ---

/// ルール 8 — **ラン長列の谷** (設計書 7.3 ・6.4)．
///
/// 検出そのものは [`pxsmith_core::geom::jaggy`] が持っている (M1a で作った) ．ここは
/// 報告に写すだけである．**対象は輪郭線ではなく全色境界** (D33) ．
///
/// > [!warning] **深刻度を blocking から advisory へ落とした (D85)．**
/// > 設計書 7.3 は blocking と定めるが，**良い絵と «ぎざぎざの縁» が分かれない**．
/// >
/// > | 群 | 谷の件数 | 1 件以上鳴る絵 | 1 区間あたりの最大 | $\delta \ge 2$ が 1 件以上 |
/// > | --- | --- | --- | --- | --- |
/// > | 良い絵 (CC0 61 枚) | 146 | **36 / 61** | 2 (2 枚) | 14 / 61 |
/// > | 負例 (段幅を崩した階段 8 枚) | 0 〜 4 | 5 / 8 | 2 (2 枚) | 1 / 8 |
/// >
/// > 絵あたりの件数 ・ラン数に対する比 (良い絵は中央 0.0027 ・最大 0.060，負例は
/// > 0 〜 0.10) ・1 区間あたりの密度 ・谷の深さのどれで切っても**群が重なる**．
/// > blocking にすると良い絵の 59% が止まる — D70 の «超えられない検査で出力を
/// > 止めない» と同じ判断で advisory に置く．
/// >
/// > **負例が弱いのではない．** 3 通り作り直して測った (境界画素の 40% を入れ替える ・
/// > 25% を間隔を空けて突く ・ラン長列を見て削る) ．前 2 つは**刻みすぎて谷が消え**，
/// > 3 つ目は種 64 枚に «3 ラン以上の単調区間» がほとんど無く 1 件も削れなかった．
/// > 最後に段幅の揃わない階段を描いたのが上の負例である．
fn rule_8_jaggies(canvas: &IndexedCanvas, cfg: &LintConfig, report: &mut Report) {
    let r = rule(8).expect("ルール 8 は定義済み");
    let found = pxsmith_core::geom::jaggy::analyze_canvas(canvas, cfg.jaggy_max_move);
    for jaggy in &found.jaggies {
        let mut v = Violation::new(
            r,
            format!(
                "ラン長 {} が両隣 (目標 {}) より短い{}",
                jaggy.length,
                jaggy.target,
                if jaggy.on_straight_chain {
                    // **一定の傾きの直線には谷が必ず現れる** (D169)．`pxsmith smooth` は
                    // ここを触らないので «直せる» と書くと嘘になる — 助言は
                    // 道具が実際にすることと合っていなければならない
                    "．一定の傾きの直線なので幾何が決めた刻みである (pxsmith smooth は触らない)"
                } else if jaggy.within_limit {
                    "．pxsmith smooth で直せる"
                } else {
                    // 移動上限を超える谷は**意図的なディテールの可能性がある** (R22)
                    "．移動上限を超えるので意図的なディテールかもしれない"
                }
            ),
        );
        if let Some(p) = jaggy.at() {
            v = v.at(p);
        }
        report.push(v);
    }
}

// --- ルール 12: バンディング ---

/// ルール 12 — **同じ長さのランが並走している** (設計書 7.3)．
///
/// バンディングは «段の並びが 2 本そろって同じ形で走る» ことで見える — 陰影の帯が
/// 輪郭を一定の間隔でなぞると，段の列が完全に同期して縞に見える [^pl] ．
///
/// 判定は 3 つを同時に満たす区間の組である．
///
/// 1. 主軸が同じ (どちらも横向き，またはどちらも縦向き)
/// 2. 主軸方向に重なりがあり，**直交方向の隔たりが `band_max_gap` 以内**
/// 3. ラン長列が `band_min_runs` 本以上そろって一致する
///
/// **1 本の区間の中で «同じ長さのランが続く» ことは違反にしない** — それは
/// 揃った階段であって，むしろ良い形である (ルール 8 の裏返し) ．
fn rule_12_banding(canvas: &IndexedCanvas, cfg: &LintConfig, report: &mut Report) {
    use pxsmith_core::geom::runs::run_lengths;
    use pxsmith_core::geom::{split_monotone, trace_contours};

    let r = rule(12).expect("ルール 12 は定義済み");
    let mut indices: Vec<u8> = canvas.pixels().to_vec();
    indices.sort_unstable();
    indices.dedup();

    /// 区間 1 本ぶんの «形»．
    struct Band {
        runs: Vec<u32>,
        horizontal: bool,
        /// 主軸方向の範囲と，直交方向の位置 (中央値の代わりに先頭の点を使う)．
        along: (i32, i32),
        across: i32,
    }

    let mut bands: Vec<Band> = Vec::new();
    for index in indices {
        if canvas.transparent() == Some(index) {
            continue;
        }
        let mask = canvas.mask_of(index);
        for contour in trace_contours(&mask) {
            for chain in split_monotone(&contour) {
                let runs = run_lengths(&chain);
                if runs.len() < cfg.band_min_runs {
                    continue;
                }
                let pts = chain.points();
                let (Some(first), Some(last)) = (pts.first(), pts.last()) else {
                    continue;
                };
                let horizontal = chain.is_horizontal();
                let (along, across) = if horizontal {
                    ((first.x.min(last.x), first.x.max(last.x)), first.y)
                } else {
                    ((first.y.min(last.y), first.y.max(last.y)), first.x)
                };
                bands.push(Band {
                    runs,
                    horizontal,
                    along,
                    across,
                });
            }
        }
    }

    let mut reported: Vec<(i32, i32)> = Vec::new();
    for (i, a) in bands.iter().enumerate() {
        for b in bands.iter().skip(i + 1) {
            if a.horizontal != b.horizontal {
                continue;
            }
            let gap = (a.across - b.across).abs();
            if gap == 0 || gap > cfg.band_max_gap as i32 {
                continue;
            }
            // 主軸方向に重なっていること
            if a.along.1 < b.along.0 || b.along.1 < a.along.0 {
                continue;
            }
            let n = shared_run_prefix(&a.runs, &b.runs);
            if n < cfg.band_min_runs {
                continue;
            }
            let at = if a.horizontal {
                ivec2(a.along.0, a.across)
            } else {
                ivec2(a.across, a.along.0)
            };
            if reported.contains(&(at.x, at.y)) {
                continue;
            }
            reported.push((at.x, at.y));
            report.push(
                Violation::new(
                    r,
                    format!(
                        "同じ長さのランが {n} 本そろって並走している (隔たり {gap} 画素)．\
                         帯が輪郭をなぞって縞に見える"
                    ),
                )
                .at(at),
            );
        }
    }
}

/// 2 つのラン長列が «そろって走る» 長さ — どこかで揃い始めて続く最長の一致．
///
/// 端は切れ方が揃わない (区間の切り口は形で決まる) ので，**先頭合わせではなく
/// ずらして探す**．
fn shared_run_prefix(a: &[u32], b: &[u32]) -> usize {
    let mut best = 0usize;
    for offset in 0..a.len() {
        let mut n = 0usize;
        while offset + n < a.len() && n < b.len() && a[offset + n] == b[n] {
            n += 1;
        }
        best = best.max(n);
    }
    for offset in 0..b.len() {
        let mut n = 0usize;
        while offset + n < b.len() && n < a.len() && b[offset + n] == a[n] {
            n += 1;
        }
        best = best.max(n);
    }
    best
}

// --- ルール 10: ディザの塊化 ---

fn rule_10_dither_clumping(canvas: &IndexedCanvas, cfg: &LintConfig, report: &mut Report) {
    let r = rule(10).expect("ルール 10 は定義済み");
    // ディザとみなせる領域を先に見つけ，その中で同色が続いていないかを見る
    let opts = DenoiseOptions::default();
    for area in dither_areas(canvas, &opts) {
        let sub = crop(canvas, area);
        let map = label_regions(&sub);
        for region in map.regions() {
            let longest = region.bbox.w.max(region.bbox.h);
            if region.area > 1 && longest > cfg.dither_clump {
                report.push(
                    Violation::new(
                        r,
                        format!(
                            "ディザ領域で添字 {} が {longest} 画素続いている",
                            region.index
                        ),
                    )
                    .area(area),
                );
                break;
            }
        }
    }
}

/// ディザとみなせる窓を既定の設定で求める．
///
/// **«ディザとは何か» の定義はここ 1 つである** — ルール 10 ・15 ・24 が同じ口を
/// 使う (D110) ．
/// 窓の一辺だけを変えて同じ検出器を掛ける．
///
/// **静止画のルール 10 ・15 は既定の窓 (8) を使い，ルール 24 は 4 を使う** —
/// «ディザとは何か» の定義は 1 つのままで，見る大きさだけが違う (D110)．
pub(crate) fn dither_areas_windowed(canvas: &IndexedCanvas, window: u32) -> Vec<IRect> {
    dither_areas(
        canvas,
        &DenoiseOptions {
            window,
            ..DenoiseOptions::default()
        },
    )
}

/// ディザとみなせる窓．規則的かどうかは問わない — 塊化はどちらでも問題になる．
fn dither_areas(canvas: &IndexedCanvas, opts: &DenoiseOptions) -> Vec<IRect> {
    let loose = DenoiseOptions {
        // 規則的なディザも対象に含めるため，規則性の判定を無効にする
        regularity: 2.0,
        ..*opts
    };
    detect_dither_noise(canvas, &loose)
        .into_iter()
        .map(|n| n.area)
        .collect()
}

fn crop(canvas: &IndexedCanvas, area: IRect) -> IndexedCanvas {
    let fill = canvas.transparent().unwrap_or(0);
    canvas.crop(area, fill)
}

// --- ルール 11: 明度差不足 ---

fn rule_11_lightness_delta(
    regions: &RegionMap,
    palette: &Palette,
    canvas: &IndexedCanvas,
    cfg: &LintConfig,
    report: &mut Report,
) {
    let r = rule(11).expect("ルール 11 は定義済み");
    let mut reported: Vec<(u8, u8)> = Vec::new();

    for region in regions.regions() {
        if region.area < cfg.min_region_area || canvas.transparent() == Some(region.index) {
            continue;
        }
        let Some(a) = palette.lab_of(region.index) else {
            continue;
        };
        for &id in &region.neighbors {
            let other = &regions.regions()[id as usize];
            if other.area < cfg.min_region_area
                || canvas.transparent() == Some(other.index)
                || other.index == region.index
            {
                continue;
            }
            let Some(b) = palette.lab_of(other.index) else {
                continue;
            };
            let pair = (region.index.min(other.index), region.index.max(other.index));
            if reported.contains(&pair) {
                continue;
            }
            let delta = (a.l - b.l).abs();
            if delta < cfg.min_lightness_delta {
                reported.push(pair);
                report.push(
                    Violation::new(
                        r,
                        format!(
                            "隣接する添字 {} と {} の ΔL が {delta:.3} (下限 {:.3})",
                            pair.0, pair.1, cfg.min_lightness_delta
                        ),
                    )
                    .area(region.bbox),
                );
            }
        }
    }
}

// --- ルール 13: pillow shading ---

/// **距離場と明度の相関** $\rho = \mathrm{corr}(d(p), L(p))$ (設計書 7.3)．
///
/// 1 に近いほど «縁から中心へ向かって一様に明るくなる» — 光源方向を持たない
/// 同心状の陰影 (pillow shading) の疑いである．
///
/// 返り値が `None` になるのは，シルエットが小さすぎるか，距離場か明度のどちらかが
/// **定数**のとき (相関が定義できない) ．
///
/// > [!warning] **これは代理指標である** (D58) ．
/// > 正しく陰影付けされた凸形状でも相当に相関する — `pxsmith shade` の出力自身が
/// > そうである．**`pxsmith shade` を正として閾値を決める**．
pub fn pillow_correlation(canvas: &IndexedCanvas, palette: &Palette) -> Option<f32> {
    let mask = silhouette_of(canvas, palette);
    let distance = signed_distance(&mask);

    // f64 で積む．**足す順序で答えが変わらないように**画素の走査順に固定する (規則 3)
    let mut xs: Vec<f64> = Vec::new();
    let mut ys: Vec<f64> = Vec::new();
    for p in canvas.bounds().iter() {
        if !mask.get(p) {
            continue;
        }
        let (Some(d), Some(lab)) = (
            distance.copied(p),
            canvas.get_at(p).and_then(|i| palette.lab_of(i)),
        ) else {
            continue;
        };
        xs.push(d as f64);
        ys.push(lab.l as f64);
    }
    pearson(&xs, &ys).map(|r| r as f32)
}

/// **シルエット — キャンバスの透明添字とパレットのアルファの両方で見る．**
///
/// 添字の面が «透明添字» を持たないまま渡されることがある (PNG をそのまま添字に
/// した場合など) ．そのときキャンバスだけを見ると**背景まで形の一部**になり，
/// 距離場が画像の縁から測られて相関が丸ごと別物になる．
fn silhouette_of(canvas: &IndexedCanvas, palette: &Palette) -> pxsmith_core::geom::Mask {
    let mut m = pxsmith_core::geom::Mask::new(canvas.width(), canvas.height());
    for p in canvas.bounds().iter() {
        let Some(i) = canvas.get_at(p) else { continue };
        let opaque = palette.get(i).is_some_and(|c| c.a != 0);
        if opaque && canvas.transparent() != Some(i) {
            m.set(p, true);
        }
    }
    m
}

/// ピアソンの相関係数．標本が 2 未満か，どちらかが定数なら `None`．
fn pearson(xs: &[f64], ys: &[f64]) -> Option<f64> {
    let n = xs.len();
    if n < 2 {
        return None;
    }
    let mean = |v: &[f64]| v.iter().sum::<f64>() / n as f64;
    let (mx, my) = (mean(xs), mean(ys));
    let mut sxy = 0.0;
    let mut sxx = 0.0;
    let mut syy = 0.0;
    for (x, y) in xs.iter().zip(ys) {
        let (dx, dy) = (x - mx, y - my);
        sxy += dx * dy;
        sxx += dx * dx;
        syy += dy * dy;
    }
    if sxx <= f64::EPSILON || syy <= f64::EPSILON {
        return None;
    }
    Some(sxy / (sxx * syy).sqrt())
}

fn rule_13_pillow_shading(
    canvas: &IndexedCanvas,
    palette: &Palette,
    cfg: &LintConfig,
    report: &mut Report,
) {
    let r = rule(13).expect("ルール 13 は定義済み");
    // 小さいシルエットでは相関が雑音で振り切れる．**先に落とす**
    if silhouette_of(canvas, palette).count() < cfg.pillow_min_pixels as usize {
        return;
    }
    let Some(rho) = pillow_correlation(canvas, palette) else {
        return;
    };
    if rho > cfg.max_pillow_correlation {
        report.push(Violation::new(
            r,
            format!(
                "距離場と明度の相関が {rho:.3} (上限 {:.3})．\
                 縁から中心へ一様に明るくなっている疑い",
                cfg.max_pillow_correlation
            ),
        ));
    }
}

// --- ルール 14: AA 過多 ---

/// ルール 14 — **中間色の画素が多すぎる** (設計書 7.3)．
///
/// 中間色の数え方は [`pxsmith_core::aa::intermediate_pixels`] にまとめてある．
/// 閾値の根拠は [`LintConfig::max_intermediate_ratio`] を読むこと．
fn rule_14_too_much_aa(
    canvas: &IndexedCanvas,
    palette: &Palette,
    cfg: &LintConfig,
    report: &mut Report,
) {
    let r = rule(14).expect("ルール 14 は定義済み");
    let opaque = canvas
        .pixels()
        .iter()
        .filter(|i| canvas.transparent() != Some(**i))
        .count();
    if opaque == 0 {
        return;
    }
    let count = pxsmith_core::aa::intermediate_pixels(canvas, palette, cfg.intermediate_tolerance);
    let ratio = count as f32 / opaque as f32;
    if ratio > cfg.max_intermediate_ratio {
        report.push(Violation::new(
            r,
            format!(
                "不透明画素の {:.1}% が中間色 (上限 {:.1}%)．\
                 AA は多すぎるより少ない方が良い",
                ratio * 100.0,
                cfg.max_intermediate_ratio * 100.0
            ),
        ));
    }
}

// --- ルール 15: ディザ過多 ---

fn rule_15_dither_ratio(canvas: &IndexedCanvas, cfg: &LintConfig, report: &mut Report) {
    let r = rule(15).expect("ルール 15 は定義済み");
    let total = canvas.size().area();
    if total == 0 {
        return;
    }
    let covered: usize = dither_areas(canvas, &DenoiseOptions::default())
        .iter()
        .map(|a| (a.w * a.h) as usize)
        .sum();
    let ratio = covered as f32 / total as f32;
    if ratio > cfg.max_dither_ratio {
        report.push(Violation::new(
            r,
            format!(
                "画面の {:.1}% がディザ (上限 {:.1}%)．低解像度では密度が過剰に見える",
                ratio * 100.0,
                cfg.max_dither_ratio * 100.0
            ),
        ));
    }
}

// --- ルール 16: 大面積の高彩度色 ---

fn rule_16_large_saturated(
    regions: &RegionMap,
    palette: &Palette,
    canvas: &IndexedCanvas,
    cfg: &LintConfig,
    report: &mut Report,
) {
    let r = rule(16).expect("ルール 16 は定義済み");
    let total = canvas.size().area() as f32;
    if total <= 0.0 {
        return;
    }
    // 同じ色の領域は面積を合算する — 面積効果は色ごとに効く
    let mut by_index: BTreeMap<u8, (u32, IRect)> = BTreeMap::new();
    for region in regions.regions() {
        if canvas.transparent() == Some(region.index) {
            continue;
        }
        let slot = by_index.entry(region.index).or_insert((0, region.bbox));
        slot.0 += region.area;
        slot.1 = slot.1.union(region.bbox);
    }

    for (index, (area, bbox)) in by_index {
        if (area as f32 / total) < cfg.large_area_ratio {
            continue;
        }
        let Some(lab) = palette.lab_of(index) else {
            continue;
        };
        if lab.chroma() > cfg.max_large_chroma || lab.l > cfg.max_large_lightness {
            report.push(
                Violation::new(
                    r,
                    format!(
                        "添字 {index} が画面の {:.1}% を占め，彩度 {:.3} / 明度 {:.3} が高い (面積効果)",
                        area as f32 / total * 100.0,
                        lab.chroma(),
                        lab.l
                    ),
                )
                .area(bbox),
            );
        }
    }
}

// --- ルール 17: 高コントラスト間のディザ ---

fn rule_17_high_contrast_dither(
    canvas: &IndexedCanvas,
    palette: &Palette,
    cfg: &LintConfig,
    report: &mut Report,
) {
    let r = rule(17).expect("ルール 17 は定義済み");
    for noise in detect_dither_noise(
        canvas,
        &DenoiseOptions {
            regularity: 2.0,
            ..DenoiseOptions::default()
        },
    ) {
        let (a, b) = noise.colors;
        let (Some(la), Some(lb)) = (palette.lab_of(a), palette.lab_of(b)) else {
            continue;
        };
        let delta = (la.l - lb.l).abs();
        if delta > cfg.high_contrast_delta {
            report.push(
                Violation::new(
                    r,
                    format!(
                        "添字 {a} と {b} (ΔL = {delta:.3}) をディザで混ぜている (上限 {:.3})",
                        cfg.high_contrast_delta
                    ),
                )
                .area(noise.area),
            );
        }
    }
}

// --- ルール 18: 純黒の使用 ---

fn rule_18_pure_black(palette: &Palette, cfg: &LintConfig, report: &mut Report) {
    let r = rule(18).expect("ルール 18 は定義済み");
    for (i, (c, lab)) in palette.entries().iter().zip(palette.lab()).enumerate() {
        if c.a == 0 {
            continue;
        }
        if lab.l <= cfg.pure_black_lightness && lab.chroma() <= cfg.pure_black_chroma {
            report.push(Violation::new(
                r,
                format!(
                    "添字 {i} が純黒に近い (L = {:.3}，彩度 = {:.3})．暗部にも色を残すこと",
                    lab.l,
                    lab.chroma()
                ),
            ));
        }
    }
}

/// `oklab_of` はルール実装から直接は呼ばないが，パレットを持たない検査で要る．
#[allow(dead_code)]
fn lab(c: pxsmith_core::Rgba8) -> Oklab {
    oklab_of(c)
}

// --- ルール 19: 形の乱雑さ / 20: 接線 / 21: 隣接領域の同色 (M7) ---

/// ルール 19 — **シルエットの縁がその広がりに対して長すぎる**．
///
/// # 何の «連結成分» か
///
/// 設計書 7.3 は «連結成分の周囲長/面積比の異常» とだけ書く．**色の領域ごとに
/// 掛けると良い絵の 93.8% が鳴る** — ドット絵の陰影の帯は 1 画素幅が普通なので，
/// 乱れていなくても比が大きくなるからである (D70 と同じ «適用範囲» の誤り) ．
///
/// 書籍が可読性の章で問うているのは**シルエットが読めるか**である [^pl5]．
/// そこで**不透明な画素の連結成分**に掛ける — «連結成分» の読み方としても
/// 素直であり，色の塗り分けではなく形を見ることになる．
///
/// # 何を測るか
///
/// $P^2 / A$ ([`pxsmith_core::geom::Region::compactness`]) は**細さ**を測ってしまう．
/// 代わりに $P / P_{\mathrm{bbox}}$ ([`pxsmith_core::geom::Region::boundary_excess`]) を
/// 使う — 矩形も対角線も 1 に近く，でこぼこだけが大きくなる．
///
/// | 量 | 閾値 | 良い絵で鳴る | 荒らした絵で捕捉 |
/// | --- | --- | --- | --- |
/// | $P^2/A$ | 25 | 29.5% | 100% |
/// | $P^2/A$ | 40 | 19.7% | 97.1% |
/// | **$P / P_{\mathrm{bbox}}$** | **1.20** | **21.3%** | **100%** |
/// | $P / P_{\mathrm{bbox}}$ | 1.50 | 8.2% | 65.7% |
///
/// **同じ捕捉なら誤爆が小さい方を採る** (付録 C 要調査事項 #2 を閉じた) ．
/// advisory なので 21.3% は許容範囲である (ルール 6 は 39.1% ・18 は 31.2%) ．
///
/// [^pl5]: Pixel Logic 第四章 可読性 «シルエット» (PAGE:103)．
fn rule_19_shape_noise(canvas: &IndexedCanvas, cfg: &LintConfig, report: &mut Report) {
    use pxsmith_core::geom::{Mask, regions::label_mask};
    let r = rule(19).expect("ルール 19 は定義済み");

    let Some(transparent) = canvas.transparent() else {
        // **透明の宣言が無い絵は «全部が絵»** なので，シルエットに乱れは無い
        return;
    };
    let mut mask = Mask::new(canvas.width(), canvas.height());
    for y in 0..canvas.height() as i32 {
        for x in 0..canvas.width() as i32 {
            if canvas.get(x, y).is_some_and(|i| i != transparent) {
                mask.set(ivec2(x, y), true);
            }
        }
    }

    for component in label_mask(&mask, false).components() {
        if component.len() < cfg.shape_noise_min_area as usize {
            continue;
        }
        let mut only = Mask::new(mask.width(), mask.height());
        for p in component {
            only.set(*p, true);
        }
        let Some(bbox) = only.bbox() else { continue };
        let perimeter: u32 = only
            .iter_set()
            .map(|p| {
                [(1, 0), (-1, 0), (0, 1), (0, -1)]
                    .iter()
                    .filter(|(dx, dy)| !only.get(ivec2(p.x + dx, p.y + dy)))
                    .count() as u32
            })
            .sum();
        let box_perimeter = 2 * (bbox.w + bbox.h);
        if box_perimeter == 0 {
            continue;
        }
        let excess = perimeter as f32 / box_perimeter as f32;
        if excess > cfg.max_boundary_excess {
            report.push(
                Violation::new(
                    r,
                    format!(
                        "シルエットの成分 (面積 {}) の縁が広がりに対して長い \
                         (周囲長 {perimeter} / 外接矩形 {box_perimeter} = {excess:.2}．\
                         上限 {:.2}) — 形が読み取りにくい",
                        component.len(),
                        cfg.max_boundary_excess
                    ),
                )
                .area(bbox),
            );
        }
    }
}

/// ルール 20 — **異なる領域が角で 1 点だけ触れている** (接線)．
///
/// 書籍は «パーツ同士が隣接してしまうと，見る人には何が描かれているのか
/// わからなくなります» とする [^pl4]．**辺で接していれば «並んでいる» のであって
/// 接線ではない** — 角だけで触れている組を数える
/// ([`pxsmith_core::geom::RegionMap::corner_touching`])．
///
/// **閾値は面積の下限だけである** (数え上げ．D92) — ディザの 1 画素どうしは
/// いくらでも角で触れるので，**両方が一定の面積を持つ組**に限る．
///
/// > [!warning] **触れている点の «脇» を見ないと `pxsmith shade` の出力が 75% 鳴る．**
/// > 陰影の帯どうしは，中間の帯がくびれた場所で角が出会う (実測で面積 240 と
/// > 196 の帯が «接線» と報告された) ．それは **1 つの面の階調**であって
/// > 部品どうしの接触ではない．書籍が問うているのは «何も挟まずに出会って
/// > いる» 場合なので，**脇が両方とも背景の組**に限る (D58 — 道具を正として
/// > ルールの適用範囲を直す) ．
///
/// [^pl4]: Pixel Logic 第四章 可読性 «空間を空ける» (PAGE:106)．
fn rule_20_tangent(
    regions: &RegionMap,
    canvas: &IndexedCanvas,
    cfg: &LintConfig,
    report: &mut Report,
) {
    let r = rule(20).expect("ルール 20 は定義済み");
    for (a, b) in regions.corner_touching_across(canvas.transparent()) {
        let (ra, rb) = (
            &regions.regions()[a as usize],
            &regions.regions()[b as usize],
        );
        if ra.index == rb.index {
            // 同じ色なら «接線» ではなく «同化» — ルール 21 の持ち場である
            continue;
        }
        if ra.area < cfg.min_touch_area || rb.area < cfg.min_touch_area {
            continue;
        }
        if canvas.transparent() == Some(ra.index) || canvas.transparent() == Some(rb.index) {
            continue;
        }
        report.push(
            Violation::new(
                r,
                format!(
                    "添字 {} (面積 {}) と添字 {} (面積 {}) が角で 1 点だけ触れている — \
                     どちらが手前か読み取れない",
                    ra.index, ra.area, rb.index, rb.area
                ),
            )
            .area(ra.bbox),
        );
    }
}

/// ルール 21 — **隣り合う別領域が同じ添字である**．
///
/// 書籍の «後ろ姿は髪が紺色でヘッドバンドと同化しており，そこからも判断でき
/// ませんでした» がこの状態である [^pl4]．同じ色で隣り合えば境目が消える．
///
/// > [!warning] **4 近傍で探すと構造的に空になる** (D80 と同じ形) ．
/// > 同じ添字で辺を接する 2 領域は塗りつぶしの時点で 1 つに併合されているので，
/// > 別領域のまま隣り合うとは**斜めに接している**ということである．
/// > ただし**市松のディザはまさにそれ**なので，**両方が一定の面積を持つ組**に限る．
fn rule_21_same_colour_neighbours(
    regions: &RegionMap,
    canvas: &IndexedCanvas,
    cfg: &LintConfig,
    report: &mut Report,
) {
    let r = rule(21).expect("ルール 21 は定義済み");
    for (a, b) in regions.same_index_neighbors() {
        let (ra, rb) = (
            &regions.regions()[a as usize],
            &regions.regions()[b as usize],
        );
        if ra.area < cfg.min_touch_area || rb.area < cfg.min_touch_area {
            continue;
        }
        if canvas.transparent() == Some(ra.index) {
            continue;
        }
        report.push(
            Violation::new(
                r,
                format!(
                    "添字 {} の 2 つの領域 (面積 {} と {}) が斜めに接している — \
                     同じ色なので境目が消える",
                    ra.index, ra.area, rb.area
                ),
            )
            .area(ra.bbox),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pxsmith_core::palette::{ChromaCurve, Ramp};
    use pxsmith_core::ramp::{RampSpec, generate_ramp};
    use pxsmith_core::{Rgba8, ivec2};

    fn ramp_palette() -> Palette {
        Palette::new(generate_ramp(&RampSpec::default())).unwrap()
    }

    fn has(report: &Report, id: u8) -> bool {
        report.violations.iter().any(|v| v.rule == id)
    }

    /// 市松の絵 (升 `cell` 画素)．
    fn checker(side: u32, cell: u32) -> RgbaCanvas {
        let mut c = RgbaCanvas::filled(side, side, Rgba8::new(0, 0, 0, 255));
        for y in 0..side as i32 {
            for x in 0..side as i32 {
                let v = if ((x / cell as i32) + (y / cell as i32)) % 2 == 0 {
                    255
                } else {
                    0
                };
                c.set(x, y, Rgba8::new(v, v, v, 255));
            }
        }
        c
    }

    /// **ルール 9 が書籍の言うミクセルを鳴らす** (D172)．
    ///
    /// D164 の時点では**構造的に鳴らなかった** — 統計的推定器は等倍の領域に
    /// 票を立てられず，一致率が必ず 1.0 になるためである．
    ///
    /// 壊れると: 見逃す状態へ戻る．
    #[test]
    fn rule_9_fires_on_native_art_with_an_upscaled_patch() {
        let mut img = checker(64, 1);
        for y in 0..64i32 {
            for x in 32..64i32 {
                let v = if ((x / 2) + (y / 2)) % 2 == 0 { 255 } else { 0 };
                img.set(x, y, Rgba8::new(v, v, v, 255));
            }
        }
        let report = lint_grid(&img, &LintConfig::default());
        assert!(
            has(&report, 9),
            "ミクセルを見逃した: {:?}",
            report.violations
        );
    }

    /// **等倍の絵そのものでは鳴らない** — 正しく描いた絵をミクセルと呼ばない．
    #[test]
    fn rule_9_stays_quiet_on_plain_native_art() {
        let report = lint_grid(&checker(64, 1), &LintConfig::default());
        assert!(
            !has(&report, 9),
            "等倍の絵で鳴った: {:?}",
            report.violations
        );
    }

    /// **等倍の絵に広い背景があってもミクセルと呼ばない** — 実素材の形である．
    ///
    /// 平らな窓を «格子が決まった» と数えると，背景は一番大きい升で揃うので
    /// **等倍の模様 (1) と背景 (16) が並び立ち，普通のドット絵が blocking になる**．
    /// 実素材 64 枚のうち背景を持つ絵はいくらでもある．
    ///
    /// 壊れると: 背景のあるドット絵が軒並みミクセルになる．
    #[test]
    fn native_art_with_a_large_flat_background_is_not_a_mixel() {
        let mut img = checker(64, 1);
        // 右半分を背景 (単色) にする — 等倍の絵として普通の形
        for y in 0..64i32 {
            for x in 32..64i32 {
                img.set(x, y, Rgba8::new(0, 0, 0, 255));
            }
        }
        let report = lint_grid(&img, &LintConfig::default());
        assert!(
            !has(&report, 9),
            "背景のある等倍の絵をミクセルと呼んだ: {:?}",
            report.violations
        );
    }

    /// **平らな窓を «格子 1» に数えない** — 数えると，全体を 2 倍で描いた絵が
    /// «等倍が混ざっている» と誤爆する．
    ///
    /// 壊れると: 背景の広い拡大素材が軒並みミクセルになる．
    #[test]
    fn a_uniformly_upscaled_image_with_flat_areas_is_not_a_mixel() {
        // 左半分は 2 倍の模様，右半分は平ら (背景)
        let mut img = checker(64, 2);
        for y in 0..64i32 {
            for x in 32..64i32 {
                img.set(x, y, Rgba8::new(0, 0, 0, 255));
            }
        }
        let report = lint_grid(&img, &LintConfig::default());
        assert!(
            !has(&report, 9),
            "平らな窓を格子 1 に数えている: {:?}",
            report.violations
        );
    }

    #[test]
    fn a_clean_sprite_has_no_violations() {
        let palette = ramp_palette();
        let mut canvas = IndexedCanvas::filled(16, 16, 1);
        for p in IRect::new(4, 4, 8, 8).iter() {
            canvas.set_at(p, 3);
        }
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(report.is_empty(), "きれいな絵に違反: {report}");
    }

    #[test]
    fn rule_1_detects_indices_outside_the_palette() {
        let palette = Palette::new(vec![Rgba8::rgb(1, 2, 3), Rgba8::rgb(4, 5, 6)]).unwrap();
        let canvas = IndexedCanvas::from_pixels(2, 1, vec![0, 9]).unwrap();
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(has(&report, 1), "{report}");
        assert!(report.has_blocking());
        assert_eq!(report.violations[0].at, Some([1, 0]));
    }

    #[test]
    fn rule_3_detects_a_single_stray_pixel() {
        let palette = ramp_palette();
        let mut canvas = IndexedCanvas::filled(8, 8, 1);
        canvas.set(3, 3, 4);
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(has(&report, 3), "{report}");
    }

    #[test]
    fn rule_3_ignores_the_transparent_index() {
        let palette = ramp_palette();
        let mut canvas = IndexedCanvas::filled(8, 8, 1).with_transparent(Some(0));
        canvas.set(3, 3, 0);
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(
            !has(&report, 3),
            "透明の穴を孤立ピクセルと言っている: {report}"
        );
    }

    #[test]
    fn rule_5_flags_a_monotone_chroma_ramp() {
        // 彩度が明度に対し単調増加するランプ
        let mut palette = Palette::new(vec![
            Rgba8::rgb(0x30, 0x30, 0x30),
            Rgba8::rgb(0x70, 0x50, 0x50),
            Rgba8::rgb(0xc0, 0x50, 0x50),
        ])
        .unwrap();
        palette.add_ramp(Ramp::new(vec![0, 1, 2], ChromaCurve::Uniform));
        let report = lint_palette(&palette, &LintConfig::default());
        assert!(has(&report, 5), "{report}");
    }

    #[test]
    fn rule_5_accepts_the_generated_default_ramp() {
        let colors = generate_ramp(&RampSpec::default());
        let mut palette = Palette::new(colors).unwrap();
        let n = palette.len() as u8;
        palette.add_ramp(Ramp::new((0..n).collect(), ChromaCurve::PeakMiddle));
        let report = lint_palette(&palette, &LintConfig::default());
        assert!(
            !has(&report, 5),
            "自作のランプが自分の lint に落ちている: {report}"
        );
    }

    #[test]
    fn rule_11_flags_neighbours_that_are_too_close_in_lightness() {
        // 明度がほぼ同じ 2 色を隣接させる
        let palette = Palette::new(vec![
            Rgba8::rgb(0x80, 0x40, 0x40),
            Rgba8::rgb(0x40, 0x70, 0x48),
        ])
        .unwrap();
        let mut canvas = IndexedCanvas::filled(8, 8, 0);
        for p in IRect::new(4, 0, 4, 8).iter() {
            canvas.set_at(p, 1);
        }
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(has(&report, 11), "{report}");
        assert!(
            !report
                .violations
                .iter()
                .find(|v| v.rule == 11)
                .unwrap()
                .is_blocking()
        );
    }

    #[test]
    fn rule_16_flags_a_large_saturated_area() {
        let palette = Palette::new(vec![Rgba8::rgb(0xff, 0x00, 0x00)]).unwrap();
        let canvas = IndexedCanvas::filled(16, 16, 0);
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(has(&report, 16), "{report}");
    }

    #[test]
    fn rule_16_accepts_a_muted_large_area() {
        let palette = Palette::new(vec![Rgba8::rgb(0x60, 0x62, 0x70)]).unwrap();
        let canvas = IndexedCanvas::filled(16, 16, 0);
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(!has(&report, 16), "{report}");
    }

    #[test]
    fn rule_17_flags_dither_between_distant_lightnesses() {
        let palette = Palette::new(vec![
            Rgba8::rgb(0x10, 0x10, 0x18),
            Rgba8::rgb(0xf0, 0xf0, 0xe8),
        ])
        .unwrap();
        let mut canvas = IndexedCanvas::filled(16, 16, 0);
        for p in canvas.bounds().iter() {
            canvas.set_at(p, if (p.x + p.y) % 2 == 0 { 0 } else { 1 });
        }
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(has(&report, 17), "{report}");
    }

    #[test]
    fn rule_17_accepts_dither_between_close_lightnesses() {
        let colors = generate_ramp(&RampSpec::default());
        let palette = Palette::new(colors).unwrap();
        let mut canvas = IndexedCanvas::filled(16, 16, 2);
        for p in canvas.bounds().iter() {
            canvas.set_at(p, if (p.x + p.y) % 2 == 0 { 2 } else { 3 });
        }
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(
            !has(&report, 17),
            "隣り合う段のディザを違反にしている: {report}"
        );
    }

    #[test]
    fn rule_18_flags_pure_black() {
        let palette = Palette::new(vec![Rgba8::rgb(0, 0, 0), Rgba8::rgb(200, 200, 200)]).unwrap();
        let report = lint_palette(&palette, &LintConfig::default());
        assert!(has(&report, 18), "{report}");
    }

    #[test]
    fn rule_18_accepts_the_generated_ramp() {
        let palette = ramp_palette();
        let report = lint_palette(&palette, &LintConfig::default());
        assert!(
            !has(&report, 18),
            "純黒回避したランプが純黒と言われている: {report}"
        );
    }

    #[test]
    fn a_locally_shifted_grid_is_caught_as_mixels() {
        // 8 倍に拡大したあと，右半分だけ 3 画素ずらす．**場所によって格子が違う**ので
        // これはルール 9 (ミクセル) の相手である — ルール 2 は «全体として格子が
        // 見つからない» 側を担当し，役割が分かれている
        //
        // ルール 2 が崩れた格子をどれだけ捕まえるかは合成データセットで測ってある
        // (崩れた格子 199 枚のうち 67.3%) ．単体の合成例では両者を分けられない
        let mut small = RgbaCanvas::filled(8, 8, Rgba8::TRANSPARENT);
        for y in 0..8 {
            for x in 0..8 {
                let v = ((x * 31 + y * 17) % 4) as u8;
                small.set(x, y, Rgba8::rgb(v * 60, 40 + v * 50, 200 - v * 40));
            }
        }
        let s = 8i32;
        let (w, h) = (8 * s, 8 * s);
        let mut img = RgbaCanvas::filled(w as u32, h as u32, Rgba8::TRANSPARENT);
        for y in 0..h {
            for x in 0..w {
                let shift = if x >= w / 2 { 3 } else { 0 };
                let sx = ((x + shift) / s).min(7);
                let sy = (y / s).min(7);
                img.set(x, y, small.get(sx, sy).expect("元絵の範囲内"));
            }
        }
        let report = lint_grid(&img, &LintConfig::default());
        assert!(has(&report, 9), "{report}");
    }

    /// **壊れると: ルール 9 が «検査していない» 絵を «問題なし» として通す** (D164)．
    ///
    /// 升は窓ごとに決まるので，窓が 1 つしか並ばなければ 2 通りになりようがない．
    ///
    /// > [!note] **境界は D172 で動いた．** 窓が 32 だった頃は 32x32 の画布が
    /// > «検査できない» 側だったが，厳密判定へ替えて窓を 16 にしたので
    /// > **32x32 は検査できるようになった** — 実素材 64 枚のうち 32 枚が
    /// > «1 度も検査していない» から «検査して 0 件» へ移っている．
    /// > **16x16 の 32 枚は今も検査できない．**
    #[test]
    fn rule_9_says_when_it_could_not_check_at_all() {
        let mut small = RgbaCanvas::filled(16, 16, Rgba8::TRANSPARENT);
        for y in 0..16 {
            for x in 0..16 {
                let v = ((x * 31 + y * 17) % 4) as u8;
                small.set(x, y, Rgba8::rgb(v * 60, 40 + v * 50, 200 - v * 40));
            }
        }
        let cov = mixel_coverage(&small, &LintConfig::default());
        assert!(!cov.checked(), "16x16 の画布で検査できたと言っている");
        assert!(
            cov.why_not().is_some_and(|s| s.contains("1 つしか")),
            "理由が «窓が 1 つ» になっていない: {:?}",
            cov.why_not()
        );

        // 窓が並ぶ大きさなら検査できたと言う
        let mut big = RgbaCanvas::filled(256, 256, Rgba8::TRANSPARENT);
        for y in 0..256 {
            for x in 0..256 {
                let v = ((x / 4 * 31 + y / 4 * 17) % 4) as u8;
                big.set(x, y, Rgba8::rgb(v * 60, 40 + v * 50, 200 - v * 40));
            }
        }
        let cov = mixel_coverage(&big, &LintConfig::default());
        assert!(cov.checked(), "256x256 の画布で検査できないと言っている");
        assert!(cov.why_not().is_none(), "検査できたのに理由が付いている");
    }

    /// **原寸のドット絵を «格子崩れ» と呼ばない．**
    ///
    /// 1 画素 = 1 セルなので $s \ge 2$ の格子は無いのが正しい．以前はこれを違反に
    /// しており，CC0 の実物のドット絵 61 枚のうち 58 枚が blocking になっていた．
    #[test]
    fn rule_2_accepts_native_resolution_art() {
        let mut img = RgbaCanvas::filled(16, 16, Rgba8::TRANSPARENT);
        for y in 0..16 {
            for x in 0..16 {
                let v = ((x * 31 + y * 17) % 5) as u8;
                img.set(x, y, Rgba8::rgb(v * 50, 30 + v * 40, 210 - v * 35));
            }
        }
        let report = lint_grid(&img, &LintConfig::default());
        assert!(
            !has(&report, 2),
            "原寸のドット絵を格子崩れと言っている: {report}"
        );
    }

    #[test]
    fn rule_2_accepts_a_clean_upscale() {
        let mut small = RgbaCanvas::filled(8, 8, Rgba8::TRANSPARENT);
        for y in 0..8 {
            for x in 0..8 {
                let v = ((x * 31 + y * 17) % 4) as u8;
                small.set(x, y, Rgba8::rgb(v * 60, 40 + v * 30, 200 - v * 40));
            }
        }
        let mut big = RgbaCanvas::filled(32, 32, Rgba8::TRANSPARENT);
        for y in 0..32 {
            for x in 0..32 {
                big.set(x, y, small.get(x / 4, y / 4).unwrap());
            }
        }
        let report = lint_grid(&big, &LintConfig::default());
        assert!(
            !has(&report, 2),
            "きれいな 4 倍拡大が格子崩れと言われている: {report}"
        );
    }

    #[test]
    fn the_report_is_deterministic() {
        let palette = Palette::new(vec![Rgba8::rgb(0, 0, 0)]).unwrap();
        let mut canvas = IndexedCanvas::filled(8, 8, 0);
        canvas.set(1, 1, 5);
        let a = lint_canvas(&canvas, &palette, &LintConfig::default());
        let b = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert_eq!(a, b);
    }

    #[test]
    fn blocking_and_advisory_are_separated() {
        let palette = Palette::new(vec![Rgba8::rgb(0, 0, 0)]).unwrap();
        let mut canvas = IndexedCanvas::filled(8, 8, 0);
        canvas.set(1, 1, 5);
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(report.has_blocking(), "{report}");
        assert!(!report.to_prompt_hint().is_empty());
        // advisory は生成ループの判定に影響しない
        let only_advisory = lint_palette(&palette, &LintConfig::default());
        assert!(!only_advisory.has_blocking(), "{only_advisory}");
    }

    #[test]
    fn every_declared_rule_number_is_unique() {
        let mut ids: Vec<u8> = crate::RULES.iter().map(|r| r.id).collect();
        ids.sort_unstable();
        let mut unique = ids.clone();
        unique.dedup();
        assert_eq!(ids, unique);
        // **足すたびにここを上げる．** 設計書 7.3 は 27 ルールで，残っているのは
        // G5 依存の 3 件 (19 形の乱雑さ ・20 接線 ・21 隣接領域の同色) だけである
        assert_eq!(ids.len(), 27, "設計書 7.3 の 27 ルールがすべて入っている");
        let missing: Vec<u8> = (1..=27u8).filter(|id| !ids.contains(id)).collect();
        assert!(missing.is_empty(), "実装していないルール: {missing:?}");
    }

    /// **壊れると: 陰影の帯のような «細いだけの» 領域を «乱雑» と呼ぶ．**
    ///
    /// 設計書の «周囲長/面積比» を $P^2/A$ で取ると良い絵の 93.8% が鳴る．
    /// **シルエットの広がりに対する縁の長さ**で見る．
    #[test]
    fn a_thin_but_smooth_shape_is_not_ragged() {
        let palette = Palette::new(vec![Rgba8::TRANSPARENT, Rgba8::rgb(0x1a, 0x1c, 0x2c)]).unwrap();
        // 1 画素幅 x 20 の帯 — P^2/A は 88 だが乱れてはいない
        let mut c = IndexedCanvas::filled(24, 4, 0);
        c.set_transparent(Some(0));
        for x in 2..22 {
            c.set(x, 1, 1);
        }
        let report = lint_canvas(&c, &palette, &LintConfig::default());
        assert!(
            !report.violations.iter().any(|v| v.rule == 19),
            "{:?}",
            report.violations
        );
    }

    /// **壊れると: でこぼこしたシルエットを見逃す．**
    #[test]
    fn a_ragged_silhouette_is_reported() {
        let palette = Palette::new(vec![Rgba8::TRANSPARENT, Rgba8::rgb(0x1a, 0x1c, 0x2c)]).unwrap();
        let mut c = IndexedCanvas::filled(16, 16, 0);
        c.set_transparent(Some(0));
        for y in 4..12 {
            for x in 4..12 {
                c.set(x, y, 1);
            }
        }
        // 縁に櫛歯を生やす
        for x in (4..12).step_by(2) {
            c.set(x, 3, 1);
            c.set(x, 12, 1);
        }
        for y in (4..12).step_by(2) {
            c.set(3, y, 1);
            c.set(12, y, 1);
        }
        let report = lint_canvas(&c, &palette, &LintConfig::default());
        assert!(
            report.violations.iter().any(|v| v.rule == 19),
            "{:?}",
            report.violations
        );
    }

    /// **壊れると: 角で 1 点だけ触れている 2 つの部品を見逃す (ルール 20)．**
    #[test]
    fn two_parts_touching_only_at_a_corner_are_reported() {
        let palette = Palette::new(vec![
            Rgba8::TRANSPARENT,
            Rgba8::rgb(0x1a, 0x1c, 0x2c),
            Rgba8::rgb(0xb1, 0x3e, 0x53),
        ])
        .unwrap();
        let mut c = IndexedCanvas::filled(16, 16, 0);
        c.set_transparent(Some(0));
        for y in 2..7 {
            for x in 2..7 {
                c.set(x, y, 1);
            }
        }
        for y in 7..12 {
            for x in 7..12 {
                c.set(x, y, 2);
            }
        }
        let report = lint_canvas(&c, &palette, &LintConfig::default());
        assert!(
            report.violations.iter().any(|v| v.rule == 20),
            "{:?}",
            report.violations
        );
    }

    /// **壊れると: 辺で接しているだけの 2 面を «接線» と呼ぶ．**
    #[test]
    fn two_parts_sharing_an_edge_are_not_tangent() {
        let palette = Palette::new(vec![
            Rgba8::TRANSPARENT,
            Rgba8::rgb(0x1a, 0x1c, 0x2c),
            Rgba8::rgb(0xb1, 0x3e, 0x53),
        ])
        .unwrap();
        let mut c = IndexedCanvas::filled(16, 16, 0);
        c.set_transparent(Some(0));
        for y in 2..7 {
            for x in 2..7 {
                c.set(x, y, 1);
                c.set(x + 5, y, 2);
            }
        }
        let report = lint_canvas(&c, &palette, &LintConfig::default());
        assert!(
            !report.violations.iter().any(|v| v.rule == 20),
            "{:?}",
            report.violations
        );
    }

    /// **壊れると: 同じ色で隣り合う 2 つの部品を見逃す (ルール 21)．**
    ///
    /// 書籍の «後ろ姿は髪が紺色でヘッドバンドと同化しており判断できなかった»．
    #[test]
    fn two_parts_of_the_same_colour_touching_diagonally_are_reported() {
        let palette = Palette::new(vec![Rgba8::TRANSPARENT, Rgba8::rgb(0x1a, 0x1c, 0x2c)]).unwrap();
        let mut c = IndexedCanvas::filled(16, 16, 0);
        c.set_transparent(Some(0));
        for y in 2..7 {
            for x in 2..7 {
                c.set(x, y, 1);
            }
        }
        for y in 7..12 {
            for x in 7..12 {
                c.set(x, y, 1);
            }
        }
        let report = lint_canvas(&c, &palette, &LintConfig::default());
        assert!(
            report.violations.iter().any(|v| v.rule == 21),
            "{:?}",
            report.violations
        );
    }

    /// **壊れると: 市松のディザで鳴りっぱなしになる．**
    #[test]
    fn a_checkerboard_does_not_trip_the_touching_rules() {
        let palette = Palette::new(vec![
            Rgba8::rgb(0x1a, 0x1c, 0x2c),
            Rgba8::rgb(0xb1, 0x3e, 0x53),
        ])
        .unwrap();
        let mut c = IndexedCanvas::filled(16, 16, 0);
        for y in 0..16 {
            for x in 0..16 {
                c.set(x, y, ((x + y) % 2) as u8);
            }
        }
        let report = lint_canvas(&c, &palette, &LintConfig::default());
        assert!(
            !report
                .violations
                .iter()
                .any(|v| v.rule == 20 || v.rule == 21),
            "{:?}",
            report.violations
        );
    }

    #[test]
    fn the_report_serialises_to_json() {
        let palette = Palette::new(vec![Rgba8::rgb(0, 0, 0)]).unwrap();
        let canvas = IndexedCanvas::filled(4, 4, 0);
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        let json = serde_json::to_string(&report).unwrap();
        let back: Report = serde_json::from_str(&json).unwrap();
        assert_eq!(report, back);
    }

    #[test]
    fn positions_are_reported_for_locatable_violations() {
        let palette = ramp_palette();
        let mut canvas = IndexedCanvas::filled(8, 8, 1);
        canvas.set(5, 6, 4);
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        let v = report.violations.iter().find(|v| v.rule == 3).unwrap();
        assert_eq!(v.at, Some([5, 6]));
        assert_eq!(v.area, Some([5, 6, 1, 1]));
    }

    #[test]
    fn lint_frame_covers_palette_and_canvas_rules() {
        let palette = Palette::new(vec![Rgba8::rgb(0, 0, 0), Rgba8::rgb(0xff, 0, 0)]).unwrap();
        let mut frame = Frame::new(pxsmith_core::uvec2(8, 8), palette);
        let mut canvas = IndexedCanvas::filled(8, 8, 1);
        canvas.set(2, 2, 9);
        frame.layers.push(pxsmith_core::Layer::new(
            pxsmith_core::LayerMeta::named("art"),
            Surface::Indexed(canvas),
        ));
        let report = lint_frame(&frame, &LintConfig::default());
        assert!(
            has(&report, 1),
            "キャンバス側のルールが効いていない: {report}"
        );
        assert!(
            has(&report, 18),
            "パレット側のルールが効いていない: {report}"
        );
    }

    /// **ランプの隣り合う段の 1 画素は «迷子» ではない** (陰影の最終段である)．
    ///
    /// `pxsmith shade` は光へ正対した画素に光ランプの最上段を置く．それが平らな面の中に
    /// 1 つだけ落ちると «その色は他に無く単色に囲まれている» を満たしてしまう —
    /// 実際に `pxsmith shade` の出力 320 通りで 8 件が blocking になっていた．
    ///
    /// **色差が離れていても，宣言があれば段である** — ここは宣言の側だけを見る
    /// (色差の側は [`LintConfig::stray_min_distance`] が見る．D81) ．
    #[test]
    fn rule_3_does_not_call_the_top_step_of_a_ramp_a_stray_pixel() {
        // 白と黒 — **色差では «段» と言えない** 2 色を，同じランプとして宣言する
        let mut palette = Palette::new(vec![
            Rgba8::TRANSPARENT,
            Rgba8::rgb(0x10, 0x10, 0x14),
            Rgba8::rgb(0xf0, 0xf0, 0xe8),
        ])
        .unwrap();
        let mut canvas = IndexedCanvas::filled(16, 16, 1).with_transparent(Some(0));
        canvas.set_at(ivec2(8, 8), 2);
        let regions = label_regions(&canvas);

        // 宣言が無ければ «迷子» である (色差も離れている)
        let mut report = Report::default();
        rule_3_isolated(
            &regions,
            &canvas,
            &palette,
            &LintConfig::default(),
            &mut report,
        );
        assert!(has(&report, 3), "離れた色の 1 画素を見逃した: {report}");

        // 同じランプの隣り合う段だと宣言すれば «段» である
        palette.add_ramp(Ramp::new(vec![1, 2], ChromaCurve::default()));
        let mut report = Report::default();
        rule_3_isolated(
            &regions,
            &canvas,
            &palette,
            &LintConfig::default(),
            &mut report,
        );
        assert!(!has(&report, 3), "宣言した段を迷子と呼んだ: {report}");
    }

    /// **宣言が残らない経路でも色差で救う** (D81)．
    ///
    /// ランプの宣言は `.aseprite` にも `.hex` にも欄が無いので，`pxsmith shade` の出力を
    /// ファイル経由で `pxsmith lint` へ渡すと消える．**色差なら残る．**
    #[test]
    fn rule_3_lets_a_near_colour_pass_even_without_a_ramp_declaration() {
        let palette = shaded_palette();
        let mut canvas = IndexedCanvas::filled(16, 16, 4).with_transparent(Some(0));
        canvas.set_at(ivec2(8, 8), 5);
        assert!(
            palette.ramps().is_empty(),
            "この試験は «宣言が無い» 状態を見る"
        );

        let regions = label_regions(&canvas);
        let mut report = Report::default();
        rule_3_isolated(
            &regions,
            &canvas,
            &palette,
            &LintConfig::default(),
            &mut report,
        );
        assert!(!has(&report, 3), "隣の段を迷子と呼んだ: {report}");
    }

    /// **ランプの中にあっても «飛んだ段» なら迷子である．**
    /// 隣り合う段だけを陰影とみなす — 3 段離れた色が 1 画素だけ落ちているのは
    /// 陰影ではない．
    #[test]
    fn rule_3_still_flags_a_pixel_that_skips_ramp_steps() {
        let mut palette = ramp_palette();
        palette.add_ramp(Ramp::new(vec![0, 1, 2, 3, 4], ChromaCurve::default()));
        let mut canvas = IndexedCanvas::filled(16, 16, 0);
        canvas.set_at(ivec2(8, 8), 4);

        let regions = label_regions(&canvas);
        let mut report = Report::default();
        rule_3_isolated(
            &regions,
            &canvas,
            &palette,
            &LintConfig::default(),
            &mut report,
        );
        assert!(has(&report, 3), "段を飛ばした 1 画素を見逃した: {report}");
    }

    /// 円板のマスクを 1 つ作る (ルール 13 の試験の土台)．
    fn disc_canvas(size: u32, radius: f32, colour: impl Fn(f32) -> u8) -> IndexedCanvas {
        // 添字 0 は透明にする (パレットの先頭を透明色にしてある)
        let mut canvas = IndexedCanvas::filled(size, size, 0).with_transparent(Some(0));
        let c = (size as f32 - 1.0) / 2.0;
        for p in canvas.bounds().iter() {
            let (dx, dy) = (p.x as f32 - c, p.y as f32 - c);
            let r = (dx * dx + dy * dy).sqrt();
            if r <= radius {
                // 縁で 0 ・中心で 1
                canvas.set_at(p, colour(1.0 - r / radius));
            }
        }
        canvas
    }

    /// 透明色を先頭に置いたランプのパレット (添字 1 が最暗，末尾が最明)．
    fn shaded_palette() -> Palette {
        let mut colors = vec![Rgba8::TRANSPARENT];
        colors.extend(generate_ramp(&RampSpec {
            steps: 8,
            ..RampSpec::default()
        }));
        Palette::new(colors).unwrap()
    }

    /// **教科書どおりの pillow shading を捕まえる．**
    /// 縁からの距離だけで明るさが決まる絵は $\rho \approx 1$ になる．
    #[test]
    fn rule_13_catches_shading_that_only_follows_the_distance_from_the_edge() {
        let palette = shaded_palette();
        let canvas = disc_canvas(24, 10.0, |t| 1 + (t * 7.0).round() as u8);
        let rho = pillow_correlation(&canvas, &palette).expect("相関を測れない");
        assert!(rho > 0.9, "同心状の陰影なのに rho が {rho:.3}");

        let mut report = Report::default();
        rule_13_pillow_shading(&canvas, &palette, &LintConfig::default(), &mut report);
        assert!(has(&report, 13), "pillow shading を見逃した");
    }

    /// **向きのある陰影は鳴らない．** 明るさを «縁からの距離» ではなく
    /// «光源への向き» で決めると，光の縁と影の縁がどちらも $d \approx 0$ に来るので
    /// 相関が潰れる (`pxsmith shade` の出力がこの形である) ．
    #[test]
    fn rule_13_leaves_directional_shading_alone() {
        let palette = shaded_palette();
        let mut canvas = IndexedCanvas::filled(24, 24, 0).with_transparent(Some(0));
        let c = 11.5;
        for p in canvas.bounds().iter() {
            let (dx, dy) = (p.x as f32 - c, p.y as f32 - c);
            if dx * dx + dy * dy > 100.0 {
                continue;
            }
            // 左上ほど明るい (光が左上から来ている)
            let t = (1.0 - (dx + dy) / 28.0).clamp(0.0, 1.0);
            canvas.set_at(p, 1 + (t * 7.0).round() as u8);
        }
        let rho = pillow_correlation(&canvas, &palette).expect("相関を測れない");
        assert!(rho.abs() < 0.5, "向きのある陰影なのに rho が {rho:.3}");

        let mut report = Report::default();
        rule_13_pillow_shading(&canvas, &palette, &LintConfig::default(), &mut report);
        assert!(!has(&report, 13), "向きのある陰影を pillow と呼んだ");
    }

    /// **透明添字を宣言していない面でも同じ答えを出す．**
    ///
    /// PNG をそのまま添字にすると透明添字が付かないことがある．キャンバスだけを見て
    /// シルエットを決めると**背景まで形の一部**になり，距離場が画像の縁から測られて
    /// 相関が丸ごと別物になる — 実際に校正の測定がそれで汚れていた．
    #[test]
    fn rule_13_finds_the_silhouette_even_without_a_declared_transparent_index() {
        let palette = shaded_palette();
        let declared = disc_canvas(24, 10.0, |t| 1 + (t * 7.0).round() as u8);
        let undeclared = declared.clone().with_transparent(None);
        let a = pillow_correlation(&declared, &palette).expect("相関を測れない");
        let b = pillow_correlation(&undeclared, &palette).expect("相関を測れない");
        assert!(
            (a - b).abs() < 1e-6,
            "透明添字の宣言で答えが変わる ({a:.3} 対 {b:.3})"
        );
    }

    /// **小さすぎるシルエットでは測らない．** 数画素の相関は雑音で振り切れる．
    #[test]
    fn rule_13_does_not_judge_a_tiny_silhouette() {
        let palette = shaded_palette();
        let canvas = disc_canvas(8, 3.0, |t| 1 + (t * 7.0).round() as u8);
        let mut report = Report::default();
        rule_13_pillow_shading(&canvas, &palette, &LintConfig::default(), &mut report);
        assert!(!has(&report, 13), "小さすぎる形を判定した");
    }

    /// **段が 1 つ短い階段はジャギーである** (設計書 6.4 の `[3, 3, 1, 3]`)．
    #[test]
    fn rule_8_flags_a_step_that_is_shorter_than_its_neighbours() {
        let palette = Palette::new(vec![
            Rgba8::rgb(0x30, 0x34, 0x5a),
            Rgba8::rgb(0xc8, 0xcc, 0xe0),
        ])
        .unwrap();
        let mut canvas = IndexedCanvas::filled(12, 8, 0);
        let widths = [3i32, 3, 1, 3];
        let mut x = 0i32;
        for (step, w) in widths.iter().enumerate() {
            for dx in 0..*w {
                for y in (step as i32 + 1)..8 {
                    canvas.set(x + dx, y, 1);
                }
            }
            x += w;
        }
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(has(&report, 8), "短い段を見逃した: {report}");
        // **advisory である (D85)．** blocking にすると良い絵の 59% が止まる
        assert!(
            !report
                .violations
                .iter()
                .find(|v| v.rule == 8)
                .unwrap()
                .is_blocking(),
            "ルール 8 が blocking になっている: {report}"
        );
    }

    /// **段が揃った階段はジャギーではない．** 理想形の谷を違反と呼ばないこと．
    #[test]
    fn rule_8_leaves_an_even_staircase_alone() {
        let palette = Palette::new(vec![
            Rgba8::rgb(0x30, 0x34, 0x5a),
            Rgba8::rgb(0xc8, 0xcc, 0xe0),
        ])
        .unwrap();
        let mut canvas = IndexedCanvas::filled(12, 8, 0);
        for step in 0..4i32 {
            for dx in 0..3i32 {
                for y in (step + 1)..8 {
                    canvas.set(step * 3 + dx, y, 1);
                }
            }
        }
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(!has(&report, 8), "揃った階段をジャギーと呼んだ: {report}");
    }

    /// **中間フレームにはジャギーを課さない** (設計書 7.1 ・D47)．
    ///
    /// これが無いと `pxsmith anim` の中割りが自らの lint に大量に落ちる．
    #[test]
    fn keyframe_rules_do_not_apply_to_inbetween_frames() {
        let palette = Palette::new(vec![
            Rgba8::rgb(0x30, 0x34, 0x5a),
            Rgba8::rgb(0xc8, 0xcc, 0xe0),
        ])
        .unwrap();
        let mut canvas = IndexedCanvas::filled(12, 8, 0);
        let widths = [3i32, 3, 1, 3];
        let mut x = 0i32;
        for (step, w) in widths.iter().enumerate() {
            for dx in 0..*w {
                for y in (step as i32 + 1)..8 {
                    canvas.set(x + dx, y, 1);
                }
            }
            x += w;
        }
        let mut frame = Frame::new(pxsmith_core::uvec2(12, 8), palette);
        frame.layers.push(pxsmith_core::Layer::new(
            pxsmith_core::LayerMeta::named("art"),
            Surface::Indexed(canvas),
        ));

        frame.kind = pxsmith_core::frame::FrameKind::Key;
        assert!(has(&lint_frame(&frame, &LintConfig::default()), 8));

        frame.kind = pxsmith_core::frame::FrameKind::Inbetween;
        let report = lint_frame(&frame, &LintConfig::default());
        assert!(!has(&report, 8), "中間フレームにジャギーを課した: {report}");
    }

    /// **影が光と同一色相の明度違いだけなら単色影である．**
    #[test]
    fn rule_6_flags_a_shadow_that_only_differs_in_lightness() {
        // **sRGB で各成分を半分にした色**．OKLab の色相はほぼ動かない (実測 0.27 度)
        //
        // > [!note] 明度だけ引いて `oklab_to_rgba` で戻す作り方は使えない．
        // > 8 ビットへの丸めと色域の切り詰めで**色相が 13 度動く** — ルール 6 の
        // > 閾値 (3 度) が «丸めの雑音の水準» である理由がここに出ている．
        let palette = Palette::new(vec![
            Rgba8::rgb(0xc0, 0x90, 0x60),
            Rgba8::rgb(0x60, 0x48, 0x30),
        ])
        .unwrap();
        let mut canvas = IndexedCanvas::filled(16, 16, 0);
        for p in IRect::new(0, 8, 16, 8).iter() {
            canvas.set_at(p, 1);
        }
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(has(&report, 6), "単色影を見逃した: {report}");
    }

    /// **色相をずらした影は鳴らない** (`pxsmith palette ramp` が作る形)．
    #[test]
    fn rule_6_accepts_a_hue_shifted_shadow() {
        // 明るい橙 (色相 ≒ 50 度) と暗い青紫 (色相 ≒ 260 度)
        let palette = Palette::new(vec![
            Rgba8::rgb(0xc0, 0x90, 0x60),
            Rgba8::rgb(0x38, 0x34, 0x60),
        ])
        .unwrap();
        let mut canvas = IndexedCanvas::filled(16, 16, 0);
        for p in IRect::new(0, 8, 16, 8).iter() {
            canvas.set_at(p, 1);
        }
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(
            !has(&report, 6),
            "色相をずらした影を単色影と言った: {report}"
        );
    }

    /// **灰色だけの陰影は単色影ではない．** 色相をずらしていないのではなく，
    /// ずらす先が無い．
    #[test]
    fn rule_6_leaves_a_greyscale_ramp_alone() {
        let palette = Palette::new(vec![
            Rgba8::rgb(0xd0, 0xd0, 0xd0),
            Rgba8::rgb(0x50, 0x50, 0x50),
        ])
        .unwrap();
        let mut canvas = IndexedCanvas::filled(16, 16, 0);
        for p in IRect::new(0, 8, 16, 8).iter() {
            canvas.set_at(p, 1);
        }
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(!has(&report, 6), "灰色の陰影を単色影と言った: {report}");
    }

    /// 左上が明るい円板 (光が左上から来ている絵)．
    fn lit_from_upper_left() -> (IndexedCanvas, Palette) {
        let palette = shaded_palette();
        let mut canvas = IndexedCanvas::filled(24, 24, 0).with_transparent(Some(0));
        let c = 11.5f32;
        for p in canvas.bounds().iter() {
            let (dx, dy) = (p.x as f32 - c, p.y as f32 - c);
            if dx * dx + dy * dy > 100.0 {
                continue;
            }
            let t = (1.0 - (dx + dy) / 28.0).clamp(0.0, 1.0);
            canvas.set_at(p, 1 + (t * 7.0).round() as u8);
        }
        (canvas, palette)
    }

    /// 左上から照らす光源 (`dir` は**光源から面へ向かう向き**なので右下を指す)．
    fn from_upper_left() -> pxsmith_core::ramp::LightSource {
        pxsmith_core::ramp::LightSource::Directional {
            dir: pxsmith_core::math::Vec2 { x: 1.0, y: 1.0 },
        }
    }

    /// **光源が宣言されていなければ検査しない．**
    ///
    /// 絵だけを見て «光源方向» は決まらない — 決めるなら絵から推定することになり，
    /// 推定した向きと絵が合うかを見るのは同語反復である．
    #[test]
    fn rule_7_does_nothing_without_a_declared_light() {
        let (canvas, palette) = lit_from_upper_left();
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(!has(&report, 7), "宣言が無いのに鳴った: {report}");
    }

    /// **宣言した向きと合っている絵は鳴らない．**
    #[test]
    fn rule_7_accepts_shading_that_agrees_with_the_light() {
        let (canvas, palette) = lit_from_upper_left();
        let cfg = LintConfig {
            light: Some(from_upper_left()),
            ..LintConfig::default()
        };
        let report = lint_canvas(&canvas, &palette, &cfg);
        assert!(!has(&report, 7), "合っている陰影で鳴った: {report}");
    }

    /// **左右反転すると光源方向と矛盾する** (設計書 6.7 ・6.8 の自動ミラー)．
    #[test]
    fn rule_7_catches_a_mirrored_sprite_that_kept_its_shading() {
        let (canvas, palette) = lit_from_upper_left();
        let mut flipped = canvas.clone();
        for p in canvas.bounds().iter() {
            let q = ivec2(canvas.width() as i32 - 1 - p.x, p.y);
            if let Some(i) = canvas.get_at(q) {
                flipped.set_at(p, i);
            }
        }
        let cfg = LintConfig {
            light: Some(from_upper_left()),
            ..LintConfig::default()
        };
        let report = lint_canvas(&flipped, &palette, &cfg);
        assert!(has(&report, 7), "反転した陰影を見逃した: {report}");
        assert!(report.has_blocking());
    }

    /// **同じ形の段の列が並走したらバンディングである．**
    #[test]
    fn rule_12_flags_parallel_bands_with_the_same_run_lengths() {
        let palette = Palette::new(vec![
            Rgba8::rgb(0x60, 0x50, 0x40),
            Rgba8::rgb(0x90, 0x78, 0x60),
        ])
        .unwrap();
        let mut canvas = IndexedCanvas::filled(32, 32, 0);
        for p in canvas.bounds().iter() {
            // 傾き 2 の帯を 3 画素おきに敷く — どの縁も同じラン長列を持つ
            let band = (p.x * 2 + p.y).div_euclid(3) % 2 == 0;
            canvas.set_at(p, if band { 0 } else { 1 });
        }
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(has(&report, 12), "並走する帯を見逃した: {report}");
        assert!(
            !report
                .violations
                .iter()
                .find(|v| v.rule == 12)
                .unwrap()
                .is_blocking(),
            "ルール 12 は advisory である: {report}"
        );
    }

    /// **1 本の揃った階段はバンディングではない．**
    ///
    /// 段の長さが揃っていること自体は良い形である (ルール 8 の裏返し) ．
    /// 並走して初めて縞に見える．
    #[test]
    fn rule_12_leaves_a_single_even_staircase_alone() {
        let palette = Palette::new(vec![
            Rgba8::rgb(0x60, 0x50, 0x40),
            Rgba8::rgb(0x90, 0x78, 0x60),
        ])
        .unwrap();
        let mut canvas = IndexedCanvas::filled(32, 32, 0);
        for y in 0..32i32 {
            for x in 0..32i32 {
                if x < y / 2 {
                    canvas.set(x, y, 1);
                }
            }
        }
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(!has(&report, 12), "1 本の階段を縞と言った: {report}");
    }

    /// **中間色だらけの絵は AA 過多である．**
    ///
    /// 端の 2 色の «間» にある色を 5 色置き，どれも端より狭く使う．
    ///
    /// > [!note] **中間色が少ないと上限 0.55 には届かない．**
    /// > 端の 2 色は中間色より広く使われている必要があるので，中間色 1 色では
    /// > 割合が 1/3 未満にしかならない．0.55 は «中間色が 4 色以上ある» ことを
    /// > 暗に求めている (設計書 6.5 の «中間色は 1 〜 2 色 ・上限 3 色» より緩い —
    /// > 閾値は `pxsmith shade` の出力の上に置いてあるためである) ．
    #[test]
    fn rule_14_flags_art_that_is_mostly_intermediate_colours() {
        let (dark, light) = (Rgba8::rgb(0x20, 0x20, 0x28), Rgba8::rgb(0xe0, 0xe0, 0xd8));
        // **中点は Oklab で取る** — sRGB の平均は L が非線形なので中点にならない
        let (la, lb) = (oklab_of(dark), oklab_of(light));
        let mid = Oklab::new(
            (la.l + lb.l) * 0.5,
            (la.a + lb.a) * 0.5,
            (la.b + lb.b) * 0.5,
        );
        let mut colours = vec![dark, light];
        // 中点のまわりに 5 色 (許容 0.05 の内側に収める)
        for k in 0..5 {
            let d = (k as f32 - 2.0) * 0.008;
            colours.push(pxsmith_core::quantize::oklab_to_rgba(Oklab::new(
                mid.l + d,
                mid.a + d * 0.2,
                mid.b - d * 0.2,
            )));
        }
        let palette = Palette::new(colours).unwrap();
        // 端は 3 行ずつ (48 画素) ・中間色は 2 行ずつ (32 画素) → 中間色は 62.5%
        let mut canvas = IndexedCanvas::filled(16, 16, 0);
        for p in canvas.bounds().iter() {
            let i = match p.y {
                0..=2 => 0,
                13..=15 => 1,
                y => 2 + ((y - 3) / 2) as u8,
            };
            canvas.set_at(p, i.min(6));
        }
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(has(&report, 14), "中間色だらけの絵を見逃した: {report}");
        assert!(
            !report
                .violations
                .iter()
                .find(|v| v.rule == 14)
                .unwrap()
                .is_blocking(),
            "ルール 14 は advisory である: {report}"
        );
    }

    /// **中間色の無い絵は鳴らない．**
    #[test]
    fn rule_14_leaves_two_flat_colours_alone() {
        let palette = Palette::new(vec![
            Rgba8::rgb(0x20, 0x20, 0x28),
            Rgba8::rgb(0xe0, 0xe0, 0xd8),
        ])
        .unwrap();
        let mut canvas = IndexedCanvas::filled(16, 16, 0);
        for p in IRect::new(0, 8, 16, 8).iter() {
            canvas.set_at(p, 1);
        }
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(!has(&report, 14), "中間色の無い絵で鳴った: {report}");
    }

    /// 縁取りのあるシルエットを 1 つ作る (ルール 4 の試験の土台)．
    ///
    /// 添字 0 = 透明 ・1 = 中身 ・2 = 縁取り．`double_corners` を立てると
    /// **角で内側へ 1 画素はみ出させる** — 角を «曲がる» のではなく «足した» 形で，
    /// そこだけ縁が 2 画素幅になる．
    fn outlined_blob(double_corners: bool) -> (IndexedCanvas, Palette) {
        let palette = Palette::new(vec![
            Rgba8::TRANSPARENT,
            Rgba8::rgb(0xc0, 0x90, 0x50),
            Rgba8::rgb(0x20, 0x18, 0x14),
        ])
        .unwrap();
        let mut canvas = IndexedCanvas::filled(16, 16, 0).with_transparent(Some(0));
        for p in IRect::new(2, 2, 12, 12).iter() {
            canvas.set_at(p, 1);
        }
        // 1 画素幅の縁取り
        for p in IRect::new(2, 2, 12, 12).iter() {
            if p.x == 2 || p.y == 2 || p.x == 13 || p.y == 13 {
                canvas.set_at(p, 2);
            }
        }
        if double_corners {
            for p in [ivec2(3, 3), ivec2(12, 3), ivec2(3, 12), ivec2(12, 12)] {
                canvas.set_at(p, 2);
            }
        }
        (canvas, palette)
    }

    /// **角で線を «足す» と縁取りが 2x2 になる．**
    #[test]
    fn rule_4_flags_an_outline_whose_corners_overlap() {
        let (canvas, palette) = outlined_blob(true);
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(has(&report, 4), "重なった角を見逃した: {report}");
        assert!(report.has_blocking());
    }

    /// **1 画素幅の縁取りは鳴らない．**
    #[test]
    fn rule_4_leaves_a_one_pixel_outline_alone() {
        let (canvas, palette) = outlined_blob(false);
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(!has(&report, 4), "正しい縁取りを違反と言った: {report}");
    }

    /// **縁取りの無い絵には掛からない．**
    ///
    /// 画面いっぱいのタイル (実測で 61 枚中 26 枚) は透明な隣を持たないので，
    /// «縁» がそもそも無い．ここを画像の端で代用すると，端に並んだ幅 2 画素の帯が
    /// «縁取り» になり，タイル 1 枚で 30 件の違反が出た．
    #[test]
    fn rule_4_does_not_apply_to_a_full_bleed_tile() {
        let palette = Palette::new(vec![
            Rgba8::rgb(0x20, 0x18, 0x14),
            Rgba8::rgb(0xc0, 0x90, 0x50),
        ])
        .unwrap();
        let mut canvas = IndexedCanvas::filled(16, 16, 1);
        // 端 2 画素を «縁取り» 色で塗る (透明は 1 画素も無い)
        for p in canvas.bounds().iter() {
            if p.x < 2 || p.y < 2 || p.x >= 14 || p.y >= 14 {
                canvas.set_at(p, 0);
            }
        }
        let report = lint_canvas(&canvas, &palette, &LintConfig::default());
        assert!(!has(&report, 4), "縁の無いタイルで鳴った: {report}");
    }

    #[test]
    fn violations_carry_their_declared_severity_and_scope() {
        for r in crate::RULES {
            let v = Violation::new(r, "x");
            assert_eq!(v.severity, r.severity);
            assert_eq!(v.scope, r.scope);
        }
        let _ = ivec2(0, 0);
    }
}
