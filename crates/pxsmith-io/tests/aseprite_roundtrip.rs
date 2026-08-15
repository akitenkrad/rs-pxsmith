//! `.aseprite` のバイト一致往復 (実装計画書 M0 の中核，R3・R15)．
//!
//! `testdata/aseprite/` に置いた実ファイル群に対し，読み込み → 書き出しが
//! **バイト単位で一致する**ことを確かめる．二層構造と R3 の対策はこの性質に
//! 全面的に依存しているため，ここが崩れたら設計を見直す必要がある．
//!
//! 素材が 1 つも無い場合は既定で警告を出して通す (素材調達は M0 と並行に進める
//! 作業のため)．`PXFORGE_REQUIRE_ASEPRITE_CORPUS=1` を立てると，素材が無いことを
//! 失敗として扱う — M0 の完了判定にはこちらを使う．

use std::path::{Path, PathBuf};

use pxsmith_io::{Document, FrameId};

fn testdata_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/aseprite")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from("testdata/aseprite"))
}

fn corpus() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        // 走査順を固定して，失敗の再現性を保つ
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
    let mut out = Vec::new();
    walk(&testdata_dir(), &mut out);
    out
}

fn require_corpus() -> bool {
    std::env::var("PXFORGE_REQUIRE_ASEPRITE_CORPUS").is_ok_and(|v| v == "1")
}

fn warn_or_fail(files: &[PathBuf]) -> bool {
    if !files.is_empty() {
        return true;
    }
    let dir = testdata_dir().display().to_string();
    assert!(
        !require_corpus(),
        "{dir} に .aseprite が 1 つも無い．\
         M0 の完了条件はこの往復検証なので，未知チャンク・タイルマップ・\
         リンクセルを含む実ファイルを置くこと"
    );
    eprintln!("警告: {dir} に .aseprite が無いため往復検証を飛ばした");
    false
}

#[test]
fn real_files_round_trip_byte_for_byte() {
    let files = corpus();
    if !warn_or_fail(&files) {
        return;
    }
    for path in &files {
        let original = std::fs::read(path).expect("読み込みに失敗");
        let doc = Document::from_bytes(&original)
            .unwrap_or_else(|e| panic!("{} の解釈に失敗した: {e}", path.display()));
        let written = doc.to_bytes().unwrap();
        assert_eq!(
            written.len(),
            original.len(),
            "{} の書き出しで長さが変わった",
            path.display()
        );
        if written != original {
            let at = written
                .iter()
                .zip(&original)
                .position(|(a, b)| a != b)
                .unwrap_or(0);
            panic!(
                "{} がバイト一致しない (最初の相違 = {at} バイト目)",
                path.display()
            );
        }
    }
    let independent = files.iter().filter(|p| is_independent(p)).count();
    eprintln!(
        "{} 件の .aseprite がバイト一致した (うち独立素材 {independent} 件)",
        files.len()
    );
    report_independence(&files);
}

/// **`aseprite-io` の fixtures 由来でない素材か** (R3 の残り．D167)．
fn is_independent(path: &Path) -> bool {
    path.components()
        .any(|c| c.as_os_str() == std::ffi::OsStr::new("independent"))
}

/// **«何件通った» と «何を独立に確かめた» を分けて言う** (D77 ・D104 ・D164 の作法)．
///
/// 19 件は Aseprite 公式のテストスプライトだが，取得元は **`aseprite-io` 自身の
/// `tests/fixtures/`** である．つまり `aseprite-io` はこの 19 件に対して
/// 開発されており，**ここが全部通ることは «忠実である» の独立な証拠にならない**．
/// 黙って «19 件通った» とだけ言うと，通った数が独立性の証拠に見える．
fn report_independence(files: &[PathBuf]) {
    if files.iter().any(|p| is_independent(p)) {
        return;
    }
    eprintln!(
        "** 独立素材が 0 件 — R3 は閉じていない **\n\
         \u{3000}\u{3000}この {} 件は aseprite-io 自身の fixtures 由来なので，\n\
         \u{3000}\u{3000}«aseprite-io が Aseprite に忠実か» を独立に確かめたことにはならない．\n\
         \u{3000}\u{3000}最新版 Aseprite で作ったファイルを testdata/aseprite/independent/ へ置く\n\
         \u{3000}\u{3000}(何を作ればよいかは同ディレクトリの README.md にある)",
        files.len()
    );
}

/// 射影 → 無変更の書き戻し → 書き出しでもバイトが変わらないこと．
///
/// 作業層を経由しても保持層が汚れないことが二層構造の要点である．
#[test]
fn projection_and_merge_back_preserve_bytes() {
    let files = corpus();
    if !warn_or_fail(&files) {
        return;
    }
    for path in &files {
        let original = std::fs::read(path).expect("読み込みに失敗");
        let mut doc = match Document::from_bytes(&original) {
            Ok(d) => d,
            Err(e) => panic!("{} の解釈に失敗した: {e}", path.display()),
        };
        for i in 0..doc.frame_count() {
            let frame = match doc.project(FrameId(i)) {
                Ok(f) => f,
                // 半透明パレットなど作業層の不変条件に合わない素材はここで弾かれる．
                // 往復そのものの検証は上のテストで済んでいるので，読み飛ばす．
                Err(e) => {
                    eprintln!("{} のフレーム {i} は射影できない: {e}", path.display());
                    continue;
                }
            };
            doc.merge_back(FrameId(i), &frame).unwrap_or_else(|e| {
                panic!("{} のフレーム {i} の書き戻しに失敗: {e}", path.display())
            });
        }
        assert_eq!(
            doc.to_bytes().unwrap(),
            original,
            "{} が射影と書き戻しでバイト一致しなくなった",
            path.display()
        );
    }
}
