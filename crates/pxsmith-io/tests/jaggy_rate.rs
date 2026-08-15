//! ジャギー検出の誤検出率を実素材で測る (M1a の完了条件，R22・要調査 #1)．
//!
//! 「ラン長の谷検出が実データで誤検出しないか (手描きの意図的な凹凸)」を，
//! `testdata/aseprite/` の実スプライトに対して数える．
//!
//! **これは合否を判定するテストではない．** 正解ラベルの付いた素材が無いので
//! 「正しい検出」と「誤検出」は分けられない．測っているのは**検出の頻度**で，
//! それが極端でないこと (全ランの過半が違反になる，あるいは 1 件も出ない) だけを
//! 確かめる．閾値の決定は M2・M3 で `testdata/lint-cases/` を揃えてから行う．

use std::path::{Path, PathBuf};

use pxsmith_core::geom::jaggy::{DEFAULT_MAX_MOVE, analyze_canvas};
use pxsmith_io::{Document, FrameId};

fn corpus() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/aseprite");
    let mut out = Vec::new();
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        paths.sort();
        for p in paths {
            if p.is_dir() {
                walk(&p, out);
            } else if matches!(
                p.extension().and_then(|e| e.to_str()),
                Some("aseprite") | Some("ase")
            ) {
                out.push(p);
            }
        }
    }
    walk(&dir, &mut out);
    out
}

#[test]
fn jaggy_detection_rate_on_real_sprites() {
    let mut total_runs = 0usize;
    let mut total_jaggies = 0usize;
    let mut total_fixable = 0usize;
    let mut files = 0usize;

    for path in corpus() {
        let Ok(doc) = Document::read(&path) else {
            continue;
        };
        let mut per_file = (0usize, 0usize, 0usize);
        for i in 0..doc.frame_count() {
            let Ok(frame) = doc.project(FrameId(i)) else {
                continue;
            };
            for layer in &frame.layers {
                let Some(canvas) = layer.surface.as_indexed() else {
                    continue;
                };
                let report = analyze_canvas(canvas, DEFAULT_MAX_MOVE);
                per_file.0 += report.runs;
                per_file.1 += report.jaggies.len();
                per_file.2 += report.fixable();
            }
        }
        if per_file.0 == 0 {
            continue;
        }
        files += 1;
        total_runs += per_file.0;
        total_jaggies += per_file.1;
        total_fixable += per_file.2;
        eprintln!(
            "{:<28} ラン {:>4} / 検出 {:>3} ({:>5.1}%) / うち直せる {:>3}",
            path.file_name().unwrap().to_string_lossy(),
            per_file.0,
            per_file.1,
            100.0 * per_file.1 as f32 / per_file.0 as f32,
            per_file.2,
        );
    }

    assert!(files > 0, "測れる素材が 1 つも無い");
    let rate = 100.0 * total_jaggies as f32 / total_runs as f32;
    eprintln!(
        "\n合計 {files} ファイル: ラン {total_runs} / 検出 {total_jaggies} ({rate:.1}%) / \
         うち移動上限内 {total_fixable}"
    );

    // 「全ランの過半が違反」はどう考えても検出が壊れている
    assert!(
        rate < 50.0,
        "検出が多すぎる ({rate:.1}%)．谷検出か単調区間分割が壊れている疑い"
    );
}
