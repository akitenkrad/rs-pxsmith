//! フレーム間のルール 22 〜 27 (設計書 7.3 «フレーム間 sequence»)．
//!
//! 静止画のルールと違って**列そのものを見る**．道具の側では既に見ているものが
//! 多い — `px anim tween` はトポロジー変化を数え (22) ，`px anim subpixel` は
//! 孤立列を直し (25) ・除外レイヤを守り (26) ，`px anim squash` は体積の誤差を
//! 報告する (27) ．**ここで足すのは «外から来た列を検査する» 側**である．
//!
//! # 閾値が要るのは 2 つだけ
//!
//! | # | 量 | 校正 |
//! | --- | --- | --- |
//! | 22 | オイラー標数 | **数え上げ．校正しない** (D92) |
//! | 23 | 揺れた画素の割合 | 要る |
//! | 24 | 動く部位に載ったディザの画素数 | 要る |
//! | 25 | 新しい列のドット数 | **`px anim subpixel` の下限から引く** (D124) |
//! | 26 | 除外マスクで動いた画素 | **数え上げ．校正しない** |
//! | 27 | 体積の誤差 | **`px anim squash` の実測から引く** (D123) |
//!
//! # 「軌跡」は重心の列である (付録 C 要調査事項 #3)
//!
//! 書籍は線揺れの原因を 3 つ挙げ，そのうち 1 つを «**動きが軌跡に従っていない**»
//! とし，直し方を «**選択ツールでパーツをスライドさせる**» とする [^pl]．
//! つまり**正しい動きとは平行移動**であって，揺れとはそこからの外れである．
//!
//! では «軌跡» をどう取るか．**重心の列**で取る — これは新しい発明ではなく，
//! D114 (中割り) ・D120 (おばけ) で «重心を先に取り除くと平行移動が真値と
//! 画素単位で一致する» ことが分かっているからである．**同じ道具の別の場所で
//! 既に «動きの本体» として使っている量**を持ってくる．
//!
//! 重心で揃えた後に残るのが «形の変化» で，そのうち**行きつ戻りつするもの**が
//! 揺れである — 画素ごとに «入る / 出る» の反転を数え，**2 回以上反転した画素**を
//! 揺れとみなす．書籍が «止まっているキャラクターの線が揺れ続けていると特に
//! 目立つ» と言うのは，重心が動かないときに反転がそのまま見えるからである．
//!
//! **1 回だけの変化は揺れではない** — 形が進んだだけだからである (歩きの脚) ．
//!
//! [^pl]: Pixel Logic 第九章 (PAGE:234) «揺れる線»．

use px_core::canvas::IndexedCanvas;
use px_core::frame::{Frame, Surface};
use px_core::geom::topology::euler_characteristic;
use px_core::geom::{Mask, mask::Field};
use px_core::math::{IVec2, ivec2};

use crate::rules::LintConfig;
use crate::{Report, Violation, rule};

// ------------------------------------------------------------------ 下ごしらえ

/// フレームの不透明な画素．**画布が宣言した透明添字とパレットのアルファの両方を
/// 見る** (D109 — «透明添字» はパレットのアルファではない)．
pub fn opaque_mask(frame: &Frame) -> Mask {
    let mut m = Mask::new(frame.size.x, frame.size.y);
    for layer in &frame.layers {
        let Surface::Indexed(c) = &layer.surface else {
            continue;
        };
        for y in 0..c.height() as i32 {
            for x in 0..c.width() as i32 {
                let Some(i) = c.get(x, y) else { continue };
                if c.transparent() == Some(i) || frame.palette.get(i).is_some_and(|c| c.a == 0) {
                    continue;
                }
                m.set(ivec2(x, y), true);
            }
        }
    }
    m
}

/// 不透明な画素の重心 = **列の «軌跡»**．
///
/// **自前で書かない** — `px anim tween` ・`smear` ・`extrapolate` が使っている
/// [`px_core::tween::centroid`] をそのまま呼ぶ．重心の取り方が 2 つあると
/// «道具が動かした量» と «検査が見る量» が食い違う (D110)．
fn centroid(mask: &Mask) -> Option<IVec2> {
    (!mask.is_empty()).then(|| px_core::tween::centroid(mask))
}

fn shifted(m: &Mask, d: IVec2) -> Mask {
    let mut out = Mask::new(m.width(), m.height());
    for p in m.iter_set() {
        out.set(p + d, true);
    }
    out
}

