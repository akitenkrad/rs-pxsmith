//! 生成の依頼と，その素性 (設計書 8.3)．
//!
//! # 依頼は «宣言» だけでできている
//!
//! バックエンド ・モデル ・思考の深さ ・プロンプトはすべて呼ぶ側が書く．
//! **道具は 1 つも推測しない** — 推測した宣言は外れたときだけ静かに壊れる
//! (D89 ・D111) ．`GenRequest` にあるものが生成を決めるすべてで，
//! ここに無いものは結果に影響してはいけない．
//!
//! # 鍵は依頼そのものから引く
//!
//! `GenRequest` の正規形を blake3 に通したものが鍵である (M5 のステップキーと
//! 同じ道具．設計書 6.15) ．**時刻も試行回数も混ぜない** — 混ぜると同じ依頼が
//! 毎回違う鍵になり，キャッシュが 1 度も当たらなくなる．

use serde::{Deserialize, Serialize};

/// 生成の種類．
///
/// **`Image` は書いていない** (D156) ．設計書 8.1 が «汎用拡散モデルの出力は
/// 非一様格子になることがあり，その場合は棄却する» と決めているので，
/// `px gen image` は成功率がモデル依存で**上限が読めない**．D92 の作法で
/// «書いていない» と報告する．
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GenKind {
    /// LLM に L0 テキストを書かせる (経路 B．設計書 8.2)．
    Prog,
}

impl GenKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prog => "prog",
        }
    }
}

/// 思考の深さ．**モデル側の `effort` にそのまま渡す**．
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    Medium,
    #[default]
    High,
    Xhigh,
    Max,
}

impl Effort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "xhigh" => Some(Self::Xhigh),
            "max" => Some(Self::Max),
            _ => None,
        }
    }

    pub const ALL: [Self; 5] = [Self::Low, Self::Medium, Self::High, Self::Xhigh, Self::Max];
}

/// バックエンドの宣言．**推測しない** (D156)．
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Backend {
    /// バックエンドの名前 (`anthropic` など)．
    pub name: String,
    /// 叩き先．
    pub endpoint: String,
    /// モデルの識別子．
    pub model: String,
}

impl Backend {
    /// 既定の宛先．**モデルは呼ぶ側が選ぶもの**なので，これは «宛先の形» を
    /// 埋めるだけの補助である．
    pub fn anthropic(model: &str) -> Self {
        Self {
            name: "anthropic".to_string(),
            endpoint: "https://api.anthropic.com/v1/messages".to_string(),
            model: model.to_string(),
        }
    }
}

/// 生成の依頼．
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenRequest {
    pub kind: GenKind,
    pub backend: Backend,
    pub effort: Effort,
    /// 作らせるものの説明 (利用者が書く)．
    pub prompt: String,
    /// 守らせる制約 — パレット ・寸法 ・コマ数．
    pub constraints: Constraints,
    /// 検査に落ちたときに作り直す上限 (設計書 8.2 の $n_{\max}$)．
    pub max_attempts: u32,
    /// 応答の上限トークン．**思考と本文の合算に掛かる** (D160)．
    ///
    /// **上限であって目標ではない** — 上げても応答は長くならない．
    /// 既定は [`crate::anthropic::DEFAULT_MAX_TOKENS`]．
    ///
    /// > [!note] **鍵に混ぜる** (D165)．
    /// > 同じ依頼でも上限が違えば «切れた応答» と «通った応答» に分かれるので，
    /// > 混ぜないと**切れた結果を上限の広い依頼へ返してしまう**．
    /// > 当たりが減る代わりに，返ってきたものが依頼に対応していることを保つ．
    pub max_tokens: u32,
}

/// 生成物が満たすべき形．**絵から決まらないので宣言させる** (D89)．
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Constraints {
    pub width: u32,
    pub height: u32,
    /// 使ってよい色 (`RRGGBB` の並び)．**L0 は 62 色まで** (D8)．
    pub palette: Vec<String>,
    /// コマ数．
    pub frames: u32,
}

impl GenRequest {
    /// **依頼の正規形から鍵を引く** (設計書 6.15 と同じ道具)．
    ///
    /// 時刻も試行回数も混ぜない — 混ぜると同じ依頼が毎回違う鍵になる．
    pub fn key(&self) -> String {
        let mut h = blake3::Hasher::new();
        h.update(b"px-gen/1\0");
        h.update(self.kind.as_str().as_bytes());
        h.update(b"\0");
        h.update(self.backend.name.as_bytes());
        h.update(b"\0");
        h.update(self.backend.endpoint.as_bytes());
        h.update(b"\0");
        h.update(self.backend.model.as_bytes());
        h.update(b"\0");
        h.update(self.effort.as_str().as_bytes());
        h.update(b"\0");
        h.update(self.prompt.as_bytes());
        h.update(b"\0");
        h.update(&self.constraints.width.to_le_bytes());
        h.update(&self.constraints.height.to_le_bytes());
        h.update(&self.constraints.frames.to_le_bytes());
        // **上限も混ぜる** (D165) — 上限が違えば «切れた» と «通った» に分かれる
        h.update(&self.max_tokens.to_le_bytes());
        for c in &self.constraints.palette {
            h.update(c.as_bytes());
            h.update(b"\0");
        }
        h.finalize().to_hex().to_string()
    }
}

