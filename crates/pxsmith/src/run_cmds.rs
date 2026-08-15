//! `pxsmith run` — レシピ実行 (設計書 4.2 ・5 章 ・6.15)．
//!
//! # `op` からコマンド行を組む表は書かない
//!
//! 設計書 4.2 は «`px <group> <verb>` は `op = "<group>.<verb>"` に 1 対 1 対応
//! する» と定める．**その対応表を手で書くと，1 対 1 かどうかは «書いた人が
//! 正しかったか» になる．** clap の木からそのまま引けば，1 対 1 は構造として
//! 成り立つ — 引けなければ落ちるので，取り違えようがない．
//!
//! | 引くもの | どこから |
//! | --- | --- |
//! | サブコマンドがあるか | `Command::find_subcommand` |
//! | 位置引数の名前と順 | `Command::get_positionals` |
//! | 旗が値を取るか | `Arg::get_action` (`SetTrue` なら値を取らない) |
//! | 旗が繰り返せるか | `Arg::get_action` (`Append`) |
//!
//! これは D92 «数え上げで決まることは校正しない» と同じ性質の仕事である．
//!
//! # 述語は «前段が実体化してから» でないと答えられない
//!
//! 設計書 6.15 の注記どおり，組込み述語は入力データを見るので
//! $\mathrm{params}_i$ は前段の出力ができてはじめて確定する．したがって
//!
//! 1. **1 度目**は述語を «後回し» にして展開し，依存グラフだけを組む
//! 2. **実行の直前**に，そのステップの引数をもう 1 度評価する (述語つき)
//! 3. その引数でステップキーを作る
//!
//! **入力 ・出力のパスに述語は書けない** — 書けるとグラフが «実行してみないと
//! 分からない» ものになり，依存を先に並べられなくなる．そう言って落とす．

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, CommandFactory, Parser, Subcommand};
use pxsmith_recipe::cache::Cache;
use pxsmith_recipe::error::RecipeError;
use pxsmith_recipe::expr::{Env, Predicates};
use pxsmith_recipe::graph::Graph;
use pxsmith_recipe::key::{StepInputs, StepKey, Versions, step_key};
use pxsmith_recipe::recipe::{RECIPE_FORMAT, Recipe, ResolvedStep};
use pxsmith_recipe::value::Value;

#[derive(Args, Clone, Debug)]
pub struct RunArgs {
    /// レシピ (`.toml`)
    pub recipe: PathBuf,
    /// キャッシュを**参照しない**．**貯めるのは続ける** (次回まで遅くしないため)
    #[arg(long)]
    pub no_cache: bool,
    /// 今のレシピが使わないキャッシュを捨てる
    #[arg(long)]
    pub gc: bool,
    /// `op = "gen"` の生成を許す (既定はキャッシュ参照のみ．D31)
    #[arg(long)]
    pub allow_generate: bool,
    /// 実行せず，**何をどの順で回すか**だけを出す
    #[arg(long)]
    pub dry_run: bool,
    /// ステップキーとその材料を出す
    #[arg(long)]
    pub explain: bool,
    /// **生成過程の GIF** を書く (設計書 3 章)．中間成果物を順に連結する
    #[arg(long)]
    pub progress: Option<PathBuf>,
    /// どの成果物の生成過程か．**省くと行き止まりが 1 つのときだけ推せる**
    #[arg(long)]
    pub progress_of: Option<PathBuf>,
    /// 生成過程 GIF の 1 コマの表示時間 (ミリ秒)．**GIF は 10 ms 刻みしか持てない**
    #[arg(long, default_value_t = 500)]
    pub progress_delay: u32,
}

// ------------------------------------------------------------------ 述語

/// 組込み述語の対象 (設計書 4.2 の `PredicateTarget`)．
#[derive(Clone, Debug)]
enum Target {
    Input,
    Previous,
    Named(String),
}

impl Target {
    fn parse(s: &str) -> Self {
        match s {
            "input" => Self::Input,
            "previous" => Self::Previous,
            other => Self::Named(other.to_string()),
        }
    }
}

/// 設計書 4.2 の組込み述語 6 種．**これ以外は呼べない**．
const PREDICATES: &[&str] = &[
    "has_dither",
    "color_count",
    "size",
    "has_transparency",
    "is_indexed",
    "lint_clean",
];

/// 1 度目の展開で使う «後回し» の解決器．
///
/// 述語を**呼ばれたことだけ記録**して，空の値を返す．入力 ・出力のパスを
/// 評価している最中に呼ばれたら，そこで落とす材料になる．
struct Deferred {
    called: Cell<bool>,
}

impl Deferred {
    fn new() -> Self {
        Self {
            called: Cell::new(false),
        }
    }
}

impl Predicates for Deferred {
    fn eval(&self, _name: &str, _args: &[Value]) -> pxsmith_recipe::Result<Value> {
        self.called.set(true);
        // 1 度目は形だけ通す．**この値は捨てる**
        Ok(Value::Bool(false))
    }

    fn names(&self) -> Vec<&'static str> {
        PREDICATES.to_vec()
    }
}

/// 実行の直前に使う，本物の解決器．
struct Live<'a> {
    /// レシピの置き場所．
    root: &'a Path,
    /// このステップの入力．
    inputs: &'a [PathBuf],
    /// 直前に実体化した出力．
    previous: Option<PathBuf>,
}

