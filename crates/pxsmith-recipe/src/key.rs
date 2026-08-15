//! ステップキー (設計書 6.15)．
//!
//! $$ h_i = H\big(h_{i-1} \parallel H(\mathrm{params}_i) \parallel H(\mathrm{versions}) \parallel \textstyle\bigparallel_{f \in \mathrm{inputs}_i} H(f)\big) $$
//!
//! # 鎖ではなく «依存のキー» を混ぜる
//!
//! 設計書の式は $h_{i-1}$ と書いていて**一直線の鎖**を前提にしている．
//! しかしレシピは有向非巡回グラフなので，前が 1 つとは限らない．
//! ここでは $h_{i-1}$ を**そのステップが待っているステップのキーを昇順に並べたもの**
//! に読み替える．鎖はその特別な場合 (依存がちょうど 1 つ) である．
//!
//! **昇順に並べるのは決定論性の規則 1** — 依存を集める順は実行のたびに違いうる．
//!
//! # パスは «レシピからの相対» だけを混ぜる
//!
//! 6.15 の表は «パス ✕» と書いている．そのまま読むと出力パスをキーから外すことに
//! なるが，**それでは足りない場面がある** — `save_frames` は L0 のパレット参照に
//! **出力ファイル名を書き込む** (D127) ので，出力の名前が変われば中身も変わる．
//! パスをまるごと外すと，`a.px.toml` と `b.px.toml` が同じキーになって
//! **中身が取り違えられる**．
//!
//! かといって絶対パスを混ぜると，プロジェクトを別の場所へ移した瞬間に
//! キャッシュが全部外れる — «パス ✕» が防ぎたいのはそれである．
//!
//! そこで**レシピの置き場所からの相対パスだけ**を混ぜる．
//!
//! | やること | 結果 |
//! | --- | --- |
//! | プロジェクトを別のディレクトリへ移す | **キーは変わらない** |
//! | 出力の名前を変える | **キーが変わる** |
//! | mtime だけ変える (touch) | **キーは変わらない** |
//!
//! この 3 つは試験で固定してある．
//!
//! # `Cargo.lock` は混ぜていない (D92 の作法)
//!
//! 6.15 は «ツール版 + `Cargo.lock`» と書いているが，**入れているのはツール版だけ**
//! である — 配ったバイナリの隣に `Cargo.lock` は無いので，実行時に読むと
//! «ある環境では混ざり，ある環境では混ざらない» という一番悪い形になる．
//! 依存の版が変わったのにツール版が同じなら取りこぼす — **その穴は開いている**．
//! 埋めるならビルド時に埋め込む必要があり，そこは書いていない．

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::{RecipeError, Result};
use crate::value::Value;

/// キー 1 つ (16 進 64 桁)．
pub type StepKey = String;

/// キーに混ぜる «版» の情報．
#[derive(Clone, Debug)]
pub struct Versions {
    /// ツールの版 (`CARGO_PKG_VERSION`)．
    pub tool: String,
    /// レシピのスキーマ版．
    pub schema: u32,
    /// lint 設定など，呼ぶ側が足したいもの．**キー順で混ぜる**．
    pub extra: BTreeMap<String, String>,
}

impl Versions {
    pub fn new(tool: &str, schema: u32) -> Self {
        Self {
            tool: tool.to_string(),
            schema,
            extra: BTreeMap::new(),
        }
    }

    fn hash(&self) -> blake3::Hash {
        let mut h = blake3::Hasher::new();
        h.update(b"versions\0");
        h.update(self.tool.as_bytes());
        h.update(b"\0");
        h.update(self.schema.to_le_bytes().as_slice());
        for (k, v) in &self.extra {
            h.update(b"\0");
            h.update(k.as_bytes());
            h.update(b"=");
            h.update(v.as_bytes());
        }
        h.finalize()
    }
}

/// キーを作るのに要るものを 1 つにまとめたもの．
#[derive(Clone, Debug)]
pub struct StepInputs<'a> {
    pub op: &'a str,
    /// 評価済みの引数．**キー順で混ぜる** (規則 1)．
    pub params: &'a BTreeMap<String, Value>,
    /// レシピからの相対パス．
    pub inputs: &'a [PathBuf],
    /// レシピからの相対パス．
    pub outputs: &'a [PathBuf],
    /// 待っているステップのキー．**呼ぶ側が昇順に渡す必要はない** (ここで並べる)．
    pub deps: &'a [StepKey],
}

