//! **`--allow-generate` の門が `gen` の配下すべてに掛かることを固定する** (D158)．
//!
//! D31 は «`op = "gen"` は既定でキャッシュ参照のみ» と決めている．op は
//! サブコマンドの木から引く (D130) ので，**`px gen` に枝を足すたびに新しい op
//! が生える** — 門を完全一致で書いていると，足した枝が素通りして
//! `--allow-generate` 無しで外部 API を叩けてしまう．
//!
//! M6 で `px gen prog` を足した瞬間に実際そうなった．**枝が増えても門が
//! 閉じていること**をここで固定する．
//!
//! 本物の `px` を起こして測る — 門は CLI の側にあり，ライブラリ試験では
//! 通らない道である (D81 «単体試験は通るのに CLI で落ちる» の裏返し)．

use std::path::PathBuf;
use std::process::Command;

/// `op = "gen.prog"` を 1 段だけ持つレシピ．
///
/// **叩き先を存在しない宛先にしてある** — 門が破れていたら «宛先に届かない»
/// で落ちるので，«門が閉じた» と «叩きに行った» が取り違えようがない．
const RECIPE: &str = r#"
[project]
format = 1

[[step]]
op = "gen.prog"
output = "out/x.px.toml"
prompt = "t"
palette = "1a1c2c,f4f4f4"
size = "8x8"
endpoint = "http://127.0.0.1:1/v1/messages"
"#;

fn project(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("px-gengate-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("out")).expect("作れる");
    std::fs::write(root.join("build.toml"), RECIPE).expect("レシピを置ける");
    root
}

fn run(root: &PathBuf, allow: bool) -> (bool, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_px"));
    cmd.current_dir(root).arg("run").arg("build.toml");
    if allow {
        cmd.arg("--allow-generate");
    }
    // **鍵を消してから起こす** — 門を抜けた先で本当に叩きに行かないように
    cmd.env_remove("ANTHROPIC_API_KEY");
    let out = cmd.output().expect("px を起こせる");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

/// **壊れると: `gen.prog` が `--allow-generate` 無しで外部 API を叩く．**
///
/// 門が完全一致 (`op == "gen"`) だと，M6 で足した枝がそのまま素通りする．
#[test]
fn a_gen_subcommand_is_gated_just_like_bare_gen() {
    let root = project("gated");
    let (ok, text) = run(&root, false);
    assert!(!ok, "門が開いている — 生成が走った:\n{text}");
    assert!(
        text.contains("--allow-generate"),
        "落ちた理由が «キャッシュに無い» になっていない:\n{text}"
    );
    // **叩きに行っていないこと**を確かめる — 宛先の話が出たら門を抜けている
    assert!(
        !text.contains("127.0.0.1"),
        "門を抜けて叩きに行っている:\n{text}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// **壊れると: 門を開けても «書いていない» で止まり，生成が一生走らない．**
///
/// 種類の付いていない `gen` は «書いていない» で正しいが，`gen.prog` は
/// 実行されなければならない．ここでは鍵を外してあるので，**鍵が無い**という
/// 生成側の理由で落ちるのが正しい — つまり門は抜けている．
#[test]
fn allowing_generation_lets_a_written_subcommand_actually_run() {
    let root = project("allowed");
    let (ok, text) = run(&root, true);
    assert!(!ok, "鍵が無いのに通ってしまった:\n{text}");
    assert!(
        !text.contains("--allow-generate"),
        "門で止まっている (抜けるべき):\n{text}"
    );
    assert!(
        text.contains("ANTHROPIC_API_KEY"),
        "生成側まで届いていない — 落ちた理由が鍵の話になっていない:\n{text}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
