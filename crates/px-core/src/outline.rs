//! 縁取り (`px outline --style`．設計書 D36)．
//!
//! 設計書が決めているのは «5 分類 + 選択的輪郭線．**背景想定を引数に取る**» だけで，
//! 置き方は決まっていない．実測して 2 つを決めた．
//!
//! # 既定は «内側» に描く
//!
//! 外へ 1 画素太らせると，**画像の縁に接している絵ははみ出して切れる** —
//! CC0 の実物 61 枚のうち **56 枚が縁に接しており，26 枚は外に 1 画素の余地も無い**
//! (画面いっぱいのタイル) ．外側は明示的に頼まれたときだけにする．
//!
//! 内側に描くと**シルエットが 1 画素も動かない**ので，`px conform` で戻した格子や
//! タイルの継ぎ目を壊さない．
//!
//! # 5 分類
//!
//! | 分類 | 縁の色 |
//! | --- | --- |
//! | [`OutlineStyle::Black`] | 純黒 (最も強い．lint ルール 18 が advisory で鳴る) |
//! | [`OutlineStyle::Tinted`] | **内側の色を暗くした色**．固有色になじむ |
//! | [`OutlineStyle::Contrast`] | **背景想定**に対して明暗を逆に取る (D36 の «背景想定») |
//! | [`OutlineStyle::Shaded`] | 光の当たる側は明るく ・影の側は暗く |
//! | [`OutlineStyle::None`] | **縁取りを剥がす** (内側の色へ戻す) |
//!
//! `selective` を立てると**光の当たる側を描かない** (選択的輪郭線) ．
//!
//! # 冪等である
//!
//! 内側に描く縁は «シルエットの縁の環» であり，2 度目も同じ環を同じ色にするだけ
//! なので**掛け直しても太らない** (`px aa` と違う点である) ．色の元は
//! **1 つ内側の画素**から取る — 自分の色から取ると，2 度目に «暗くした色を
//! さらに暗くする» ことになって色が沈んでいく．

use crate::canvas::IndexedCanvas;
use crate::color::{Oklab, Rgba8, distance_sq, oklab_of};
use crate::error::Result;
use crate::math::{IVec2, Vec2, ivec2};
use crate::palette::Palette;
use crate::quantize::oklab_to_rgba;
use crate::ramp::LightSource;

/// 縁取りの分類 (設計書 D36 の «5 分類»)．
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum OutlineStyle {
    /// 縁取りを剥がす (内側の色へ戻す)．
    None,
    /// 純黒．
    Black,
    /// 内側の色を暗くした色．
    Tinted,
    /// 背景想定に対して明暗を逆に取る．
    Contrast,
    /// 光の当たる側は明るく ・影の側は暗く．
    Shaded,
}

impl OutlineStyle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Black => "black",
            Self::Tinted => "tinted",
            Self::Contrast => "contrast",
            Self::Shaded => "shaded",
        }
    }

    /// 光源が要る分類か．
    pub fn needs_light(self) -> bool {
        matches!(self, Self::Shaded)
    }
}

/// 縁取りの設定．
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct OutlineOptions {
    pub style: OutlineStyle,
    /// **光の当たる側を描かない** (選択的輪郭線．D36)．
    pub selective: bool,
    /// **背景想定** (D36)．[`OutlineStyle::Contrast`] が明暗を決めるのに使う．
    pub background: Option<Rgba8>,
    /// 光源．`Shaded` と `selective` が使う．
    pub light: LightSource,
    /// **外側に描く**．既定は内側 (実測で 61 枚中 56 枚が画像の縁に接する)．
    pub outer: bool,
    /// 縁の色を作ってよい色数．
    pub max_new_colors: usize,
    /// 既にある色を «同じ色» とみなす色距離．
    pub tolerance: f32,
    /// `Tinted` ・`Shaded` が内側の色を暗くする量 (明度の割合)．
    ///
    /// **暫定値である．** 0.45 は «縁と分かるが黒くはない» ところを目で選んだだけで，
    /// 正例 ・負例で決めていない．
    pub darken: f32,
}

impl Default for OutlineOptions {
    fn default() -> Self {
        Self {
            style: OutlineStyle::Tinted,
            selective: false,
            background: None,
            light: LightSource::Directional {
                dir: Vec2 { x: -0.6, y: 0.8 },
            },
            outer: false,
            max_new_colors: 4,
            tolerance: 0.03,
            darken: 0.45,
        }
    }
}

