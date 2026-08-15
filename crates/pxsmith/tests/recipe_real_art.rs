//! **レシピを実素材で端から端まで回す．**
//!
//! M5 の完了条件のうち，机上では確かめられないものをここで固定する．
//!
//! | 見るもの | 壊れると |
//! | --- | --- |
//! | `RAYON_NUM_THREADS` を変えて**バイト一致** | 並列度が出力に漏れている (規則 1 〜 3) |
//! | 2 度目が**全部キャッシュ当たり** | 何かがキーに漏れている (時刻 ・絶対パス ・反復順) |
//! | **移動しても当たる** | 絶対パスがキーに漏れている (6.15 の «パス ✕») |
//! | 入力を変えたら**外れる** | 差分ビルドの一番悪い壊れ方 (古い出力が返る) |
//! | キャッシュ当たりの木が**読める** | 宣言していない出力を貯め損ねている |
//! | 生成過程 GIF が**祖先の連鎖だけ** | 無関係な枝が混ざって «生成過程» でなくなる |
//!
//! **本物の `pxsmith` を起こして測る** — `RAYON_NUM_THREADS` はプロセスの始めにしか
//! 効かないので，同じプロセスの中では «並列度を変えた» ことにならない．

use std::path::{Path, PathBuf};
use std::process::Command;

fn seeds() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/grid-eval/seeds")
        .canonicalize()
        .expect("種の置き場所がある")
}

