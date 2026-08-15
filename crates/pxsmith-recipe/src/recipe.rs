//! レシピの形式と検証 (設計書 4.2)．
//!
//! ```toml
//! [project]
//! format = 1
//! palette = "palettes/aurora.hex"
//!
//! [vars]
//! equips = ["sword", "axe", "none"]
//! base_frames = 4
//!
//! [[step]]
//! op = "compose"
//! input = "parts/hero_${equip}.px.toml"
//! output = "out/hero_${equip}.aseprite"
//! for_each = { equip = "${equips}" }
//! ```
//!
//! # 依存の辺は «宣言した出力» からしか生えない
//!
//! `output` に書いたものだけが «このステップが作るもの» である．
//! 後のステップのどれかの値がその文字列と一致したら，そこに辺を張る．
//!
//! **`sheet.pack` の `meta` のように «op が書くがここに宣言していない» ファイルは
//! 追跡しない．** 推測して辺を張ると，**当たっているときは何も起きず，外れた
//! ときだけ静かに壊れる** (設計書 6.8 の «47 枚が静かに壊れる» と同じ形) ．
//! 追跡したいなら `output = ["dist/hero.png", "dist/hero.json"]` と並べて書く．
//!
//! なお**キーの方は取りこぼさない** — `meta` はステップの引数なので
//! $\mathrm{params}$ に入り，値が変われば必ずキーが変わる (6.15) ．
//! 宣言し忘れて困るのは «後のステップが待ってくれない» ことだけである．
//!
//! # 版が違う文書は黙って読まない
//!
//! `[project] format` が違えば落とす (D110 で正規 JSON に課したのと同じ作法) ．

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::error::{RecipeError, Result};
use crate::expr::{Env, Predicates, has_interpolation, interpolate};
use crate::value::Value;

/// このツールが読めるレシピのスキーマ版．
pub const RECIPE_FORMAT: u32 = 1;

/// `[project]`．
#[derive(Clone, Debug, Default)]
pub struct Project {
    pub format: u32,
    pub palette: Option<String>,
    /// `constraints = { max_colors = 32, ... }`．**キー順で持つ** (キーが揺れないため)．
    pub constraints: BTreeMap<String, i64>,
}

/// `[[step]]` 1 つ (展開する前)．
#[derive(Clone, Debug)]
pub struct Step {
    /// `op = "anim.subpixel"`．
    pub op: String,
    /// `for_each = { equip = "${equips}" }`．
    pub for_each: BTreeMap<String, String>,
    /// `op` と `for_each` を除いた残り全部．**評価する前の字面**である．
    pub fields: BTreeMap<String, Field>,
}

/// レシピに書ける値の字面．
#[derive(Clone, Debug, PartialEq)]
pub enum Field {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    List(Vec<Field>),
    /// `part_delay = { torso = 1 }` のような表．
    Table(BTreeMap<String, Field>),
}

/// 読み込んだレシピ．
#[derive(Clone, Debug)]
pub struct Recipe {
    pub project: Project,
    pub vars: Env,
    pub steps: Vec<Step>,
    /// レシピそのものの置き場所 (相対パスの基準)．
    pub root: PathBuf,
}

/// 直積展開したあとのステップ 1 つ．
#[derive(Clone, Debug)]
pub struct ResolvedStep {
    /// 何番目の `[[step]]` か．
    pub index: usize,
    /// 直積展開の何番目か (`for_each` が無ければ 0)．
    pub instance: usize,
    pub op: String,
    /// 評価済みの引数．**`op` ・`for_each` ・`input` ・`output` は含まない**．
    pub params: BTreeMap<String, Value>,
    /// このステップが読むもの．
    pub inputs: Vec<PathBuf>,
    /// このステップが作るもの (宣言したものだけ)．
    pub outputs: Vec<PathBuf>,
    /// 直積展開で束ねた変数 (報告に出す)．
    pub bindings: BTreeMap<String, Value>,
}

impl ResolvedStep {
    /// 報告に出す名前．
    pub fn label(&self) -> String {
        if self.bindings.is_empty() {
            return format!("[{}] {}", self.index, self.op);
        }
        let parts: Vec<String> = self
            .bindings
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        format!("[{}] {} ({})", self.index, self.op, parts.join(" "))
    }
}