impl Predicates for Live<'_> {
    fn eval(&self, name: &str, args: &[Value]) -> pxsmith_recipe::Result<Value> {
        let want = match name {
            "color_count" | "size" => 3,
            _ => 1,
        };
        if args.len() != want {
            return Err(RecipeError::PredicateArity {
                name: name.to_string(),
                want,
                got: args.len(),
            });
        }
        let target = Target::parse(&args[0].to_string());
        let path = self.resolve(name, &target)?;
        let full = self.root.join(&path);

        match name {
            "has_dither" => Ok(Value::Bool(has_dither(&full, name, &path)?)),
            "has_transparency" => Ok(Value::Bool(has_transparency(&full, name, &path)?)),
            "is_indexed" => Ok(Value::Bool(is_indexed(&full))),
            "lint_clean" => Ok(Value::Bool(lint_clean(&full, name, &path)?)),
            "color_count" => {
                let got = color_count(&full, name, &path)? as i64;
                compare(&args[1], got, &args[2], name)
            }
            "size" => {
                let (w, h) = size_of(&full, name, &path)?;
                // `size(input, '>', 32)` は «長辺» で比べる — 2 つの数を 1 つの
                // 順序で比べられないので，**どちらで比べたかを決めておく**
                compare(&args[1], w.max(h) as i64, &args[2], name)
            }
            _ => Err(RecipeError::ExprUnknownFunction {
                name: name.to_string(),
                known: PREDICATES.join(" ・"),
            }),
        }
    }

    fn names(&self) -> Vec<&'static str> {
        PREDICATES.to_vec()
    }
}

impl Live<'_> {
    fn resolve(&self, name: &str, target: &Target) -> pxsmith_recipe::Result<PathBuf> {
        match target {
            Target::Input => {
                self.inputs
                    .first()
                    .cloned()
                    .ok_or_else(|| RecipeError::PredicateNotMaterialised {
                        name: name.to_string(),
                        target: "input".into(),
                    })
            }
            Target::Previous => {
                self.previous
                    .clone()
                    .ok_or_else(|| RecipeError::PredicateNotMaterialised {
                        name: name.to_string(),
                        target: "previous".into(),
                    })
            }
            Target::Named(n) => {
                let p = PathBuf::from(n);
                if self.root.join(&p).exists() {
                    return Ok(p);
                }
                Err(RecipeError::PredicateBadTarget {
                    name: name.to_string(),
                    target: n.clone(),
                })
            }
        }
    }
}

fn compare(op: &Value, left: i64, right: &Value, name: &str) -> pxsmith_recipe::Result<Value> {
    let right = match right {
        Value::Int(v) => *v,
        other => {
            return Err(RecipeError::ExprBadType {
                op: format!("{name} の比較対象"),
                got: other.type_name().into(),
            });
        }
    };
    let out = match op.to_string().as_str() {
        "<" => left < right,
        "<=" => left <= right,
        ">" => left > right,
        ">=" => left >= right,
        "==" => left == right,
        "!=" => left != right,
        other => {
            return Err(RecipeError::ExprBadType {
                op: format!("{name} の比較子 '{other}'"),
                got: "比較子でない".into(),
            });
        }
    };
    Ok(Value::Bool(out))
}

fn load_indexed(
    path: &Path,
    name: &str,
    shown: &Path,
) -> pxsmith_recipe::Result<(
    pxsmith_core::canvas::IndexedCanvas,
    pxsmith_core::palette::Palette,
)> {
    let frames = crate::load_frames(path).map_err(|e| RecipeError::PredicateRead {
        name: name.to_string(),
        path: shown.display().to_string(),
        source: std::io::Error::other(e.to_string()),
    })?;
    let frame = frames
        .first()
        .ok_or_else(|| RecipeError::PredicateNotMaterialised {
            name: name.to_string(),
            target: shown.display().to_string(),
        })?;
    let canvas = frame
        .layers
        .iter()
        .find_map(|l| l.surface.as_indexed())
        .ok_or_else(|| RecipeError::PredicateNotMaterialised {
            name: name.to_string(),
            target: shown.display().to_string(),
        })?;
    Ok((canvas.clone(), frame.palette.clone()))
}

fn is_indexed(path: &Path) -> bool {
    crate::load_frames(path)
        .map(|f| {
            f.first()
                .is_some_and(|f| f.layers.iter().any(|l| l.surface.as_indexed().is_some()))
        })
        .unwrap_or(false)
}

fn has_dither(path: &Path, name: &str, shown: &Path) -> pxsmith_recipe::Result<bool> {
    let (canvas, _) = load_indexed(path, name, shown)?;
    let opts = pxsmith_core::clean::DenoiseOptions::default();
    Ok(!pxsmith_core::clean::detect_dither_noise(&canvas, &opts).is_empty())
}

fn has_transparency(path: &Path, name: &str, shown: &Path) -> pxsmith_recipe::Result<bool> {
    let (canvas, _) = load_indexed(path, name, shown)?;
    let Some(t) = canvas.transparent() else {
        return Ok(false);
    };
    Ok(canvas.pixels().contains(&t))
}

