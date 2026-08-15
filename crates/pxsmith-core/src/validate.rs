//! 実機制約検証 (`pxsmith validate`．設計書 5 章)．
//!
//! 設計書が決めているのは «実機制約検証» と «制約違反時に非ゼロ終了コード» の 2 つ
//! だけである．中身はここで決めた．
//!
//! # lint と何が違うか
//!
//! | | 見るもの | 判断の性質 |
//! | --- | --- | --- |
//! | `pxsmith lint` | **絵として良いか** | 主観が混ざる．閾値は素材から校正する |
//! | `pxsmith validate` | **その出力先に載るか** | **数え上げで決まる**．閾値は出力先の仕様である |
//!
//! この違いが実装にも出る — lint の閾値は正例 ・負例で校正したが，**ここの数値は
//! 校正の対象ではない**．「タイルあたり 4 色」は測って決めるものではなく，
//! 出力先がそう決めているというだけである．したがって**根拠は出典であって統計では
//! ない**．出典の無い数値は置かない．
//!
//! # 何を検査しないか
//!
//! **こちらのデータモデルで数えられないものは検査しない．**
//!
//! | 検査しない制約 | 理由 |
//! | --- | --- |
//! | 走査線あたりのスプライト数 | 画面の配置が要る (こちらは 1 枚の絵しか持たない) |
//! | パレット本数の割り当て | どのタイルにどのパレットを割り当てるかは出力側の仕事 |
//! | VRAM のタイル本数 | 重複除去の結果に依る (`pxsmith tileset extract` の領分) |
//!
//! **«検査していない» ことは黙らせない** — [`Report::unchecked`] に並べて報告する．

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::canvas::IndexedCanvas;
use crate::frame::{Frame, Surface};
use crate::math::IRect;

/// 出力先の制約 (プロファイル)．
///
/// `None` の項目は «制約が無い» ではなく **«この出力先では検査しない»** を意味する．
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Target {
    pub name: String,
    /// パレット全体の色数の上限 (透明を含む)．
    pub max_colors: Option<usize>,
    /// **1 タイルの中に同時に現れてよい色数** (透明を含む)．
    ///
    /// 出力先のビット深度がそのまま出る項目である (2bpp なら 4，4bpp なら 16) ．
    pub max_colors_per_tile: Option<usize>,
    /// タイルの一辺．画像の寸法がこの倍数でなければ違反．
    pub tile_size: Option<u32>,
    /// 画像の寸法の上限 (幅, 高さ)．
    pub max_size: Option<(u32, u32)>,
    /// **透明はパレットの先頭 (添字 0) であること．**
    pub transparent_first: bool,
    /// **表示周期 (ミリ秒)．** フレームの表示時間がこの倍数でなければ違反．
    ///
    /// 実機は垂直同期の整数倍でしか絵を切り替えられない (設計書 6.11 の D40) ．
    pub frame_ms: Option<f32>,
}

impl Default for Target {
    fn default() -> Self {
        Self {
            name: "custom".to_string(),
            max_colors: None,
            max_colors_per_tile: None,
            tile_size: None,
            max_size: None,
            transparent_first: false,
            frame_ms: None,
        }
    }
}

/// 表示周期の «倍数» とみなす許容 (ミリ秒)．
///
/// `duration_ms` は整数ミリ秒なので，60 Hz (16.666… ms) の倍数はぴったりにはならない．
/// 33 ms は 2 フレーム (33.33 ms) の丸めであって違反ではない．
const FRAME_MS_TOLERANCE: f32 = 1.0;