/// 生成の素性 (設計書 8.3 «アセットごとにモデル ・プロンプト ・seed を manifest に残す»)．
///
/// > [!warning] **seed は書けない．書けないことを書く．**
/// > 設計書 8.3 は «seed を残す» と言うが，**採ったバックエンドに seed が無い**
/// > — 標本の温度も種も受け付けない (D157) ．無い欄を 0 で埋めると «固定した»
/// > という嘘になるので，**欄ごと落として «決定論の担い手はキャッシュである»
/// > と書く**．D92 «書いていないものを黙らない» の形である．
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    /// 生成した道具．
    pub tool: String,
    pub backend: String,
    pub endpoint: String,
    /// **依頼したモデル** — 応答したモデルとは限らない ([`Provenance::served_model`])．
    pub model: String,
    /// **実際にこの絵を作ったモデル** (応答が名乗った名前．D171)．
    ///
    /// 道具は `fallbacks` を送っているので，断られた依頼は**別のモデルが
    /// 肩代わりして返す**ことがある．`model` だけを書くと，素性が
    /// «この絵を作ったモデル» を取り違える — D157 が素性に求めたのはまさに
    /// そこである．**読めなかったときは `None`** で，«同じだった» とは書かない．
    pub served_model: Option<String>,
    /// 断りを別のモデルが肩代わりしたか (D171)．
    pub fell_back: bool,
    pub effort: String,
    /// 依頼の鍵 — **これが同じなら同じものを頼んでいる**．
    pub request_key: String,
    /// プロンプト全文 (**要約しない** — 後から «何を頼んだか» が読めなくなる)．
    pub prompt: String,
    pub constraints: Constraints,
    /// 何度作り直したか (1 なら 1 発で通った)．
    pub attempts: u32,
    /// 通った時点で残っていた advisory の数 (blocking は 0 のはず)．
    pub advisory: usize,
    /// **seed は無い** — なぜ無いかを書いておく．
    pub determinism: String,
    /// いつ生成したか (ISO 8601)．**鍵には混ぜない**．
    pub created_at: String,
}

/// [`Provenance::determinism`] に入れる説明．
pub const DETERMINISM_NOTE: &str = "\
このバックエンドは seed も温度も受け付けないので，同じ依頼が同じ出力を返す保証は無い．\
決定論を担うのはキャッシュ (.pxcache) と，生成物をリポジトリにコミットすること \
(設計書 8.3 ・1407 行) である．op = \"gen\" が既定でキャッシュ参照のみなのはこのためで \
(D31)，--allow-generate を付けた実行だけが新しい出力を作る．";

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> GenRequest {
        GenRequest {
            kind: GenKind::Prog,
            backend: Backend::anthropic("claude-opus-5"),
            effort: Effort::High,
            prompt: "16x16 の宝箱".to_string(),
            constraints: Constraints {
                width: 16,
                height: 16,
                palette: vec!["1a1c2c".to_string(), "f4f4f4".to_string()],
                frames: 1,
            },
            max_attempts: 3,
            max_tokens: crate::anthropic::DEFAULT_MAX_TOKENS,
        }
    }

    /// **壊れると: 同じ依頼が違う鍵になり，キャッシュが 1 度も当たらない．**
    #[test]
    fn the_same_request_always_yields_the_same_key() {
        assert_eq!(req().key(), req().key());
    }

    /// **壊れると: 依頼を変えても同じものが戻る (取り違え)．**
    #[test]
    fn every_declared_field_changes_the_key() {
        let base = req().key();

        let mut m = req();
        m.backend.model = "claude-sonnet-5".to_string();
        assert_ne!(m.key(), base, "モデルが鍵に入っていない");

        let mut e = req();
        e.effort = Effort::Low;
        assert_ne!(e.key(), base, "思考の深さが鍵に入っていない");

        let mut p = req();
        p.prompt = "16x16 の樽".to_string();
        assert_ne!(p.key(), base, "プロンプトが鍵に入っていない");

        let mut c = req();
        c.constraints.width = 32;
        assert_ne!(c.key(), base, "寸法が鍵に入っていない");

        let mut pal = req();
        pal.constraints.palette.push("b13e53".to_string());
        assert_ne!(pal.key(), base, "パレットが鍵に入っていない");

        // **壊れると: 上限を絞った依頼へ «切れていない» 結果が返る** (D165)
        let mut t = req();
        t.max_tokens = 256;
        assert_ne!(t.key(), base, "上限トークンが鍵に入っていない");

        let mut ep = req();
        ep.backend.endpoint = "https://example.invalid/v1".to_string();
        assert_ne!(ep.key(), base, "叩き先が鍵に入っていない");
    }

    /// **壊れると: 作り直しの上限が鍵に入り，同じ依頼が別物になる．**
    ///
    /// `max_attempts` は «何度やり直してよいか» であって «何を頼んだか» では
    /// ないので，鍵に入れてはいけない．
    #[test]
    fn the_retry_budget_is_not_part_of_the_key() {
        let mut a = req();
        a.max_attempts = 9;
        assert_eq!(a.key(), req().key(), "作り直しの上限が鍵に混ざっている");
    }

    /// **壊れると: パレットの区切りが消えて別の並びが同じ鍵になる．**
    #[test]
    fn palette_entries_cannot_run_together() {
        let mut a = req();
        a.constraints.palette = vec!["aabb".to_string(), "cc".to_string()];
        let mut b = req();
        b.constraints.palette = vec!["aa".to_string(), "bbcc".to_string()];
        assert_ne!(a.key(), b.key(), "区切りが効いていない");
    }
}
