//! サブピクセル (設計書 6.10．D38 ・D39)．
//!
//! 「**フレームを複製してドットをシフトする**」— ドットを半分動かすと隣のドットへ
//! 明度が引き継がれる (コップの水の比喩) ．動く向きは**動きの方向ではなく，
//! 線 ・曲線 ・形が向いている方向 (接線方向)** である．
//!
//! # $f$ を丸めてはいけない — これは代数である
//!
//! 設計書は «$\mathrm{round}(f \cdot \tau)$ のように整数オフセットへ丸めると
//! $f$ が 2 値に潰れる» と言う．**確かめられる主張なので試験にしてある．**
//!
//! | 接線 | $f < 0.5$ | $f \geq 0.5$ |
//! | --- | --- | --- |
//! | 軸平行 $(1, 0)$ | 一律 $(0,0)$ — 何も起きない | 一律 $(1,0)$ — まるごと 1 画素 |
//! | 対角 $(0.707, 0.707)$ | **$f < 0.707$ の全域が $(0,0)$** | — |
//!
//! したがって $f$ は**オフセットではなく «どれだけ色を渡すか»** に写す．
//!
//! # 水を注ぐ規則 1 つで，設計書の 4 行の表が出る
//!
//! 接線の単位ステップ ([`Vec2::unit_step`](crate::math::Vec2::unit_step)) を
//! $s$ として，画素 $p$ の色の $f$ を $q = p + s$ へ渡す．
//! すると $q$ の色は $(1-f)\,c_q + f\,c_p$ の混色になり，**それをパレットの
//! 既存色へ寄せる**．場合分けは要らない — 設計書 6.10 の表はこの 1 つの規則から出る．
//!
//! | $f$ | 出てくる形 | 表の記述 |
//! | --- | --- | --- |
//! | $0 < f < 0.5$ | $q$ は $c_q$ 寄りの中間色，$p$ は $c_p$ のまま | 「進行方向側に中間色を 1 段」 |
//! | $f = 0.5$ | 両方が中点 | 「中間色を両側に 1 段ずつ」 |
//! | $0.5 < f < 1$ | $q$ が $c_p$ に寄り，$p$ に中間色が残る | 「シフトし，後方に中間色」 |
//! | $f = 1$ | まるごと入れ替わる | 「通常の移動」 |
//!
//! # 中間色はランプからではなくパレットから引く (D81 ・D83 と同じ)
//!
//! 設計書の `SelectAAIndex(R, ...)` はランプを引くが，**ランプの宣言はファイルに
//! 残らない** (D81) ．`pxsmith aa` が既に «パレットの中で 2 色の間にある色» を探す形に
//! 直してあるので (D83) ，**同じ [`nearest_between`] を使う** — 付けた中間色を
//! `pxsmith clean --remove-aa` で外せる性質もそのまま引き継ぐ．
//!
//! **無ければ作らない．** 中間色が無い組は動かさずに数える (`no_colour`) —
//! 設計書 6.10 の «既にパレットにある色を再利用し，新色を作らない» はここである．
//!
//! # 透明との間に中間色は無い (D4 の帰結)
//!
//! アルファは 2 値なので (D4) ，**シルエットの外側との間には中間色が存在しない**．
//! したがってサブピクセルが効くのは «色が接線方向に変わっている» 画素だけである．
//!
//! # 実素材で測った結果 (`pxsmith-calib subpixel`，CC0 61 枚)
//!
//! パレットの色の組のうち «間の色» があるのは**中央 53.6%** (最小 16.7% ・
//! 最大 98.2%) ．`pxsmith aa` の 81.3% (D83) より低いのは，あちらが «実際に隣り合って
//! いる色の組» を数えているのに対しこちらが総当たりだからである．
//!
//! | 方法 ・範囲 | 鎖 | 候補 | 中間色が無い | 動いた画素 | **輪郭が動いた** | blocking 増 |
//! | --- | --- | --- | --- | --- | --- | --- |
//! | 接線 ・シルエットだけ | 591 | 1431 | 594 (41.5%) | 837 | **0 枚** | 0 / 61 |
//! | **接線 ・色境界も (既定)** | 10812 | 9764 | 4013 (41.1%) | 5751 | **0 枚** | 1 / 61 |
//! | 高速法 | — | 14873 | 0 | 12759 | **32 / 61 枚** | 5 / 61 |
//!
//! **シルエットの輪郭だけでは効く画素が 7 分の 1 になる** (絵あたり中央 6 画素) ．
//! 色境界の輪郭も取ると 47 画素になり，**どちらもシルエットは 1 画素も動かない**．
//!
//! > [!warning] **D39 «パレット強制で滲みは構造的に解消する» は当たっているが，
//! > «だから書籍の手順より良い» は出てこない．**
//! > パレット外の色は確かに 0 になる (最近傍で必ず既存色へ落ちる) ．しかし
//! > 高速法は**シルエットを動かす** (61 枚中 32 枚) — 200% 拡大して 1 画素
//! > ずらす以上，形そのものが半画素動くからである．**滲みは消えても，
//! > 書籍が «信頼性が低い» と言う理由がもう 1 つ残っている．**
//! > 既定は接線法にしてある．
//!
//! 移動率 $f$ は**候補の «数» を変えない** — [`nearest_between`] が «間にあるか»
//! だけで候補を絞り，$f$ はその中から選ぶだけだからである．変わるのは色の方で，
//! **$f = 0.25$ と $0.75$ で 61 枚中 54 枚の出力が違う** (違う画素は中央 22) ．
//! 間の色が 2 色以上ある組が 92.6% あるので，$f$ は実際に効いている．