// ------------------------------------------------------------------ 読み込み

impl Recipe {
    /// レシピの置き場所．
    ///
    /// > [!warning] **`parent()` は裸のファイル名に `None` を返さない．**
    /// > `Path::new("build.toml").parent()` は `Some("")` なので，
    /// > `unwrap_or(".")` の守りは**一度も働かない** — そして空の場所へは
    /// > 移れないので，`pxsmith run build.toml` (手引きが載せているそのままの形) が
    /// > 落ちていた．**起きない場合を守って，起きる場合を素通りさせていた**
    /// > 形で，D80 ・D145 «補助関数が構造的に空だった» と同じである．
    /// >
    /// > 既存の試験が絶対パスしか渡していなかったので気付けなかった．
    pub(crate) fn root_of(path: &Path) -> PathBuf {
        match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from("."),
        }
    }

    pub fn read(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|source| RecipeError::RecipeRead {
            path: path.display().to_string(),
            source,
        })?;
        Self::parse(&text, &Self::root_of(path), &path.display().to_string())
    }

    pub fn parse(text: &str, root: &Path, name: &str) -> Result<Self> {
        let doc: toml::Value = toml::from_str(text).map_err(|source| RecipeError::RecipeParse {
            path: name.to_string(),
            source: Box::new(source),
        })?;

        let project = match doc.get("project") {
            Some(t) => Project {
                format: t.get("format").and_then(|v| v.as_integer()).unwrap_or(0) as u32,
                palette: t
                    .get("palette")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                constraints: t
                    .get("constraints")
                    .and_then(|v| v.as_table())
                    .map(|t| {
                        t.iter()
                            .filter_map(|(k, v)| v.as_integer().map(|n| (k.clone(), n)))
                            .collect()
                    })
                    .unwrap_or_default(),
            },
            None => Project::default(),
        };
        // **版が違う文書は黙って読まない**
        if project.format != RECIPE_FORMAT {
            return Err(RecipeError::RecipeVersion {
                got: project.format,
                want: RECIPE_FORMAT,
            });
        }

        let mut vars = Env::new();
        if let Some(table) = doc.get("vars").and_then(|v| v.as_table()) {
            for (k, v) in table {
                vars.insert(k.clone(), toml_to_value(v, k)?);
            }
        }

        let raw = doc
            .get("step")
            .and_then(|v| v.as_array())
            .ok_or(RecipeError::RecipeNoSteps)?;
        if raw.is_empty() {
            return Err(RecipeError::RecipeNoSteps);
        }

        let mut steps = Vec::with_capacity(raw.len());
        for (at, item) in raw.iter().enumerate() {
            let table = item.as_table().ok_or_else(|| RecipeError::RecipeBadField {
                at,
                op: "?".into(),
                key: "step".into(),
                got: "表でない".into(),
            })?;
            let op = table
                .get("op")
                .and_then(|v| v.as_str())
                .ok_or_else(|| RecipeError::RecipeBadField {
                    at,
                    op: "?".into(),
                    key: "op".into(),
                    got: "無い".into(),
                })?
                .to_string();

            let mut for_each = BTreeMap::new();
            if let Some(t) = table.get("for_each").and_then(|v| v.as_table()) {
                for (k, v) in t {
                    let s = v.as_str().ok_or_else(|| RecipeError::RecipeBadField {
                        at,
                        op: op.clone(),
                        key: format!("for_each.{k}"),
                        got: "文字列でない".into(),
                    })?;
                    for_each.insert(k.clone(), s.to_string());
                }
            }

            let mut fields = BTreeMap::new();
            for (k, v) in table {
                if k == "op" || k == "for_each" {
                    continue;
                }
                fields.insert(k.clone(), toml_to_field(v, at, &op, k)?);
            }
            steps.push(Step {
                op,
                for_each,
                fields,
            });
        }

        Ok(Recipe {
            project,
            vars,
            steps,
            root: root.to_path_buf(),
        })
    }

    /// 直積展開して，評価済みのステップ列にする．
    ///
    /// **順序は決定論的である** — `for_each` のキーは辞書順，値は書いた順で
    /// 回すので，同じレシピからは必ず同じ並びが出る (規則 1) ．
    pub fn resolve(&self, preds: &dyn Predicates) -> Result<Vec<ResolvedStep>> {
        let mut out = Vec::new();
        for (index, step) in self.steps.iter().enumerate() {
            let combos = self.combinations(index, step, preds)?;
            for (instance, bindings) in combos.into_iter().enumerate() {
                let mut env = self.vars.clone();
                for (k, v) in &bindings {
                    env.insert(k.clone(), v.clone());
                }
                let mut params = BTreeMap::new();
                let mut inputs = Vec::new();
                let mut outputs = Vec::new();
                for (key, field) in &step.fields {
                    let value = eval_field(field, &env, preds)?;
                    match key.as_str() {
                        "input" => collect_paths(&value, &mut inputs),
                        "output" => collect_paths(&value, &mut outputs),
                        _ => {
                            params.insert(key.clone(), value);
                        }
                    }
                }
                out.push(ResolvedStep {
                    index,
                    instance,
                    op: step.op.clone(),
                    params,
                    inputs,
                    outputs,
                    bindings,
                });
            }
        }
        Ok(out)
    }

    /// `for_each` の直積．
    fn combinations(
        &self,
        at: usize,
        step: &Step,
        preds: &dyn Predicates,
    ) -> Result<Vec<BTreeMap<String, Value>>> {
        if step.for_each.is_empty() {
            return Ok(vec![BTreeMap::new()]);
        }
        let mut combos: Vec<BTreeMap<String, Value>> = vec![BTreeMap::new()];
        // **キーは辞書順** (`BTreeMap` の反復順) — 直積の並びが揺れないため
        for (key, src) in &step.for_each {
            let value = interpolate(src, &self.vars, preds)?;
            let items = value
                .as_list()
                .ok_or_else(|| RecipeError::RecipeForEachNotList {
                    at,
                    op: step.op.clone(),
                    key: key.clone(),
                    got: value.type_name().into(),
                })?;
            let mut next = Vec::with_capacity(combos.len() * items.len());
            for base in &combos {
                for item in items {
                    let mut c = base.clone();
                    c.insert(key.clone(), item.clone());
                    next.push(c);
                }
            }
            combos = next;
        }
        Ok(combos)
    }
}

