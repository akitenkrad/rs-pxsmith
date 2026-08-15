//! レシピの式が扱う値 (設計書 4.2)．
//!
//! # 文字列にするところが差分ビルドの鍵になる
//!
//! 値は最終的に**コマンド行の引数**になり，その引数がそのままステップキーの
//! $\mathrm{params}$ になる (6.15) ．したがって **`to_argv` は同じ値から必ず
//! 同じ文字列を返さなければならない** — ここが揺れると，何も変えていないのに
//! キャッシュが外れる．
//!
//! 浮動小数点は Rust の `{}` (最短往復表現) を使う．`{:?}` や `{:.3}` は
//! 桁数が値によって変わったり丸めたりするので使わない．

use std::fmt;

/// レシピの式の値．
///
/// **これだけしか無い** — 設計書 4.2 の «許さないもの» (関数定義 ・ループ ・
/// I/O ・副作用) を持ち込まないために，構造体も辞書も持たせない．
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    /// 直積展開 (`for_each`) に使う列．**入れ子にはしない**．
    List(Vec<Value>),
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Int(_) => "整数",
            Self::Float(_) => "小数",
            Self::Str(_) => "文字列",
            Self::Bool(_) => "真偽",
            Self::List(_) => "列",
        }
    }

    /// 真偽として読む．**数や文字列を真偽に読み替えない** — 0 を偽と扱う言語は
    /// 誤りを黙って通すので，型が違えばエラーにする．
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Self::List(v) => Some(v),
            _ => None,
        }
    }

    /// **コマンド行の引数にするときの表現**．
    ///
    /// 列はここでは文字列にしない (直積展開で 1 つずつに割れている前提) ．
    pub fn to_argv(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Int(v) => write!(f, "{v}"),
            // **最短往復表現**．`{:?}` は `1.0` を `1.0`，`{}` も `1` ではなく `1`
            // になるので，整数値の小数は整数と同じ字面になる — キーが揺れないこと
            // の方が大事なので許す
            Self::Float(v) => write!(f, "{v}"),
            Self::Str(v) => write!(f, "{v}"),
            Self::Bool(v) => write!(f, "{v}"),
            Self::List(v) => {
                let parts: Vec<String> = v.iter().map(|x| x.to_string()).collect();
                write!(f, "{}", parts.join(","))
            }
        }
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Self::Str(v.to_string())
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Self::Str(v)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Self::Int(v)
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **壊れると: 何も変えていないのにキャッシュが外れる．**
    ///
    /// 値からコマンド行の引数への写像が揺れると，ステップキーが毎回変わる．
    #[test]
    fn the_same_value_always_prints_the_same_way() {
        let cases = [
            (Value::Int(42), "42"),
            (Value::Int(-7), "-7"),
            (Value::Float(0.5), "0.5"),
            (Value::Float(2.0), "2"),
            (Value::Str("hero".into()), "hero"),
            (Value::Bool(true), "true"),
            (Value::List(vec![Value::from("a"), Value::from("b")]), "a,b"),
        ];
        for (value, want) in cases {
            assert_eq!(value.to_argv(), want);
            // 2 度呼んでも同じ
            assert_eq!(value.to_argv(), want);
        }
    }

    /// **壊れると: 0 や空文字列が «偽» として通り，誤ったレシピが黙って動く．**
    #[test]
    fn only_a_boolean_reads_as_a_boolean() {
        assert_eq!(Value::Bool(false).as_bool(), Some(false));
        assert_eq!(Value::Int(0).as_bool(), None);
        assert_eq!(Value::Str(String::new()).as_bool(), None);
        assert_eq!(Value::List(vec![]).as_bool(), None);
    }
}