use std::collections::{BTreeMap, BTreeSet};

use crate::aa::nearest_between;
use crate::canvas::IndexedCanvas;
use crate::color::{Oklab, Rgba8};
use crate::error::{CoreError, Result};
use crate::geom::{Mask, split_monotone, trace_color_boundaries, trace_contours};
use crate::math::{IVec2, ivec2};
use crate::palette::Palette;
use crate::quantize::oklab_to_rgba;

/// 生成のしかた (設計書 6.10)．
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum SubpixelMethod {
    /// **接線方向へ色を渡す** (D38)．形の向きを見る．
    #[default]
    Tangent,
    /// **高速法** (D39)．200% 最近傍 → 1 画素移動 → 50% 縮小 → **パレット強制**．
    ///
    /// **形の向きを見ない** — 参考書籍が «信頼性が低い» と言うのはここである．
    Fast,
}

impl SubpixelMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tangent => "tangent",
            Self::Fast => "fast",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "tangent" => Some(Self::Tangent),
            "fast" => Some(Self::Fast),
            _ => None,
        }
    }
}

/// どの輪郭から接線を取るか．
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum SubpixelScope {
    /// シルエットの輪郭だけ (設計書 6.10 の `TraceChains(F)` の素直な読み)．
    Silhouette,
    /// **色ごとの領域の輪郭も含める．**
    ///
    /// シルエットの輪郭だけだと，輪郭に沿って色が変わらない絵では 1 画素も動かない．
    /// «線 ・曲線 ・形» には内部の線も含まれるので，色境界の輪郭も取る．
    #[default]
    Colours,
}

impl SubpixelScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Silhouette => "silhouette",
            Self::Colours => "colours",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "silhouette" => Some(Self::Silhouette),
            "colours" | "colors" => Some(Self::Colours),
            _ => None,
        }
    }
}

/// サブピクセルの設定 (設計書 3.5 の `SubpixelSpec`)．
#[derive(Clone, Debug)]
pub struct SubpixelOptions {
    /// 移動率．**オフセットではなく «どれだけ色を渡すか»** である．
    pub fraction: f32,
    pub method: SubpixelMethod,
    pub scope: SubpixelScope,
    /// 顔 ・目など，触ってはいけない領域 (設計書 6.10．**運用上必須**)．
    pub exclude: Option<Mask>,
    /// 中間色とみなす許容．`pxsmith aa` と同じ値を使う (往復できる性質のため)．
    pub tolerance: f32,
    /// 新しい列に何ドット未満なら «孤立» とみなすか (設計書 6.10: 2〜3 ドット残す)．
    pub min_run: u32,
    /// 高速法で動かす向き．**接線を見ないので呼ぶ側が決める**．
    pub direction: IVec2,
}

/// 既定の移動率 (設計書 6.10 の表)．
pub const DEFAULT_FRACTION: f32 = 0.5;
/// 中間色の許容．`pxsmith aa` (D83) と同じ．
pub const DEFAULT_TOLERANCE: f32 = 0.04;
/// 孤立列の下限 (設計書 6.10 «2〜3 ドット残す»)．
pub const DEFAULT_MIN_RUN: u32 = 2;

impl Default for SubpixelOptions {
    fn default() -> Self {
        Self {
            fraction: DEFAULT_FRACTION,
            method: SubpixelMethod::default(),
            scope: SubpixelScope::default(),
            exclude: None,
            tolerance: DEFAULT_TOLERANCE,
            min_run: DEFAULT_MIN_RUN,
            direction: ivec2(1, 0),
        }
    }
}