/// 縁取りの結果．
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutlineReport {
    /// 縁の色にした画素の数．
    pub painted: usize,
    /// 剥がした画素の数 ([`OutlineStyle::None`])．
    pub removed: usize,
    /// 新しく作った色の数．
    pub added_colors: usize,
    /// **外側に描けなかった画素の数** (画像の外へ出る)．
    pub no_room: usize,
    /// 選択的輪郭線で飛ばした画素の数．
    pub skipped_lit: usize,
    /// **細くて内側が無く，触らなかった画素の数**．
    ///
    /// 幅 1 〜 2 画素の部分は «縁» と «中身» を分けられない．そこへ縁を描くと
    /// 中身が消えるうえ，色の元が縁の色になって**掛け直すたびに沈む**．
    pub skipped_thin: usize,
}

/// **縁取りを描く** (設計書 D36)．
pub fn outline(
    canvas: &mut IndexedCanvas,
    palette: &mut Palette,
    opts: &OutlineOptions,
) -> Result<OutlineReport> {
    let mut report = OutlineReport::default();
    let source = canvas.clone();
    let transparent = source.transparent();
    let opaque = |p: IVec2| source.get_at(p).is_some_and(|i| transparent != Some(i));

    // 面から光源へ向かう向き ($\ell$)．選択的輪郭線と `Shaded` が使う
    let l = light_direction(opts.light);

    let mut paint: Vec<(IVec2, u8)> = Vec::new();
    // 画素ごとの «欲しい色»．**色を決めるのは全部集めてから** (下記)
    let mut wanted: Vec<(IVec2, Oklab)> = Vec::new();
    for p in source.bounds().iter() {
        if !opaque(p) {
            continue;
        }
        // 外を向いている向き (透明な隣の方向を足す)
        let mut outward = Vec2 { x: 0.0, y: 0.0 };
        let mut border = false;
        let mut outside: Vec<IVec2> = Vec::new();
        for d in [ivec2(1, 0), ivec2(-1, 0), ivec2(0, 1), ivec2(0, -1)] {
            let q = p + d;
            let out_of_canvas = source.get_at(q).is_none();
            if out_of_canvas || !opaque(q) {
                border = true;
                outward.x += d.x as f32;
                outward.y += d.y as f32;
                if !out_of_canvas {
                    outside.push(q);
                }
            }
        }
        if !border {
            continue;
        }

        // **選択的輪郭線** — 外を向いている面が光を受けているなら描かない
        if opts.selective && outward.normalize().is_some_and(|n| n.dot(l) > 0.0) {
            report.skipped_lit += 1;
            continue;
        }

        // 縁の色の元は**1 つ内側の «縁でない» 画素**から取る．
        // 自分の色から取ると掛け直すたびに沈み，隣の縁の画素から取ると
        // «縁の色をさらに暗くした色» になる — どちらも冪等でなくなる
        let Some(inner) = inward_colour(&source, p, outward) else {
            // 内側が無い (幅 1 〜 2 画素の細い部分) ．**縁と中身を分けられないので触らない**
            report.skipped_thin += 1;
            continue;
        };

        match opts.style {
            // 剥がす — 縁の画素を内側の色へ戻す
            OutlineStyle::None => {
                if source.get_at(p) != Some(inner) {
                    paint.push((p, inner));
                    report.removed += 1;
                }
            }
            _ => {
                let Some(base) = palette.lab_of(inner) else {
                    continue;
                };
                let target = outline_colour(opts, base, outward, l);
                if opts.outer {
                    // 外側へ描く．画像の外へは出られない
                    if outside.is_empty() || outside_is_missing(&source, p) {
                        report.no_room += 1;
                    }
                    for q in outside {
                        wanted.push((q, target));
                    }
                } else {
                    wanted.push((p, target));
                }
            }
        }
    }

    // **縁の色は «全部集めてからまとめて決める»．**
    // 1 画素ずつ決めると，内側の色ごとに違う縁の色ができて**色数が増え，
    // 1 度しか使わない色が生まれる** — lint ルール 3 が «孤立ピクセル» と呼ぶ形である
    // (実測で 17 色の絵が 23 色になり blocking が 1 件出た) ．
    let resolved = resolve_all(palette, &wanted, opts)?;
    report.added_colors += resolved.added;
    for ((at, _), index) in wanted.iter().zip(&resolved.indices) {
        if source.get_at(*at) != Some(*index) {
            paint.push((*at, *index));
            report.painted += 1;
        }
    }

    for (at, to) in paint {
        canvas.set_at(at, to);
    }
    Ok(report)
}