fn collect_paths(value: &Value, out: &mut Vec<PathBuf>) {
    match value {
        Value::List(items) => {
            for i in items {
                collect_paths(i, out);
            }
        }
        other => out.push(PathBuf::from(other.to_string())),
    }
}

/// 欄 1 つを評価する (`pxsmith run` が実行の直前にもう 1 度呼ぶ)．
pub fn eval_field_pub(field: &Field, env: &Env, preds: &dyn Predicates) -> Result<Value> {
    eval_field(field, env, preds)
}

fn eval_field(field: &Field, env: &Env, preds: &dyn Predicates) -> Result<Value> {
    Ok(match field {
        Field::Str(s) => {
            if has_interpolation(s) {
                interpolate(s, env, preds)?
            } else {
                Value::Str(s.clone())
            }
        }
        Field::Int(v) => Value::Int(*v),
        Field::Float(v) => Value::Float(*v),
        Field::Bool(v) => Value::Bool(*v),
        Field::List(items) => Value::List(
            items
                .iter()
                .map(|i| eval_field(i, env, preds))
                .collect::<Result<Vec<_>>>()?,
        ),
        // 表は `key=value` の列にする (`--part-delay 'torso=1'` の形)．
        // **キー順で回す**ので並びが揺れない
        Field::Table(t) => Value::List(
            t.iter()
                .map(|(k, v)| eval_field(v, env, preds).map(|x| Value::Str(format!("{k}={x}"))))
                .collect::<Result<Vec<_>>>()?,
        ),
    })
}