/// 生成の素性．
#[derive(Clone, Debug, Default)]
pub struct SubpixelReport {
    /// 接線を取った鎖の数．
    pub chains: usize,
    /// 色を渡す相手がいた画素の対の数 (**接線方向に色が変わっている画素**)．
    pub candidates: usize,
    /// 実際に添字が変わった画素の数．
    pub changed: usize,
    /// 中間色がパレットに無くて動かさなかった対の数．**作らない** (設計書 6.10)．
    pub no_colour: usize,
    /// 除外マスクで飛ばした画素の数．
    pub excluded: usize,
    /// 直した孤立列の数．
    pub isolated_fixed: usize,
    /// 使った添字の数 (前, 後)．
    ///
    /// > [!warning] **ここが増えるのは正常である．**
    /// > 中間色を置くとは «パレットの中の，まだ使っていない色を使い始める» こと
    /// > だから，増えて当たり前である (設計書 6.10 «既にパレットにある色を
    /// > 再利用し，新色を作らない») ．
    /// >
    /// > **端から端まで CLI で通して分かった** — `pxsmith shade` の 17 色パレットに
    /// > 掛けると «色が 3 増えた» と警告が出た．単体試験も `pxsmith-calib` も
    /// > **パレットが «使っている色ちょうど» の絵でしか測っていなかった**ので，
    /// > 使える予備の色が無く，増えようが無かった．
    /// >
    /// > 守るべき不変条件は «パレットの外の添字を出さないこと» の方で，
    /// > それは [`SubpixelReport::escapes_palette`] で見る．
    pub colors: (usize, usize),
    /// **シルエットが動いた画素の数** (透明 / 不透明が入れ替わった画素)．
    ///
    /// > [!warning] **0 でないなら «サブピクセル» ではない** (付録 C #5 を閉じた)．
    /// > 設計書 6.10 の表は $f = 1$ を «単位ステップ分シフト
    /// > (**サブピクセルではなく通常の移動**)» と定めている．シルエットが動くとは
    /// > 輪郭が丸ごと 1 画素ずれることなので，それは中間フレームではなく
    /// > **次のコマ**である．
    /// >
    /// > **lint はこの欠陥を見ない** — 実素材 61 枚で高速法を測ると，
    /// > **28 枚がシルエットを動かしながら blocking を 1 件も増やさない**．
    /// > 輪郭がずれた絵は «壊れた絵» ではなく **«正しく描かれた別の絵»** だから
    /// > である．だから検査ではなく**この欄で報告する** (D101 ・D107 と同じ側)．
    pub silhouette_moved: usize,
}

impl SubpixelReport {
    /// パレットの外の添字が出たか (**これが本当の不変条件**)．
    pub fn escapes_palette(canvas: &IndexedCanvas, palette: &Palette) -> bool {
        canvas.pixels().iter().any(|&i| palette.get(i).is_none())
    }

    /// **1 画素も動かなかったか．**
    ///
    /// «効かなかった» と «効いた» を混ぜない (D77 の作法) ．
    pub fn still(&self) -> bool {
        self.changed == 0
    }
}

/// サブピクセルを 1 段掛ける (設計書 6.10)．
pub fn subpixel(
    canvas: &IndexedCanvas,
    palette: &Palette,
    opts: &SubpixelOptions,
) -> Result<(IndexedCanvas, SubpixelReport)> {
    if !(0.0..=1.0).contains(&opts.fraction) || !opts.fraction.is_finite() {
        return Err(CoreError::SubpixelBadFraction {
            fraction: opts.fraction,
        });
    }
    let before = used_indices(canvas);
    let (mut out, mut report) = match opts.method {
        SubpixelMethod::Tangent => tangent(canvas, palette, opts)?,
        SubpixelMethod::Fast => fast(canvas, palette, opts)?,
    };
    report.isolated_fixed = fix_isolated_columns(&mut out, canvas, opts.min_run);
    report.colors = (before.len(), used_indices(&out).len());
    // **孤立列を直した «後» で数える** — 直す前に数えると，道具が自分で戻した
    // ぶんまで «動いた» と報告してしまう
    report.silhouette_moved = silhouette_diff(canvas, &out);
    Ok((out, report))
}