fn color_count(path: &Path, name: &str, shown: &Path) -> pxsmith_recipe::Result<usize> {
    let (canvas, _) = load_indexed(path, name, shown)?;
    let used: BTreeSet<u8> = canvas
        .pixels()
        .iter()
        .copied()
        .filter(|&i| canvas.transparent() != Some(i))
        .collect();
    Ok(used.len())
}

fn size_of(path: &Path, name: &str, shown: &Path) -> pxsmith_recipe::Result<(u32, u32)> {
    let (canvas, _) = load_indexed(path, name, shown)?;
    Ok((canvas.width(), canvas.height()))
}

fn lint_clean(path: &Path, name: &str, shown: &Path) -> pxsmith_recipe::Result<bool> {
    let (canvas, palette) = load_indexed(path, name, shown)?;
    let cfg = pxsmith_lint::LintConfig::default();
    let mut r = pxsmith_lint::rules::lint_palette(&palette, &cfg);
    r.extend(pxsmith_lint::lint_canvas(&canvas, &palette, &cfg));
    // **blocking だけを見る** (7.2 — advisory は報告のみ)
    Ok(r.blocking().count() == 0)
}

// -------------------------------------------------- op からコマンド行を組む

/// `op` を clap のサブコマンドの並びにする．
fn op_path(op: &str) -> Vec<String> {
    op.split('.').map(|s| s.to_string()).collect()
}

/// `op` に当たる clap のコマンドを引く．**無ければ何があるかを並べて落とす**．
fn find_command(op: &str) -> Result<clap::Command> {
    let root = crate::Cli::command();
    let mut here = root.clone();
    for part in op_path(op) {
        let Some(next) = here.find_subcommand(part.as_str()).cloned() else {
            bail!(
                "op '{op}' に当たるコマンドが無い ('{part}' が引けない)．\n\
                 op は px のサブコマンドと 1 対 1 である (設計書 4.2)．\n\
                 ここで使えるのは: {}",
                subcommand_names(&here).join(" ・")
            );
        };
        here = next;
    }
    Ok(here)
}

fn subcommand_names(cmd: &clap::Command) -> Vec<String> {
    let mut names: Vec<String> = cmd
        .get_subcommands()
        .map(|c| c.get_name().to_string())
        .filter(|n| n != "help")
        .collect();
    names.sort();
    names
}

/// レシピの欄の名前を clap の引数名に合わせる (`part_delay` → `part-delay`)．
fn flag_of(key: &str) -> String {
    key.replace('_', "-")
}

/// 評価済みのステップからコマンド行を組む．
///
/// 返すのは `["anim", "subpixel", "in.px.toml", "out.px.toml", "--fraction", "0.5"]`
/// の形 (先頭の `pxsmith` は含まない) ．
fn build_argv(op: &str, fields: &BTreeMap<String, Value>) -> Result<Vec<String>> {
    let cmd = find_command(op)?;
    let mut argv: Vec<String> = op_path(op);
    let mut used: BTreeSet<String> = BTreeSet::new();

    // 1. 位置引数 — **clap が持っている順**に埋める
    for arg in cmd.get_positionals() {
        let name = arg.get_id().as_str().to_string();
        let Some(value) = fields.get(&name) else {
            if arg.is_required_set() {
                bail!(
                    "op '{op}' には '{name}' が要る (位置引数)．レシピに {name} = ... を書くこと"
                );
            }
            continue;
        };
        used.insert(name);
        match value {
            Value::List(items) => argv.extend(items.iter().map(|v| v.to_argv())),
            other => argv.push(other.to_argv()),
        }
    }

    // 2. 旗 — **キー順**で回す (規則 1．並びが揺れるとキーも揺れる)
    for (key, value) in fields {
        if used.contains(key) {
            continue;
        }
        let flag = flag_of(key);
        // > [!warning] **clap の «id» と «長い名前» は同じとは限らない．**
        // > `pxsmith compose` の `--part` は id が `parts` である (`Vec` の欄名) ．
        // > id で引くと «そんな引数は無い» になる — レシピに書くのは利用者が
        // > 打つ名前 (長い名前) の方なので，**長い名前で引く**．
        let Some(arg) = cmd
            .get_arguments()
            .find(|a| a.get_long() == Some(flag.as_str()))
        else {
            let mut known: Vec<String> = cmd
                .get_arguments()
                .filter_map(|a| a.get_long().map(|l| l.replace('-', "_")))
                .filter(|n| n != "help" && n != "version")
                .collect();
            for a in cmd.get_positionals() {
                known.push(a.get_id().as_str().to_string());
            }
            known.sort();
            bail!(
                "op '{op}' に '{key}' という引数は無い．\n\
                 使えるのは: {}",
                known.join(" ・")
            );
        };
        let takes_value = !matches!(
            arg.get_action(),
            clap::ArgAction::SetTrue | clap::ArgAction::SetFalse
        );
        if !takes_value {
            // 真偽の旗は **true のときだけ** 置く
            match value.as_bool() {
                Some(true) => argv.push(format!("--{flag}")),
                Some(false) => {}
                None => bail!(
                    "op '{op}' の '{key}' は真偽で書くこと ({} が来た)",
                    value.type_name()
                ),
            }
            continue;
        }
        match value {
            Value::List(items) => {
                let repeatable = matches!(arg.get_action(), clap::ArgAction::Append);
                if repeatable {
                    for item in items {
                        argv.push(format!("--{flag}"));
                        argv.push(item.to_argv());
                    }
                } else {
                    // 値の区切りを持つ引数なら 1 つにまとめられる
                    argv.push(format!("--{flag}"));
                    argv.push(
                        items
                            .iter()
                            .map(|v| v.to_argv())
                            .collect::<Vec<_>>()
                            .join(","),
                    );
                }
            }
            other => {
                argv.push(format!("--{flag}"));
                argv.push(other.to_argv());
            }
        }
    }
    Ok(argv)
}