/// まとめて色を決めた結果．
struct Resolved {
    indices: Vec<u8>,
    added: usize,
}

/// **欲しい色を «少ない色» へ寄せる．**
///
/// > [!warning] **束ね方を «その場の顔ぶれ» に依らせない．**
/// > 最初は欲しい色どうしを束ねて代表を作っていたが，**2 度目には顔ぶれが変わる**
/// > (1 度目に作った色が既にあるので «欲しい色» の集合が減る) ．束ね方が変われば
/// > 代表も変わり，掛け直すたびに色が増えた — 実測で良い絵 61 枚のうち 28 枚．
/// >
/// > **元の色を粗い格子へ丸めてから縁の色を決める**と，欲しい色は元の色だけで決まる
/// > (その場の顔ぶれに依らない) ．2 度目は同じ色がパレットに在るので何も増えない．
fn resolve_all(
    palette: &mut Palette,
    wanted: &[(IVec2, Oklab)],
    opts: &OutlineOptions,
) -> Result<Resolved> {
    // **色を作るのが先，割り当ては後．**
    // 作りながら割り当てると «その時点のパレット» で最も近い色が決まるので，
    // 2 度目には答えが変わる (1 度目に後から作った色が候補に増えているため) ．
    // 最後のパレットに対してまとめて割り当てれば，2 度目は同じ答えになる．
    let mut counts: std::collections::BTreeMap<[u32; 3], (usize, Oklab)> = Default::default();
    for (_, target) in wanted {
        // 並べ替えのために整数の鍵にする (浮動小数を鍵にしない — 規則 3)
        let key = [
            (target.l * 10000.0).round() as u32,
            ((target.a + 1.0) * 10000.0).round() as u32,
            ((target.b + 1.0) * 10000.0).round() as u32,
        ];
        let e = counts.entry(key).or_insert((0, *target));
        e.0 += 1;
    }
    // 多い順 ・同数なら色の順 (決定論性の規則 2)
    let mut ranked: Vec<(usize, Oklab)> = counts.into_values().collect();
    ranked.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(a.1.l.total_cmp(&b.1.l))
            .then(a.1.a.total_cmp(&b.1.a))
            .then(a.1.b.total_cmp(&b.1.b))
    });

    // **予算は «作った数» ではなく «上位いくつを見るか» に掛ける．**
    // 作った数で数えると，2 度目には上位が既にあるぶん予算が余り，**次の候補を
    // 作ってしまう** — 掛けるたびに色が増える (実測で 61 枚中 30 枚が 2 度目も塗った) ．
    let mut added = 0usize;
    for (_, target) in ranked.iter().take(opts.max_new_colors) {
        if palette.len() >= Palette::MAX_COLORS {
            break;
        }
        if nearest_within(palette, *target, opts.tolerance).is_none() {
            palette.push(oklab_to_rgba(*target))?;
            added += 1;
        }
    }

    // 割り当ては**最後のパレット**に対して行う
    let indices = wanted
        .iter()
        .map(|(_, target)| palette.nearest(oklab_to_rgba(*target), 1.0).unwrap_or(0))
        .collect();
    Ok(Resolved { indices, added })
}

/// 許容以内で最も近い色 (同点は小さい添字．決定論性の規則 2)．
fn nearest_within(palette: &Palette, target: Oklab, tolerance: f32) -> Option<u8> {
    let mut best: Option<(f32, u8)> = None;
    for (i, lab) in palette.lab().iter().enumerate() {
        let i = i as u8;
        if palette.get(i).is_some_and(|c| c.a == 0) {
            continue;
        }
        let d = distance_sq(*lab, target, 1.0).sqrt();
        if d <= tolerance && best.as_ref().is_none_or(|(b, _)| d < *b) {
            best = Some((d, i));
        }
    }
    best.map(|(_, i)| i)
}