/// シルエットが動いた画素を数える (透明 / 不透明が入れ替わった画素)．
///
/// **取り方は 1 か所にしか置かない** (D110) — `pxsmith-calib` は自前で持たずここを呼ぶ．
fn silhouette_diff(a: &IndexedCanvas, b: &IndexedCanvas) -> usize {
    let mut n = 0usize;
    for y in 0..a.height() as i32 {
        for x in 0..a.width() as i32 {
            let p = ivec2(x, y);
            if a.is_transparent_at(p) != b.is_transparent_at(p) {
                n += 1;
            }
        }
    }
    n
}

/// 接線法 (D38)．
fn tangent(
    canvas: &IndexedCanvas,
    palette: &Palette,
    opts: &SubpixelOptions,
) -> Result<(IndexedCanvas, SubpixelReport)> {
    let mut report = SubpixelReport::default();

    // **画素ごとの «渡す向き»** を鎖から集める．同じ画素が複数の鎖に乗ることが
    // あるので，**最初に決まった向きを採る** (決定論性の規則 2 — 走査順は
    // 輪郭追跡の順で固定されている)
    let mut step_at: BTreeMap<(i32, i32), IVec2> = BTreeMap::new();
    let mut add_chains = |mask: &Mask, report: &mut SubpixelReport| {
        for contour in trace_contours(mask) {
            for chain in split_monotone(&contour) {
                let step = chain.tangent().unit_step();
                if step == ivec2(0, 0) {
                    continue;
                }
                report.chains += 1;
                for p in chain.points() {
                    step_at.entry((p.x, p.y)).or_insert(step);
                }
            }
        }
    };

    add_chains(&silhouette(canvas), &mut report);
    if opts.scope == SubpixelScope::Colours {
        for (index, contours) in trace_color_boundaries(canvas) {
            if canvas.transparent() == Some(index) {
                continue;
            }
            for contour in contours {
                for chain in split_monotone(&contour) {
                    let step = chain.tangent().unit_step();
                    if step == ivec2(0, 0) {
                        continue;
                    }
                    report.chains += 1;
                    for p in chain.points() {
                        step_at.entry((p.x, p.y)).or_insert(step);
                    }
                }
            }
        }
    }

    // **読むのは元の絵，書くのは新しい絵．** 途中の結果を読むと，渡した色が
    // さらに先へ流れて 1 段のはずが何段にもなる
    let mut out = canvas.clone();
    for (&(x, y), &step) in &step_at {
        let p = ivec2(x, y);
        let q = p + step;
        if opts.exclude.as_ref().is_some_and(|m| m.get(p) || m.get(q)) {
            report.excluded += 1;
            continue;
        }
        let (Some(cp), Some(cq)) = (canvas.get_at(p), canvas.get_at(q)) else {
            continue;
        };
        if cp == cq {
            continue;
        }
        // **透明との間に中間色は無い** (D4)．シルエットの外へは渡せない
        if canvas.is_transparent_at(p) || canvas.is_transparent_at(q) {
            continue;
        }
        report.candidates += 1;
        match pour(palette, cq, cp, opts.fraction, opts.tolerance) {
            Some(index) => {
                if index != cq && out.set_at(q, index) {
                    report.changed += 1;
                }
            }
            None => report.no_colour += 1,
        }
    }
    Ok((out, report))
}

/// $c_\text{into}$ に $c_\text{from}$ を $f$ だけ注いだ色を，**パレットの中から**引く．
///
/// 端 ($f = 0$ ・$1$) はそのまま両端を返す．間は [`nearest_between`] で探し，
/// **無ければ `None`** — 作らない (設計書 6.10) ．
fn pour(palette: &Palette, into: u8, from: u8, f: f32, tolerance: f32) -> Option<u8> {
    if f <= 0.0 {
        return Some(into);
    }
    if f >= 1.0 {
        return Some(from);
    }
    let (a, b) = (palette.lab_of(into)?, palette.lab_of(from)?);
    let target = Oklab {
        l: a.l + (b.l - a.l) * f,
        a: a.a + (b.a - a.a) * f,
        b: a.b + (b.b - a.b) * f,
    };
    // 間に色があればそれを使う．無い場合は «近い方の端» に落ちるが，それは
    // «動かない» か «まるごと動く» のどちらかであって中間色ではないので，
    // 中間を要求された ($0 < f < 1$) 以上は無いと言う
    nearest_between(palette, into, from, target, tolerance)
}