/// プロジェクトの下にあるファイルの «状態» を控える．
///
/// > [!warning] **宣言した出力だけを貯めると，キャッシュ当たりが «壊れた木» を作る．**
/// > `pxsmith shade out.px.toml` は隣に `out.px.hex` も書く (D127) ．レシピが
/// > `output` に書いていなければ，キャッシュから戻すのは `.px.toml` だけになり，
/// > **`pxsmith run` は «3 件当たり» と言って 0 で終わるのに，出力が読めない**．
/// > 端から端まで通して見つけた．
/// >
/// > 直し方を 2 つに分ける．
/// >
/// > | 何のため | どうするか |
/// > | --- | --- |
/// > | **戻すのを正しくする** | 実際に書かれたファイルを全部貯める |
/// > | **依存を正しくする** | 宣言していない出力を**報告する** (推測で辺は張らない) |
/// >
/// > 辺の方を推測しないのは，外れたときだけ静かに壊れるからである (D111 と同じ) ．
fn snapshot(root: &Path) -> BTreeMap<PathBuf, (u64, std::time::SystemTime)> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            let name = path.file_name().map(|s| s.to_string_lossy().to_string());
            // **キャッシュ自身は数えない** (数えると毎回«変わった»ことになる)
            if name.as_deref() == Some(".pxcache") {
                continue;
            }
            let Ok(meta) = e.metadata() else { continue };
            if meta.is_dir() {
                stack.push(path);
                continue;
            }
            let Ok(rel) = path.strip_prefix(root) else {
                continue;
            };
            let stamp = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
            out.insert(rel.to_path_buf(), (meta.len(), stamp));
        }
    }
    out
}

/// 実行の前後を比べて «このステップが書いたもの» を出す．
fn written_between(
    before: &BTreeMap<PathBuf, (u64, std::time::SystemTime)>,
    after: &BTreeMap<PathBuf, (u64, std::time::SystemTime)>,
) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = after
        .iter()
        .filter(|(path, now)| before.get(*path).is_none_or(|was| was != *now))
        .map(|(path, _)| path.clone())
        .collect();
    // **決定論的な順** — キャッシュのマニフェストの並びになる
    out.sort();
    out
}

// ---------------------------------------------------------------- 実行

/// 1 ステップの結果．
struct Outcome {
    key: StepKey,
    hit: bool,
    argv: Vec<String>,
}