/// 重心を原点へ揃えたマスク．**軌跡を取り除いた «形» だけが残る**．
fn aligned(m: &Mask, to: IVec2) -> Option<Mask> {
    let c = centroid(m)?;
    Some(shifted(m, to - c))
}

fn indexed_layers(frame: &Frame) -> impl Iterator<Item = (&str, &IndexedCanvas)> {
    frame.layers.iter().filter_map(|l| match &l.surface {
        Surface::Indexed(c) => Some((l.meta.name.as_str(), c)),
        _ => None,
    })
}

// ------------------------------------------------------------------ 検査できなかったもの

/// **何を検査しなかったか**．
///
/// D92 «書いていないことを黙らない» ・D104 ««測れない» の理由も分ける» と同じ
/// 扱いである．`kind` や除外マスクを持てるのは L0 だけなので (D119 ・D135) ，
/// `.aseprite` を渡すと黙って «違反 0» になる — それを言うための欄である．
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SequenceCoverage {
    pub frames: usize,
    /// `kind = "inbetween"` と印のあるフレームの数．**0 ならルール 22 は働かない**．
    pub inbetweens: usize,
    /// `subpixel_exclude` の付いたレイヤの数．**0 ならルール 26 は働かない**．
    pub excluded_layers: usize,
    /// 不透明な画素が無くて飛ばしたフレームの数．
    pub empty_frames: usize,
    /// 体積を見た «伸び縮みしているコマ対» の数．**0 ならルール 27 は働かない**．
    pub squash_pairs: usize,
}

impl SequenceCoverage {
    /// 働かなかったルールの番号．**«鳴らなかった» と «検査していない» を分ける** (D77)．
    pub fn unchecked(&self) -> Vec<u8> {
        let mut out = Vec::new();
        if self.inbetweens == 0 {
            out.push(22);
        }
        if self.excluded_layers == 0 {
            out.push(26);
        }
        if self.squash_pairs == 0 {
            out.push(27);
        }
        out
    }
}

// ------------------------------------------------------------------ 入口

/// フレーム列を検査する (ルール 22 〜 27)．
///
/// **静止画のルールは掛けない** — それは [`crate::lint_frame`] の仕事である．
pub fn lint_sequence(frames: &[Frame], cfg: &LintConfig) -> (Report, SequenceCoverage) {
    let mut report = Report::default();
    let mut cov = SequenceCoverage {
        frames: frames.len(),
        ..Default::default()
    };
    if frames.len() < 2 {
        return (report, cov);
    }

    let masks: Vec<Mask> = frames.iter().map(opaque_mask).collect();
    cov.empty_frames = masks.iter().filter(|m| m.is_empty()).count();

    rule_22_topology(frames, &masks, &mut cov, &mut report);
    rule_23_wobble(&masks, cfg, &mut report);
    rule_24_dither_in_motion(frames, &masks, cfg, &mut report);
    rule_25_orphan_column(&masks, cfg, &mut report);
    rule_26_exclusion(frames, &mut cov, &mut report);
    rule_27_volume(&masks, cfg, &mut cov, &mut report);

    (report.sorted(), cov)
}

// ------------------------------------------------------------------ 22

/// ルール 22 — **中割りのオイラー標数が前後のどちらとも違う**．
///
/// 設計書 6.9 は «SDF 補間はトポロジーを保証できない» と言い，D114 は実測で
/// **45 件中 11 件が両端のどちらとも違う**ことを確かめた．道具 (`px anim tween`)
/// は自分の出力について既に数えているので，ここが見るのは**外から来た列**である．
///
/// **閾値は無い** — 標数は数え上げなので校正の対象ではない (D92)．
///
/// > [!warning] **`kind` を持てるのは L0 だけである** (D119)．
/// > `.aseprite` を読み直すと中割りが `key` に戻るので，このルールは 1 度も
/// > 働かない．[`SequenceCoverage::unchecked`] がそれを言う．
fn rule_22_topology(
    frames: &[Frame],
    masks: &[Mask],
    cov: &mut SequenceCoverage,
    report: &mut Report,
) {
    let r = rule(22).expect("ルール 22 は定義済み");
    for i in 1..frames.len().saturating_sub(1) {
        if !frames[i].kind.is_inbetween() {
            continue;
        }
        cov.inbetweens += 1;
        if masks[i].is_empty() || masks[i - 1].is_empty() || masks[i + 1].is_empty() {
            continue;
        }
        let (a, m, b) = (
            euler_characteristic(&masks[i - 1]),
            euler_characteristic(&masks[i]),
            euler_characteristic(&masks[i + 1]),
        );
        if m != a && m != b {
            report.push(Violation::new(
                r,
                format!(
                    "フレーム {i} (中割り) のオイラー標数 {m} が前 {a} ・後 {b} の\
                     どちらとも違う (成分の分裂か穴の出入りが起きている)"
                ),
            ));
        }
    }
}