impl Target {
    /// 名前から組み込みのプロファイルを引く．
    ///
    /// **数値はどれも出力先の公開仕様から取ったもので，こちらで校正した値は 1 つも
    /// 無い．** タイルの一辺とビット深度 (＝タイルあたりの色数) ・画面の寸法だけを
    /// 入れてある — 走査線あたりのスプライト数のように**こちらで数えられないもの**は
    /// 入れない (モジュールの説明を読むこと) ．
    pub fn builtin(name: &str) -> Option<Self> {
        let t = |name: &str| Target {
            name: name.to_string(),
            ..Target::default()
        };
        Some(match name {
            // 2bpp ・8x8 タイル ・画面 160x144．4 階調
            "gb" => Target {
                max_colors: Some(4),
                max_colors_per_tile: Some(4),
                tile_size: Some(8),
                max_size: Some((160, 144)),
                transparent_first: true,
                frame_ms: Some(1000.0 / 59.73),
                ..t("gb")
            },
            // 2bpp ・8x8 タイル ・画面 256x240
            "nes" => Target {
                max_colors_per_tile: Some(4),
                tile_size: Some(8),
                max_size: Some((256, 240)),
                transparent_first: true,
                frame_ms: Some(1000.0 / 60.0988),
                ..t("nes")
            },
            // 4bpp ・8x8 タイル ・画面 256x224
            "snes" => Target {
                max_colors_per_tile: Some(16),
                tile_size: Some(8),
                max_size: Some((256, 224)),
                transparent_first: true,
                frame_ms: Some(1000.0 / 60.0988),
                ..t("snes")
            },
            // 4bpp ・8x8 タイル ・画面 240x160
            "gba" => Target {
                max_colors_per_tile: Some(16),
                tile_size: Some(8),
                max_size: Some((240, 160)),
                transparent_first: true,
                frame_ms: Some(1000.0 / 59.7275),
                ..t("gba")
            },
            // 固定 16 色 ・8x8 スプライト ・画面 128x128 ・30 fps
            "pico8" => Target {
                max_colors: Some(16),
                max_colors_per_tile: Some(16),
                tile_size: Some(8),
                max_size: Some((128, 128)),
                transparent_first: true,
                frame_ms: Some(1000.0 / 30.0),
                ..t("pico8")
            },
            _ => return None,
        })
    }

    /// 組み込みのプロファイル名．
    pub const BUILTIN: &'static [&'static str] = &["gb", "nes", "snes", "gba", "pico8"];

    /// **こちらで数えられない制約**．報告に添える (黙って通したことにしない)．
    pub fn unchecked(&self) -> Vec<&'static str> {
        let mut out = vec![
            "走査線あたりのスプライト数 (画面の配置が要る)",
            "パレット本数の割り当て (出力側の仕事)",
        ];
        if self.tile_size.is_some() {
            out.push("VRAM のタイル本数 (重複除去の結果に依る)");
        }
        out
    }
}

/// 違反 1 件．
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Violation {
    /// 制約の名前 (`colors` / `colors-per-tile` / `tile-size` / `size` /
    /// `transparent-index` / `frame-ms`)．
    pub constraint: String,
    pub message: String,
    /// フレームの番号．
    pub frame: usize,
    /// 違反の場所 (タイルなど)．
    #[serde(skip_serializing_if = "Option::is_none")]
    pub area: Option<[i32; 4]>,
}

/// 検証の結果．
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub target: String,
    pub violations: Vec<Violation>,
    /// **検査していない制約**．通ったことと «見ていない» ことを混ぜない．
    pub unchecked: Vec<String>,
}

impl Report {
    pub fn is_ok(&self) -> bool {
        self.violations.is_empty()
    }
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.violations.is_empty() {
            writeln!(f, "{} の制約をすべて満たしている", self.target)?;
        }
        for v in &self.violations {
            write!(f, "[{}] フレーム {}: {}", v.constraint, v.frame, v.message)?;
            if let Some([x, y, w, h]) = v.area {
                write!(f, " ({x}, {y}, {w}x{h})")?;
            }
            writeln!(f)?;
        }
        if !self.unchecked.is_empty() {
            writeln!(f, "\n検査していない制約 (こちらでは数えられない):")?;
            for u in &self.unchecked {
                writeln!(f, "  - {u}")?;
            }
        }
        Ok(())
    }
}

/// **フレーム列を出力先の制約に照らす** (設計書 5 章)．
pub fn validate_frames(frames: &[Frame], target: &Target) -> Report {
    let mut report = Report {
        target: target.name.clone(),
        unchecked: target
            .unchecked()
            .into_iter()
            .map(|s| s.to_string())
            .collect(),
        ..Report::default()
    };

    for (i, frame) in frames.iter().enumerate() {
        check_palette(frame, target, i, &mut report);
        check_size(frame, target, i, &mut report);
        check_timing(frame, target, i, &mut report);
        for layer in &frame.layers {
            if let Surface::Indexed(canvas) = &layer.surface {
                check_tiles(canvas, target, i, &mut report);
            }
        }
    }
    report
}