fn toml_to_value(v: &toml::Value, name: &str) -> Result<Value> {
    Ok(match v {
        toml::Value::String(s) => Value::Str(s.clone()),
        toml::Value::Integer(n) => Value::Int(*n),
        toml::Value::Float(f) => Value::Float(*f),
        toml::Value::Boolean(b) => Value::Bool(*b),
        toml::Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for i in items {
                // **列の入れ子は書けない** — 直積展開が «列の列» を扱えないので，
                // 黙って平らにせず落とす
                if i.is_array() {
                    return Err(RecipeError::RecipeNestedList {
                        name: name.to_string(),
                    });
                }
                out.push(toml_to_value(i, name)?);
            }
            Value::List(out)
        }
        other => Value::Str(other.to_string()),
    })
}

fn toml_to_field(v: &toml::Value, at: usize, op: &str, key: &str) -> Result<Field> {
    Ok(match v {
        toml::Value::String(s) => Field::Str(s.clone()),
        toml::Value::Integer(n) => Field::Int(*n),
        toml::Value::Float(f) => Field::Float(*f),
        toml::Value::Boolean(b) => Field::Bool(*b),
        toml::Value::Array(items) => Field::List(
            items
                .iter()
                .map(|i| toml_to_field(i, at, op, key))
                .collect::<Result<Vec<_>>>()?,
        ),
        toml::Value::Table(t) => Field::Table(
            t.iter()
                .map(|(k, v)| toml_to_field(v, at, op, key).map(|f| (k.clone(), f)))
                .collect::<Result<BTreeMap<_, _>>>()?,
        ),
        toml::Value::Datetime(_) => {
            return Err(RecipeError::RecipeBadField {
                at,
                op: op.to_string(),
                key: key.to_string(),
                got: "日時".into(),
            });
        }
    })
}

