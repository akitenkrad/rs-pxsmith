//! 実データ (合成でない入力) を格子推定にかける．
//!
//! 合成 500 件は劣化の作り方をこちらが決めているので，**実運用対象との分布のずれが
//! 残る**．実装計画書 M2 はこれを承知のうえで実データ 20〜30 件を**別枠で報告する**と
//! 定めている．ここはその受け皿である．
//!
//! | 合成データと違うところ | 扱い |
//! | --- | --- |
//! | 正解が分からない件がある | `truth` を省ける．推定結果を人が見て判断する |
//! | 劣化の作り方が分からない | 条件で切った集計をしない．1 件ずつ並べる |
//! | 件数が少ない | 率で語らない．**20〜30 件では 1 件が 3〜5% 動く** |
//!
//! 目録は `testdata/grid-eval/real/manifest.json`．素材はリポジトリへ入るので，
//! `testdata/SOURCES.md` のライセンス方針 (CC0 原則・MIT は表示同梱で可) に従う．

use std::path::Path;

use anyhow::{Context, Result};
use px_core::grid::{GridParams, estimate_grid};
use serde::{Deserialize, Serialize};

/// 素材の出どころ．**分布のずれを見るための区分**なので，合成と同じ扱いにしない．
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    /// 生成 AI の出力．
    AiOutput,
    /// 3D レンダを縮小したもの．
    Render,
    /// CC0 素材で組んだ画面を撮ったもの．
    Screenshot,
    /// その他 (`note` に書く)．
    Other,
}

/// 実データ 1 件．
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    /// 目録から見た相対パス．
    pub file: String,
    pub category: Category,
    /// ライセンス表記 (`SOURCES.md` と揃える)．
    pub license: String,
    /// 出典．自作なら「自作」と書く．
    pub source: String,
    /// 分かっていれば正解．**分からないなら省く** — 推測を正解として置かない．
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truth: Option<Truth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// 分かっている正解．
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Truth {
    pub scale: u32,
    /// 位相まで分かっているとは限らない．
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<(u32, u32)>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub items: Vec<Item>,
}

/// 判定．正解が無い件は「人が見る」で止める — **推測で正誤を付けない**．
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// スケールも位相も一致．
    Exact,
    /// スケールだけ一致 (位相の正解が無い場合を含む)．
    ScaleOnly,
    Wrong,
    Rejected,
    /// 正解が無いので機械では判定できない．
    Unknown,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::ScaleOnly => "scale_only",
            Self::Wrong => "wrong",
            Self::Rejected => "rejected",
            Self::Unknown => "unknown",
        }
    }
}

/// 1 件の結果．
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub file: String,
    pub category: Category,
    pub width: u32,
    pub height: u32,
    pub scale_hat: Option<u32>,
    pub phase_hat: Option<(u32, u32)>,
    pub confidence: Option<f32>,
    pub error: Option<String>,
    pub verdict: Verdict,
}

pub const HEADER: &str =
    "file,category,width,height,scale_hat,phase_hat_x,phase_hat_y,confidence,error,verdict";

impl Outcome {
    pub fn to_csv(&self) -> String {
        let opt = |v: Option<String>| v.unwrap_or_default();
        format!(
            "{},{:?},{},{},{},{},{},{},{},{}",
            self.file,
            self.category,
            self.width,
            self.height,
            opt(self.scale_hat.map(|v| v.to_string())),
            opt(self.phase_hat.map(|p| p.0.to_string())),
            opt(self.phase_hat.map(|p| p.1.to_string())),
            opt(self.confidence.map(|v| format!("{v:.4}"))),
            self.error.clone().unwrap_or_default(),
            self.verdict.as_str(),
        )
    }
}

