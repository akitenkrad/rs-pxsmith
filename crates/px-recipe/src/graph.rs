//! 依存グラフ (設計書 5 章 `px run`)．
//!
//! 辺は**宣言された出力**からしか生えない — ステップ $B$ の `input` が
//! ステップ $A$ の `output` と文字列として一致したら $A \to B$ である．
//!
//! # 有向非巡回でなければならない
//!
//! 循環があれば «どちらが先か» が決まらず，ステップキーの連鎖 (6.15) も
//! 定義できない．**循環は落とす** — 適当な順で回して «それらしく» 動かすと，
//! 実行のたびに違う絵が出る．
//!
//! # 並べる順も決定論的である
//!
//! Kahn の算法で回すが，**同時に実行できるものが複数あるときは
//! `(step 番号, 直積展開の番号)` の小さい順に取る** (決定論性の規則 2 —
//! 同点にタイブレークを付ける) ．並列に走らせても «並べた順» は変わらない．

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use crate::error::{RecipeError, Result};
use crate::recipe::{ResolvedStep, declared_outputs};

/// 依存グラフ．
#[derive(Clone, Debug)]
pub struct Graph {
    /// ステップ列 (展開済み)．
    pub steps: Vec<ResolvedStep>,
    /// `deps[i]` = ステップ `i` が待つステップの番号 (昇順)．
    pub deps: Vec<Vec<usize>>,
    /// 実行順 (トポロジカル順序)．
    pub order: Vec<usize>,
    /// レシピの外から来る入力 (どのステップも作らないファイル)．
    pub sources: BTreeSet<PathBuf>,
}

impl Graph {
    /// 展開済みのステップ列からグラフを組む．
    ///
    /// `root` はレシピの置き場所で，**相対パスの基準**である．
    pub fn build(steps: Vec<ResolvedStep>, root: &Path) -> Result<Self> {
        let producers = declared_outputs(&steps)?;
        let mut deps: Vec<Vec<usize>> = vec![Vec::new(); steps.len()];
        let mut sources = BTreeSet::new();

        for (at, step) in steps.iter().enumerate() {
            for path in &step.inputs {
                match producers.get(path) {
                    Some(&from) => {
                        if from == at {
                            return Err(RecipeError::GraphCycle {
                                cycle: format!("{} が自分の出力を読んでいる", step.label()),
                            });
                        }
                        deps[at].push(from);
                    }
                    None => {
                        // どのステップも作らないなら，レシピの外から来る入力である．
                        // **無ければ落とす** — 作るステップを書き忘れたのか，
                        // ファイルを置き忘れたのかは，人にしか分からない
                        if !root.join(path).exists() {
                            return Err(RecipeError::GraphMissingInput {
                                at,
                                op: step.op.clone(),
                                path: path.display().to_string(),
                            });
                        }
                        sources.insert(path.clone());
                    }
                }
            }
            deps[at].sort_unstable();
            deps[at].dedup();
        }

        let order = topological(&deps, &steps)?;
        Ok(Self {
            steps,
            deps,
            order,
            sources,
        })
    }

    /// あるステップと，それが (間接にでも) 待っているステップ全部．
    ///
    /// **生成過程の GIF はここを使う** — 「この 1 枚がどうやってできたか」は
    /// 祖先の連鎖であって，レシピ全体ではない (無関係な枝まで並べても
    /// «生成過程» にならない) ．
    pub fn ancestry(&self, at: usize) -> BTreeSet<usize> {
        let mut seen = BTreeSet::new();
        let mut stack = vec![at];
        while let Some(now) = stack.pop() {
            if !seen.insert(now) {
                continue;
            }
            stack.extend(self.deps[now].iter().copied());
        }
        seen
    }

    /// 誰にも読まれない出力を持つステップ (**行き止まり**)．
    ///
    /// `--progress-of` を省いたときの既定を決めるのに使う．
    /// **2 つ以上あるなら選ばせる** — どれの生成過程かは推測できない．
    pub fn sinks(&self) -> Vec<usize> {
        let mut consumed: BTreeSet<&PathBuf> = BTreeSet::new();
        for step in &self.steps {
            consumed.extend(step.inputs.iter());
        }
        (0..self.steps.len())
            .filter(|&at| {
                let outs = &self.steps[at].outputs;
                !outs.is_empty() && outs.iter().all(|o| !consumed.contains(o))
            })
            .collect()
    }

    /// 何段の «波» に分かれるか (並列に回せる塊の数)．報告に出す．
    pub fn levels(&self) -> Vec<Vec<usize>> {
        let mut level: Vec<usize> = vec![0; self.steps.len()];
        for &at in &self.order {
            let mut want = 0usize;
            for &d in &self.deps[at] {
                want = want.max(level[d] + 1);
            }
            level[at] = want;
        }
        let depth = level.iter().copied().max().map(|v| v + 1).unwrap_or(0);
        let mut out = vec![Vec::new(); depth];
        for &at in &self.order {
            out[level[at]].push(at);
        }
        out
    }
}