/// 画像の外へ出る縁があるか (外側に描けない画素)．
fn outside_is_missing(canvas: &IndexedCanvas, p: IVec2) -> bool {
    [ivec2(1, 0), ivec2(-1, 0), ivec2(0, 1), ivec2(0, -1)]
        .iter()
        .any(|&d| canvas.get_at(p + d).is_none())
}

/// 面から光源へ向かう単位ベクトル．
/// **面から光源へ向かう単位ベクトル** ($\ell$)．
///
/// lint ルール 7 も同じものを使う — 陰影を作る側と «向きが合っているか» を検査する
/// 側で規約がずれると，正しい絵を «光源方向と矛盾» と言う．
pub fn light_direction(source: LightSource) -> Vec2 {
    match source {
        // `dir` は光源から面へ向かう向きなので逆にする
        LightSource::Directional { dir } => {
            (dir * -1.0).normalize().unwrap_or(Vec2 { x: 0.0, y: -1.0 })
        }
        LightSource::Point { pos, .. } => pos.normalize().unwrap_or(Vec2 { x: 0.0, y: -1.0 }),
        LightSource::Line { a, b, .. } => Vec2 {
            x: (a.x + b.x) * 0.5,
            y: (a.y + b.y) * 0.5,
        }
        .normalize()
        .unwrap_or(Vec2 { x: 0.0, y: -1.0 }),
        LightSource::Area { rect, .. } => rect
            .center()
            .normalize()
            .unwrap_or(Vec2 { x: 0.0, y: -1.0 }),
        LightSource::Ambient => Vec2 { x: 0.0, y: -1.0 },
    }
}

/// 符号 (0 は 0 のまま)．
///
/// > [!warning] **`f32::signum(0.0)` は `1.0` を返す．**
/// > そのまま使うと «真上が外» の画素で内側を斜めに探してしまい，**環の隣の画素**を
/// > 拾う．2 度目に «縁の色をさらに暗くした色» を作ることになり，掛け直すたびに
/// > 色が沈んで増えていった (冪等性の試験が捕まえた) ．
fn sign(v: f32) -> i32 {
    match v {
        _ if v > 0.0 => 1,
        _ if v < 0.0 => -1,
        _ => 0,
    }
}

/// **1 つ内側の «縁でない» 画素の色** (外を向いている向きの逆へ 1 歩)．
///
/// > [!warning] **縁の画素から色を取ってはいけない．**
/// > 掛け直すと «縁の色をさらに暗くした色» になり，色が沈みながら増えていく．
/// > 実測では良い絵 61 枚のうち **32 枚が 2 度目でも塗っていた**．
fn inward_colour(canvas: &IndexedCanvas, p: IVec2, outward: Vec2) -> Option<u8> {
    let step = ivec2(-sign(outward.x), -sign(outward.y));
    if step == ivec2(0, 0) {
        return None;
    }
    let transparent = canvas.transparent();
    let opaque = |q: IVec2| canvas.get_at(q).is_some_and(|i| transparent != Some(i));
    let on_border = |q: IVec2| {
        [ivec2(1, 0), ivec2(-1, 0), ivec2(0, 1), ivec2(0, -1)]
            .iter()
            .any(|&d| !opaque(q + d))
    };
    // 斜めに 1 歩 → 縦横に 1 歩 の順で探す
    for d in [step, ivec2(step.x, 0), ivec2(0, step.y)] {
        if d == ivec2(0, 0) {
            continue;
        }
        let q = p + d;
        if opaque(q) && !on_border(q) {
            return canvas.get_at(q);
        }
    }
    None
}

/// **元の色を粗い格子へ丸める．**
///
/// 縁の色をその場の顔ぶれに依らせないための下ごしらえである (`resolve_all` の警告) ．
/// 刻みは «許容» の 4 倍 — これより細かいと，隣り合う面のわずかな色違いが
/// そのまま縁の色数になる．
fn quantise(base: Oklab, step: f32) -> Oklab {
    let snap = |v: f32| (v / step).round() * step;
    Oklab::new(snap(base.l), snap(base.a), snap(base.b))
}

