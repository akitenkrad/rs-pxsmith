//! `px-lint` — 品質検査 (設計書 7 章)．
//!
//! lint は 2 つの役割を持つ．人の目の代わりに形式的性質を検査する役割と，
//! **生成 AI ループの評価関数**としての役割である．後者のために `px-gen` からも
//! 参照されるので独立したクレートにしてある (設計書 2.1)．
//!
//! # 分類 (7.2)
//!
//! | 分類 | 扱い |
//! | --- | --- |
//! | `blocking` | 生成ループの判定に用いる．違反があれば再生成する |
//! | `advisory` | 報告のみ．偽陽性が避けられない主観寄りのルールを置く |
//!
//! # スコープ (7.1，D47)
//!
//! | スコープ | 対象 |
//! | --- | --- |
//! | `static` | 単一の静止画 |
//! | `keyframe` | `kind = "key"` / `"breakdown"` のフレームのみ |
//! | `sequence` | フレーム列全体 |
//!
//! ジャギー・AA 系を `keyframe` に限定するのが要点である．これがないと `px anim` の
//! 出力が自らの lint に大量に落ちる．
//!
//! # 実装済みのルール
//!
//! M2 で実装するのは**色・格子・ディザ系の 11 ルール**である
//! (1 ・2 ・3 ・5 ・9 ・10 ・11 ・15 ・16 ・17 ・18)．形・陰影系は M3 で足した
//! (4 ・6 ・7 ・8 ・12 ・13 ・14)．**フレーム間の 6 件 (22 〜 27) は M4** —
//! [`sequence`] にある．G5 依存の 3 件 (19 ・20 ・21) は M7．

pub mod rules;
pub mod sequence;

pub use rules::{LintConfig, lint_canvas, lint_canvas_scoped, lint_frame};
pub use sequence::{SequenceCoverage, lint_sequence};

use px_core::math::{IRect, IVec2};
use serde::{Deserialize, Serialize};

/// 違反の重さ (設計書 7.2)．
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// 生成ループはこれがあれば再生成する．
    Blocking,
    /// 報告のみ．
    Advisory,
}

/// 検査の範囲 (設計書 7.1)．
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Static,
    Keyframe,
    Sequence,
}

/// ルールの定義．
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    /// 設計書 7.3 の番号．
    pub id: u8,
    pub name: &'static str,
    pub severity: Severity,
    pub scope: Scope,
}

/// M2 で実装するルール (設計書 7.3)．
pub const RULES: &[Rule] = &[
    Rule {
        id: 1,
        name: "パレット逸脱",
        severity: Severity::Blocking,
        scope: Scope::Static,
    },
    // **advisory である (D70)．** «格子が見つからない» と «格子が壊れている» を
    // 区別できるのは推定器までで，きれいな拡大でも 26.7% で鳴る．これは推定器自身の
    // 棄却率がそのまま出ているもので**閾値では下がらない** — `conform` の再現率を
    // 超えられない検査で出力を止めるべきではない
    Rule {
        id: 2,
        name: "格子崩れ",
        severity: Severity::Advisory,
        scope: Scope::Static,
    },
    Rule {
        id: 3,
        name: "孤立ピクセル",
        severity: Severity::Blocking,
        scope: Scope::Static,
    },
    Rule {
        id: 4,
        name: "アウトライン角の重なり",
        severity: Severity::Blocking,
        scope: Scope::Keyframe,
    },
    Rule {
        id: 5,
        name: "彩度カーブ異常",
        severity: Severity::Blocking,
        scope: Scope::Static,
    },
    // **advisory である (D86)．** 設計書 7.3 は blocking と定めるが，**実物の
    // ドット絵は色相をずらさない陰影を普通に使う** — 色相差が 0.5 度未満 (丸めの
    // 揺れより小さい，文字どおり同一色相) の光 / 影の組を持つ良い絵が 64 枚中
    // 20 枚あり，35 度 (ランプ生成が狙う分離量) で切ると 41 枚が止まる．
    // 7.2 の «偽陽性が避けられない主観寄りのルール» に当たる
    Rule {
        id: 6,
        name: "単色影",
        severity: Severity::Advisory,
        scope: Scope::Static,
    },
    Rule {
        id: 7,
        name: "反転同値の陰影不整合",
        severity: Severity::Blocking,
        scope: Scope::Static,
    },
    // **advisory である (D85)．** 設計書 7.3 は blocking と定めるが，**この特徴量
    // (ラン長列の谷) では良い絵と «ぎざぎざの縁» が分かれない**．CC0 の実物 61 枚は
    // 谷を 146 件持ち **36 枚が 1 件以上**で，わざとぎざぎざに描いた負例 8 枚
    // (0 〜 4 件) とどの切り方でも重なる (絵あたりの件数 ・ラン比 ・1 区間あたりの
    // 密度 ・谷の深さ $\delta \ge 2$ をすべて測った) ．**分かれない検査で出力を
    // 止めない** — 直す道具 (`px smooth`) はあるので，報告して人に渡す
    Rule {
        id: 8,
        name: "ジャギー",
        severity: Severity::Advisory,
        scope: Scope::Keyframe,
    },
    Rule {
        id: 9,
        name: "ミクセル",
        severity: Severity::Blocking,
        scope: Scope::Static,
    },
    Rule {
        id: 10,
        name: "ディザの塊化",
        severity: Severity::Blocking,
        scope: Scope::Static,
    },
    Rule {
        id: 11,
        name: "明度差不足",
        severity: Severity::Advisory,
        scope: Scope::Static,
    },
    Rule {
        id: 12,
        name: "バンディング",
        severity: Severity::Advisory,
        scope: Scope::Keyframe,
    },
    Rule {
        id: 13,
        name: "pillow shading",
        severity: Severity::Advisory,
        scope: Scope::Static,
    },
    Rule {
        id: 14,
        name: "AA 過多",
        severity: Severity::Advisory,
        scope: Scope::Keyframe,
    },
    Rule {
        id: 15,
        name: "ディザ過多",
        severity: Severity::Advisory,
        scope: Scope::Static,
    },
    Rule {
        id: 16,
        name: "大面積の高彩度色",
        severity: Severity::Advisory,
        scope: Scope::Static,
    },
    Rule {
        id: 17,
        name: "高コントラスト間のディザ",
        severity: Severity::Advisory,
        scope: Scope::Static,
    },
    Rule {
        id: 18,
        name: "純黒の使用",
        severity: Severity::Advisory,
        scope: Scope::Static,
    },
    Rule {
        id: 19,
        name: "形の乱雑さ",
        severity: Severity::Advisory,
        scope: Scope::Keyframe,
    },
    Rule {
        id: 20,
        name: "接線",
        severity: Severity::Advisory,
        scope: Scope::Keyframe,
    },
    Rule {
        id: 21,
        name: "隣接領域の同色",
        severity: Severity::Advisory,
        scope: Scope::Static,
    },
    Rule {
        id: 22,
        name: "トポロジー変化",
        severity: Severity::Blocking,
        scope: Scope::Sequence,
    },
    Rule {
        id: 23,
        name: "揺れる線",
        severity: Severity::Blocking,
        scope: Scope::Sequence,
    },
    Rule {
        id: 24,
        name: "アニメ領域のディザ",
        severity: Severity::Blocking,
        scope: Scope::Sequence,
    },
    Rule {
        id: 25,
        name: "孤立列の残留",
        severity: Severity::Blocking,
        scope: Scope::Sequence,
    },
    Rule {
        id: 26,
        name: "除外マスク侵犯",
        severity: Severity::Blocking,
        scope: Scope::Sequence,
    },
    Rule {
        id: 27,
        name: "体積不保存",
        severity: Severity::Advisory,
        scope: Scope::Sequence,
    },
];

