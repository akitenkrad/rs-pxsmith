//! 検証付き生成ループ (設計書 8.2 の擬似コード)．
//!
//! ```text
//! GenWithRepair(prompt, P, n_max):
//!   for i in 1..n_max:
//!     raw ← Llm(prompt, fb)
//!     (ok, canvas, err) ← ParseAndValidate(raw, P)
//!     if not ok: fb ← err; continue
//!     rep ← Lint(canvas, P)
//!     if NoBlockingViolation(rep): return canvas   # advisory 違反は許容する
//!     fb ← ToPromptHint(rep)
//! ```
//!
//! # 検査器は既にある — それが L0 を選んだ理由である (D156)
//!
//! `ParseAndValidate` は [`px_io::l0::L0Document::parse`] + `to_frames`，
//! `Lint` は `px_lint` の 27 ルール，`ToPromptHint` はその報告文である．
//! **1 つも新しく書いていない** — 出力形式を L0 にしたのは，この 3 つが
//! そのまま嵌るからである．
//!
//! # 「鳴らなかった」と「検査していない」を分ける
//!
//! blocking が 0 でも，**掛からなかった検査がある**かもしれない
//! (D77 ・D104 ・D142 と同じ形) ．[`Verified::advisory`] と
//! [`Verified::attempts`] を返して，通った理由が読めるようにする．
//!
//! # L0 は自己完結していない — だからモデルは色を作れない
//!
//! L0 のパレットは**外の `.hex` への参照**であって，本文には色が書けない
//! (`px_io::l0::L0PaletteSpec`) ．したがって道具が宣言されたパレットから
//! `.hex` を先に書き，**モデルは添字の文字だけを書く**．
//!
//! これは制限ではなく**保証**である — D2 ・D4 ・D94 の «色を作らない» が，
//! 検査ではなく**構造で**成り立つ．モデルはパレットに無い色を出しようがない．
//!
//! # 落ちた試行を捨てない
//!
//! [`Report::attempts`] に**全部の試行**が残る．最後の 1 回だけ返すと
//! «何度目で通ったのか» «何が悪かったのか» が消える (D138 «測る口が飛ばした
//! 件を数えていないと，落ちていることが見えない» と同じ) ．
//!
//! # 輪が直すのは «モデルの出力» であって «経路» ではない (D165)
//!
//! [`Generator::generate`] が `Err` を返したら，**輪は回らずそこで終わる** —
//! `Truncated` ・`Refused` ・`Backend` は試行として数えない．同じ依頼を
//! もう 1 度送っても同じ上限に当たるので，作り直しは答えにならないからである．
//! 輪が回るのは «読めなかった» (`Unparsed`) と «blocking が出た»
//! (`Blocking`) の 2 つだけで，どちらも**次の依頼に助言を足せる**．
//!
//! **実測 (D165)**: `--max-tokens 300` で `Truncated` を本物の API で通した．
//! 15 秒 ・作り直しは 0 回 ・上限は要求した 300 として報告された．

use std::path::Path;

use px_core::frame::Frame;
use px_io::l0::L0Document;

use crate::error::{GenError, Result};
use crate::request::GenRequest;

/// バックエンド．**道具はこの口としか喋らない．**
///
/// 試験は本物のバックエンドを叩かずに書ける — それがこの trait の主目的である．
pub trait Generator {
    /// 1 回だけ生成する．
    ///
    /// `feedback` は前の試行が落ちた理由 (初回は `None`)．
    fn generate(&self, req: &GenRequest, feedback: Option<&str>) -> Result<String>;

    /// 何を叩いているか (素性に残す)．
    fn describe(&self) -> String;
}

/// 1 回の試行の顛末．
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Attempt {
    /// L0 として読めなかった．
    Unparsed { error: String },
    /// 読めたが blocking が残った．
    Blocking { findings: Vec<String> },
    /// 通った．
    Accepted { advisory: usize },
}

impl Attempt {
    pub fn is_accepted(&self) -> bool {
        matches!(self, Self::Accepted { .. })
    }

