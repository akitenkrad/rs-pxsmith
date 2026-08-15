//! パーツ合成 (`pxsmith compose`．設計書 5 章 ・4.2 ・D42)．
//!
//! 設計書が決めているのは «アンカー付きパーツ合成 ・variants 展開 ・`--part-delay`»
//! の 3 行だけである．**置き方は実測で決めた** (`pxsmith-calib compose`)．
//!
//! # 画布は広げる — 切らない
//!
//! CC0 の実物 (Dungeon Crawl の 32 枚) を測ると，**28 枚 (87.5%) のシルエットが
//! 画布の縁に接しており，四辺すべてに 1 画素以上の余白がある絵は 4 枚しかない**．
//! アンカーで 1 画素でも動かせば，動かした側は必ず切れる．**合成した画布は
//! パーツの矩形の和にする．**
//!
//! [`crate::outline`] (D84) が «内側に描く» を選んだのと**逆の答え**になる．
//! 縁取りは «同じ絵を書き換える» 道具なので，シルエットが動くと `pxsmith conform` が
//! 戻した格子やタイルの継ぎ目が壊れる．合成は «新しい絵を組む» 道具なので，
//! 画布は結果であって入力ではない．**同じ測定でも道具の役目が違えば答えが変わる．**
//!
//! 既に揃っているパーツ (Crawl のドール素材のように全部が同じ画布で原点合わせ) は
//! ずれが 0 なので**画布が 1 画素も動かない**．広げるのは動かしたときだけである．
//!
//! # パレットは «使っている色» を集めて束ねる
//!
//! 実素材で胴体 12 枚 x 装備 3 枚を併合すると **色数は中央 29 ・最大 66**，
//! **36 組中 2 組が L0 の 62 色を超える**が，256 色は 1 組も超えない．
//! 装備の色のうち胴体と共有しているものは中央で 14% しかないので，
//! **添字はそのままでは通らない — 併合して付け替える**．
//!
//! # `--part-delay` の «埋め方» は校正の対象ではない
//!
//! 遅らせたぶん先頭に隙間ができる．そこを埋める方法は 2 つある．
//!
//! | 埋め方 | 何を仮定するか |
//! | --- | --- |
//! | [`DelayMode::Hold`] (既定) | 何も仮定しない．**著者が描いていないフレームを作らない** |
//! | [`DelayMode::Wrap`] | **列がループであること**．ループでなければ末尾の絵が頭に来る |
//!
//! D92 (`pxsmith validate`) と同じで，**根拠は出典であって統計ではない**．
//! ループかどうかは絵から決まらない (推定して当てはめるのは同語反復である) ので，
//! **仮定の少ない方を既定にして，ループだと分かっている側が明示する**．

use std::collections::BTreeMap;

use crate::canvas::IndexedCanvas;
use crate::color::Rgba8;
use crate::error::{CoreError, Result};
use crate::frame::{Frame, Layer, LayerMeta, Surface};
use crate::math::{IRect, IVec2, UVec2, ivec2};
use crate::palette::Palette;

/// パーツをどのアンカーで合わせるか．
///
/// **解決先は先頭のパーツだけである．** 腕 → 前腕 → 手のような連鎖は
/// `compose` の反復で解く (設計書 6.9 «関節運動は `Parts` により `pxsmith compose` の
/// 反復として解く») ．途中のパーツへ次々と合わせる形にすると，**結果が宣言順に
/// 依存する**うえ，どのパーツが基準なのかが読めなくなる．
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alignment {
    /// このパーツ側のアンカー名．
    pub part: String,
    /// 先頭のパーツ側のアンカー名．
    pub base: String,
}

/// 合成に渡す 1 パーツ．
#[derive(Clone, Debug)]
pub struct Part {
    /// レイヤ名の接頭辞になる．`--part-delay` の鍵でもある (設計書 4.2)．
    pub name: String,
    /// フレーム列．**すべて同じ大きさでなければならない**．
    pub frames: Vec<Frame>,
    pub anchors: BTreeMap<String, IVec2>,
    /// `None` なら原点合わせ．
    pub align: Option<Alignment>,
    /// オーバーラップ / フォロースルーの遅延フレーム数 (D42)．
    pub delay: u32,
}

