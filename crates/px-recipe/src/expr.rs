//! 制限式言語 (設計書 4.2．D25)．
//!
//! 設計書は許すものと許さないものを表で切っている．**その線をそのまま実装する．**
//!
//! | 許すもの | 許さないもの |
//! | --- | --- |
//! | 変数参照 ・算術 ・文字列連結 ・比較 ・三項演算子 ・直積展開 | 関数定義 ・ループ ・I/O ・任意の述語 ・副作用 |
//!
//! # 書いていないもの (D92 の作法)
//!
//! **論理積 `&&` ・論理和 `||` ・否定 `!` は書いていない．** 設計書 4.2 の
//! «許すもの» に無いためである — 組込み述語と三項演算子があれば書ける範囲は
//! 変わらないので，**言語を広げる前に «要る場面» を出してもらう**ことにした．
//! 使うと «その演算子は無い» と，理由付きで落ちる．
//!
//! 呼べる関数は**組込み述語 6 種だけ**である ([`crate::predicate`]) ．
//! 名前が違えば «関数定義はできない» と言って落とす — 任意の述語を許すと
//! レシピが «スクリプト» になり，D25 の «ステップキーが逐次確定できる» が崩れる．
//!
//! # 割り算は整数どうしなら整数である
//!
//! `${8 / 3}` は `2` になる．**小数へ勝手に昇格させない** — フレーム数や画素数を
//! 計算する言語なので，`2.6666666666666665` がコマンド行に流れる方が困る．
//! 小数が要るなら `${8.0 / 3}` と書けばよい．0 で割ったら落とす．

use std::collections::BTreeMap;

use crate::error::{RecipeError, Result};
use crate::value::Value;

/// 式が読める環境 (変数表)．
///
/// **`BTreeMap` である** — 反復順が決まっていないと，環境を畳んで作る
/// ステップキーが実行ごとに変わる (決定論性の規則 1) ．
pub type Env = BTreeMap<String, Value>;

// ------------------------------------------------------------------ 字句

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Int(i64),
    Float(f64),
    Str(String),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Question,
    Colon,
    Comma,
    LParen,
    RParen,
}

fn lex(src: &str) -> Result<Vec<Tok>> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        // **書いていない演算子は «無い» と言って落とす．** 黙って別の意味に
        // 読み替えると，レシピの作者が気付かないまま違う絵ができる
        if c == '&' || c == '|' || c == '!' {
            let name = match c {
                '&' => "論理積 (&&)",
                '|' => "論理和 (||)",
                _ => "否定 (!)",
            };
            return Err(RecipeError::ExprOperatorNotWritten {
                name: name.to_string(),
            });
        }
        if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            if text.contains('.') {
                out.push(Tok::Float(text.parse().map_err(|_| {
                    RecipeError::ExprBadNumber { text: text.clone() }
                })?));
            } else {
                out.push(Tok::Int(text.parse().map_err(|_| {
                    RecipeError::ExprBadNumber { text: text.clone() }
                })?));
            }
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            out.push(Tok::Ident(chars[start..i].iter().collect()));
            continue;
        }
        if c == '\'' || c == '"' {
            let quote = c;
            i += 1;
            let start = i;
            while i < chars.len() && chars[i] != quote {
                i += 1;
            }
            if i >= chars.len() {
                return Err(RecipeError::ExprUnterminatedString);
            }
            out.push(Tok::Str(chars[start..i].iter().collect()));
            i += 1;
            continue;
        }
        let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
        let tok = match two.as_str() {
            "==" => Some(Tok::Eq),
            "!=" => Some(Tok::Ne),
            "<=" => Some(Tok::Le),
            ">=" => Some(Tok::Ge),
            _ => None,
        };
        if let Some(t) = tok {
            out.push(t);
            i += 2;
            continue;
        }
        let one = match c {
            '+' => Tok::Plus,
            '-' => Tok::Minus,
            '*' => Tok::Star,
            '/' => Tok::Slash,
            '%' => Tok::Percent,
            '<' => Tok::Lt,
            '>' => Tok::Gt,
            '?' => Tok::Question,
            ':' => Tok::Colon,
            ',' => Tok::Comma,
            '(' => Tok::LParen,
            ')' => Tok::RParen,
            _ => return Err(RecipeError::ExprBadChar { ch: c }),
        };
        out.push(one);
        i += 1;
    }
    Ok(out)
}