/// ステップキーを作る．
///
/// `root` はレシピの置き場所で，入力ファイルを読むのに使う．
/// **パスは `root` からの相対のまま混ぜる** (モジュールの説明) ．
pub fn step_key(at: &StepInputs<'_>, versions: &Versions, root: &Path) -> Result<StepKey> {
    let mut h = blake3::Hasher::new();

    // 1. 依存のキー (昇順)．鎖はこの特別な場合である
    let mut deps: Vec<&StepKey> = at.deps.iter().collect();
    deps.sort();
    h.update(b"deps\0");
    for d in deps {
        h.update(d.as_bytes());
        h.update(b"\0");
    }

    // 2. params — op ・引数 ・出力の «名前» まで含める
    h.update(b"params\0");
    h.update(at.op.as_bytes());
    for (k, v) in at.params {
        h.update(b"\0");
        h.update(k.as_bytes());
        h.update(b"=");
        h.update(v.to_argv().as_bytes());
    }
    h.update(b"\0outputs\0");
    for o in at.outputs {
        h.update(slashed(o).as_bytes());
        h.update(b"\0");
    }

    // 3. versions
    h.update(b"versions\0");
    h.update(versions.hash().as_bytes());

    // 4. 入力ファイルの **中身**．名前も混ぜる (どの入力がどれかで結果が変わる)
    h.update(b"inputs\0");
    for f in at.inputs {
        h.update(slashed(f).as_bytes());
        h.update(b"=");
        h.update(file_hash(&root.join(f), f)?.as_bytes());
        h.update(b"\0");
    }

    Ok(h.finalize().to_hex().to_string())
}