/// 宣言された出力を集める (重複はエラー)．
pub fn declared_outputs(steps: &[ResolvedStep]) -> Result<BTreeMap<PathBuf, usize>> {
    let mut out: BTreeMap<PathBuf, usize> = BTreeMap::new();
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    for (at, step) in steps.iter().enumerate() {
        for path in &step.outputs {
            if !seen.insert(path.clone()) {
                return Err(RecipeError::GraphDuplicateOutput {
                    path: path.display().to_string(),
                    first: out[path],
                    second: at,
                });
            }
            out.insert(path.clone(), at);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expr::NoPredicates;

    const SAMPLE: &str = r#"
[project]
format = 1
palette = "palettes/aurora.hex"
constraints = { max_colors = 32, colors_per_tile = 4 }

[vars]
equips = ["sword", "axe", "none"]
base_frames = 4

[[step]]
op = "compose"
input = "parts/hero_${equip}.px.toml"
output = "out/hero_${equip}.aseprite"
frames = "${base_frames * 2}"
for_each = { equip = "${equips}" }

[[step]]
op = "anim.subpixel"
input = "out/hero_sword.aseprite"
output = "out/sub.aseprite"
fraction = 0.5
"#;

    fn parse(text: &str) -> Result<Recipe> {
        Recipe::parse(text, Path::new("."), "test.toml")
    }

    /// **壊れると: 設計書 4.2 のレシピが読めない．**
    #[test]
    fn the_recipe_in_the_design_parses_and_expands() {
        let r = parse(SAMPLE).expect("読める");
        assert_eq!(r.project.format, 1);
        assert_eq!(r.project.constraints["max_colors"], 32);
        assert_eq!(r.steps.len(), 2);

        let steps = r.resolve(&NoPredicates).expect("展開");
        // 1 つ目は 3 通りに展開され，2 つ目は 1 つのまま
        assert_eq!(steps.len(), 4);
        assert_eq!(steps[0].bindings["equip"], Value::from("sword"));
        assert_eq!(
            steps[0].inputs,
            vec![PathBuf::from("parts/hero_sword.px.toml")]
        );
        assert_eq!(steps[0].params["frames"], Value::Int(8));
        assert_eq!(steps[2].bindings["equip"], Value::from("none"));
        assert_eq!(steps[3].op, "anim.subpixel");
    }

    /// **壊れると: 同じレシピから違う順序のビルドが出て，キャッシュが外れる．**
    #[test]
    fn expansion_is_deterministic() {
        let r = parse(SAMPLE).expect("読める");
        let first: Vec<String> = r
            .resolve(&NoPredicates)
            .unwrap()
            .iter()
            .map(|s| s.label())
            .collect();
        for _ in 0..8 {
            let again: Vec<String> = r
                .resolve(&NoPredicates)
                .unwrap()
                .iter()
                .map(|s| s.label())
                .collect();
            assert_eq!(first, again);
        }
    }

    /// **壊れると: 版が違うレシピを黙って読み，違う意味で動く (D110 と同じ作法)．**
    #[test]
    fn a_recipe_of_another_schema_version_is_refused() {
        let text = SAMPLE.replace("format = 1", "format = 2");
        assert!(matches!(
            parse(&text),
            Err(RecipeError::RecipeVersion { got: 2, want: 1 })
        ));
        // 版が無いものも読まない
        let text = SAMPLE.replace("format = 1\n", "");
        assert!(matches!(
            parse(&text),
            Err(RecipeError::RecipeVersion { .. })
        ));
    }

    /// **壊れると: 直積展開に列でないものを渡して 1 通りだけ回る．**
    #[test]
    fn for_each_needs_a_list() {
        let text = SAMPLE.replace(
            r#"{ equip = "${equips}" }"#,
            r#"{ equip = "${base_frames}" }"#,
        );
        assert!(matches!(
            parse(&text).unwrap().resolve(&NoPredicates),
            Err(RecipeError::RecipeForEachNotList { .. })
        ));
    }

    /// **壊れると: 2 つのステップが同じファイルを書き，どちらが残るか実行順に依る．**
    #[test]
    fn two_steps_may_not_declare_the_same_output() {
        let text = SAMPLE.replace(
            r#"output = "out/sub.aseprite""#,
            r#"output = "out/hero_axe.aseprite""#,
        );
        let steps = parse(&text).unwrap().resolve(&NoPredicates).unwrap();
        assert!(matches!(
            declared_outputs(&steps),
            Err(RecipeError::GraphDuplicateOutput { .. })
        ));
    }

    /// **壊れると: 表の引数の並びが実行ごとに変わり，キャッシュが外れる．**
    #[test]
    fn a_table_field_becomes_key_value_pairs_in_a_fixed_order() {
        let text = r#"
[project]
format = 1
[[step]]
op = "compose"
output = "o.aseprite"
part_delay = { torso = 1, cape = 2, arm = 3 }
"#;
        let steps = parse(text).unwrap().resolve(&NoPredicates).unwrap();
        assert_eq!(
            steps[0].params["part_delay"],
            Value::List(vec![
                Value::from("arm=3"),
                Value::from("cape=2"),
                Value::from("torso=1"),
            ]),
            "キー順になっていない"
        );
    }

    /// **壊れると: ステップの無いレシピが «成功» する．**
    #[test]
    fn a_recipe_without_steps_is_an_error() {
        assert!(matches!(
            parse("[project]\nformat = 1\n"),
            Err(RecipeError::RecipeNoSteps)
        ));
    }
}

#[cfg(test)]
mod root_tests {
    use super::*;

    /// **壊れると: `pxsmith run build.toml` (裸のファイル名) が落ちる．**
    ///
    /// `parent()` は裸のファイル名に `Some("")` を返すので，`unwrap_or(".")` の
    /// 守りは働かない．**空の場所へは移れない**．
    #[test]
    fn a_bare_file_name_resolves_to_the_current_directory() {
        assert_eq!(Recipe::root_of(Path::new("build.toml")), PathBuf::from("."));
        assert_eq!(
            Recipe::root_of(Path::new("proj/build.toml")),
            PathBuf::from("proj")
        );
        assert_eq!(
            Recipe::root_of(Path::new("/abs/proj/build.toml")),
            PathBuf::from("/abs/proj")
        );
    }

    /// **壊れると: 置き場所が空文字になり «移れない» で落ちる．**
    #[test]
    fn the_root_is_never_empty() {
        for p in ["build.toml", "a/b.toml", "/x/y.toml", "b"] {
            let root = Recipe::root_of(Path::new(p));
            assert!(!root.as_os_str().is_empty(), "{p} で空の場所が出た");
        }
    }
}