/// 高速法 (D39)．
///
/// $$ \text{200\% 最近傍} \to 1\text{px 移動} \to \text{50\% 縮小}
///    \to \textbf{palette apply} \to \text{孤立列の修正} $$
///
/// **減色の段は要らない** — 入力は既に指標付き (量子化済み) である．
/// 参考書籍の欠点は «滲み» すなわちパレット外の色の発生なので，
/// **50% 縮小の直後にパレットへ戻せば構造的に起きない** (D39) ．
fn fast(
    canvas: &IndexedCanvas,
    palette: &Palette,
    opts: &SubpixelOptions,
) -> Result<(IndexedCanvas, SubpixelReport)> {
    if opts.direction == ivec2(0, 0) {
        return Err(CoreError::SubpixelNoDirection);
    }
    let mut report = SubpixelReport::default();
    let (w, h) = (canvas.width(), canvas.height());
    let transparent = canvas.transparent();
    let fill = transparent.unwrap_or(0);

    // 200% 最近傍 + 1 画素移動 (2 倍の画布での 1 画素 = 元の半画素)
    let (bw, bh) = (w * 2, h * 2);
    let mut big = vec![fill; (bw * bh) as usize];
    for y in 0..bh as i32 {
        for x in 0..bw as i32 {
            let sx = (x - opts.direction.x).div_euclid(2);
            let sy = (y - opts.direction.y).div_euclid(2);
            if let Some(index) = canvas.get(sx, sy) {
                big[(y as u32 * bw + x as u32) as usize] = index;
            }
        }
    }

    // 50% 縮小 — 2x2 を混ぜてからパレットへ戻す．**混色は OKLab で取る**
    let mut out = IndexedCanvas::filled(w, h, fill).with_transparent(transparent);
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let mut opaque = 0usize;
            let (mut l, mut a, mut b) = (0.0f32, 0.0f32, 0.0f32);
            let mut same: BTreeSet<u8> = BTreeSet::new();
            for dy in 0..2i32 {
                for dx in 0..2i32 {
                    let index = big[(((y * 2 + dy) as u32) * bw + (x * 2 + dx) as u32) as usize];
                    if transparent == Some(index) {
                        continue;
                    }
                    let Some(lab) = palette.lab_of(index) else {
                        continue;
                    };
                    opaque += 1;
                    same.insert(index);
                    l += lab.l;
                    a += lab.a;
                    b += lab.b;
                }
            }
            // **透明が過半なら透明のまま** — アルファは 2 値なので (D4) 半端は無い．
            // 2 対 2 は «過半» ではないので残す
            if opaque < 2 {
                continue;
            }
            let n = opaque as f32;
            let mixed = Oklab {
                l: l / n,
                a: a / n,
                b: b / n,
            };
            if same.len() > 1 {
                report.candidates += 1;
            }
            // **palette apply** — ここが D39 で足した段である
            let index = match palette.nearest(oklab_to_rgba(mixed), 1.0) {
                Some(i) => i,
                None => {
                    report.no_colour += 1;
                    continue;
                }
            };
            if opts.exclude.as_ref().is_some_and(|m| m.get(ivec2(x, y))) {
                report.excluded += 1;
                if let Some(orig) = canvas.get(x, y) {
                    out.set(x, y, orig);
                }
                continue;
            }
            out.set(x, y, index);
            if canvas.get(x, y) != Some(index) {
                report.changed += 1;
            }
        }
    }
    Ok((out, report))
}

/// 新しい列に 1 ドットだけ残さない (設計書 6.10 ・lint ルール 25)．
///
/// **«新しい列» は «元の絵では透明だったのに，今は不透明になった画素の列»** で
/// ある．そこに `min_run` 未満しか無ければ**元へ戻す** — 増やす方向へ直すと
/// 形が変わるので，触った側を取り消す．
fn fix_isolated_columns(out: &mut IndexedCanvas, before: &IndexedCanvas, min_run: u32) -> usize {
    if min_run < 2 {
        return 0;
    }
    let mut fixed = 0usize;
    for x in 0..out.width() as i32 {
        let mut run: Vec<IVec2> = Vec::new();
        let flush = |run: &mut Vec<IVec2>, out: &mut IndexedCanvas, fixed: &mut usize| {
            if !run.is_empty() && (run.len() as u32) < min_run {
                for p in run.iter() {
                    let index = before.get_at(*p).unwrap_or(0);
                    out.set_at(*p, index);
                }
                *fixed += 1;
            }
            run.clear();
        };
        for y in 0..out.height() as i32 {
            let p = ivec2(x, y);
            let fresh = before.is_transparent_at(p) && !out.is_transparent_at(p);
            if fresh {
                run.push(p);
            } else {
                flush(&mut run, out, &mut fixed);
            }
        }
        flush(&mut run, out, &mut fixed);
    }
    fixed
}