/// ファイルの中身のハッシュ．
///
/// **mtime も所有者も見ない** — 中身だけである (6.15 の表) ．
pub fn file_hash(path: &Path, shown: &Path) -> Result<String> {
    let bytes = std::fs::read(path).map_err(|source| RecipeError::CacheRead {
        path: shown.display().to_string(),
        source,
    })?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

/// パスを `/` 区切りの文字列にする．
///
/// **windows と mac で同じキーにするため** — 区切り文字が混ざるとキーが変わる．
fn slashed(p: &Path) -> String {
    p.components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pxsmith-key-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("作れる");
        dir
    }

    fn params(pairs: &[(&str, &str)]) -> BTreeMap<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), Value::from(*v)))
            .collect()
    }

    fn key_in(root: &Path, out: &str, params: &BTreeMap<String, Value>) -> String {
        let inputs = vec![PathBuf::from("a.png")];
        let outputs = vec![PathBuf::from(out)];
        step_key(
            &StepInputs {
                op: "shade",
                params,
                inputs: &inputs,
                outputs: &outputs,
                deps: &[],
            },
            &Versions::new("0.1.0", 1),
            root,
        )
        .expect("キーが作れる")
    }

    /// **壊れると: 何も変えていないのにキャッシュが外れる．**
    #[test]
    fn the_same_step_gives_the_same_key_every_time() {
        let root = tmp("same");
        std::fs::write(root.join("a.png"), b"hello").expect("書ける");
        let p = params(&[("base", "8A6A4A")]);
        let first = key_in(&root, "o.aseprite", &p);
        for _ in 0..8 {
            assert_eq!(key_in(&root, "o.aseprite", &p), first);
        }
    }

    /// **壊れると: プロジェクトを移動しただけでキャッシュが全部外れる．**
    ///
    /// 6.15 の «パス ✕» が防ぎたいのはこれである．
    #[test]
    fn moving_the_project_does_not_change_the_key() {
        let p = params(&[("base", "8A6A4A")]);
        let a = tmp("move-a");
        let b = tmp("move-b");
        std::fs::write(a.join("a.png"), b"hello").expect("書ける");
        std::fs::write(b.join("a.png"), b"hello").expect("書ける");
        assert_eq!(key_in(&a, "o.aseprite", &p), key_in(&b, "o.aseprite", &p));
    }

    /// **壊れると: 出力の名前を変えても古い中身が返る．**
    ///
    /// `save_frames` は L0 のパレット参照に**出力ファイル名を書く** (D127) ので，
    /// 名前が変われば中身も変わる．**パスをまるごとキーから外してはいけない．**
    #[test]
    fn renaming_the_output_changes_the_key() {
        let root = tmp("rename");
        std::fs::write(root.join("a.png"), b"hello").expect("書ける");
        let p = params(&[("base", "8A6A4A")]);
        assert_ne!(
            key_in(&root, "a.px.toml", &p),
            key_in(&root, "b.px.toml", &p)
        );
    }

    /// **壊れると: 触っただけ (mtime) で全部やり直しになる．**
    #[test]
    fn touching_a_file_does_not_change_the_key() {
        let root = tmp("touch");
        let file = root.join("a.png");
        std::fs::write(&file, b"hello").expect("書ける");
        let p = params(&[("base", "8A6A4A")]);
        let before = key_in(&root, "o.aseprite", &p);
        // 中身を変えずに書き直す = mtime だけが動く
        std::fs::write(&file, b"hello").expect("書ける");
        assert_eq!(key_in(&root, "o.aseprite", &p), before);
    }

    /// **壊れると: 入力が変わったのに古い出力が返る (差分ビルドの一番悪い壊れ方)．**
    #[test]
    fn every_ingredient_moves_the_key() {
        let root = tmp("ingredients");
        let file = root.join("a.png");
        std::fs::write(&file, b"hello").expect("書ける");
        let p = params(&[("base", "8A6A4A")]);
        let base = key_in(&root, "o.aseprite", &p);

        // 入力の中身
        std::fs::write(&file, b"world").expect("書ける");
        assert_ne!(key_in(&root, "o.aseprite", &p), base, "入力の中身");
        std::fs::write(&file, b"hello").expect("書ける");

        // 引数
        assert_ne!(
            key_in(&root, "o.aseprite", &params(&[("base", "112233")])),
            base,
            "引数"
        );
        // 引数の名前 (値が同じでも)
        assert_ne!(
            key_in(&root, "o.aseprite", &params(&[("tint", "8A6A4A")])),
            base,
            "引数の名前"
        );
        // 版
        let inputs = vec![PathBuf::from("a.png")];
        let outputs = vec![PathBuf::from("o.aseprite")];
        let other = step_key(
            &StepInputs {
                op: "shade",
                params: &p,
                inputs: &inputs,
                outputs: &outputs,
                deps: &[],
            },
            &Versions::new("0.2.0", 1),
            &root,
        )
        .expect("キー");
        assert_ne!(other, base, "ツール版");
        // op
        let other = step_key(
            &StepInputs {
                op: "aa",
                params: &p,
                inputs: &inputs,
                outputs: &outputs,
                deps: &[],
            },
            &Versions::new("0.1.0", 1),
            &root,
        )
        .expect("キー");
        assert_ne!(other, base, "op");
        // 依存のキー
        let other = step_key(
            &StepInputs {
                op: "shade",
                params: &p,
                inputs: &inputs,
                outputs: &outputs,
                deps: &["deadbeef".to_string()],
            },
            &Versions::new("0.1.0", 1),
            &root,
        )
        .expect("キー");
        assert_ne!(other, base, "依存のキー");
    }

    /// **壊れると: 依存を集める順でキーが変わり，並列にすると外れる．**
    #[test]
    fn the_order_dependencies_arrive_in_does_not_matter() {
        let root = tmp("deps");
        std::fs::write(root.join("a.png"), b"hello").expect("書ける");
        let p = params(&[("base", "8A6A4A")]);
        let inputs = vec![PathBuf::from("a.png")];
        let outputs = vec![PathBuf::from("o.aseprite")];
        let mk = |deps: &[StepKey]| {
            step_key(
                &StepInputs {
                    op: "shade",
                    params: &p,
                    inputs: &inputs,
                    outputs: &outputs,
                    deps,
                },
                &Versions::new("0.1.0", 1),
                &root,
            )
            .expect("キー")
        };
        let a = mk(&["aaa".to_string(), "bbb".to_string()]);
        let b = mk(&["bbb".to_string(), "aaa".to_string()]);
        assert_eq!(a, b);
    }
}