// ------------------------------------------------------------------ 構文木

#[derive(Clone, Debug)]
pub enum Expr {
    Lit(Value),
    Var(String),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// 三項演算子．
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    /// 組込み述語の呼び出し．**任意の関数は作れない**．
    Call(String, Vec<Expr>),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

struct Parser {
    toks: Vec<Tok>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.at)
    }

    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == Some(t) {
            self.at += 1;
            return true;
        }
        false
    }

    fn expr(&mut self) -> Result<Expr> {
        let cond = self.compare()?;
        if self.eat(&Tok::Question) {
            let then = self.expr()?;
            if !self.eat(&Tok::Colon) {
                return Err(RecipeError::ExprExpected {
                    what: ": (三項演算子)".into(),
                });
            }
            let other = self.expr()?;
            return Ok(Expr::If(Box::new(cond), Box::new(then), Box::new(other)));
        }
        Ok(cond)
    }

    fn compare(&mut self) -> Result<Expr> {
        let mut lhs = self.add()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Eq) => BinOp::Eq,
                Some(Tok::Ne) => BinOp::Ne,
                Some(Tok::Lt) => BinOp::Lt,
                Some(Tok::Le) => BinOp::Le,
                Some(Tok::Gt) => BinOp::Gt,
                Some(Tok::Ge) => BinOp::Ge,
                _ => break,
            };
            self.at += 1;
            let rhs = self.add()?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn add(&mut self) -> Result<Expr> {
        let mut lhs = self.mul()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Plus) => BinOp::Add,
                Some(Tok::Minus) => BinOp::Sub,
                _ => break,
            };
            self.at += 1;
            let rhs = self.mul()?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn mul(&mut self) -> Result<Expr> {
        let mut lhs = self.unary()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Star) => BinOp::Mul,
                Some(Tok::Slash) => BinOp::Div,
                Some(Tok::Percent) => BinOp::Rem,
                _ => break,
            };
            self.at += 1;
            let rhs = self.unary()?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Expr> {
        if self.eat(&Tok::Minus) {
            return Ok(Expr::Unary(UnOp::Neg, Box::new(self.unary()?)));
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Expr> {
        let tok = self.peek().cloned().ok_or(RecipeError::ExprUnexpectedEnd)?;
        self.at += 1;
        match tok {
            Tok::Int(v) => Ok(Expr::Lit(Value::Int(v))),
            Tok::Float(v) => Ok(Expr::Lit(Value::Float(v))),
            Tok::Str(v) => Ok(Expr::Lit(Value::Str(v))),
            Tok::Ident(name) => {
                if name == "true" {
                    return Ok(Expr::Lit(Value::Bool(true)));
                }
                if name == "false" {
                    return Ok(Expr::Lit(Value::Bool(false)));
                }
                if self.eat(&Tok::LParen) {
                    let mut args = Vec::new();
                    if !self.eat(&Tok::RParen) {
                        loop {
                            args.push(self.expr()?);
                            if self.eat(&Tok::Comma) {
                                continue;
                            }
                            if self.eat(&Tok::RParen) {
                                break;
                            }
                            return Err(RecipeError::ExprExpected {
                                what: ") か ,".into(),
                            });
                        }
                    }
                    return Ok(Expr::Call(name, args));
                }
                Ok(Expr::Var(name))
            }
            Tok::LParen => {
                let inner = self.expr()?;
                if !self.eat(&Tok::RParen) {
                    return Err(RecipeError::ExprExpected { what: ")".into() });
                }
                Ok(inner)
            }
            other => Err(RecipeError::ExprUnexpectedToken {
                text: format!("{other:?}"),
            }),
        }
    }
}

/// 式を構文木にする．
pub fn parse(src: &str) -> Result<Expr> {
    let mut p = Parser {
        toks: lex(src)?,
        at: 0,
    };
    if p.toks.is_empty() {
        return Err(RecipeError::ExprEmpty);
    }
    let e = p.expr()?;
    if p.at != p.toks.len() {
        return Err(RecipeError::ExprTrailing {
            text: src.to_string(),
        });
    }
    Ok(e)
}

// ------------------------------------------------------------------ 評価

/// 組込み述語を解く側．**式評価器は述語の中身を知らない**．
///
/// 述語は入力データを見るので，**前段の出力が実体化してはじめて答えられる**
/// (設計書 6.15 のキーの確定タイミング) ．
pub trait Predicates {
    fn eval(&self, name: &str, args: &[Value]) -> Result<Value>;
    /// 呼べる名前の一覧 (エラー文で «何が呼べるか» を出すため)．
    fn names(&self) -> Vec<&'static str>;
}

/// 述語を 1 つも解かない解決器．**述語を使っていないことを確かめる**のに使う．
pub struct NoPredicates;

impl Predicates for NoPredicates {
    fn eval(&self, name: &str, _args: &[Value]) -> Result<Value> {
        Err(RecipeError::ExprUnknownFunction {
            name: name.to_string(),
            known: String::new(),
        })
    }

    fn names(&self) -> Vec<&'static str> {
        Vec::new()
    }
}