fn silhouette(canvas: &IndexedCanvas) -> Mask {
    let mut m = Mask::new(canvas.width(), canvas.height());
    for y in 0..canvas.height() as i32 {
        for x in 0..canvas.width() as i32 {
            let p = ivec2(x, y);
            if !canvas.is_transparent_at(p) {
                m.set(p, true);
            }
        }
    }
    m
}

fn used_indices(canvas: &IndexedCanvas) -> BTreeSet<u8> {
    canvas
        .pixels()
        .iter()
        .copied()
        .filter(|&i| canvas.transparent() != Some(i))
        .collect()
}

/// パレットに «2 色の間» の色がどれだけあるか (設計書 6.10 «中間色 1〜2 色»)．
///
/// 効く見込みを測るための口．`pxsmith aa` の 81.3% (D83) と同じ量である．
pub fn pairs_with_intermediate(palette: &Palette, tolerance: f32) -> (usize, usize) {
    let n = palette.len().min(256);
    let (mut have, mut total) = (0usize, 0usize);
    for a in 0..n {
        for b in (a + 1)..n {
            let (a, b) = (a as u8, b as u8);
            if palette.get(a).is_none_or(|c| c.a == 0) || palette.get(b).is_none_or(|c| c.a == 0) {
                continue;
            }
            total += 1;
            let (Some(x), Some(y)) = (palette.lab_of(a), palette.lab_of(b)) else {
                continue;
            };
            let mid = Oklab {
                l: (x.l + y.l) * 0.5,
                a: (x.a + y.a) * 0.5,
                b: (x.b + y.b) * 0.5,
            };
            if nearest_between(palette, a, b, mid, tolerance).is_some() {
                have += 1;
            }
        }
    }
    (have, total)
}

/// 参考: 設計書が «やってはいけない» と言う «$f$ をオフセットへ丸める» 方．
///
/// **試験のためだけに置いてある** — これが 2 値に潰れることを縛る．
pub fn rounded_offset(tangent: crate::math::Vec2, f: f32) -> IVec2 {
    ivec2(
        (tangent.x * f).round() as i32,
        (tangent.y * f).round() as i32,
    )
}