// ------------------------------------------------------------------ 23

/// ルール 23 — **重心で揃えた輪郭が 1 画素だけ行きつ戻りつしている**．
///
/// 軌跡の取り方はモジュール文書のとおり．**2 回以上反転した画素**だけを数え，
/// シルエットの平均面積で割る．
///
/// > [!warning] **«2 回以上反転» だけでは潰しと分かれない．**
/// > `px anim squash` が作った潰し (0 → −0.1 → −0.2 → −0.1 → 0) は**設計上
/// > 非単調**なので，設計書 7.3 の «輪郭変位が軌跡に対して非単調» をそのまま
/// > 読むと必ず鳴る — 掃引しても正例の 20.0% (潰し 35 本すべて) が消えず，
/// > **むしろ潰しの方が揺れより比が大きい**ので閾値の向きが逆になる．
/// >
/// > **直したのは «適用範囲» の方である** (D70 と同じ形) — 伸び縮みは
/// > **ルール 27 の持ち場**なので，**外接矩形の寸法が変わっているコマを外す**．
/// > これで正例の誤爆が 20.0% → **0.0%** になり，負例の捕捉は 97.1% になる．
/// >
/// > | 試したこと | 正例で鳴る | 負例で捕捉 |
/// > | --- | --- | --- |
/// > | そのまま (非単調なら鳴らす) | 20.0% | 100% |
/// > | 反転を «芯に接するもの» に絞る | 20.0% | 100% |
/// > | 反転を «厚さ 1 の殻» に絞る | 20.0% | 100% |
/// > | **外接矩形が変わるコマを外す** | **0.0%** | **97.1%** |
/// >
/// > **絞り方を 2 つ捨ててある．同じ案を作り直さないこと** — 反転の «場所» で
/// > 絞っても分かれない．潰しは輪郭が 1 〜 2 画素しか動かないので，
/// > 揺れと同じ帯の中に入る．分かれるのは «そもそも形が変わっているか» である．
fn rule_23_wobble(masks: &[Mask], cfg: &LintConfig, report: &mut Report) {
    let r = rule(23).expect("ルール 23 は定義済み");
    if masks.len() < 3 {
        // **2 枚では «行きつ戻りつ» が定義できない** — D44 «最小 3 枚» と同じ理由
        return;
    }
    let Some(origin) = masks.iter().find_map(centroid) else {
        return;
    };
    // **外接矩形が変わっているコマは外す** — 伸び縮みはルール 27 の持ち場であり，
    // 潰しと伸ばしは «設計上 非単調» だからここで裁いてはいけない
    let boxes: Vec<Option<px_core::math::IRect>> = masks.iter().map(|m| m.bbox()).collect();
    let steady: Vec<usize> = (0..masks.len())
        .filter(|i| match (boxes[*i], boxes[0]) {
            (Some(a), Some(b)) => a.w == b.w && a.h == b.h,
            _ => false,
        })
        .collect();
    let frames: Vec<Mask> = steady
        .iter()
        .filter_map(|i| aligned(&masks[*i], origin))
        .collect();
    if frames.len() < 3 {
        return;
    }

    let (w, h) = (frames[0].width(), frames[0].height());
    let mut flips: Field<u32> = Field::filled(w, h, 0);
    for t in 1..frames.len() {
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let p = ivec2(x, y);
                if frames[t].get(p) != frames[t - 1].get(p)
                    && let Some(v) = flips.get_mut(p)
                {
                    *v += 1;
                }
            }
        }
    }

    let wobbling: Vec<IVec2> = frames[0]
        .bounds()
        .iter()
        .filter(|p| flips.copied(*p).unwrap_or(0) >= 2)
        .collect();
    let area = frames.iter().map(|m| m.count()).sum::<usize>() as f32 / frames.len() as f32;
    if area <= 0.0 {
        return;
    }
    let ratio = wobbling.len() as f32 / area;
    if ratio > cfg.wobble_ratio {
        let mut v = Violation::new(
            r,
            format!(
                "重心を揃えると {} 画素が 2 度以上出入りする (面積比 {:.1}%．\
                 上限 {:.1}%) — 動きが軌跡に従っていない",
                wobbling.len(),
                ratio * 100.0,
                cfg.wobble_ratio * 100.0
            ),
        );
        if let Some(p) = wobbling.first() {
            v = v.at(*p);
        }
        report.push(v);
    }
}

