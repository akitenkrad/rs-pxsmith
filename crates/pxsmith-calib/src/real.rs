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
use pxsmith_core::grid::{GridParams, estimate_grid};
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
    /// **負例** — 整数の格子が無いと分かっている件．棄却が正解である．
    ///
    /// `truth` とは排他である ([`read`] が両立する目録を拒む) ．`truth` を省いただけの
    /// 件は正解が**分からない**という意味で，こちらは**無いと分かっている**という意味で
    /// あり，混ぜると採点が壊れる．
    ///
    /// 根拠は 2 通りある．どちらなのかは `note` に書く．
    ///
    /// | 根拠 | 例 |
    /// | --- | --- |
    /// | 作り方から分かる | 非整数倍リサイズを掛けた (`pxsmith-calib render`) |
    /// | 測って分かった | `ingest` が周期を読めなかった (縁が境界へ集中していない) |
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_grid: bool,
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
    /// 負例を棄却した — **正解**．
    CorrectReject,
    /// 負例に答えを返した — **誤り**．格子が無いのに読めたと言っている．
    FalseAccept,
    /// 正解が無いので機械では判定できない．
    Unknown,
}

impl Verdict {
    /// CSV と画面の両方で使う識別子．**英語のまま変えない．**
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::ScaleOnly => "scale_only",
            Self::Wrong => "wrong",
            Self::Rejected => "rejected",
            Self::CorrectReject => "correct_reject",
            Self::FalseAccept => "false_accept",
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

fn judge(
    truth: Option<Truth>,
    no_grid: bool,
    scale_hat: Option<u32>,
    phase_hat: Option<(u32, u32)>,
) -> Verdict {
    // 負例は棄却が正解．**答えを返したら誤り**であって「惜しい」ではない
    if no_grid {
        return match scale_hat {
            None => Verdict::CorrectReject,
            Some(_) => Verdict::FalseAccept,
        };
    }
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

/// 判定を外から呼ぶための口 (診断用)．
///
/// [`run`] と同じ規則で採点する — **採点の規則を 2 か所に書かない**．
pub fn judge_public(
    truth: Option<Truth>,
    no_grid: bool,
    estimate: &std::result::Result<pxsmith_core::grid::GridEstimate, pxsmith_core::grid::GridError>,
) -> Verdict {
    let (scale_hat, phase_hat) = match estimate {
        Ok(e) => (
            Some(e.scale),
            Some((e.phase.x.max(0) as u32, e.phase.y.max(0) as u32)),
        ),
        Err(_) => (None, None),
    };
    judge(truth, no_grid, scale_hat, phase_hat)
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
    let manifest: Manifest = serde_json::from_str(&text)?;
    // 「正解が分からない」と「格子が無いと分かっている」を取り違えた目録は，黙って
    // 採点すると誤りが正解に化ける．読んだ時点で落とす
    if let Some(bad) = manifest
        .items
        .iter()
        .find(|i| i.no_grid && i.truth.is_some())
    {
        anyhow::bail!(
            "{}: truth と no_grid の両方が書いてある．格子が無い件に正解は無い",
            bad.file
        );
    }
    Ok(manifest)
}

/// 全件を推定する．
pub fn run(dir: &Path, manifest: &Manifest, params: &GridParams) -> Result<Vec<Outcome>> {
    manifest
        .items
        .iter()
        .map(|item| {
            let img = pxsmith_io::png::read_rgba(dir.join(&item.file))
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
                verdict: judge(item.truth, item.no_grid, scale_hat, phase_hat),
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
        assert_eq!(judge(None, false, Some(4), Some((0, 0))), Verdict::Unknown);
        assert_eq!(judge(None, false, None, None), Verdict::Unknown);
    }

    #[test]
    fn a_truth_without_a_phase_stops_at_the_scale() {
        let t = Truth {
            scale: 4,
            phase: None,
        };
        assert_eq!(
            judge(Some(t), false, Some(4), Some((1, 2))),
            Verdict::ScaleOnly
        );
        assert_eq!(judge(Some(t), false, Some(5), Some((1, 2))), Verdict::Wrong);
        assert_eq!(judge(Some(t), false, None, None), Verdict::Rejected);
    }

    #[test]
    fn a_full_truth_can_be_exact() {
        let t = Truth {
            scale: 4,
            phase: Some((1, 2)),
        };
        assert_eq!(judge(Some(t), false, Some(4), Some((1, 2))), Verdict::Exact);
        assert_eq!(
            judge(Some(t), false, Some(4), Some((0, 0))),
            Verdict::ScaleOnly
        );
    }

    #[test]
    fn a_negative_is_scored_the_other_way_round() {
        // 負例は棄却が正解．答えを返したら誤り
        assert_eq!(judge(None, true, None, None), Verdict::CorrectReject);
        assert_eq!(
            judge(None, true, Some(2), Some((0, 0))),
            Verdict::FalseAccept
        );
    }

    #[test]
    fn a_negative_is_not_the_same_as_an_unknown() {
        // 正解が分からない件は unknown のまま — 負例に格上げしない
        assert_eq!(judge(None, false, None, None), Verdict::Unknown);
        assert_ne!(
            judge(None, false, None, None),
            judge(None, true, None, None)
        );
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
                no_grid: false,
                note: None,
            }],
        };
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(serde_json::from_str::<Manifest>(&json).unwrap(), m);
        // 正解を省いた形も読めること．負例の印は既定で降りている
        let bare = r#"{"items":[{"file":"a.png","category":"ai-output","license":"CC0","source":"自作"}]}"#;
        let parsed: Manifest = serde_json::from_str(bare).unwrap();
        assert_eq!(parsed.items[0].truth, None);
        assert!(!parsed.items[0].no_grid);
        // 負例は truth を持たない — 書き出しに truth が出ないこと
        let neg = Item {
            no_grid: true,
            truth: None,
            ..m.items[0].clone()
        };
        let json = serde_json::to_string(&neg).unwrap();
        assert!(!json.contains("truth"), "{json}");
        assert!(json.contains("\"no_grid\":true"), "{json}");
    }

    #[test]
    fn a_manifest_that_claims_both_is_refused() {
        // truth と no_grid が両立する目録は，読んだ時点で落とす
        let dir = std::env::temp_dir().join("pxforge-real-both");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            r#"{"items":[{"file":"a.png","category":"other","license":"CC0",
                "source":"自作","no_grid":true,"truth":{"scale":4}}]}"#,
        )
        .unwrap();
        let err = read(&dir).unwrap_err().to_string();
        assert!(err.contains("no_grid"), "{err}");
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