/// 参考: 混色を作るだけの関数 (パレット強制を外したときに何が起きるかを測る口)．
pub fn raw_mix(palette: &Palette, into: u8, from: u8, f: f32) -> Option<Rgba8> {
    let (a, b) = (palette.lab_of(into)?, palette.lab_of(from)?);
    Some(oklab_to_rgba(Oklab {
        l: a.l + (b.l - a.l) * f,
        a: a.a + (b.a - a.a) * f,
        b: a.b + (b.b - a.b) * f,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::vec2;
    use crate::palette::Palette;

    /// 明度が等間隔に並ぶ 6 色 (添字 0 は透明)．
    fn palette() -> Palette {
        let mut colors = vec![Rgba8::TRANSPARENT];
        for i in 0..5u8 {
            let v = 30 + i * 50;
            colors.push(Rgba8::new(v, v, v, 255));
        }
        Palette::new(colors).expect("パレット")
    }

    /// **壊れると: サブピクセルが «1 画素まるごと動く» か «何もしない» の 2 値に潰れる．**
    ///
    /// 設計書 6.10 の主張そのものである — 代数なので測定ではなく試験で縛る．
    #[test]
    fn rounding_the_offset_collapses_the_fraction_into_two_values() {
        let axis = vec2(1.0, 0.0);
        for f in [0.1f32, 0.2, 0.3, 0.4, 0.49] {
            assert_eq!(rounded_offset(axis, f), ivec2(0, 0), "f={f} で動いてしまう");
        }
        for f in [0.5f32, 0.7, 0.9, 1.0] {
            assert_eq!(rounded_offset(axis, f), ivec2(1, 0), "f={f}");
        }
        // 対角ではさらに悪い — 0.707 未満の全域が止まる
        let diag = vec2(0.70710677, 0.70710677);
        for f in [0.5f32, 0.6, 0.7] {
            assert_eq!(rounded_offset(diag, f), ivec2(0, 0), "f={f} の斜めが動く");
        }
    }

    /// **壊れると: $f$ の表 4 行が «1 つの規則から出る» という前提が崩れる．**
    #[test]
    fn one_pouring_rule_reproduces_the_four_rows_of_the_table() {
        let p = palette();
        // 添字 1 (最暗) へ添字 5 (最明) を注ぐ
        assert_eq!(pour(&p, 1, 5, 0.0, DEFAULT_TOLERANCE), Some(1), "f=0");
        assert_eq!(pour(&p, 1, 5, 1.0, DEFAULT_TOLERANCE), Some(5), "f=1");
        let mid = pour(&p, 1, 5, 0.5, DEFAULT_TOLERANCE).expect("中点");
        assert_eq!(mid, 3, "f=0.5 は中点の色");
        let near = pour(&p, 1, 5, 0.25, DEFAULT_TOLERANCE).expect("手前");
        assert_eq!(near, 2, "f<0.5 は元の色寄りの中間色");
        let far = pour(&p, 1, 5, 0.75, DEFAULT_TOLERANCE).expect("奥");
        assert_eq!(far, 4, "f>0.5 は渡す色寄りの中間色");
    }

    /// **壊れると: 中間色を «作って» パレットが増える (設計書 6.10)．**
    #[test]
    fn a_pair_without_an_intermediate_is_reported_rather_than_invented() {
        // 2 色だけのパレットには «間» が無い
        let p = Palette::new(vec![
            Rgba8::TRANSPARENT,
            Rgba8::new(20, 20, 20, 255),
            Rgba8::new(230, 230, 230, 255),
        ])
        .expect("パレット");
        assert_eq!(pour(&p, 1, 2, 0.5, DEFAULT_TOLERANCE), None);
    }

    /// **壊れると: サブピクセルがパレットに無い色を出す (D39 の «滲み»)．**
    ///
    /// 高速法は混色を作るので，**パレット強制を外すと本当に外れる**ことも同時に
    /// 縛る — 外れないなら «足した段» が効いていない証拠になる．
    #[test]
    fn the_fast_method_stays_inside_the_palette_because_of_the_forcing_step() {
        let p = palette();
        let mut c = IndexedCanvas::filled(16, 16, 0).with_transparent(Some(0));
        for y in 4..12i32 {
            for x in 4..12i32 {
                c.set(x, y, if x < 8 { 1 } else { 5 });
            }
        }
        let (out, _) = subpixel(
            &c,
            &p,
            &SubpixelOptions {
                method: SubpixelMethod::Fast,
                ..Default::default()
            },
        )
        .expect("高速法");
        for &i in out.pixels() {
            assert!(p.get(i).is_some(), "パレットに無い添字 {i}");
        }
        // パレット強制が無ければ外れる色が本当に出ることを示す
        let raw = raw_mix(&p, 1, 5, 0.5).expect("混色");
        assert!(
            !(0..p.len() as u8).any(|i| p.get(i) == Some(raw)),
            "混色がたまたまパレットにある — 試験が意味を失っている"
        );
    }

    /// **壊れると: 輪郭が動いたことを黙る — 中間フレームでない絵を返してしまう．**
    ///
    /// 付録 C #5．設計書 6.10 の表は $f = 1$ を «サブピクセルではなく通常の移動»
    /// と定めているので，**輪郭が動いた時点で中間フレームではない**．
    /// lint はこの形を見ない (壊れた絵ではなく «別の絵» だから) ので，
    /// 数えて報告する側にしか居場所が無い．
    #[test]
    fn a_moved_silhouette_is_counted_so_the_caller_can_see_it() {
        let p = palette();
        let mut c = IndexedCanvas::filled(16, 16, 0).with_transparent(Some(0));
        for y in 4..12i32 {
            for x in 4..12i32 {
                c.set(x, y, if x < 8 { 1 } else { 5 });
            }
        }
        // 接線法は**輪郭を動かさない** — 実素材 61 枚でも 0 / 61 である
        let (out, r) = subpixel(
            &c,
            &p,
            &SubpixelOptions {
                method: SubpixelMethod::Tangent,
                ..Default::default()
            },
        )
        .expect("接線法");
        assert_eq!(r.silhouette_moved, 0, "接線法が輪郭を動かした");

        // 数え方そのものも縛る — 透明を 1 画素足せば 1 と数えるはずである
        let mut moved = out.clone();
        moved.set(4, 4, 0);
        assert_eq!(silhouette_diff(&c, &moved), 1, "数え方が壊れている");
    }

    /// **壊れると: 孤立列を直した «前» で数えて，道具が自分で戻した分まで報告する．**
    #[test]
    fn the_silhouette_is_counted_after_the_isolated_columns_are_fixed() {
        let p = palette();
        let mut c = IndexedCanvas::filled(16, 16, 0).with_transparent(Some(0));
        for y in 4..12i32 {
            for x in 4..12i32 {
                c.set(x, y, if x < 8 { 1 } else { 5 });
            }
        }
        let (out, r) = subpixel(
            &c,
            &p,
            &SubpixelOptions {
                method: SubpixelMethod::Fast,
                ..Default::default()
            },
        )
        .expect("高速法");
        // 報告された数は «返した絵» と «元の絵» の差でなければならない
        assert_eq!(
            r.silhouette_moved,
            silhouette_diff(&c, &out),
            "報告と返した絵が食い違っている"
        );
    }

    /// **壊れると: 除外マスクを踏んで顔のドットが動く (ルール 26)．**
    #[test]
    fn the_exclusion_mask_is_never_touched() {
        let p = palette();
        let mut c = IndexedCanvas::filled(16, 16, 0).with_transparent(Some(0));
        for y in 4..12i32 {
            for x in 4..12i32 {
                c.set(x, y, if x < 8 { 1 } else { 5 });
            }
        }
        let mut exclude = Mask::new(16, 16);
        for y in 0..16i32 {
            for x in 0..16i32 {
                exclude.set(ivec2(x, y), true);
            }
        }
        for method in [SubpixelMethod::Tangent, SubpixelMethod::Fast] {
            let (out, r) = subpixel(
                &c,
                &p,
                &SubpixelOptions {
                    method,
                    exclude: Some(exclude.clone()),
                    ..Default::default()
                },
            )
            .expect("生成");
            assert_eq!(
                out.pixels(),
                c.pixels(),
                "{} が除外を踏んだ",
                method.as_str()
            );
            assert!(r.excluded > 0);
        }
    }

    /// **壊れると: パレットの外の添字を出す (パレット逸脱．ルール 1)．**
    ///
    /// > [!note] «使った添字の数が増えないこと» を不変条件にしてはいけない．
    /// > **予備の色があるパレットでは増えるのが正しい** — それが «中間色を
    /// > 置く» ということである．端から端まで通して初めて分かった．
    #[test]
    fn neither_method_leaves_the_palette() {
        // **予備の色があるパレットで測る** — 使っている色ちょうどのパレットだと，
        // 増えようが無いので試験が何も見ていないことになる
        let p = palette();
        let mut c = IndexedCanvas::filled(24, 24, 0).with_transparent(Some(0));
        for y in 4..20i32 {
            for x in 4..20i32 {
                c.set(x, y, if x < 12 { 1 } else { 5 });
            }
        }
        let used_before = used_indices(&c);
        assert!(used_before.len() < p.len() - 1, "予備の色が無い");

        for method in [SubpixelMethod::Tangent, SubpixelMethod::Fast] {
            let (got, r) = subpixel(
                &c,
                &p,
                &SubpixelOptions {
                    method,
                    ..Default::default()
                },
            )
            .expect("生成");
            assert!(
                !SubpixelReport::escapes_palette(&got, &p),
                "{}: パレットの外の添字が出た",
                method.as_str()
            );
            // 予備の色を «使い始める» のは正常な動作である
            assert!(r.colors.1 >= r.colors.0, "{:?}", r.colors);
        }
    }

    /// **壊れると: 新しい列に 1 ドットだけ残る (ルール 25)．**
    #[test]
    fn a_single_dot_in_a_fresh_column_is_put_back() {
        let p = palette();
        // 高さ 1 の出っ張り — 動かすと «新しい列に 1 ドット» ができる
        let mut c = IndexedCanvas::filled(12, 12, 0).with_transparent(Some(0));
        for x in 2..9i32 {
            c.set(x, 6, 3);
        }
        c.set(9, 6, 5);
        let (out, r) = subpixel(
            &c,
            &p,
            &SubpixelOptions {
                method: SubpixelMethod::Fast,
                min_run: 2,
                ..Default::default()
            },
        )
        .expect("生成");
        for x in 0..12i32 {
            let fresh: Vec<i32> = (0..12i32)
                .filter(|&y| {
                    c.is_transparent_at(ivec2(x, y)) && !out.is_transparent_at(ivec2(x, y))
                })
                .collect();
            assert!(fresh.len() != 1, "x={x} に 1 ドットだけ残った");
        }
        let _ = r;
    }

    /// **壊れると: 移動率が範囲外でも黙って動く．**
    #[test]
    fn a_fraction_outside_the_unit_interval_is_an_error() {
        let p = palette();
        let c = IndexedCanvas::filled(4, 4, 1).with_transparent(Some(0));
        assert!(matches!(
            subpixel(
                &c,
                &p,
                &SubpixelOptions {
                    fraction: 1.5,
                    ..Default::default()
                }
            ),
            Err(CoreError::SubpixelBadFraction { .. })
        ));
    }
}