fn check_palette(frame: &Frame, target: &Target, at: usize, report: &mut Report) {
    if let Some(max) = target.max_colors
        && frame.palette.len() > max
    {
        report.violations.push(Violation {
            constraint: "colors".to_string(),
            message: format!("パレットが {} 色 (上限 {max} 色)", frame.palette.len()),
            frame: at,
            area: None,
        });
    }
    // 透明は先頭 — 出力先が «添字 0 は透明» を前提にしている場合
    if target.transparent_first
        && let Some(i) = frame.palette.entries().iter().position(|c| c.a == 0)
        && i != 0
    {
        report.violations.push(Violation {
            constraint: "transparent-index".to_string(),
            message: format!("透明色が添字 {i} にある (添字 0 でなければならない)"),
            frame: at,
            area: None,
        });
    }
}

fn check_size(frame: &Frame, target: &Target, at: usize, report: &mut Report) {
    let (w, h) = (frame.size.x, frame.size.y);
    if let Some(tile) = target.tile_size
        && tile > 0
        && (w % tile != 0 || h % tile != 0)
    {
        report.violations.push(Violation {
            constraint: "tile-size".to_string(),
            message: format!("寸法 {w}x{h} が {tile} の倍数でない"),
            frame: at,
            area: None,
        });
    }
    if let Some((mw, mh)) = target.max_size
        && (w > mw || h > mh)
    {
        report.violations.push(Violation {
            constraint: "size".to_string(),
            message: format!("寸法 {w}x{h} が上限 {mw}x{mh} を超えている"),
            frame: at,
            area: None,
        });
    }
}

/// **表示時間は表示周期の倍数でなければならない** (設計書 6.11 の D40)．
fn check_timing(frame: &Frame, target: &Target, at: usize, report: &mut Report) {
    let Some(period) = target.frame_ms else {
        return;
    };
    if period <= 0.0 || frame.duration_ms == 0 {
        return;
    }
    let ms = frame.duration_ms as f32;
    let frames = (ms / period).round().max(1.0);
    let gap = (ms - frames * period).abs();
    if gap > FRAME_MS_TOLERANCE {
        report.violations.push(Violation {
            constraint: "frame-ms".to_string(),
            message: format!(
                "表示時間 {ms:.0} ms が表示周期 {period:.2} ms の倍数でない \
                 (最も近いのは {frames:.0} フレームぶんの {:.1} ms)",
                frames * period
            ),
            frame: at,
            area: None,
        });
    }
}