/// Kahn の算法．**同時に取れるものは番号の小さい順**．
fn topological(deps: &[Vec<usize>], steps: &[ResolvedStep]) -> Result<Vec<usize>> {
    let n = deps.len();
    let mut remaining: Vec<usize> = deps.iter().map(|d| d.len()).collect();
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (at, ds) in deps.iter().enumerate() {
        for &d in ds {
            dependents[d].push(at);
        }
    }

    // **順序付きの集合で持つ** — `VecDeque` に押した順ではなく番号順で取る
    let mut ready: BTreeSet<usize> = (0..n).filter(|&i| remaining[i] == 0).collect();
    let mut out = Vec::with_capacity(n);
    let mut queue: VecDeque<usize> = VecDeque::new();
    while !ready.is_empty() {
        let at = *ready.iter().next().expect("空でない");
        ready.remove(&at);
        out.push(at);
        for &next in &dependents[at] {
            remaining[next] -= 1;
            if remaining[next] == 0 {
                ready.insert(next);
            }
        }
        queue.clear();
    }

    if out.len() != n {
        let stuck: Vec<String> = (0..n)
            .filter(|&i| remaining[i] > 0)
            .map(|i| steps[i].label())
            .collect();
        return Err(RecipeError::GraphCycle {
            cycle: stuck.join(" → "),
        });
    }
    Ok(out)
}

/// ステップ番号から «そのステップが作るもの» を引く表．
pub fn outputs_by_step(steps: &[ResolvedStep]) -> BTreeMap<usize, Vec<PathBuf>> {
    steps
        .iter()
        .enumerate()
        .map(|(at, s)| (at, s.outputs.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expr::NoPredicates;
    use crate::recipe::Recipe;

    fn graph_of(text: &str, root: &Path) -> Result<Graph> {
        let r = Recipe::parse(text, root, "t.toml").expect("読める");
        Graph::build(r.resolve(&NoPredicates).expect("展開"), root)
    }

    fn tmp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("px-graph-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("作れる");
        dir
    }

    /// **壊れると: 依存を無視した順で回り，前段の出力が無いまま次が走る．**
    #[test]
    fn a_step_waits_for_the_one_that_writes_its_input() {
        let root = tmp();
        std::fs::write(root.join("a.png"), b"x").expect("書ける");
        let text = r#"
[project]
format = 1
[[step]]
op = "shade"
input = "a.png"
output = "b.aseprite"
[[step]]
op = "aa"
input = "b.aseprite"
output = "c.aseprite"
"#;
        let g = graph_of(text, &root).expect("組める");
        assert_eq!(g.deps[1], vec![0]);
        assert_eq!(g.order, vec![0, 1]);
        assert_eq!(g.levels().len(), 2);
        assert!(g.sources.contains(&PathBuf::from("a.png")));
    }

    /// **壊れると: 循環したレシピが «それらしく» 動き，実行ごとに違う絵が出る．**
    #[test]
    fn a_cycle_is_refused() {
        let root = tmp();
        let text = r#"
[project]
format = 1
[[step]]
op = "aa"
input = "b.aseprite"
output = "a.aseprite"
[[step]]
op = "aa"
input = "a.aseprite"
output = "b.aseprite"
"#;
        assert!(matches!(
            graph_of(text, &root),
            Err(RecipeError::GraphCycle { .. })
        ));
    }

    /// **壊れると: 自分の出力を読むステップが無限に «最新» でなくなる．**
    #[test]
    fn a_step_may_not_read_what_it_writes() {
        let root = tmp();
        let text = r#"
[project]
format = 1
[[step]]
op = "aa"
input = "a.aseprite"
output = "a.aseprite"
"#;
        assert!(matches!(
            graph_of(text, &root),
            Err(RecipeError::GraphCycle { .. })
        ));
    }

    /// **壊れると: 入力の書き忘れが «空のファイルを読んだ» として通る．**
    #[test]
    fn an_input_that_nobody_writes_and_does_not_exist_is_an_error() {
        let root = tmp();
        let text = r#"
[project]
format = 1
[[step]]
op = "aa"
input = "missing-xyz.aseprite"
output = "o.aseprite"
"#;
        assert!(matches!(
            graph_of(text, &root),
            Err(RecipeError::GraphMissingInput { .. })
        ));
    }

    /// **壊れると: 生成過程の GIF に無関係な枝が混ざる．**
    ///
    /// «この 1 枚がどうやってできたか» は祖先の連鎖であって，レシピ全体ではない．
    #[test]
    fn the_ancestry_of_a_step_leaves_out_the_unrelated_branches() {
        let root = tmp();
        std::fs::write(root.join("a.png"), b"x").expect("書ける");
        std::fs::write(root.join("z.png"), b"y").expect("書ける");
        let text = r#"
[project]
format = 1
[[step]]
op = "shade"
input = "a.png"
output = "b.aseprite"
[[step]]
op = "aa"
input = "b.aseprite"
output = "c.aseprite"
[[step]]
op = "shade"
input = "z.png"
output = "unrelated.aseprite"
"#;
        let g = graph_of(text, &root).expect("組める");
        assert_eq!(g.ancestry(1), [0usize, 1].into_iter().collect());
        assert_eq!(g.ancestry(2), [2usize].into_iter().collect());
        // 行き止まりは 2 つ (c.aseprite と unrelated.aseprite)
        assert_eq!(g.sinks(), vec![1, 2]);
    }

    /// **壊れると: 並べる順が実行ごとに変わり，キャッシュも報告も揺れる．**
    #[test]
    fn the_order_is_the_same_every_time() {
        let root = tmp();
        std::fs::write(root.join("a.png"), b"x").expect("書ける");
        let text = r#"
[project]
format = 1
[vars]
names = ["c", "a", "b"]
[[step]]
op = "shade"
input = "a.png"
output = "out_${n}.aseprite"
for_each = { n = "${names}" }
[[step]]
op = "aa"
input = "out_a.aseprite"
output = "final.aseprite"
"#;
        let first = graph_of(text, &root).expect("組める").order;
        for _ in 0..8 {
            assert_eq!(graph_of(text, &root).expect("組める").order, first);
        }
        // 直積展開は書いた順 (c ・a ・b)．最後の集約はそのあと
        assert_eq!(first, vec![0, 1, 2, 3]);
    }
}