/// 式を評価する．
pub fn eval(e: &Expr, env: &Env, preds: &dyn Predicates) -> Result<Value> {
    match e {
        Expr::Lit(v) => Ok(v.clone()),
        Expr::Var(name) => env
            .get(name)
            .cloned()
            .ok_or_else(|| RecipeError::ExprUnknownVariable {
                name: name.clone(),
                known: keys_of(env),
            }),
        Expr::Unary(UnOp::Neg, inner) => match eval(inner, env, preds)? {
            Value::Int(v) => Ok(Value::Int(-v)),
            Value::Float(v) => Ok(Value::Float(-v)),
            other => Err(RecipeError::ExprBadType {
                op: "-".into(),
                got: other.type_name().into(),
            }),
        },
        Expr::If(cond, then, other) => {
            let c = eval(cond, env, preds)?;
            let b = c.as_bool().ok_or_else(|| RecipeError::ExprBadType {
                op: "三項演算子の条件".into(),
                got: c.type_name().into(),
            })?;
            eval(if b { then } else { other }, env, preds)
        }
        Expr::Call(name, args) => {
            let mut values = Vec::with_capacity(args.len());
            for a in args {
                values.push(eval(a, env, preds)?);
            }
            if !preds.names().contains(&name.as_str()) {
                return Err(RecipeError::ExprUnknownFunction {
                    name: name.clone(),
                    known: preds.names().join(" ・"),
                });
            }
            preds.eval(name, &values)
        }
        Expr::Binary(op, l, r) => {
            let (a, b) = (eval(l, env, preds)?, eval(r, env, preds)?);
            binary(*op, a, b)
        }
    }
}

fn keys_of(env: &Env) -> String {
    env.keys().cloned().collect::<Vec<_>>().join(" ・")
}

fn binary(op: BinOp, a: Value, b: Value) -> Result<Value> {
    use BinOp::*;
    use Value::*;
    // 比較はどの型でも «同じ型どうし» なら答えられる
    if matches!(op, Eq | Ne) {
        let same = a == b;
        return Ok(Bool(if op == Eq { same } else { !same }));
    }
    match (op, &a, &b) {
        // **文字列の + は連結** (設計書 4.2 の «文字列連結»)
        (Add, Str(x), _) => Ok(Str(format!("{x}{b}"))),
        (Add, _, Str(y)) => Ok(Str(format!("{a}{y}"))),
        (_, Int(x), Int(y)) => int_op(op, *x, *y),
        (_, Float(_) | Int(_), Float(_) | Int(_)) => {
            let (x, y) = (as_f64(&a), as_f64(&b));
            float_op(op, x, y)
        }
        _ => Err(RecipeError::ExprBadOperands {
            op: format!("{op:?}"),
            left: a.type_name().into(),
            right: b.type_name().into(),
        }),
    }
}

fn as_f64(v: &Value) -> f64 {
    match v {
        Value::Int(x) => *x as f64,
        Value::Float(x) => *x,
        _ => f64::NAN,
    }
}

fn int_op(op: BinOp, x: i64, y: i64) -> Result<Value> {
    use BinOp::*;
    Ok(match op {
        Add => Value::Int(x + y),
        Sub => Value::Int(x - y),
        Mul => Value::Int(x * y),
        // **整数どうしは整数で割る** (モジュールの説明)
        Div => {
            if y == 0 {
                return Err(RecipeError::ExprDivideByZero);
            }
            Value::Int(x / y)
        }
        Rem => {
            if y == 0 {
                return Err(RecipeError::ExprDivideByZero);
            }
            Value::Int(x % y)
        }
        Lt => Value::Bool(x < y),
        Le => Value::Bool(x <= y),
        Gt => Value::Bool(x > y),
        Ge => Value::Bool(x >= y),
        Eq | Ne => unreachable!("上で処理している"),
    })
}