pub fn run(args: &RunArgs) -> Result<()> {
    let recipe = Recipe::read(&args.recipe)?;
    let root = recipe.root.clone();

    // 1 度目 — 述語は後回しにして，依存グラフだけを組む
    let defer = Deferred::new();
    let steps = recipe.resolve(&defer)?;
    if defer.called.get() {
        // 入力 ・出力に述語が混ざっていないかは，パスを見れば分かる
        for s in &steps {
            for p in s.inputs.iter().chain(&s.outputs) {
                if p.as_os_str().is_empty() || p.to_string_lossy() == "false" {
                    bail!(
                        "{} の入力 ・出力に述語が使われている．\n\
                         述語は前段を実行してみないと答えが出ないので，\n\
                         依存グラフを先に組めなくなる — パスには書けない",
                        s.label()
                    );
                }
            }
        }
    }

    let graph = Graph::build(steps, &root)?;
    let cache = Cache::new(&root);
    let versions = Versions::new(env!("CARGO_PKG_VERSION"), RECIPE_FORMAT);

    println!(
        "{} — {} ステップ ・{} 段 ・外から来る入力 {} 件",
        args.recipe.display(),
        graph.steps.len(),
        graph.levels().len(),
        graph.sources.len()
    );

    // **生成過程の GIF に入れるステップを決める．**
    //
    // «この 1 枚がどうやってできたか» は祖先の連鎖であって，レシピ全体ではない
    // — 無関係な枝まで並べても «生成過程» にはならない．
    let progress_of: Option<BTreeSet<usize>> = match (&args.progress, &args.progress_of) {
        (None, _) => None,
        (Some(_), Some(path)) => {
            let at = graph
                .steps
                .iter()
                .position(|s| s.outputs.iter().any(|o| o == path))
                .with_context(|| {
                    format!(
                        "{} を作るステップが無い．レシピの output に書いてあるものを指すこと",
                        path.display()
                    )
                })?;
            Some(graph.ancestry(at))
        }
        (Some(_), None) => {
            let sinks = graph.sinks();
            match sinks.len() {
                1 => Some(graph.ancestry(sinks[0])),
                0 => bail!("行き止まりのステップが無いので，どの生成過程か決められない"),
                _ => {
                    // **推測しない．選ばせる** (D111 と同じ作法)
                    let names: Vec<String> = sinks
                        .iter()
                        .flat_map(|&at| graph.steps[at].outputs.iter())
                        .map(|p| p.display().to_string())
                        .collect();
                    bail!(
                        "行き止まりが {} 件あるので，どの成果物の生成過程かを決められない．\n\
                         --progress-of で 1 つ選ぶこと:\n  {}",
                        sinks.len(),
                        names.join("\n  ")
                    );
                }
            }
        }
    };

    if args.dry_run {
        for &at in &graph.order {
            let step = &graph.steps[at];
            let waits: Vec<String> = graph.deps[at]
                .iter()
                .map(|d| graph.steps[*d].label())
                .collect();
            println!(
                "  {} {}",
                step.label(),
                if waits.is_empty() {
                    String::new()
                } else {
                    format!("← {}", waits.join(" ・"))
                }
            );
        }
        println!("  --dry-run なので実行していない");
        return Ok(());
    }

    let mut keys: Vec<StepKey> = vec![String::new(); graph.steps.len()];
    let mut previous: Option<PathBuf> = None;
    let (mut hits, mut misses) = (0usize, 0usize);
    let mut undeclared: BTreeSet<(String, PathBuf)> = BTreeSet::new();
    let mut progress: Vec<pxsmith_io::gif::GifFrame> = Vec::new();
    let mut skipped_frames = 0usize;
    let mut outcomes: Vec<(usize, Outcome)> = Vec::new();

    for &at in &graph.order {
        let step = &graph.steps[at];
        // **実行の直前にもう 1 度評価する** — 述語は前段が実体化してから答えが出る
        let live = Live {
            root: &root,
            inputs: &step.inputs,
            previous: previous.clone(),
        };
        let fields = evaluate_fields(&recipe, step, &live)?;
        let argv = build_argv(&step.op, &fields)?;

        let deps: Vec<StepKey> = graph.deps[at].iter().map(|d| keys[*d].clone()).collect();
        let key = step_key(
            &StepInputs {
                op: &step.op,
                params: &fields,
                inputs: &step.inputs,
                outputs: &step.outputs,
                deps: &deps,
            },
            &versions,
            &root,
        )?;
        keys[at] = key.clone();

        let entry = if args.no_cache {
            None
        } else {
            cache.lookup(&key)
        };
        let hit = match entry {
            Some(e) if !step.outputs.is_empty() => {
                cache.restore(&e, &root)?;
                hits += 1;
                true
            }
            _ => {
                // **門は «gen» とその配下すべてに掛ける** (D158)．
                //
                // > [!warning] M6 で `pxsmith gen prog` を足した瞬間に穴が開いた．
                // > op はサブコマンドの木から引く (D130) ので，`gen.prog` は
                // > **足しただけで実行できる op になる** — 完全一致で見ている
                // > 門はそれを素通しし，`--allow-generate` 無しで外部 API を
                // > 叩けてしまう．**新しい枝が既存の門をすり抜ける**形で，
                // > D110 «決める場所が 2 つあると必ず食い違う» の裏返しである．
                if step.op == "gen" || step.op.starts_with("gen.") {
                    // **既定はキャッシュ参照のみ** (D31)
                    if !args.allow_generate {
                        return Err(RecipeError::GenNotCached { at: step.index }.into());
                    }
                    // 種類の付いていない `gen` は何を作るのか決まっていない
                    if step.op == "gen" {
                        return Err(RecipeError::GenNotWritten.into());
                    }
                    // `gen.prog` などは普通に実行し，普通にキャッシュへ貯める
                }
                let before = snapshot(&root);
                execute(&argv, &root).with_context(|| format!("{} が失敗した", step.label()))?;
                let produced = written_between(&before, &snapshot(&root));
                // **実際に書かれたものを全部貯める** — 宣言した分だけ貯めると，
                // 次のキャッシュ当たりで «一部だけ戻った木» ができる
                if !produced.is_empty() {
                    cache.store(&key, &root, &produced)?;
                }
                let declared: BTreeSet<&PathBuf> = step.outputs.iter().collect();
                for p in &produced {
                    if !declared.contains(p) {
                        undeclared.insert((step.label(), p.clone()));
                    }
                }
                misses += 1;
                false
            }
        };
        previous = step.outputs.first().cloned().or(previous);
        if progress_of.as_ref().is_some_and(|sel| sel.contains(&at)) {
            // **当たりでも実行でも同じように撮る** — 出力はどちらでも実体化している
            if let Some(f) = capture(&root, step, args.progress_delay) {
                progress.push(f);
            } else {
                skipped_frames += 1;
            }
        }
        outcomes.push((at, Outcome { key, hit, argv }));
    }

    println!(
        "\n  {} ステップ — キャッシュ当たり {hits} ・実行 {misses}",
        graph.steps.len()
    );
    if args.explain {
        println!("\n  ステップキー:");
        for (at, o) in &outcomes {
            println!(
                "    {} {} {}",
                &o.key[..12],
                if o.hit { "当" } else { "実" },
                graph.steps[*at].label()
            );
            println!("      px {}", o.argv.join(" "));
        }
    }
    if args.no_cache {
        println!("  --no-cache: 参照はしていないが，結果は貯めてある");
    }
    if !undeclared.is_empty() {
        // **推測で辺は張らない．言う** (D111 と同じ作法)
        println!(
            "\n  ** output に書いていないファイルが {} 件できている **",
            undeclared.len()
        );
        for (label, path) in &undeclared {
            println!("    {label} → {}", path.display());
        }
        println!(
            "    戻すのはキャッシュがやる (実際に書かれたものを貯めている) が，\n\
             **後のステップはこれを待たない**．待たせたいなら output に並べて書くこと"
        );
    }

    if let Some(path) = &args.progress {
        let report = pxsmith_io::gif::write_progress(path, &progress)
            .with_context(|| format!("{} を書けない", path.display()))?;
        println!(
            "\n  生成過程 -> {} ({} コマ ・{}x{} ・最大 {} 色)",
            path.display(),
            report.frames,
            report.size.0,
            report.size.1,
            report.max_colors
        );
        if report.padded > 0 {
            println!(
                "    画布の広がったコマが {} 件あるので，**左上を合わせて継ぎ足した**",
                report.padded
            );
        }
        for (want, got) in &report.rounded {
            // **黙って丸めない** (D40 ・D116 と同じ作法)
            println!("    表示時間 {want} ms は GIF の 10 ms 刻みに乗らないので {got} ms にした");
        }
        if skipped_frames > 0 {
            println!(
                "    絵として読めない出力が {skipped_frames} 件あったので飛ばした (JSON ・XML など)"
            );
        }
    }

    if args.gc {
        let keep: BTreeSet<String> = keys.iter().cloned().collect();
        let dropped = cache.gc(&keep)?;
        println!("  --gc: 使っていないキャッシュを {dropped} 件捨てた");
    }
    // **書いていないものを黙らない** (D92 の作法)
    println!("  書いていない: op = \"gen\" (M6) ・Cargo.lock のキーへの混入");
    Ok(())
}