// ------------------------------------------------------------------ 24

/// ルール 24 — **ディザが物体と一緒に動いていない**．
///
/// 設計書は «動く部位にディザがある» の 1 行しか決めていない．そのまま読むと
/// **動く絵にディザがあれば必ず鳴る**ので blocking として使えない — 実際，
/// «変わった画素とディザ領域の重なり» で測ると実素材の平行移動 35 本のうち
/// 鳴ったのは 0 本，欠陥を入れた負例でも 1 本しか捕まらなかった (**効かない**)．
///
/// そこで**他の判断から引いた** (D135 と同じ形) ．D105 が示したとおり，ディザの
/// 位相は $x \mapsto x + d$ で $d$ が奇数のときだけ反転する — **問題はディザが
/// あることではなく，ディザが画布に貼り付いていて物体に付いてこないこと**である．
/// 付いてくれば見た目は静止し，付いてこなければ毎コマ位相が変わってちらつく．
///
/// したがって**重心で揃えて中身を突き合わせる** — 軌跡の取り方はルール 23 と
/// 同じである («動きの本体» の定義が 2 つあってはいけない．D110)．
fn rule_24_dither_in_motion(
    frames: &[Frame],
    masks: &[Mask],
    cfg: &LintConfig,
    report: &mut Report,
) {
    let r = rule(24).expect("ルール 24 は定義済み");
    for t in 1..frames.len() {
        let (Some(c0), Some(c1)) = (centroid(&masks[t - 1]), centroid(&masks[t])) else {
            continue;
        };
        let d = c1 - c0;
        if d.x == 0 && d.y == 0 {
            // 動いていないならちらつきようがない
            continue;
        }
        // **伸び縮みしているコマ対は外す** — ルール 23 ・25 と同じ理由．
        // 形が変われば模様も動くので «付いてきていない» と区別できない
        let (Some(ba), Some(bb)) = (masks[t - 1].bbox(), masks[t].bbox()) else {
            continue;
        };
        if ba.w != bb.w || ba.h != bb.h {
            continue;
        }
        let (mut checked, mut differs) = (0usize, 0usize);
        for ((_, before), (_, after)) in
            indexed_layers(&frames[t - 1]).zip(indexed_layers(&frames[t]))
        {
            let areas = crate::rules::dither_areas_windowed(after, cfg.moving_dither_window);
            if areas.is_empty() {
                continue;
            }
            for area in areas {
                for p in area.iter() {
                    // **軌跡を戻した位置**の画素と突き合わせる
                    let (Some(x), Some(y)) = (after.get_at(p), before.get_at(p - d)) else {
                        continue;
                    };
                    if after.transparent() == Some(x) || before.transparent() == Some(y) {
                        continue;
                    }
                    checked += 1;
                    if x != y {
                        differs += 1;
                    }
                }
            }
        }
        if checked == 0 {
            continue;
        }
        let ratio = differs as f32 / checked as f32;
        if ratio > cfg.moving_dither_ratio {
            report.push(Violation::new(
                r,
                format!(
                    "フレーム {t}: 重心を戻してもディザ領域の {differs} / {checked} 画素 \
                     ({:.1}%) が入れ替わる (上限 {:.1}%) — ディザが画布に貼り付いていて\
                     物体に付いてきていない．動かすとちらつく",
                    ratio * 100.0,
                    cfg.moving_dither_ratio * 100.0
                ),
            ));
        }
    }
}

// ------------------------------------------------------------------ 25