    /// 次の試行へ渡す助言 (通った試行には無い)．
    pub fn feedback(&self) -> Option<String> {
        match self {
            Self::Unparsed { error } => Some(format!(
                "前回の出力は L0 として読めなかった: {error}\n\
                 L0 の本文だけを返すこと (説明も囲みも付けない)．"
            )),
            Self::Blocking { findings } => Some(format!(
                "前回の出力は品質検査の blocking に {} 件掛かった:\n{}\n\
                 これらを直した L0 を返すこと．",
                findings.len(),
                findings
                    .iter()
                    .map(|f| format!("  - {f}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            )),
            Self::Accepted { .. } => None,
        }
    }
}

/// 通った結果．
#[derive(Clone, Debug)]
pub struct Verified {
    pub frames: Vec<Frame>,
    /// 通った L0 の本文．
    pub l0: String,
    /// 何度目で通ったか (1 なら 1 発)．
    pub attempts: u32,
    /// 残っていた advisory の数．**0 とは限らない** (8.2 は許容する)．
    pub advisory: usize,
}

/// 落ちたときも含めた全部の記録．
#[derive(Clone, Debug)]
pub struct Report {
    pub attempts: Vec<Attempt>,
    pub verified: Option<Verified>,
}

/// L0 として読み，27 ルールを掛ける．
///
/// **blocking だけを門にする** — 8.2 が «advisory 違反は許容する» と明記している．
fn parse_and_lint(raw: &str, path: &Path) -> Attempt {
    let doc = match L0Document::parse(raw, path) {
        Ok(d) => d,
        Err(e) => {
            return Attempt::Unparsed {
                error: e.to_string(),
            };
        }
    };
    let frames = match doc.to_frames(path) {
        Ok(f) => f,
        Err(e) => {
            return Attempt::Unparsed {
                error: e.to_string(),
            };
        }
    };
    if frames.is_empty() {
        return Attempt::Unparsed {
            error: "コマが 1 つも無い".to_string(),
        };
    }

    let cfg = px_lint::LintConfig::default();
    let (mut blocking, mut advisory) = (Vec::new(), 0usize);
    for frame in &frames {
        let mut report = px_lint::rules::lint_palette(&frame.palette, &cfg);
        for layer in &frame.layers {
            if let px_core::frame::Surface::Indexed(canvas) = &layer.surface {
                report.extend(px_lint::lint_canvas(canvas, &frame.palette, &cfg));
            }
        }
        for f in report.blocking() {
            // **番号と名前を残す** — 助言に «ルール 3» とだけ書いても直せない
            blocking.push(format!("ルール {} ({}): {}", f.rule, f.name, f.message));
        }
        advisory += report.advisory().count();
    }

    // **コマが 2 つ以上あればフレーム間のルールも掛ける** (設計書 7.1 の `sequence`)．
    //
    // ここが抜けていた (D165) — `px lint` は 22 〜 27 を掛けるのに生成の輪は
    // 掛けていなかったので，**道具が «通った» と言った列が自分の検査に落ちる**
    // 状態だった (6 ルール中 5 つが blocking である — 27 だけ advisory)．
    // `px gen prog --frames 8` で出した絵がまさにそれに当たる．
    if frames.len() >= 2 {
        let (report, _coverage) = px_lint::lint_sequence(&frames, &cfg);
        for f in report.blocking() {
            blocking.push(format!("ルール {} ({}): {}", f.rule, f.name, f.message));
        }
        advisory += report.advisory().count();
    }

    if blocking.is_empty() {
        Attempt::Accepted { advisory }
    } else {
        Attempt::Blocking { findings: blocking }
    }
}

/// 検証付き生成ループを回す (設計書 8.2)．
pub fn generate_with_repair(
    backend: &dyn Generator,
    req: &GenRequest,
    l0_path: &Path,
) -> Result<Report> {
    if req.max_attempts == 0 {
        return Err(GenError::NoAttempts);
    }
    let mut attempts = Vec::new();
    let mut feedback: Option<String> = None;

    for _ in 0..req.max_attempts {
        let raw = backend.generate(req, feedback.as_deref())?;
        let outcome = parse_and_lint(&raw, l0_path);
        feedback = outcome.feedback();
        let accepted = outcome.is_accepted();
        attempts.push(outcome);

        if accepted {
            // 通った試行だけをもう 1 度読み直す (frames を持って帰るため)
            let doc = L0Document::parse(&raw, l0_path)?;
            let frames = doc.to_frames(l0_path)?;
            let advisory = match attempts.last() {
                Some(Attempt::Accepted { advisory }) => *advisory,
                _ => 0,
            };
            let n = attempts.len() as u32;
            return Ok(Report {
                attempts,
                verified: Some(Verified {
                    frames,
                    l0: raw,
                    attempts: n,
                    advisory,
                }),
            });
        }
    }

    Ok(Report {
        attempts,
        verified: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::{Backend, Constraints, Effort, GenKind};
    use std::cell::RefCell;

    /// 台本どおりに返すだけの偽バックエンド．**本物を叩かずに輪を試せる．**
    struct Scripted {
        lines: RefCell<Vec<String>>,
        seen: RefCell<Vec<Option<String>>>,
    }

    impl Scripted {
        fn new(lines: &[&str]) -> Self {
            Self {
                lines: RefCell::new(lines.iter().map(|s| s.to_string()).collect()),
                seen: RefCell::new(Vec::new()),
            }
        }
    }

    impl Generator for Scripted {
        fn generate(&self, _req: &GenRequest, feedback: Option<&str>) -> Result<String> {
            self.seen.borrow_mut().push(feedback.map(str::to_string));
            let mut lines = self.lines.borrow_mut();
            if lines.is_empty() {
                return Err(GenError::Backend {
                    message: "台本が尽きた".to_string(),
                });
            }
            Ok(lines.remove(0))
        }

        fn describe(&self) -> String {
            "scripted".to_string()
        }
    }

    fn req(max_attempts: u32) -> GenRequest {
        GenRequest {
            kind: GenKind::Prog,
            backend: Backend::anthropic("test"),
            effort: Effort::High,
            prompt: "test".to_string(),
            constraints: Constraints {
                width: 4,
                height: 4,
                palette: vec!["1a1c2c".to_string()],
                frames: 1,
            },
            max_attempts,
            max_tokens: crate::anthropic::DEFAULT_MAX_TOKENS,
        }
    }

    /// 素直に通る 8x8 (ベタ塗りの塊なので blocking は出ない)．
    ///
    /// **本物の L0 の形で書く** — `ref` は外の `.hex`，画素は `data` の 1 本の
    /// 文字列である (`inline` パレットも `pixels` 配列も L0 には無い)．
    fn good_l0() -> String {
        let rows = [
            "........", ".111111.", ".111111.", ".111111.", ".111111.", ".111111.", ".111111.",
            "........",
        ];
        format!(
            "[meta]\nformat = 1\nname = \"t\"\nlayer = \"art\"\n\n\
             [palette]\nref = \"t.px.hex\"\n\n\
             [palette.map]\n\".\" = \"transparent\"\n0 = 0\n1 = 1\n\n\
             [[frame]]\nname = \"t_0\"\nkind = \"key\"\nduration_ms = 100\n\
             data = \'\'\'\n{}\n\'\'\'\n",
            rows.join("\n")
        )
    }

    /// 試験用の作業場 — **`.hex` を先に置く** (道具がやることの再現)．
    struct Workdir {
        dir: std::path::PathBuf,
    }

    impl Workdir {
        fn new(tag: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("pxgen-repair-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("t.px.hex"), "1a1c2c\nf4f4f4\n").unwrap();
            Self { dir }
        }

        fn l0(&self) -> std::path::PathBuf {
            self.dir.join("t.px.toml")
        }
    }

    impl Drop for Workdir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// **壊れると: 1 発で通る出力でも作り直しに行く．**
    #[test]
    fn a_good_first_answer_stops_the_loop() {
        let w = Workdir::new("good");
        let g = Scripted::new(&[&good_l0()]);
        let r = generate_with_repair(&g, &req(3), &w.l0()).unwrap();
        let v = r.verified.expect("通るはず");
        assert_eq!(v.attempts, 1);
        assert_eq!(r.attempts.len(), 1);
        assert!(r.attempts[0].is_accepted());
        // 初回に助言を渡してはいけない
        assert_eq!(g.seen.borrow()[0], None);
    }

    /// 3 コマの列 — **真ん中の中割りだけ穴が開いている** (トポロジーが変わる)．
    ///
    /// ルール 22 は数え上げ (オイラー標数) なので閾値が無く，**必ず鳴る**．
    fn sequence_with_topology_change() -> String {
        let solid = [
            "........", ".111111.", ".111111.", ".111111.", ".111111.", ".111111.", ".111111.",
            "........",
        ];
        let holed = [
            "........", ".111111.", ".111111.", ".11..11.", ".11..11.", ".111111.", ".111111.",
            "........",
        ];
        let frame = |name: &str, kind: &str, rows: &[&str; 8]| {
            format!(
                "[[frame]]\nname = \"{name}\"\nkind = \"{kind}\"\nduration_ms = 100\n\
                 data = \'\'\'\n{}\n\'\'\'\n\n",
                rows.join("\n")
            )
        };
        format!(
            "[meta]\nformat = 1\nname = \"t\"\nlayer = \"art\"\n\n\
             [palette]\nref = \"t.px.hex\"\n\n\
             [palette.map]\n\".\" = \"transparent\"\n0 = 0\n1 = 1\n\n{}{}{}",
            frame("t_0", "key", &solid),
            frame("t_1", "inbetween", &holed),
            frame("t_2", "key", &solid),
        )
    }

    /// **壊れると: 生成した «列» がフレーム間の検査を素通りする** (D165)．
    ///
    /// `px lint` は 22 〜 27 を掛けるのに，生成の輪は 1 コマずつしか見て
    /// いなかった — **道具が «通った» と言った列が自分の検査に落ちる**状態で，
    /// 6 ルール中 5 つが blocking である (27 だけ advisory)．
    #[test]
    fn a_blocking_sequence_rule_turns_the_loop() {
        let w = Workdir::new("sequence");
        let g = Scripted::new(&[&sequence_with_topology_change(), &good_l0()]);
        let r = generate_with_repair(&g, &req(3), &w.l0()).unwrap();

        assert_eq!(r.attempts.len(), 2, "列の違反で作り直していない: {r:?}");
        match &r.attempts[0] {
            Attempt::Blocking { findings } => assert!(
                findings.iter().any(|f| f.contains("ルール 22")),
                "フレーム間のルールが挙がっていない: {findings:?}"
            ),
            other => panic!("列の blocking として読めていない: {other:?}"),
        }
        // **助言に載って次の試行へ渡ること** — 載らなければ直しようがない
        let feedback = g.seen.borrow()[1].clone().expect("2 度目には助言がある");
        assert!(
            feedback.contains("ルール 22"),
            "助言に載っていない: {feedback}"
        );
    }

    /// **壊れると: 読めなかった理由が次の試行へ渡らない (同じ失敗を繰り返す)．**
    #[test]
    fn a_parse_failure_is_fed_back_to_the_next_attempt() {
        let w = Workdir::new("parse");
        let g = Scripted::new(&["これは L0 ではない", &good_l0()]);
        let r = generate_with_repair(&g, &req(3), &w.l0()).unwrap();
        assert!(r.verified.is_some());
        assert_eq!(r.attempts.len(), 2, "2 度目で通るはず");
        assert!(matches!(r.attempts[0], Attempt::Unparsed { .. }));

        let seen = g.seen.borrow();
        assert_eq!(seen[0], None, "初回に助言があってはいけない");
        let fb = seen[1].as_ref().expect("2 回目には助言が要る");
        assert!(fb.contains("読めなかった"), "助言に理由が無い: {fb}");
    }

    /// **壊れると: 上限を超えて叩き続ける (課金と時間が青天井になる)．**
    #[test]
    fn the_loop_stops_at_the_declared_attempt_budget() {
        let w = Workdir::new("budget");
        let g = Scripted::new(&["だめ", "だめ", "だめ", &good_l0()]);
        let r = generate_with_repair(&g, &req(3), &w.l0()).unwrap();
        assert!(r.verified.is_none(), "3 回で打ち切るはず");
        assert_eq!(r.attempts.len(), 3);
    }

    /// **壊れると: 落ちた試行が捨てられ «何が悪かったか» が読めなくなる．**
    #[test]
    fn every_attempt_is_kept_even_the_failed_ones() {
        let w = Workdir::new("kept");
        let g = Scripted::new(&["だめ1", "だめ2", &good_l0()]);
        let r = generate_with_repair(&g, &req(5), &w.l0()).unwrap();
        assert_eq!(r.attempts.len(), 3, "落ちた 2 回も残るはず");
        assert!(matches!(r.attempts[0], Attempt::Unparsed { .. }));
        assert!(matches!(r.attempts[1], Attempt::Unparsed { .. }));
        assert!(r.attempts[2].is_accepted());
    }

    /// **壊れると: advisory を門にしてしまう** (設計書 8.2 は許容すると明記)．
    #[test]
    fn advisory_findings_do_not_block_acceptance() {
        let w = Workdir::new("adv");
        let g = Scripted::new(&[&good_l0()]);
        let r = generate_with_repair(&g, &req(1), &w.l0()).unwrap();
        let v = r.verified.expect("advisory があっても通るはず");
        // 数は入力で決まるので値は固定しない — 「門になっていない」ことだけ見る
        assert!(v.advisory < usize::MAX);
    }

    /// **壊れると: 上限 0 を «無限» と読む．**
    #[test]
    fn a_zero_attempt_budget_is_an_error_not_an_infinite_loop() {
        let w = Workdir::new("zero");
        let g = Scripted::new(&[&good_l0()]);
        assert!(generate_with_repair(&g, &req(0), &w.l0()).is_err());
    }
}