/// ステップの出力を生成過程の 1 コマにする．
///
/// **最初に «絵として読めた» 出力を採る．** 読めないもの (正規 JSON ・`.tsx` など)
/// は飛ばして数える — 生成過程に «読めないもの» のコマは作れないが，
/// **飛ばしたことは黙らない**．
fn capture(root: &Path, step: &ResolvedStep, delay_ms: u32) -> Option<pxsmith_io::gif::GifFrame> {
    for rel in &step.outputs {
        let full = root.join(rel);
        let Ok(frames) = crate::load_frames(&full) else {
            continue;
        };
        let Some(frame) = frames.first() else {
            continue;
        };
        let canvas = frame.layers.iter().find_map(|l| l.surface.as_indexed())?;
        return Some(pxsmith_io::gif::GifFrame {
            canvas: canvas.clone(),
            palette: frame.palette.clone(),
            delay_ms,
            label: format!("{} {}", step.label(), rel.display()),
        });
    }
    None
}

/// ステップの欄を «述語つき» でもう 1 度評価する．
fn evaluate_fields(
    recipe: &Recipe,
    step: &ResolvedStep,
    preds: &dyn Predicates,
) -> Result<BTreeMap<String, Value>> {
    let mut env: Env = recipe.vars.clone();
    for (k, v) in &step.bindings {
        env.insert(k.clone(), v.clone());
    }
    let source = &recipe.steps[step.index];
    let mut out = BTreeMap::new();
    for (key, field) in &source.fields {
        out.insert(
            key.clone(),
            pxsmith_recipe::recipe::eval_field_pub(field, &env, preds)?,
        );
    }
    Ok(out)
}

/// コマンド行を**同じプロセスの中で**実行する．
///
/// 外のプロセスを起こさないのは，**決定論性を測れるようにするため**である —
/// 別プロセスにすると `RAYON_NUM_THREADS` の効き方が読めなくなる．
fn execute(argv: &[String], root: &Path) -> Result<()> {
    let mut full: Vec<String> = vec!["px".to_string()];
    // レシピからの相対パスで書いてあるので，レシピの置き場所を基準にする
    let here = std::env::current_dir().context("今の場所が分からない")?;
    std::env::set_current_dir(root).with_context(|| format!("{} へ移れない", root.display()))?;
    full.extend(argv.iter().cloned());
    let parsed = crate::Cli::try_parse_from(&full);
    let outcome = match parsed {
        Ok(cli) => crate::dispatch(cli.command),
        Err(e) => Err(anyhow::anyhow!("コマンド行を組み違えた: {e}")),
    };
    std::env::set_current_dir(&here).ok();
    outcome
}

#[derive(Subcommand)]
pub enum RecipeCommand {
    /// 外部データ (CSV / JSON) から `[vars]` を埋めたレシピを書き出す
    Expand {
        #[command(flatten)]
        args: ExpandArgs,
    },
}

#[derive(Args, Clone, Debug)]
pub struct ExpandArgs {
    /// もとにするレシピ
    pub input: PathBuf,
    pub output: PathBuf,
    /// `.csv` か `.json`．列名 (キー) が変数名になり，値が列になる
    #[arg(long)]
    pub data: PathBuf,
}

pub fn recipe(command: RecipeCommand) -> Result<()> {
    match command {
        RecipeCommand::Expand { args } => expand(&args),
    }
}