/// ルール 25 — **新しくできた «列の一続き» に 1 ドットしか残っていない**．
///
/// 数え方は `px anim subpixel` の `fix_isolated_columns` と**同じにする** —
/// «新しい列» は «元は透明で今は不透明になった画素の，縦に続く一かたまり» で
/// あって «列に何個あるか» ではない．直す側と検査する側で定義が違うと，
/// **道具が直した絵が検査に落ちる** (D110)．下限も
/// [`px_core::subpixel::DEFAULT_MIN_RUN`] から引く．
///
/// > [!warning] **«新しい列» は軌跡を戻してから数える．**
/// > 生の座標で数えると，**平行移動の先頭の縁がまるごと «新しい列» になる** —
/// > 実素材を 1 画素ずつ動かした列で 35 本中 33 本が誤爆した．物体が丸ごと
/// > 動いたのは «新しい列ができた» ではない．**重心を揃えてから数える**と
/// > 剛体の平行移動は新しい画素を 1 つも作らないので，残るのは
/// > «形が変わったところ» だけになる (ルール 23 ・24 と同じ軌跡を使う．D110)．
fn rule_25_orphan_column(masks: &[Mask], cfg: &LintConfig, report: &mut Report) {
    let r = rule(25).expect("ルール 25 は定義済み");
    let Some(origin) = masks.iter().find_map(centroid) else {
        return;
    };
    for t in 1..masks.len() {
        // **伸び縮みしているコマ対は外す** — ルール 23 と同じ理由で，変形は
        // ルール 27 の持ち場である．外さないと `px anim squash` の出力が
        // 35 本すべてこの blocking に落ちる (D58)
        let (Some(ba), Some(bb)) = (masks[t - 1].bbox(), masks[t].bbox()) else {
            continue;
        };
        if ba.w != bb.w || ba.h != bb.h {
            continue;
        }
        let (Some(a), Some(b)) = (aligned(&masks[t - 1], origin), aligned(&masks[t], origin))
        else {
            continue;
        };
        let (a, b) = (&a, &b);
        for x in 0..b.width() as i32 {
            let mut run = 0u32;
            let mut start = 0i32;
            for y in 0..=b.height() as i32 {
                let fresh = y < b.height() as i32 && b.get(ivec2(x, y)) && !a.get(ivec2(x, y));
                if fresh {
                    if run == 0 {
                        start = y;
                    }
                    run += 1;
                    continue;
                }
                if run > 0 && run < cfg.min_new_run {
                    report.push(
                        Violation::new(
                            r,
                            format!(
                                "フレーム {t}: 新しくできた列 {x} の一続きが {run} ドット\
                                 しかない (下限 {}) — 孤立列である",
                                cfg.min_new_run
                            ),
                        )
                        .at(ivec2(x, start)),
                    );
                }
                run = 0;
            }
        }
    }
}

// ------------------------------------------------------------------ 26