fn float_op(op: BinOp, x: f64, y: f64) -> Result<Value> {
    use BinOp::*;
    Ok(match op {
        Add => Value::Float(x + y),
        Sub => Value::Float(x - y),
        Mul => Value::Float(x * y),
        Div => {
            if y == 0.0 {
                return Err(RecipeError::ExprDivideByZero);
            }
            Value::Float(x / y)
        }
        Rem => {
            if y == 0.0 {
                return Err(RecipeError::ExprDivideByZero);
            }
            Value::Float(x % y)
        }
        Lt => Value::Bool(x < y),
        Le => Value::Bool(x <= y),
        Gt => Value::Bool(x > y),
        Ge => Value::Bool(x >= y),
        Eq | Ne => unreachable!("上で処理している"),
    })
}

// ------------------------------------------------------ 文字列への埋め込み

/// 文字列の中の `${...}` を評価して差し替える．
///
/// > [!note] **文字列全体がちょうど 1 つの `${...}` なら，値をそのまま返す．**
/// > `for_each = { equip = "${equips}" }` が列を受け取れるようにするためである．
/// > 途中に文字がある (`"parts/hero_${equip}.px.toml"`) なら文字列になる．
/// > **この 2 つを分けないと直積展開に列を渡せない．**
pub fn interpolate(src: &str, env: &Env, preds: &dyn Predicates) -> Result<Value> {
    let spans = find_spans(src)?;
    if spans.is_empty() {
        return Ok(Value::Str(src.to_string()));
    }
    if spans.len() == 1 && spans[0].0 == 0 && spans[0].1 == src.len() {
        let body = &src[spans[0].0 + 2..spans[0].1 - 1];
        return eval(&parse(body)?, env, preds);
    }
    let mut out = String::new();
    let mut at = 0usize;
    for (start, end) in spans {
        out.push_str(&src[at..start]);
        let body = &src[start + 2..end - 1];
        let value = eval(&parse(body)?, env, preds)?;
        if matches!(value, Value::List(_)) {
            // **列を文字列の途中に埋めない** — `a,b` と繋がった時点で直積展開が
            // できなくなり，何が起きたか分からない出力になる
            return Err(RecipeError::ExprListInString {
                text: src.to_string(),
            });
        }
        out.push_str(&value.to_string());
        at = end;
    }
    out.push_str(&src[at..]);
    Ok(Value::Str(out))
}

/// `${` から対応する `}` までの範囲 (バイト位置，終端は `}` の次)．
fn find_spans(src: &str) -> Result<Vec<(usize, usize)>> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] == b'$' && bytes[i + 1] == b'{' {
            let mut depth = 1usize;
            let mut j = i + 2;
            while j < bytes.len() && depth > 0 {
                match bytes[j] {
                    b'{' => depth += 1,
                    b'}' => depth -= 1,
                    _ => {}
                }
                j += 1;
            }
            if depth != 0 {
                return Err(RecipeError::ExprUnclosedInterpolation {
                    text: src.to_string(),
                });
            }
            out.push((i, j));
            i = j;
            continue;
        }
        i += 1;
    }
    Ok(out)
}