/// 分類ごとの縁の色．
fn outline_colour(opts: &OutlineOptions, base: Oklab, outward: Vec2, l: Vec2) -> Oklab {
    let base = quantise(base, opts.tolerance * 4.0);
    let darken = |k: f32| Oklab {
        l: (base.l * k).max(0.02),
        a: base.a * k.max(0.5),
        b: base.b * k.max(0.5),
    };
    match opts.style {
        OutlineStyle::None => base,
        OutlineStyle::Black => Oklab::new(0.0, 0.0, 0.0),
        OutlineStyle::Tinted => darken(opts.darken),
        OutlineStyle::Contrast => {
            // **背景想定に対して明暗を逆に取る** (D36)．背景が無ければ暗い側にする
            let bright_background = opts
                .background
                .map(|c| oklab_of(c).l > base.l)
                .unwrap_or(true);
            if bright_background {
                darken(opts.darken)
            } else {
                Oklab {
                    l: (base.l + (1.0 - base.l) * (1.0 - opts.darken)).min(0.98),
                    a: base.a,
                    b: base.b,
                }
            }
        }
        OutlineStyle::Shaded => {
            // 外を向いている面が光を受けているほど明るい縁にする
            let lit = outward.normalize().map(|n| n.dot(l)).unwrap_or(0.0);
            let k = opts.darken + (1.0 - opts.darken) * 0.5 * lit.clamp(0.0, 1.0);
            darken(k)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Mask;

    /// 中央に四角がある絵 (縁に接しない)．
    fn square() -> (IndexedCanvas, Palette) {
        let palette = Palette::new(vec![
            Rgba8::TRANSPARENT,
            Rgba8::rgb(0xc0, 0x70, 0x40),
            Rgba8::rgb(0xe0, 0xa0, 0x60),
        ])
        .unwrap();
        let mut c = IndexedCanvas::filled(12, 12, 0).with_transparent(Some(0));
        for y in 3..9i32 {
            for x in 3..9i32 {
                c.set(x, y, if y < 6 { 2 } else { 1 });
            }
        }
        (c, palette)
    }

    fn silhouette(c: &IndexedCanvas) -> Mask {
        let mut m = Mask::new(c.width(), c.height());
        for p in c.bounds().iter() {
            if !c.is_transparent_at(p) {
                m.set(p, true);
            }
        }
        m
    }

    /// **既定は内側に描く — シルエットは 1 画素も動かない．**
    #[test]
    fn the_default_draws_inside_and_never_moves_the_silhouette() {
        let (mut c, mut palette) = square();
        let before = silhouette(&c);
        let report = outline(&mut c, &mut palette, &OutlineOptions::default()).unwrap();
        assert!(report.painted > 0, "1 画素も描いていない: {report:?}");
        assert_eq!(silhouette(&c), before, "シルエットが動いた");
    }

    /// 縁の画素がすべて縁の色になる (内側は触らない)．
    #[test]
    fn every_border_pixel_is_painted_and_the_inside_is_left_alone() {
        let (original, palette) = square();
        let (mut c, mut palette) = (original.clone(), palette);
        outline(&mut c, &mut palette, &OutlineOptions::default()).unwrap();
        for p in c.bounds().iter() {
            if c.is_transparent_at(p) {
                continue;
            }
            let border = [ivec2(1, 0), ivec2(-1, 0), ivec2(0, 1), ivec2(0, -1)]
                .iter()
                .any(|&d| original.get_at(p + d).is_none_or(|i| i == 0));
            if border {
                assert_ne!(c.get_at(p), original.get_at(p), "{p:?} が縁のままである");
            } else {
                assert_eq!(c.get_at(p), original.get_at(p), "{p:?} の内側を触った");
            }
        }
    }

    /// **掛け直しても色が沈まない** (冪等)．
    #[test]
    fn drawing_the_outline_twice_changes_nothing() {
        let (mut c, mut palette) = square();
        outline(&mut c, &mut palette, &OutlineOptions::default()).unwrap();
        let once = (c.clone(), palette.clone());
        let again = outline(&mut c, &mut palette, &OutlineOptions::default()).unwrap();
        assert_eq!(again.painted, 0, "2 度目で塗った: {again:?}");
        assert_eq!(c, once.0);
        assert_eq!(palette.len(), once.1.len(), "色が増えた");
    }

    /// **`none` は縁取りを剥がす** — 付けてから剥がすと元へ戻る．
    #[test]
    fn the_none_style_takes_the_outline_off_again() {
        let (original, palette) = square();
        let (mut c, mut palette) = (original.clone(), palette);
        outline(&mut c, &mut palette, &OutlineOptions::default()).unwrap();
        assert_ne!(c, original);

        let report = outline(
            &mut c,
            &mut palette,
            &OutlineOptions {
                style: OutlineStyle::None,
                ..OutlineOptions::default()
            },
        )
        .unwrap();
        assert!(report.removed > 0, "1 画素も剥がしていない");
        assert_eq!(c, original, "元へ戻らない");
    }

    /// **選択的輪郭線は光の当たる側を描かない** (D36)．
    #[test]
    fn the_selective_outline_leaves_the_lit_side_bare() {
        let (original, palette) = square();
        let opts = OutlineOptions {
            selective: true,
            // 光は右下へ進む → 左上が光の側
            light: LightSource::Directional {
                dir: Vec2 { x: 1.0, y: 1.0 },
            },
            ..OutlineOptions::default()
        };
        let (mut c, mut palette) = (original.clone(), palette);
        let report = outline(&mut c, &mut palette, &opts).unwrap();
        assert!(report.skipped_lit > 0, "光の側を飛ばしていない: {report:?}");
        assert!(report.painted > 0, "影の側も描いていない");

        // 左上の角は光の側 — 元のまま
        assert_eq!(c.get_at(ivec2(3, 3)), original.get_at(ivec2(3, 3)));
        // 右下の角は影の側 — 縁の色になる
        assert_ne!(c.get_at(ivec2(8, 8)), original.get_at(ivec2(8, 8)));
    }

    /// **背景想定で明暗が逆になる** (D36 の «背景想定を引数に取る»)．
    #[test]
    fn the_contrast_style_follows_the_assumed_background() {
        let (base, palette) = square();
        let run = |bg: Rgba8| {
            let (mut c, mut p) = (base.clone(), palette.clone());
            outline(
                &mut c,
                &mut p,
                &OutlineOptions {
                    style: OutlineStyle::Contrast,
                    background: Some(bg),
                    ..OutlineOptions::default()
                },
            )
            .unwrap();
            let i = c.get_at(ivec2(3, 3)).unwrap();
            oklab_of(p.get(i).unwrap()).l
        };
        let on_light = run(Rgba8::rgb(0xf0, 0xf0, 0xf0));
        let on_dark = run(Rgba8::rgb(0x10, 0x10, 0x14));
        assert!(
            on_light < on_dark,
            "明るい背景では暗い縁 ・暗い背景では明るい縁のはず ({on_light:.3} と {on_dark:.3})"
        );
    }

    /// **外側に描くとシルエットが太る．** 画像の外へは出ない．
    #[test]
    fn the_outer_outline_grows_the_silhouette_but_stays_in_the_canvas() {
        let (mut c, mut palette) = square();
        let before = silhouette(&c).count();
        let report = outline(
            &mut c,
            &mut palette,
            &OutlineOptions {
                outer: true,
                ..OutlineOptions::default()
            },
        )
        .unwrap();
        assert!(report.painted > 0);
        assert!(silhouette(&c).count() > before, "太っていない");
        assert_eq!(c.width(), 12, "画像の大きさが変わった");
    }

    /// **外に余地が無ければ数える** (画像いっぱいの絵は外側に描けない)．
    #[test]
    fn an_edge_to_edge_picture_reports_that_there_is_no_room_outside() {
        let palette = Palette::new(vec![Rgba8::TRANSPARENT, Rgba8::rgb(0xc0, 0x70, 0x40)]).unwrap();
        let mut c = IndexedCanvas::filled(8, 8, 1).with_transparent(Some(0));
        let mut palette = palette;
        let report = outline(
            &mut c,
            &mut palette,
            &OutlineOptions {
                outer: true,
                ..OutlineOptions::default()
            },
        )
        .unwrap();
        assert!(report.no_room > 0, "外に描けないことを報告していない");
    }

    /// 5 分類はすべて名前を持つ (CLI の値と 1 対 1)．
    #[test]
    fn every_style_has_a_name() {
        for (style, name) in [
            (OutlineStyle::None, "none"),
            (OutlineStyle::Black, "black"),
            (OutlineStyle::Tinted, "tinted"),
            (OutlineStyle::Contrast, "contrast"),
            (OutlineStyle::Shaded, "shaded"),
        ] {
            assert_eq!(style.as_str(), name);
        }
    }
}