/// 外部データからレシピを作る (設計書 4.2 «外部データ参照は pxsmith recipe expand に分離する»)．
///
/// **レシピ本体はデータを読まない．** 読めるようにすると，ステップキーの材料が
/// «実行時に外から来るもの» に広がって差分ビルドが成り立たなくなる．
///
/// > [!warning] **行は直積ではない．**
/// > 最初は列ごとに `[vars]` の列を作っていた．`s = [...]` ・`tint = [...]` と
/// > 並べて `for_each = { s = "${s}" }` で回す形である．**これは動かない** —
/// > CSV の行は «s と tint の組» なのに，`[vars]` に分けた時点で組が消えるからで，
/// > `base = "${tint}"` が列そのものになって `--base 8A6A4A,4A6A8A` が出る
/// > (端から端まで通して落ちた) ．
/// >
/// > 直積展開を «行» に使い回すことはできない．**言語に zip を足す**か
/// > **展開の側で行を開く**かだが，設計書 4.2 の «許すもの» を広げない方を採った —
/// > `expand` は**行ごとに `[[step]]` を書き下す**．
/// > «外部データからのレシピ生成» とは，まさにそういう仕事である．
fn expand(args: &ExpandArgs) -> Result<()> {
    let text = std::fs::read_to_string(&args.data)
        .with_context(|| format!("{} を読めない", args.data.display()))?;
    let name = args.data.to_string_lossy();
    let rows: Vec<BTreeMap<String, String>> = if name.ends_with(".json") {
        from_json(&text)?
    } else {
        from_csv(&text)?
    };
    if rows.is_empty() {
        bail!("{} から行を 1 つも読めなかった", args.data.display());
    }

    let base = std::fs::read_to_string(&args.input)
        .with_context(|| format!("{} を読めない", args.input.display()))?;
    let doc: toml::Value = toml::from_str(&base)
        .with_context(|| format!("{} を TOML として読めない", args.input.display()))?;
    let template = doc
        .get("step")
        .and_then(|v| v.as_array())
        .context("もとのレシピに [[step]] が無い")?;

    // **行 x ステップの順に書き下す** — 行が外なので，同じ行のステップが隣り合う
    let mut steps: Vec<toml::Value> = Vec::with_capacity(rows.len() * template.len());
    for row in &rows {
        for step in template {
            steps.push(substitute(step, row));
        }
    }

    let mut out = toml::value::Table::new();
    for (k, v) in doc.as_table().context("一番外が表でない")? {
        if k == "step" {
            continue;
        }
        out.insert(k.clone(), v.clone());
    }
    out.insert("step".to_string(), toml::Value::Array(steps));

    let body =
        toml::to_string_pretty(&toml::Value::Table(out)).context("レシピを TOML にできない")?;
    let text = format!(
        "# pxsmith recipe expand が {} から書き下した ({} 行 x {} ステップ)\n{body}",
        args.data.display(),
        rows.len(),
        template.len()
    );
    std::fs::write(&args.output, &text)
        .with_context(|| format!("{} を書き出せない", args.output.display()))?;

    let columns: BTreeSet<&String> = rows.iter().flat_map(|r| r.keys()).collect();
    println!(
        "{} + {} -> {} ({} 行 x {} ステップ = {} ステップ)",
        args.input.display(),
        args.data.display(),
        args.output.display(),
        rows.len(),
        template.len(),
        rows.len() * template.len()
    );
    println!(
        "  差し替えた列: {}",
        columns
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(" ・")
    );
    println!("  行は直積ではないので **書き下している** — レシピ側の for_each とは別物である");
    Ok(())
}