/// **タイルごとの同時色数．**
///
/// タイルの区切りは左上を原点とする．画像の寸法がタイルの倍数でないときは
/// [`check_size`] が別に鳴るので，ここでは**はみ出したぶんも 1 枚のタイルとして
/// 数える** (数え落とすと «違反が無い» に見える) ．
fn check_tiles(canvas: &IndexedCanvas, target: &Target, at: usize, report: &mut Report) {
    let (Some(max), Some(tile)) = (target.max_colors_per_tile, target.tile_size) else {
        return;
    };
    if tile == 0 {
        return;
    }
    let (w, h) = (canvas.width(), canvas.height());
    for ty in (0..h).step_by(tile as usize) {
        for tx in (0..w).step_by(tile as usize) {
            let rect = IRect::new(tx as i32, ty as i32, tile.min(w - tx), tile.min(h - ty));
            let mut seen: BTreeSet<u8> = BTreeSet::new();
            for p in rect.iter() {
                if let Some(i) = canvas.get_at(p) {
                    seen.insert(i);
                }
            }
            if seen.len() > max {
                report.violations.push(Violation {
                    constraint: "colors-per-tile".to_string(),
                    message: format!("タイルに {} 色 (上限 {max} 色)", seen.len()),
                    frame: at,
                    area: Some([rect.x, rect.y, rect.w as i32, rect.h as i32]),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Rgba8;
    use crate::frame::{Layer, LayerMeta};
    use crate::palette::Palette;
    use crate::uvec2;

    /// 添字 0 が透明の n 色パレット．
    fn palette(n: u8) -> Palette {
        let mut colors = vec![Rgba8::TRANSPARENT];
        for i in 1..n {
            colors.push(Rgba8::rgb(
                i.wrapping_mul(37),
                i.wrapping_mul(59),
                40u8.wrapping_add(i.wrapping_mul(3)),
            ));
        }
        Palette::new(colors).unwrap()
    }

    fn frame_of(canvas: IndexedCanvas, palette: Palette) -> Frame {
        let mut frame = Frame::new(uvec2(canvas.width(), canvas.height()), palette);
        frame.layers.push(Layer::new(
            LayerMeta::named("art"),
            Surface::Indexed(canvas),
        ));
        frame
    }

    #[test]
    fn a_conforming_sprite_passes() {
        let canvas = IndexedCanvas::filled(16, 16, 1).with_transparent(Some(0));
        let frame = frame_of(canvas, palette(4));
        let target = Target::builtin("gb").unwrap();
        let report = validate_frames(&[frame], &target);
        assert!(report.is_ok(), "{report}");
    }

    #[test]
    fn too_many_colours_in_the_palette_is_a_violation() {
        let canvas = IndexedCanvas::filled(16, 16, 1).with_transparent(Some(0));
        let frame = frame_of(canvas, palette(8));
        let report = validate_frames(&[frame], &Target::builtin("gb").unwrap());
        assert!(
            report.violations.iter().any(|v| v.constraint == "colors"),
            "{report}"
        );
        assert!(!report.is_ok());
    }

    /// **1 タイルの同時色数はパレット全体とは別に数える．**
    ///
    /// パレットが 16 色でも，8x8 のタイルに 5 色入っていれば 2bpp の出力先には載らない．
    #[test]
    fn too_many_colours_in_one_tile_is_a_violation() {
        let mut canvas = IndexedCanvas::filled(16, 16, 0).with_transparent(Some(0));
        // 左上のタイルにだけ 5 色置く
        for (i, p) in IRect::new(0, 0, 5, 1).iter().enumerate() {
            canvas.set_at(p, i as u8);
        }
        let frame = frame_of(canvas, palette(16));
        let target = Target {
            max_colors: None,
            ..Target::builtin("nes").unwrap()
        };
        let report = validate_frames(&[frame], &target);
        let hit: Vec<_> = report
            .violations
            .iter()
            .filter(|v| v.constraint == "colors-per-tile")
            .collect();
        assert_eq!(hit.len(), 1, "左上のタイルだけが鳴るはず: {report}");
        assert_eq!(hit[0].area, Some([0, 0, 8, 8]));
    }

    /// **タイルからはみ出した端も数える．** 数え落とすと «違反が無い» に見える．
    #[test]
    fn a_partial_tile_at_the_edge_is_still_counted() {
        let mut canvas = IndexedCanvas::filled(12, 8, 0).with_transparent(Some(0));
        for (i, p) in IRect::new(8, 0, 4, 2).iter().enumerate() {
            canvas.set_at(p, (i % 6) as u8);
        }
        let frame = frame_of(canvas, palette(16));
        let target = Target {
            max_colors: None,
            ..Target::builtin("nes").unwrap()
        };
        let report = validate_frames(&[frame], &target);
        assert!(
            report
                .violations
                .iter()
                .any(|v| v.constraint == "colors-per-tile" && v.area == Some([8, 0, 4, 8])),
            "端の半端なタイルを数えていない: {report}"
        );
    }

    #[test]
    fn a_size_that_is_not_a_multiple_of_the_tile_is_a_violation() {
        let canvas = IndexedCanvas::filled(12, 16, 1).with_transparent(Some(0));
        let frame = frame_of(canvas, palette(4));
        let report = validate_frames(&[frame], &Target::builtin("gb").unwrap());
        assert!(
            report
                .violations
                .iter()
                .any(|v| v.constraint == "tile-size"),
            "{report}"
        );
    }

    #[test]
    fn a_canvas_larger_than_the_screen_is_a_violation() {
        let canvas = IndexedCanvas::filled(256, 256, 1).with_transparent(Some(0));
        let frame = frame_of(canvas, palette(4));
        let report = validate_frames(&[frame], &Target::builtin("gb").unwrap());
        assert!(
            report.violations.iter().any(|v| v.constraint == "size"),
            "{report}"
        );
    }

    /// **透明は添字 0 でなければならない出力先がある．**
    #[test]
    fn a_transparent_colour_away_from_index_zero_is_a_violation() {
        let colors = vec![
            Rgba8::rgb(10, 10, 10),
            Rgba8::TRANSPARENT,
            Rgba8::rgb(90, 90, 90),
        ];
        let palette = Palette::new(colors).unwrap();
        let transparent = palette.entries().iter().position(|c| c.a == 0).unwrap() as u8;
        let canvas = IndexedCanvas::filled(16, 16, 0).with_transparent(Some(transparent));
        let frame = frame_of(canvas, palette);
        let report = validate_frames(&[frame], &Target::builtin("gb").unwrap());
        // パレットの正規化で透明が先頭に来ていれば違反ではない — どちらでも
        // «報告と実際が一致している» ことだけを見る
        let fired = report
            .violations
            .iter()
            .any(|v| v.constraint == "transparent-index");
        assert_eq!(fired, transparent != 0, "報告と実際が食い違う: {report}");
    }

    /// **表示時間は表示周期の倍数でなければならない** (D40)．
    #[test]
    fn a_duration_that_is_not_a_multiple_of_the_frame_period_is_a_violation() {
        let canvas = IndexedCanvas::filled(16, 16, 1).with_transparent(Some(0));
        let mut frame = frame_of(canvas, palette(4));
        // 60 Hz は 16.67 ms 刻み．25 ms はどの倍数からも 8 ms 以上離れている
        frame.duration_ms = 25;
        let report = validate_frames(&[frame], &Target::builtin("gb").unwrap());
        assert!(
            report.violations.iter().any(|v| v.constraint == "frame-ms"),
            "{report}"
        );
    }

    /// **丸めたぶんは違反にしない．** 2 フレームは 33.33 ms なので 33 ms は正しい．
    #[test]
    fn a_rounded_duration_is_not_a_violation() {
        let canvas = IndexedCanvas::filled(16, 16, 1).with_transparent(Some(0));
        for ms in [17u32, 33, 50, 100] {
            let mut frame = frame_of(canvas.clone(), palette(4));
            frame.duration_ms = ms;
            let report = validate_frames(&[frame], &Target::builtin("gb").unwrap());
            assert!(
                !report.violations.iter().any(|v| v.constraint == "frame-ms"),
                "{ms} ms を違反と言った: {report}"
            );
        }
    }

    /// **`None` の項目は検査しない．** «制約が無い» と «見ていない» を混ぜない．
    #[test]
    fn an_empty_target_checks_nothing_but_says_so() {
        let canvas = IndexedCanvas::filled(13, 7, 1);
        let frame = frame_of(canvas, palette(200));
        let report = validate_frames(&[frame], &Target::default());
        assert!(report.is_ok(), "{report}");
        assert!(
            !report.unchecked.is_empty(),
            "検査していないことを黙っている"
        );
    }

    #[test]
    fn every_builtin_target_can_be_looked_up() {
        for name in Target::BUILTIN {
            let t = Target::builtin(name).unwrap_or_else(|| panic!("{name} が引けない"));
            assert_eq!(&t.name, name);
        }
        assert!(Target::builtin("そんな出力先は無い").is_none());
    }

    #[test]
    fn the_report_serialises_to_json() {
        let canvas = IndexedCanvas::filled(12, 16, 1).with_transparent(Some(0));
        let frame = frame_of(canvas, palette(4));
        let report = validate_frames(&[frame], &Target::builtin("gb").unwrap());
        let json = serde_json::to_string(&report).unwrap();
        let back: Report = serde_json::from_str(&json).unwrap();
        assert_eq!(report, back);
    }
}