/// ルール 26 — **除外マスクの領域でドットが動いている**．
///
/// 顔 ・目など `subpixel_exclude` の付いたレイヤは，コマ間で 1 画素も動いては
/// いけない．**`kind = "inbetween"` のフレームには適用しない** — 書籍が
/// «一瞬しか映らない中間フレームなら顔のドットをシフトしてもよい» と述べて
/// いるためである [^pl2]．**閾値は無い** (数え上げ)．
///
/// [^pl2]: Pixel Logic 第八章 (PAGE:189-212)．
fn rule_26_exclusion(frames: &[Frame], cov: &mut SequenceCoverage, report: &mut Report) {
    let r = rule(26).expect("ルール 26 は定義済み");
    let excluded: Vec<&str> = frames
        .iter()
        .flat_map(|f| f.layers.iter())
        .filter(|l| l.meta.subpixel_exclude)
        .map(|l| l.meta.name.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    cov.excluded_layers = excluded.len();

    for name in excluded {
        let mut previous: Option<(usize, Vec<u8>)> = None;
        for (t, f) in frames.iter().enumerate() {
            if f.kind.is_inbetween() {
                continue;
            }
            let Some((_, canvas)) = indexed_layers(f).find(|(n, _)| *n == name) else {
                continue;
            };
            if let Some((prev_t, prev)) = &previous
                && prev.as_slice() != canvas.pixels()
            {
                let moved = prev
                    .iter()
                    .zip(canvas.pixels())
                    .filter(|(a, b)| a != b)
                    .count();
                report.push(Violation::new(
                    r,
                    format!(
                        "レイヤ '{name}' はサブピクセル対象外だが，フレーム {prev_t} から \
                         {t} で {moved} 画素動いている"
                    ),
                ));
            }
            previous = Some((t, canvas.pixels().to_vec()));
        }
    }
}

// ------------------------------------------------------------------ 27

/// ルール 27 — **伸び縮みしているのに体積 ($h \times w$) が変わっている**．
///
/// **どのコマ対にも掛けるのではない．** 片方の辺が伸びてもう片方が縮んだとき，
/// つまり «潰しと伸ばし» が起きているコマ対だけを見る — 歩きの脚のように
/// 両辺が同じ向きに動くものは squash / stretch ではないからである
/// (D70 が «適用範囲» でルール 2 ・3 を直したのと同じ形)．
///
/// **閾値は `px anim squash` の実測から引く** — 画素は整数なので体積は保存
/// しきれず，道具の側でも誤差の中央が 1.042% ・最悪 5.0% 残る (D123) ．
/// これより下に置くと**自分の出力が自分の検査に必ず落ちる**．
fn rule_27_volume(
    masks: &[Mask],
    cfg: &LintConfig,
    cov: &mut SequenceCoverage,
    report: &mut Report,
) {
    let r = rule(27).expect("ルール 27 は定義済み");
    for t in 1..masks.len() {
        let (Some(a), Some(b)) = (masks[t - 1].bbox(), masks[t].bbox()) else {
            continue;
        };
        let (dw, dh) = (b.w as i64 - a.w as i64, b.h as i64 - a.h as i64);
        // **伸び縮みしているコマ対だけを見る**
        if dw == 0 || dh == 0 || (dw > 0) == (dh > 0) {
            continue;
        }
        cov.squash_pairs += 1;
        let (v0, v1) = ((a.w * a.h) as f32, (b.w * b.h) as f32);
        if v0 <= 0.0 {
            continue;
        }
        let error = (v1 - v0).abs() / v0;
        if error > cfg.volume_error {
            report.push(Violation::new(
                r,
                format!(
                    "フレーム {} から {t} で外接矩形が {}x{} から {}x{} になり，\
                     体積が {:.1}% 変わっている (上限 {:.1}%)",
                    t - 1,
                    a.w,
                    a.h,
                    b.w,
                    b.h,
                    error * 100.0,
                    cfg.volume_error * 100.0
                ),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use px_core::canvas::IndexedCanvas;
    use px_core::color::Rgba8;
    use px_core::frame::{FrameKind, Layer, LayerMeta};
    use px_core::math::uvec2;
    use px_core::palette::Palette;

    fn palette() -> Palette {
        Palette::new(vec![
            Rgba8::TRANSPARENT,
            Rgba8::rgb(0x1a, 0x1c, 0x2c),
            Rgba8::rgb(0xf4, 0xf4, 0xf4),
        ])
        .unwrap()
    }

    /// 16x16 の四角を `at` に置いたフレーム．
    fn block(at: (i32, i32), size: (i32, i32), kind: FrameKind) -> Frame {
        let p = palette();
        let mut c = IndexedCanvas::filled(24, 24, 0);
        c.set_transparent(Some(0));
        for y in at.1..at.1 + size.1 {
            for x in at.0..at.0 + size.0 {
                c.set(x, y, 1);
            }
        }
        let mut f = Frame::new(uvec2(24, 24), p);
        f.kind = kind;
        f.layers
            .push(Layer::new(LayerMeta::named("art"), Surface::Indexed(c)));
        f
    }

    fn lint(frames: &[Frame]) -> (Report, SequenceCoverage) {
        lint_sequence(frames, &LintConfig::default())
    }

    fn fired(report: &Report, id: u8) -> bool {
        report.violations.iter().any(|v| v.rule == id)
    }

    /// **壊れると: 剛体の平行移動が «欠陥» として鳴る．**
    #[test]
    fn a_rigid_translation_fires_nothing() {
        let frames: Vec<Frame> = (0..4)
            .map(|t| block((2 + t * 2, 4), (8, 8), FrameKind::Key))
            .collect();
        let (report, _) = lint(&frames);
        assert!(report.violations.is_empty(), "{:?}", report.violations);
    }

    /// **壊れると: 6 つのルールが登録されていない (D130 と同じ数え上げ)．**
    #[test]
    fn all_six_sequence_rules_are_registered() {
        let ids: Vec<u8> = crate::RULES
            .iter()
            .filter(|r| r.scope == crate::Scope::Sequence)
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, vec![22, 23, 24, 25, 26, 27]);
    }

    /// **壊れると: 中割りのトポロジー変化を見逃す (ルール 22)．**
    #[test]
    fn an_inbetween_that_changes_the_euler_characteristic_is_blocking() {
        let mut middle = block((2, 4), (8, 8), FrameKind::Inbetween);
        // 真ん中に穴を開ける — 標数が 1 から 0 になる
        if let Surface::Indexed(c) = &mut middle.layers[0].surface {
            c.set(5, 7, 0);
        }
        let frames = vec![
            block((2, 4), (8, 8), FrameKind::Key),
            middle,
            block((2, 4), (8, 8), FrameKind::Key),
        ];
        let (report, cov) = lint(&frames);
        assert!(fired(&report, 22), "{:?}", report.violations);
        assert_eq!(cov.inbetweens, 1);
    }

    /// **壊れると: `kind` を持たない列で «違反 0» と黙る．**
    ///
    /// `.aseprite` は `kind` を持てないので (D119) ，ルール 22 は 1 度も働かない —
    /// **«鳴らなかった» と «検査していない» を分ける** (D77)．
    #[test]
    fn a_sequence_without_inbetweens_reports_rule_22_as_unchecked() {
        let frames: Vec<Frame> = (0..3)
            .map(|t| block((2 + t, 4), (8, 8), FrameKind::Key))
            .collect();
        let (_, cov) = lint(&frames);
        assert_eq!(cov.inbetweens, 0);
        assert!(cov.unchecked().contains(&22));
    }

    /// **壊れると: 潰しと伸ばしが «揺れる線» として blocking になる．**
    ///
    /// 設計書 7.3 の «軌跡に対して非単調» をそのまま読むと潰しは必ず鳴る —
    /// **書籍が教える技法を禁じてしまう**ので適用範囲を絞ってある．
    #[test]
    fn a_squash_and_stretch_is_not_a_wobble() {
        let frames = vec![
            block((2, 4), (8, 8), FrameKind::Key),
            block((2, 5), (8, 6), FrameKind::Key),
            block((2, 4), (8, 8), FrameKind::Key),
        ];
        let (report, _) = lint(&frames);
        assert!(!fired(&report, 23), "{:?}", report.violations);
    }

    /// **壊れると: 除外マスクを付けたレイヤが動いても鳴らない (ルール 26)．**
    #[test]
    fn a_layer_marked_subpixel_exclude_must_not_move() {
        let mut frames: Vec<Frame> = (0..3)
            .map(|_| block((2, 4), (8, 8), FrameKind::Key))
            .collect();
        for (t, f) in frames.iter_mut().enumerate() {
            let mut c = IndexedCanvas::filled(24, 24, 0);
            c.set_transparent(Some(0));
            c.set(4 + t as i32, 6, 2);
            let mut meta = LayerMeta::named("face");
            meta.subpixel_exclude = true;
            f.layers.push(Layer::new(meta, Surface::Indexed(c)));
        }
        let (report, cov) = lint(&frames);
        assert_eq!(cov.excluded_layers, 1);
        assert!(fired(&report, 26), "{:?}", report.violations);
    }

    /// **壊れると: 中割りでは許されている顔のシフトを禁じる．**
    ///
    /// 書籍が «一瞬しか映らない中間フレームなら顔のドットをシフトしてよい» と
    /// 述べているので，`kind = "inbetween"` のフレームには掛けない (7.1)．
    #[test]
    fn rule_26_does_not_apply_to_inbetweens() {
        let mut frames: Vec<Frame> = (0..3)
            .map(|t| {
                block(
                    (2, 4),
                    (8, 8),
                    if t == 1 {
                        FrameKind::Inbetween
                    } else {
                        FrameKind::Key
                    },
                )
            })
            .collect();
        for (t, f) in frames.iter_mut().enumerate() {
            let mut c = IndexedCanvas::filled(24, 24, 0);
            c.set_transparent(Some(0));
            // 中割りだけ動かす
            c.set(if t == 1 { 5 } else { 4 }, 6, 2);
            let mut meta = LayerMeta::named("face");
            meta.subpixel_exclude = true;
            f.layers.push(Layer::new(meta, Surface::Indexed(c)));
        }
        let (report, _) = lint(&frames);
        assert!(!fired(&report, 26), "{:?}", report.violations);
    }

    /// **壊れると: 下限が `px anim subpixel` とずれて，道具が直した絵が落ちる．**
    #[test]
    fn the_orphan_column_floor_comes_from_the_subpixel_tool() {
        assert_eq!(
            LintConfig::default().min_new_run,
            px_core::subpixel::DEFAULT_MIN_RUN
        );
    }
}