/// 番号からルールを引く．
pub fn rule(id: u8) -> Option<&'static Rule> {
    RULES.iter().find(|r| r.id == id)
}

/// 見つかった違反 1 件．
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Violation {
    pub rule: u8,
    pub name: String,
    pub severity: Severity,
    pub scope: Scope,
    pub message: String,
    /// 違反の位置 (画素)．
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<[i32; 2]>,
    /// 違反の範囲．
    #[serde(skip_serializing_if = "Option::is_none")]
    pub area: Option<[i32; 4]>,
}

impl Violation {
    pub(crate) fn new(rule: &Rule, message: impl Into<String>) -> Self {
        Self {
            rule: rule.id,
            name: rule.name.to_string(),
            severity: rule.severity,
            scope: rule.scope,
            message: message.into(),
            at: None,
            area: None,
        }
    }

    pub(crate) fn at(mut self, p: IVec2) -> Self {
        self.at = Some([p.x, p.y]);
        self
    }

    pub(crate) fn area(mut self, r: IRect) -> Self {
        self.area = Some([r.x, r.y, r.w as i32, r.h as i32]);
        self
    }

    pub fn is_blocking(&self) -> bool {
        self.severity == Severity::Blocking
    }
}

/// 検査の結果．
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub violations: Vec<Violation>,
}

impl Report {
    pub fn push(&mut self, v: Violation) {
        self.violations.push(v);
    }

    pub fn extend(&mut self, other: Report) {
        self.violations.extend(other.violations);
    }

    pub fn is_empty(&self) -> bool {
        self.violations.is_empty()
    }

    pub fn blocking(&self) -> impl Iterator<Item = &Violation> {
        self.violations.iter().filter(|v| v.is_blocking())
    }

    pub fn advisory(&self) -> impl Iterator<Item = &Violation> {
        self.violations.iter().filter(|v| !v.is_blocking())
    }

    /// 生成ループの判定 — **`advisory` 違反は許容する** (設計書 8.2)．
    pub fn has_blocking(&self) -> bool {
        self.blocking().next().is_some()
    }

    /// 違反をルール番号順に並べ替える (報告の決定論性)．
    pub fn sorted(mut self) -> Self {
        self.violations.sort_by_key(|v| {
            (
                v.rule,
                v.at.unwrap_or([i32::MAX, i32::MAX]),
                v.message.clone(),
            )
        });
        self
    }

    /// 生成ループへ返す助言文 (設計書 8.2 の `ToPromptHint`)．
    pub fn to_prompt_hint(&self) -> String {
        let mut out = String::new();
        for v in self.blocking() {
            out.push_str(&format!(
                "- ルール {} ({}): {}\n",
                v.rule, v.name, v.message
            ));
        }
        out
    }
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.violations.is_empty() {
            return writeln!(f, "違反なし");
        }
        for v in &self.violations {
            let tag = if v.is_blocking() {
                "blocking"
            } else {
                "advisory"
            };
            write!(f, "[{tag}] ルール {} {}: {}", v.rule, v.name, v.message)?;
            if let Some([x, y]) = v.at {
                write!(f, " ({x}, {y})")?;
            }
            writeln!(f)?;
        }
        Ok(())
    }
}