impl Part {
    /// 原点合わせ ・遅延なしのパーツ．
    pub fn new(name: impl Into<String>, frames: Vec<Frame>) -> Self {
        Self {
            name: name.into(),
            frames,
            anchors: BTreeMap::new(),
            align: None,
            delay: 0,
        }
    }
}

/// 遅延したぶんと，パーツが短いぶんを，どのフレームで埋めるか．
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum DelayMode {
    /// 端のフレームを保つ．**既定** — 著者が描いていない並びを作らない．
    #[default]
    Hold,
    /// 反対側の端から回す．**列がループのときだけ正しい**．
    Wrap,
}

impl DelayMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hold => "hold",
            Self::Wrap => "wrap",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "hold" => Some(Self::Hold),
            "wrap" => Some(Self::Wrap),
            _ => None,
        }
    }

    /// 合成の第 `i` フレームで，このパーツの何番目を引くか．
    fn pick(self, i: usize, delay: u32, len: usize) -> usize {
        debug_assert!(len > 0);
        let shifted = i as i64 - delay as i64;
        match self {
            Self::Hold => shifted.clamp(0, len as i64 - 1) as usize,
            Self::Wrap => shifted.rem_euclid(len as i64) as usize,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ComposeOptions {
    pub delay_mode: DelayMode,
    /// 真なら先頭のパーツの画布に切り揃える．**既定は偽 (広げる)** ．
    ///
    /// 実素材の 87.5% がシルエットを画布の縁まで伸ばしているので，切ると絵が欠ける．
    /// 出力先が «全フレーム同じ大きさ» を要求する場面のために残してある．
    pub clip: bool,
}

/// パーツ 1 つの置かれ方．
#[derive(Clone, Debug)]
pub struct Placement {
    pub name: String,
    /// 合成後の座標系での左上．
    pub offset: IVec2,
    /// 上に載ったパーツに隠された画素数 (全フレームの合計)．
    ///
    /// パーツがまるごと隠れているなら重ね順を間違えている見込みが高い．
    pub covered: usize,
    /// 画布の外へ出て捨てた画素数 (全フレームの合計)．`clip` が偽なら必ず 0．
    pub clipped: usize,
}

#[derive(Clone, Debug)]
pub struct ComposeReport {
    pub canvas: UVec2,
    /// 先頭のパーツの原点が合成後のどこに来たか．**負にはならない**．
    pub origin: IVec2,
    pub placements: Vec<Placement>,
    pub frames: usize,
    /// 併合したパレットの色数．
    pub colors: usize,
    /// 画布が先頭のパーツより広がったか．
    pub grew: bool,
}

impl ComposeReport {
    pub fn clipped(&self) -> usize {
        self.placements.iter().map(|p| p.clipped).sum()
    }
}

/// パーツを合成する．
///
/// 各パーツは合成後のフレームで**自分のレイヤ群**になる (平坦化しない) ．
/// レイヤ名は `パーツ名/元のレイヤ名` である．
///
/// `duration_ms` と `kind` は**先頭のパーツから取る** — 合成物の時間軸は 1 本で，
/// その権威をどこに置くかを決める必要がある．遅らせたパーツの `duration_ms` を
/// 混ぜると，同じ絵が違う長さで 2 度出ることになる．
pub fn compose(parts: &[Part], opts: &ComposeOptions) -> Result<(Vec<Frame>, ComposeReport)> {
    let base = parts.first().ok_or(CoreError::ComposeNoParts)?;
    for p in parts {
        if p.frames.is_empty() {
            return Err(CoreError::ComposeEmptyPart {
                part: p.name.clone(),
            });
        }
        let size = p.frames[0].size;
        if let Some(f) = p.frames.iter().find(|f| f.size != size) {
            return Err(CoreError::ComposePartSizeVaries {
                part: p.name.clone(),
                first: (size.x, size.y),
                other: (f.size.x, f.size.y),
            });
        }
    }

    let offsets = resolve_offsets(parts)?;
    let (canvas_rect, origin) = canvas_of(parts, &offsets, base, opts.clip);

    let (palette, transparent, maps) = merge_palettes(parts)?;

    let length = parts.iter().map(|p| p.frames.len()).max().unwrap_or(0);
    let mut placements: Vec<Placement> = parts
        .iter()
        .zip(&offsets)
        .map(|(p, o)| Placement {
            name: p.name.clone(),
            offset: *o + origin,
            covered: 0,
            clipped: 0,
        })
        .collect();

    let mut out: Vec<Frame> = Vec::with_capacity(length);
    for i in 0..length {
        let mut frame = Frame::new(
            UVec2 {
                x: canvas_rect.w,
                y: canvas_rect.h,
            },
            palette.clone(),
        );
        // 時間軸の権威は先頭のパーツにある
        let lead = &base.frames[opts.delay_mode.pick(i, base.delay, base.frames.len())];
        frame.duration_ms = lead.duration_ms;
        frame.kind = lead.kind;

        for (k, part) in parts.iter().enumerate() {
            let pick = opts.delay_mode.pick(i, part.delay, part.frames.len());
            let src = &part.frames[pick];
            let at = offsets[k] + origin;
            for layer in &src.layers {
                let Some(indexed) = layer.surface.as_indexed() else {
                    return Err(CoreError::NotIndexed {
                        name: format!("{}/{}", part.name, layer.meta.name),
                    });
                };
                let mut moved = IndexedCanvas::filled(canvas_rect.w, canvas_rect.h, transparent)
                    .with_transparent(Some(transparent));
                let mut remapped = indexed.clone();
                remapped.remap(&maps[k])?;
                remapped.set_transparent(Some(transparent));
                let clipped = outside_count(&remapped, at, canvas_rect);
                placements[k].clipped += clipped;
                moved.blit(&remapped, at, false);
                let meta = LayerMeta {
                    name: format!("{}/{}", part.name, layer.meta.name),
                    ..layer.meta.clone()
                };
                frame.layers.push(Layer::new(meta, Surface::Indexed(moved)));
            }
        }

        // 被覆は «自分より上のパーツが同じ画素を塗ったか» で数える
        count_covered(&frame, parts, transparent, &mut placements);

        out.push(frame);
    }

    let colors = palette.len();
    Ok((
        out,
        ComposeReport {
            canvas: UVec2 {
                x: canvas_rect.w,
                y: canvas_rect.h,
            },
            origin,
            placements,
            frames: length,
            colors,
            grew: canvas_rect.w != base.frames[0].size.x || canvas_rect.h != base.frames[0].size.y,
        },
    ))
}

/// アンカーからパーツごとのずれを出す．**先頭のパーツを原点 (0,0) とする**．
fn resolve_offsets(parts: &[Part]) -> Result<Vec<IVec2>> {
    let base = &parts[0];
    let mut out = Vec::with_capacity(parts.len());
    for (k, part) in parts.iter().enumerate() {
        if k == 0 {
            out.push(ivec2(0, 0));
            continue;
        }
        let Some(align) = &part.align else {
            out.push(ivec2(0, 0));
            continue;
        };
        let here =
            *part
                .anchors
                .get(&align.part)
                .ok_or_else(|| CoreError::ComposeAnchorMissing {
                    part: part.name.clone(),
                    anchor: align.part.clone(),
                })?;
        let there =
            *base
                .anchors
                .get(&align.base)
                .ok_or_else(|| CoreError::ComposeAnchorMissing {
                    part: base.name.clone(),
                    anchor: align.base.clone(),
                })?;
        out.push(there - here);
    }
    Ok(out)
}

/// 合成後の画布と，先頭のパーツの原点が来る位置．
///
/// **矩形の和で決める — 中身 (不透明な画素) の外接矩形では決めない．** 中身で
/// 決めると，たまたま空のフレームがあるだけで画布が変わり，フレームごとに
/// 大きさが揺れる．矩形なら中身に依らないので，同じパーツ構成なら必ず同じ画布になる．
fn canvas_of(parts: &[Part], offsets: &[IVec2], base: &Part, clip: bool) -> (IRect, IVec2) {
    let base_rect = IRect::new(0, 0, base.frames[0].size.x, base.frames[0].size.y);
    if clip {
        return (base_rect, ivec2(0, 0));
    }
    let mut rect = base_rect;
    for (part, off) in parts.iter().zip(offsets) {
        let size = part.frames[0].size;
        rect = rect.union(IRect::new(off.x, off.y, size.x, size.y));
    }
    // 画布の左上を原点に取り直す
    (IRect::new(0, 0, rect.w, rect.h), ivec2(-rect.x, -rect.y))
}

/// 画布の外へ出る画素数．
fn outside_count(src: &IndexedCanvas, at: IVec2, canvas: IRect) -> usize {
    let mut n = 0usize;
    for p in src.bounds().iter() {
        if src.is_transparent_at(p) {
            continue;
        }
        let q = p + at;
        if q.x < 0 || q.y < 0 || q.x >= canvas.w as i32 || q.y >= canvas.h as i32 {
            n += 1;
        }
    }
    n
}

/// 全パーツの «使っている色» を 1 つのパレットへ束ね，パーツごとの付け替え表を作る．
///
/// 返り値の 3 つ目は `maps[パーツ]` で，元の添字から併合後の添字への表である．
///
/// > [!warning] **写すのは «使っている添字» だけである．**
/// > 併合は使っている色しか集めない (そうしないと素材の未使用色で 256 色を
/// > 使い切る) ので，パレットの全項目を写そうとすると**未使用の色が «併合先に
/// > 無い» と言って落ちる**．使っていない添字は透明へ倒す — どの画素も引かない．
fn merge_palettes(parts: &[Part]) -> Result<(Palette, u8, Vec<Vec<u8>>)> {
    let mut sources: Vec<(&IndexedCanvas, &Palette)> = Vec::new();
    for part in parts {
        for frame in &part.frames {
            for layer in &frame.layers {
                if let Some(c) = layer.surface.as_indexed() {
                    sources.push((c, &frame.palette));
                }
            }
        }
    }
    let mut palette = Palette::extract_from(sources.iter().copied())?;

    // 広げた余白と，パーツの隙間を埋めるために**必ず透明が要る**
    let transparent = match palette.entries().iter().position(|c| c.a == 0) {
        Some(i) => i as u8,
        None => palette.push(Rgba8::TRANSPARENT)?,
    };

    let maps = parts
        .iter()
        .map(|part| build_map(part, &palette, transparent))
        .collect::<Result<Vec<_>>>()?;

    Ok((palette, transparent, maps))
}

/// パーツの添字を併合後の添字へ写す表．
fn build_map(part: &Part, to: &Palette, transparent: u8) -> Result<Vec<u8>> {
    let mut used = [false; 256];
    for frame in &part.frames {
        for layer in &frame.layers {
            if let Some(c) = layer.surface.as_indexed() {
                for v in c.pixels() {
                    used[*v as usize] = true;
                }
            }
        }
    }

    let mut map = vec![transparent; 256];
    for frame in &part.frames {
        for (i, u) in used.iter().enumerate() {
            if !u {
                continue;
            }
            let color = frame.palette.get(i as u8).ok_or_else(|| {
                // 画素が指しているのにパレットに無い添字．**黙って透明にしない** —
                // 元の絵が既に壊れており，合成で消すと «合成が消した» ように見える
                CoreError::ComposeIndexOutOfPalette {
                    part: part.name.clone(),
                    index: i as u8,
                    len: frame.palette.len(),
                }
            })?;
            if color.a == 0 {
                map[i] = transparent;
                continue;
            }
            let found = to
                .entries()
                .iter()
                .position(|d| *d == color)
                .ok_or(CoreError::ComposeColorLost { color })?;
            map[i] = found as u8;
        }
    }
    Ok(map)
}

/// パーツごとに «上のパーツに隠された画素» を数える．
fn count_covered(frame: &Frame, parts: &[Part], transparent: u8, placements: &mut [Placement]) {
    // レイヤは «パーツの順 x そのパーツのレイヤ順» で積んである
    let mut layer_of_part: Vec<(usize, usize)> = Vec::new();
    let mut cursor = 0usize;
    for part in parts {
        let n = part.frames[0].layers.len();
        layer_of_part.push((cursor, n));
        cursor += n;
    }
    for (k, (start, n)) in layer_of_part.iter().enumerate() {
        let above = start + n;
        for li in *start..above {
            let Some(mine) = frame.layers[li].surface.as_indexed() else {
                continue;
            };
            for p in mine.bounds().iter() {
                if mine.get_at(p) == Some(transparent) {
                    continue;
                }
                let hidden = frame.layers[above..].iter().any(|l| {
                    l.surface
                        .as_indexed()
                        .is_some_and(|c| c.get_at(p).is_some_and(|v| v != transparent))
                });
                if hidden {
                    placements[k].covered += 1;
                }
            }
        }
    }
}

/// `${name}` を差し替える (variants 展開．設計書 4.2 の `for_each`)．
///
/// レシピの式評価器 (M5) はここには置かない — **展開できるのは変数参照だけ**である．
pub fn expand_template(template: &str, vars: &BTreeMap<String, String>) -> Result<String> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(at) = rest.find("${") {
        out.push_str(&rest[..at]);
        let tail = &rest[at + 2..];
        let end = tail
            .find('}')
            .ok_or_else(|| CoreError::ComposeBadTemplate {
                template: template.to_string(),
            })?;
        let name = &tail[..end];
        let value = vars.get(name).ok_or_else(|| CoreError::ComposeUnknownVar {
            name: name.to_string(),
        })?;
        out.push_str(value);
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// 変数の直積を宣言順に並べる (設計書 4.2 の `for_each`)．
///
/// **並びは決定論的である** — 最初の変数が最も外側で回る (設計書 6.15 規則 1)．
pub fn expand_variants(vars: &[(String, Vec<String>)]) -> Vec<BTreeMap<String, String>> {
    let mut out = vec![BTreeMap::new()];
    for (name, values) in vars {
        let mut next = Vec::with_capacity(out.len() * values.len());
        for base in &out {
            for v in values {
                let mut m = base.clone();
                m.insert(name.clone(), v.clone());
                next.push(m);
            }
        }
        out = next;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette_of(colors: &[Rgba8]) -> Palette {
        let mut entries = vec![Rgba8::TRANSPARENT];
        entries.extend_from_slice(colors);
        Palette::new(entries).expect("色は 256 以内")
    }

    /// 1 レイヤ ・1 フレームのパーツを作る．`data` は添字の並び (0 が透明)．
    fn part(name: &str, w: u32, h: u32, colors: &[Rgba8], data: &[u8]) -> Part {
        let palette = palette_of(colors);
        let mut frame = Frame::new(UVec2 { x: w, y: h }, palette);
        let canvas = IndexedCanvas::from_pixels(w, h, data.to_vec())
            .expect("画素数が合っている")
            .with_transparent(Some(0));
        frame.layers.push(Layer::new(
            LayerMeta::named("main"),
            Surface::Indexed(canvas),
        ));
        Part::new(name, vec![frame])
    }

    fn red() -> Rgba8 {
        Rgba8::rgb(0xff, 0, 0)
    }

    fn blue() -> Rgba8 {
        Rgba8::rgb(0, 0, 0xff)
    }

    /// 壊れると: 既に揃っているパーツを重ねただけで画布が動く．
    /// Crawl のドール素材はすべて同じ画布の原点合わせなので，ここが動くと
    /// タイルの継ぎ目がずれる．
    #[test]
    fn origin_aligned_parts_do_not_move_the_canvas() {
        let a = part("body", 2, 2, &[red()], &[1, 1, 1, 1]);
        let b = part("cap", 2, 2, &[blue()], &[0, 1, 0, 0]);
        let (frames, report) = compose(&[a, b], &ComposeOptions::default()).expect("合成できる");
        assert_eq!(report.canvas, UVec2 { x: 2, y: 2 });
        assert!(!report.grew);
        assert_eq!(report.origin, ivec2(0, 0));
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].layers.len(), 2);
    }

    /// 壊れると: アンカーでずらしたパーツが画布の外で切れる．
    /// 実素材の 87.5% は縁に接しているので，切る実装では必ず絵が欠ける．
    #[test]
    fn an_anchored_part_grows_the_canvas_instead_of_being_clipped() {
        let mut a = part("body", 4, 4, &[red()], &[1; 16]);
        a.anchors.insert("neck".into(), ivec2(1, 0));
        let mut b = part("cap", 4, 4, &[blue()], &[1; 16]);
        b.anchors.insert("neck".into(), ivec2(3, 3));
        b.align = Some(Alignment {
            part: "neck".into(),
            base: "neck".into(),
        });

        let (frames, report) = compose(&[a, b], &ComposeOptions::default()).expect("合成できる");
        // ずれは (1,0) - (3,3) = (-2,-3)．左と上へ出るので画布は 6x7 になる
        assert_eq!(report.canvas, UVec2 { x: 6, y: 7 });
        assert!(report.grew);
        assert_eq!(report.origin, ivec2(2, 3));
        assert_eq!(report.clipped(), 0);
        // 帽子の 16 画素がすべて残っている
        let cap = frames[0].layers[1]
            .surface
            .as_indexed()
            .expect("インデックスカラー");
        let palette = &frames[0].palette;
        let kept = cap
            .pixels()
            .iter()
            .filter(|v| palette.get(**v).is_some_and(|c| c == blue()))
            .count();
        assert_eq!(kept, 16);
    }

    /// 壊れると: `--clip` で外へ出た画素が黙って消える．
    /// 「切った」と報告できなければ，欠けた絵が黙って出荷される．
    #[test]
    fn clipping_reports_how_many_pixels_it_threw_away() {
        let mut a = part("body", 4, 4, &[red()], &[1; 16]);
        a.anchors.insert("neck".into(), ivec2(0, 0));
        let mut b = part("cap", 4, 4, &[blue()], &[1; 16]);
        b.anchors.insert("neck".into(), ivec2(2, 0));
        b.align = Some(Alignment {
            part: "neck".into(),
            base: "neck".into(),
        });

        let opts = ComposeOptions {
            clip: true,
            ..ComposeOptions::default()
        };
        let (_, report) = compose(&[a, b], &opts).expect("合成できる");
        assert_eq!(report.canvas, UVec2 { x: 4, y: 4 });
        // 左へ 2 画素ずらすので 4 行 x 2 列が外へ出る
        assert_eq!(report.clipped(), 8);
    }

    /// 壊れると: パーツごとに色が別のパレットのままになり，添字が混ざる．
    #[test]
    fn palettes_are_merged_and_indices_are_remapped() {
        let a = part("body", 2, 1, &[red()], &[1, 1]);
        let b = part("cap", 2, 1, &[blue()], &[0, 1]);
        let (frames, report) = compose(&[a, b], &ComposeOptions::default()).expect("合成できる");
        // 透明 ・赤 ・青 の 3 色
        assert_eq!(report.colors, 3);
        let palette = &frames[0].palette;
        assert!(palette.entries().contains(&red()));
        assert!(palette.entries().contains(&blue()));
        // 帽子の «添字 1» は青のままでなければならない (赤に化けない)
        let cap = frames[0].layers[1].surface.as_indexed().expect("添字");
        let at = cap.get(1, 0).expect("範囲内");
        assert_eq!(palette.get(at), Some(blue()));
    }

    /// 壊れると: 遅らせたパーツが «著者が描いていない» 並びを作る．
    #[test]
    fn hold_repeats_the_first_frame_and_wrap_takes_it_from_the_end() {
        assert_eq!(DelayMode::Hold.pick(0, 2, 4), 0);
        assert_eq!(DelayMode::Hold.pick(1, 2, 4), 0);
        assert_eq!(DelayMode::Hold.pick(2, 2, 4), 0);
        assert_eq!(DelayMode::Hold.pick(3, 2, 4), 1);
        assert_eq!(DelayMode::Wrap.pick(0, 2, 4), 2);
        assert_eq!(DelayMode::Wrap.pick(1, 2, 4), 3);
        assert_eq!(DelayMode::Wrap.pick(2, 2, 4), 0);
        // 短いパーツも同じ規則で引く (遅延 0)
        assert_eq!(DelayMode::Hold.pick(5, 0, 2), 1);
        assert_eq!(DelayMode::Wrap.pick(5, 0, 2), 1);
        assert_eq!(DelayMode::Wrap.pick(4, 0, 2), 0);
    }

    /// 壊れると: 重ね順を間違えたパーツが «見えていない» まま出荷される．
    #[test]
    fn a_fully_hidden_part_is_reported_as_covered() {
        let a = part("body", 2, 1, &[red()], &[1, 1]);
        let b = part("cape", 2, 1, &[blue()], &[1, 1]);
        // cape が下 ・body が上
        let (_, report) = compose(&[b, a], &ComposeOptions::default()).expect("合成できる");
        assert_eq!(report.placements[0].covered, 2);
        assert_eq!(report.placements[1].covered, 0);
    }

    /// 壊れると: アンカー名の打ち間違いが «原点合わせ» として黙って通る．
    #[test]
    fn a_missing_anchor_is_an_error_not_a_silent_origin_align() {
        let a = part("body", 2, 2, &[red()], &[1; 4]);
        let mut b = part("cap", 2, 2, &[blue()], &[1; 4]);
        b.align = Some(Alignment {
            part: "neck".into(),
            base: "neck".into(),
        });
        let err = compose(&[a, b], &ComposeOptions::default()).expect_err("アンカーが無い");
        assert!(matches!(err, CoreError::ComposeAnchorMissing { .. }));
    }

    /// 壊れると: 使っていない色まで写そうとして，**併合が «色が無い» と言って落ちる**．
    /// 実素材のパレットには使っていない色が普通に入っている．
    #[test]
    fn unused_palette_entries_do_not_break_the_merge() {
        let green = Rgba8::rgb(0, 0xff, 0);
        // 緑を持っているが 1 画素も使っていない
        let a = part("body", 2, 1, &[red(), green], &[1, 1]);
        let b = part("cap", 2, 1, &[blue()], &[0, 1]);
        let (frames, report) = compose(&[a, b], &ComposeOptions::default()).expect("合成できる");
        assert_eq!(report.colors, 3);
        assert!(!frames[0].palette.entries().contains(&green));
    }

    /// 壊れると: 元の絵の壊れ (パレットに無い添字) が合成で黙って透明になり，
    /// **合成が消したように見える**．
    #[test]
    fn an_index_outside_the_palette_is_an_error() {
        let a = part("body", 2, 1, &[red()], &[1, 7]);
        let err = compose(&[a], &ComposeOptions::default()).expect_err("添字 7 はパレットの外");
        assert!(matches!(err, CoreError::ComposeIndexOutOfPalette { .. }));
    }

    /// 壊れると: variants の並びが実行ごとに変わり，差分ビルドの鍵が揺れる
    /// (設計書 6.15 規則 1)．
    #[test]
    fn variants_expand_in_declaration_order() {
        let vars = vec![
            ("equip".to_string(), vec!["sword".into(), "axe".into()]),
            ("dir".to_string(), vec!["n".into(), "s".into()]),
        ];
        let all = expand_variants(&vars);
        let names: Vec<String> = all
            .iter()
            .map(|v| expand_template("${equip}_${dir}", v).expect("展開できる"))
            .collect();
        assert_eq!(names, ["sword_n", "sword_s", "axe_n", "axe_s"]);
    }

    /// 壊れると: 綴じていない `${` が «そのままの文字列» としてファイル名になる．
    #[test]
    fn an_unclosed_template_is_an_error() {
        let vars = BTreeMap::new();
        assert!(expand_template("hero_${equip", &vars).is_err());
    }
}