/// 文字列が `${...}` を含むか (含まないなら評価そのものを飛ばせる)．
pub fn has_interpolation(src: &str) -> bool {
    find_spans(src).map(|v| !v.is_empty()).unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> Env {
        let mut e = Env::new();
        e.insert("base_frames".into(), Value::Int(4));
        e.insert("equip".into(), Value::from("sword"));
        e.insert(
            "equips".into(),
            Value::List(vec![
                Value::from("sword"),
                Value::from("axe"),
                Value::from("none"),
            ]),
        );
        e
    }

    fn ev(src: &str) -> Result<Value> {
        interpolate(src, &env(), &NoPredicates)
    }

    /// **壊れると: 設計書 4.2 の例が動かない．**
    #[test]
    fn the_examples_in_the_design_evaluate() {
        assert_eq!(ev("${base_frames * 2}").unwrap(), Value::Int(8));
        assert_eq!(
            ev("parts/hero_${equip}.px.toml").unwrap(),
            Value::from("parts/hero_sword.px.toml")
        );
        assert_eq!(
            ev("${equips}").unwrap(),
            Value::List(vec![
                Value::from("sword"),
                Value::from("axe"),
                Value::from("none")
            ])
        );
    }

    /// **壊れると: 直積展開に列を渡せなくなる．**
    ///
    /// «全体がちょうど 1 つの `${...}`» と «途中に文字がある» を分けないと，
    /// `for_each = { equip = "${equips}" }` が文字列 `sword,axe,none` になる．
    #[test]
    fn a_bare_interpolation_keeps_the_list_but_an_embedded_one_is_an_error() {
        assert!(matches!(ev("${equips}").unwrap(), Value::List(_)));
        assert!(matches!(
            ev("x_${equips}"),
            Err(RecipeError::ExprListInString { .. })
        ));
    }

    /// **壊れると: フレーム数の計算に小数が紛れ込む．**
    #[test]
    fn integer_division_stays_integer() {
        assert_eq!(ev("${8 / 3}").unwrap(), Value::Int(2));
        assert_eq!(ev("${8.0 / 3}").unwrap(), Value::Float(8.0 / 3.0));
        assert!(matches!(ev("${1 / 0}"), Err(RecipeError::ExprDivideByZero)));
    }

    /// **壊れると: 三項演算子が «0 を偽» のような読み替えを始める．**
    #[test]
    fn the_ternary_needs_a_real_boolean() {
        assert_eq!(
            ev("${base_frames > 2 ? 'many' : 'few'}").unwrap(),
            Value::from("many")
        );
        assert!(matches!(
            ev("${base_frames ? 'a' : 'b'}"),
            Err(RecipeError::ExprBadType { .. })
        ));
    }

    /// **壊れると: レシピがスクリプト言語になる (D25)．**
    ///
    /// 設計書 4.2 の «許さないもの» に «任意の述語» がある．知らない名前を
    /// 呼んだら，**何が呼べるかを言って落とす**．
    #[test]
    fn an_unknown_function_is_refused_rather_than_ignored() {
        assert!(matches!(
            ev("${my_helper(1)}"),
            Err(RecipeError::ExprUnknownFunction { .. })
        ));
    }

    /// **壊れると: 書いていない演算子が «別の意味» で通る．**
    ///
    /// `&&` ・`||` ・`!` は設計書 4.2 の «許すもの» に無い．
    /// **書いていないことを言って落とす** (D92 の作法) ．
    #[test]
    fn the_operators_we_did_not_write_say_so() {
        for src in ["${1 < 2 && 3 < 4}", "${1 < 2 || 3 < 4}", "${!true}"] {
            assert!(
                matches!(ev(src), Err(RecipeError::ExprOperatorNotWritten { .. })),
                "{src} が通ってしまった"
            );
        }
    }

    /// **壊れると: 変数の打ち間違いが空文字列として通る．**
    #[test]
    fn an_unknown_variable_lists_the_ones_that_exist() {
        let err = ev("${equipx}").unwrap_err();
        let text = err.to_string();
        assert!(text.contains("equipx"), "{text}");
        assert!(text.contains("equips"), "何があるかを言っていない: {text}");
    }

    /// **壊れると: 演算子の優先順位が壊れ，静かに違う数が出る．**
    #[test]
    fn precedence_follows_the_usual_order() {
        assert_eq!(ev("${1 + 2 * 3}").unwrap(), Value::Int(7));
        assert_eq!(ev("${(1 + 2) * 3}").unwrap(), Value::Int(9));
        assert_eq!(ev("${1 + 2 > 2 ? 'y' : 'n'}").unwrap(), Value::from("y"));
        assert_eq!(ev("${-2 + 5}").unwrap(), Value::Int(3));
    }

    /// **壊れると: 閉じていない `${` が黙って «残り全部» を式にする．**
    #[test]
    fn an_unclosed_interpolation_is_an_error() {
        assert!(matches!(
            ev("a${1 + 2"),
            Err(RecipeError::ExprUnclosedInterpolation { .. })
        ));
    }

    /// **壊れると: 式に紛れ込んだ余りが黙って捨てられる．**
    #[test]
    fn trailing_junk_in_an_expression_is_an_error() {
        assert!(matches!(
            ev("${1 2}"),
            Err(RecipeError::ExprTrailing { .. })
        ));
    }
}