/// `${列名}` を行の値で差し替える．**列に無い `${...}` はそのまま残す**
/// (レシピの `[vars]` を使う式かもしれないため)．
fn substitute(value: &toml::Value, row: &BTreeMap<String, String>) -> toml::Value {
    match value {
        toml::Value::String(s) => {
            let mut out = s.clone();
            for (k, v) in row {
                out = out.replace(&format!("${{{k}}}"), v);
            }
            toml::Value::String(out)
        }
        toml::Value::Array(items) => {
            toml::Value::Array(items.iter().map(|i| substitute(i, row)).collect())
        }
        toml::Value::Table(t) => toml::Value::Table(
            t.iter()
                .map(|(k, v)| (k.clone(), substitute(v, row)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn from_csv(text: &str) -> Result<Vec<BTreeMap<String, String>>> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header: Vec<String> = lines
        .next()
        .context("CSV に見出し行が無い")?
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();
    let mut out = Vec::new();
    for line in lines {
        let cells: Vec<&str> = line.split(',').collect();
        let mut row = BTreeMap::new();
        for (at, name) in header.iter().enumerate() {
            row.insert(
                name.clone(),
                cells.get(at).map(|c| c.trim()).unwrap_or("").to_string(),
            );
        }
        out.push(row);
    }
    Ok(out)
}

fn from_json(text: &str) -> Result<Vec<BTreeMap<String, String>>> {
    let doc: serde_json::Value = serde_json::from_str(text).context("JSON として読めない")?;
    let rows = doc.as_array().context("JSON の一番外は配列であること")?;
    let mut out = Vec::new();
    for row in rows {
        let table = row.as_object().context("配列の要素は表であること")?;
        out.push(
            table
                .iter()
                .map(|(k, v)| {
                    let text = match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    (k.clone(), text)
                })
                .collect(),
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **壊れると: 設計書 4.2 の «op はサブコマンドと 1 対 1» が成り立たなくなる．**
    ///
    /// 対応表を手で書いていないことの確認でもある — clap の木を歩いて，
    /// **すべての葉が op として引ける**ことを数え上げる (D92 と同じ性質) ．
    #[test]
    fn every_subcommand_is_reachable_as_an_op() {
        fn walk(cmd: &clap::Command, prefix: &str, out: &mut Vec<String>) {
            let subs: Vec<&clap::Command> = cmd
                .get_subcommands()
                .filter(|c| c.get_name() != "help")
                .collect();
            if subs.is_empty() {
                if !prefix.is_empty() {
                    out.push(prefix.to_string());
                }
                return;
            }
            for s in subs {
                let next = if prefix.is_empty() {
                    s.get_name().to_string()
                } else {
                    format!("{prefix}.{}", s.get_name())
                };
                walk(s, &next, out);
            }
        }
        let mut ops = Vec::new();
        walk(&crate::Cli::command(), "", &mut ops);
        assert!(ops.len() > 20, "サブコマンドが少なすぎる: {}", ops.len());
        for op in &ops {
            find_command(op).unwrap_or_else(|e| panic!("op '{op}' が引けない: {e}"));
        }
        // 設計書 4.2 が名指ししている op が実在すること
        for op in ["compose", "anim.subpixel", "sheet.pack", "export.tiled"] {
            assert!(ops.contains(&op.to_string()), "{op} が無い (ops: {ops:?})");
        }
    }

    /// **壊れると: 位置引数の順を取り違え，入力と出力が入れ替わる．**
    ///
    /// 順は clap から引いている — 手で書いた表ではない．
    #[test]
    fn the_positional_order_comes_from_clap() {
        let fields: BTreeMap<String, Value> = [
            ("input".to_string(), Value::from("in.px.toml")),
            ("output".to_string(), Value::from("out.px.toml")),
            ("amount".to_string(), Value::Float(-0.3)),
        ]
        .into_iter()
        .collect();
        let argv = build_argv("anim.squash", &fields).expect("組める");
        assert_eq!(
            argv,
            vec![
                "anim",
                "squash",
                "in.px.toml",
                "out.px.toml",
                "--amount",
                "-0.3"
            ]
        );

        // compose は **出力が先**で入力の位置引数を持たない
        let fields: BTreeMap<String, Value> = [
            ("output".to_string(), Value::from("hero.aseprite")),
            (
                "part".to_string(),
                Value::List(vec![Value::from("a.px.toml"), Value::from("b.px.toml")]),
            ),
        ]
        .into_iter()
        .collect();
        let argv = build_argv("compose", &fields).expect("組める");
        assert_eq!(argv[0], "compose");
        assert_eq!(argv[1], "hero.aseprite");
        assert!(argv.contains(&"--part".to_string()));
    }

    /// **壊れると: 真偽の旗に値が付いて clap が落ちる．**
    #[test]
    fn a_boolean_flag_is_placed_without_a_value() {
        let fields: BTreeMap<String, Value> = [
            ("input".to_string(), Value::from("a.png")),
            ("output".to_string(), Value::from("b.png")),
            ("remove_aa".to_string(), Value::Bool(true)),
            ("denoise_dither".to_string(), Value::Bool(false)),
        ]
        .into_iter()
        .collect();
        let argv = build_argv("clean", &fields).expect("組める");
        assert!(argv.contains(&"--remove-aa".to_string()));
        assert!(!argv.contains(&"--denoise-dither".to_string()));
        // 旗の次に値が来ていないこと
        let at = argv.iter().position(|a| a == "--remove-aa").expect("ある");
        assert!(argv.get(at + 1).is_none_or(|v| v.starts_with("--")));
    }

    /// **壊れると: レシピの打ち間違いが黙って無視される．**
    #[test]
    fn an_unknown_field_says_what_is_accepted() {
        let fields: BTreeMap<String, Value> = [
            ("input".to_string(), Value::from("a.png")),
            ("output".to_string(), Value::from("b.png")),
            ("frcation".to_string(), Value::Float(0.5)),
        ]
        .into_iter()
        .collect();
        let err = build_argv("anim.subpixel", &fields)
            .unwrap_err()
            .to_string();
        assert!(err.contains("frcation"), "{err}");
        assert!(err.contains("fraction"), "何が使えるか言っていない: {err}");
    }

    /// **壊れると: 存在しない op が «空のコマンド» として通る．**
    #[test]
    fn an_unknown_op_lists_the_ones_that_exist() {
        let err = find_command("anim.wobble").unwrap_err().to_string();
        assert!(err.contains("wobble"), "{err}");
        assert!(err.contains("subpixel"), "何があるか言っていない: {err}");
    }

    /// **壊れると: 同じレシピから違うコマンド行が出て，キャッシュが外れる．**
    #[test]
    fn the_argv_is_the_same_every_time() {
        let fields: BTreeMap<String, Value> = [
            ("input".to_string(), Value::from("in.px.toml")),
            ("output".to_string(), Value::from("out.px.toml")),
            ("amount".to_string(), Value::Float(-0.3)),
            ("anchor".to_string(), Value::from("bottom")),
            ("rule".to_string(), Value::from("derived")),
        ]
        .into_iter()
        .collect();
        let first = build_argv("anim.squash", &fields).expect("組める");
        for _ in 0..8 {
            assert_eq!(build_argv("anim.squash", &fields).expect("組める"), first);
        }
    }
}