/// 使い捨てのプロジェクトを組む．
fn project(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("pxsmith-recipe-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).expect("作れる");
    std::fs::create_dir_all(root.join("out")).expect("作れる");
    for seed in ["crawl_urand_fencer", "crawl_saint_roka"] {
        std::fs::copy(
            seeds().join(format!("{seed}.png")),
            root.join("src").join(format!("{seed}.png")),
        )
        .expect("種を置ける");
    }
    std::fs::write(root.join("build.toml"), RECIPE).expect("レシピを置ける");
    root
}

const RECIPE: &str = r#"
[project]
format = 1

[vars]
seeds = ["crawl_urand_fencer", "crawl_saint_roka"]

[[step]]
op = "shade"
input = "src/${s}.png"
output = "out/${s}.px.toml"
base = "8A6A4A"
preset = "clear"
light = "dir:-0.6,0.8"
for_each = { s = "${seeds}" }

[[step]]
op = "anim.squash"
input = "out/crawl_urand_fencer.px.toml"
output = "out/squash.px.toml"
amount = -0.3
"#;

/// `pxsmith run` を本物のプロセスとして起こす．
fn run(root: &Path, threads: Option<&str>, extra: &[&str]) -> String {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_pxsmith"));
    cmd.arg("run").arg(root.join("build.toml"));
    cmd.args(extra);
    if let Some(t) = threads {
        cmd.env("RAYON_NUM_THREADS", t);
    }
    let out = cmd.output().expect("px を起こせる");
    assert!(
        out.status.success(),
        "pxsmith run が失敗した: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// 出力の «当たり» と «実行» の数を読む．
fn tally(text: &str) -> (usize, usize) {
    let line = text
        .lines()
        .find(|l| l.contains("キャッシュ当たり"))
        .unwrap_or_else(|| panic!("集計が出ていない:\n{text}"));
    let nums: Vec<usize> = line
        .split_whitespace()
        .filter_map(|w| w.trim_matches(|c: char| !c.is_ascii_digit()).parse().ok())
        .collect();
    // 「3 ステップ — キャッシュ当たり 3 ・実行 0」
    (nums[1], nums[2])
}

fn digest(root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let dir = root.join("out");
    let mut names: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("読める")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    names.sort();
    for path in names {
        let bytes = std::fs::read(&path).expect("読める");
        out.push((
            path.file_name().unwrap().to_string_lossy().to_string(),
            blake3::hash(&bytes).to_hex().to_string(),
        ));
    }
    out
}

/// **壊れると: 並列度が出力に漏れる (決定論性の 3 規則が破れている)．**
///
/// M5 の完了条件そのものである．
#[test]
fn the_output_is_byte_identical_whatever_the_thread_count() {
    let mut seen: Option<Vec<(String, String)>> = None;
    for threads in ["1", "2", "4", "8"] {
        let root = project(&format!("threads-{threads}"));
        // **キャッシュを参照しない** — 当たってしまうと «作り直した» ことにならない
        run(&root, Some(threads), &["--no-cache"]);
        let got = digest(&root);
        assert!(!got.is_empty(), "出力が 1 つも無い");
        match &seen {
            None => seen = Some(got),
            Some(want) => assert_eq!(&got, want, "スレッド {threads} で出力が違う"),
        }
    }
}

/// **壊れると: 何も変えていないのに毎回作り直す (差分ビルドが効いていない)．**
///
/// 外れるとしたら，時刻 ・絶対パス ・反復順のどれかがキーに漏れている．
#[test]
fn running_the_same_recipe_twice_hits_the_cache_every_time() {
    let root = project("twice");
    let (hits, misses) = tally(&run(&root, None, &[]));
    assert_eq!((hits, misses), (0, 3), "1 度目から当たっている");
    for round in 0..3 {
        let (hits, misses) = tally(&run(&root, None, &[]));
        assert_eq!(
            (hits, misses),
            (3, 0),
            "{round} 回目で外れた — 何かがキーに漏れている"
        );
    }
}

/// **壊れると: プロジェクトを移しただけでキャッシュが全部外れる．**
///
/// 6.15 の «パス ✕» が防ぎたいのはこれである．キャッシュごと移して当たること
/// を見る (キーが絶対パスを含んでいたら，ここで外れる) ．
#[test]
fn moving_the_whole_project_keeps_the_cache_warm() {
    let from = project("move-from");
    run(&from, None, &[]);
    let to = std::env::temp_dir().join(format!("pxsmith-recipe-{}-move-to", std::process::id()));
    let _ = std::fs::remove_dir_all(&to);
    copy_tree(&from, &to);

    let (hits, misses) = tally(&run(&to, None, &[]));
    assert_eq!(
        (hits, misses),
        (3, 0),
        "移したら外れた — 絶対パスがキーに漏れている"
    );
}

/// **壊れると: 入力を変えたのに古い出力が返る (差分ビルドの一番悪い壊れ方)．**
#[test]
fn changing_an_input_misses_the_cache_for_that_step_only() {
    let root = project("change");
    run(&root, None, &[]);

    // 片方の種を差し替える (中身が変わる)
    std::fs::copy(
        seeds().join("crawl_goblin.png"),
        root.join("src/crawl_saint_roka.png"),
    )
    .expect("差し替えられる");

    let (hits, misses) = tally(&run(&root, None, &[]));
    assert_eq!((hits, misses), (2, 1), "変えた 1 件だけが作り直されるはず");
}

/// **壊れると: キャッシュ当たりが «一部だけ戻った木» を作り，成功と報告する．**
///
/// `pxsmith shade` は L0 の隣に `.hex` も書く (D127) ．レシピが `output` に書いて
/// いないので，**宣言した分だけ貯めると次の当たりで木が壊れる**．
/// 実際に書かれたものを全部貯めていることを，**出力が読めるか**で見る．
#[test]
fn a_cache_hit_restores_a_tree_that_can_actually_be_read() {
    let root = project("sidecar");
    let text = run(&root, None, &[]);
    // 宣言していない出力があることを言っているか
    assert!(
        text.contains("output に書いていないファイル"),
        "宣言していない出力を黙っている:\n{text}"
    );

    // 出力を «宣言した分だけ» 残して消す
    for entry in std::fs::read_dir(root.join("out")).expect("読める") {
        let path = entry.expect("読める").path();
        if path.extension().is_some_and(|e| e == "hex") {
            std::fs::remove_file(&path).expect("消せる");
        }
    }
    let (hits, _) = tally(&run(&root, None, &[]));
    assert_eq!(hits, 3);

    // 戻った木が読めること — `.hex` が無ければ L0 は読めない
    let out = Command::new(env!("CARGO_BIN_EXE_pxsmith"))
        .arg("lint")
        .arg(root.join("out/crawl_urand_fencer.px.toml"))
        .output()
        .expect("px を起こせる");
    assert!(
        out.status.success(),
        "キャッシュから戻した木が読めない: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// **壊れると: `--gc` が今使っているキャッシュまで捨てる．**
#[test]
fn gc_keeps_what_the_current_recipe_needs() {
    let root = project("gc");
    run(&root, None, &[]);
    // 入力を変えて，古いキーを «使わないもの» にする
    std::fs::copy(
        seeds().join("crawl_goblin.png"),
        root.join("src/crawl_saint_roka.png"),
    )
    .expect("差し替えられる");
    let text = run(&root, None, &["--gc"]);
    assert!(text.contains("--gc"), "{text}");

    // 掃除の直後でも，今のレシピは全部当たる
    let (hits, misses) = tally(&run(&root, None, &[]));
    assert_eq!((hits, misses), (3, 0), "gc が使うものまで捨てた");
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("作れる");
    for entry in std::fs::read_dir(from).expect("読める") {
        let entry = entry.expect("読める");
        let path = entry.path();
        let dest = to.join(entry.file_name());
        if entry.metadata().expect("読める").is_dir() {
            copy_tree(&path, &dest);
        } else {
            std::fs::copy(&path, &dest).expect("写せる");
        }
    }
}

// ------------------------------------------------------------ 生成過程 GIF

/// GIF のコマを読む．
fn gif_frames(path: &Path) -> Vec<(u16, u16, u16)> {
    let file = std::fs::File::open(path).expect("開ける");
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::Indexed);
    let mut decoder = options.read_info(file).expect("読める");
    let mut out = Vec::new();
    while let Some(frame) = decoder.read_next_frame().expect("読める") {
        out.push((frame.width, frame.height, frame.delay));
    }
    out
}

/// **壊れると: 生成過程の GIF に無関係な枝が混ざる．**
///
/// レシピは 2 本の枝を持つ (fencer → squash と roka だけ) ．
/// `out/squash.px.toml` の生成過程は**祖先の 2 コマ**であって，3 コマではない．
#[test]
fn the_progress_gif_holds_the_ancestry_and_nothing_else() {
    let root = project("gif");
    let out = root.join("progress.gif");
    let text = run(
        &root,
        None,
        &[
            "--progress",
            out.to_str().expect("パス"),
            "--progress-of",
            "out/squash.px.toml",
        ],
    );
    assert!(text.contains("生成過程"), "{text}");

    let frames = gif_frames(&out);
    assert_eq!(frames.len(), 2, "祖先は 2 件のはず (roka の枝は入らない)");
    // 画布は広がった方に揃う (squash が 32x32 を 42x32 にする)
    for (w, h, _) in &frames {
        assert_eq!((*w, *h), (42, 32), "コマの寸法が揃っていない");
    }
    // 既定の表示時間 500 ms = 50 cs
    assert!(frames.iter().all(|(_, _, d)| *d == 50), "{frames:?}");
}

/// **壊れると: どの成果物の生成過程かを勝手に決めて，違う絵の GIF が出る．**
///
/// 行き止まりが 2 つあるレシピでは決められない — **推測せず選ばせる** (D111)．
#[test]
fn an_ambiguous_target_is_refused_with_the_choices_listed() {
    let root = project("gif-ambiguous");
    let out = root.join("progress.gif");
    let result = Command::new(env!("CARGO_BIN_EXE_pxsmith"))
        .arg("run")
        .arg(root.join("build.toml"))
        .arg("--progress")
        .arg(&out)
        .output()
        .expect("px を起こせる");
    assert!(!result.status.success(), "曖昧なまま書いてしまった");
    let text = String::from_utf8_lossy(&result.stderr);
    assert!(text.contains("--progress-of"), "{text}");
    assert!(
        text.contains("out/squash.px.toml"),
        "選択肢を出していない: {text}"
    );
}

/// **壊れると: GIF の 10 ms 刻みに乗らない表示時間が黙って変わる．**
///
/// D40 ・D116 で表示周期を扱ったのと同じ作法 — 丸めたら言う．
#[test]
fn a_delay_off_the_gif_grid_is_reported_rather_than_silently_rounded() {
    let root = project("gif-delay");
    let out = root.join("progress.gif");
    let text = run(
        &root,
        None,
        &[
            "--progress",
            out.to_str().expect("パス"),
            "--progress-of",
            "out/squash.px.toml",
            "--progress-delay",
            "333",
        ],
    );
    assert!(text.contains("330 ms"), "丸めたことを言っていない:\n{text}");
    assert!(gif_frames(&out).iter().all(|(_, _, d)| *d == 33));
}