fn judge(truth: Option<Truth>, scale_hat: Option<u32>, phase_hat: Option<(u32, u32)>) -> Verdict {
    match (truth, scale_hat) {
        (None, _) => Verdict::Unknown,
        (Some(_), None) => Verdict::Rejected,
        (Some(t), Some(s)) if s != t.scale => Verdict::Wrong,
        (Some(t), Some(_)) => match (t.phase, phase_hat) {
            (Some(p), Some(q)) if p == q => Verdict::Exact,
            // 位相の正解が無ければスケール一致までしか言えない
            (None, _) => Verdict::ScaleOnly,
            _ => Verdict::ScaleOnly,
        },
    }
}

/// 目録を読む．
pub fn read(dir: &Path) -> Result<Manifest> {
    let path = dir.join("manifest.json");
    let text = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "{} が無い．素材と目録の作り方は同じディレクトリの README を見ること",
            path.display()
        )
    })?;
    Ok(serde_json::from_str(&text)?)
}

/// 全件を推定する．
pub fn run(dir: &Path, manifest: &Manifest, params: &GridParams) -> Result<Vec<Outcome>> {
    manifest
        .items
        .iter()
        .map(|item| {
            let img = px_io::png::read_rgba(dir.join(&item.file))
                .with_context(|| format!("{} を読めない", item.file))?;
            let (scale_hat, phase_hat, confidence, error) = match estimate_grid(&img, params) {
                Ok(e) => (
                    Some(e.scale),
                    Some((e.phase.x.max(0) as u32, e.phase.y.max(0) as u32)),
                    Some(e.confidence),
                    None,
                ),
                Err(e) => (None, None, None, Some(e.to_string())),
            };
            Ok(Outcome {
                file: item.file.clone(),
                category: item.category,
                width: img.width(),
                height: img.height(),
                scale_hat,
                phase_hat,
                confidence,
                error,
                verdict: judge(item.truth, scale_hat, phase_hat),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_truth_is_not_guessed() {
        // 正解が無い件を「たぶん合っている」と数えない
        assert_eq!(judge(None, Some(4), Some((0, 0))), Verdict::Unknown);
        assert_eq!(judge(None, None, None), Verdict::Unknown);
    }

    #[test]
    fn a_truth_without_a_phase_stops_at_the_scale() {
        let t = Truth {
            scale: 4,
            phase: None,
        };
        assert_eq!(judge(Some(t), Some(4), Some((1, 2))), Verdict::ScaleOnly);
        assert_eq!(judge(Some(t), Some(5), Some((1, 2))), Verdict::Wrong);
        assert_eq!(judge(Some(t), None, None), Verdict::Rejected);
    }

    #[test]
    fn a_full_truth_can_be_exact() {
        let t = Truth {
            scale: 4,
            phase: Some((1, 2)),
        };
        assert_eq!(judge(Some(t), Some(4), Some((1, 2))), Verdict::Exact);
        assert_eq!(judge(Some(t), Some(4), Some((0, 0))), Verdict::ScaleOnly);
    }

    #[test]
    fn the_manifest_round_trips() {
        let m = Manifest {
            items: vec![Item {
                file: "render/knight.png".to_string(),
                category: Category::Render,
                license: "CC0 (自作)".to_string(),
                source: "自作".to_string(),
                truth: Some(Truth {
                    scale: 6,
                    phase: Some((0, 0)),
                }),
                note: None,
            }],
        };
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(serde_json::from_str::<Manifest>(&json).unwrap(), m);
        // 正解を省いた形も読めること
        let bare = r#"{"items":[{"file":"a.png","category":"ai-output","license":"CC0","source":"自作"}]}"#;
        let parsed: Manifest = serde_json::from_str(bare).unwrap();
        assert_eq!(parsed.items[0].truth, None);
    }

    #[test]
    fn the_header_lists_as_many_columns_as_a_row_writes() {
        let o = Outcome {
            file: "a.png".to_string(),
            category: Category::Other,
            width: 100,
            height: 80,
            scale_hat: Some(4),
            phase_hat: Some((1, 2)),
            confidence: Some(0.25),
            error: None,
            verdict: Verdict::Unknown,
        };
        assert_eq!(HEADER.split(',').count(), o.to_csv().split(',').count());
    }
}
