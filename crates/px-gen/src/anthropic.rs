//! Anthropic Messages API のバックエンド (設計書 8.3 ・D156)．
//!
//! **生の HTTP で叩く** — Rust には公式 SDK が無い．叩き先もモデルも
//! [`crate::request::Backend`] の宣言から引き，ここには既定値を書き写さない
//! (「既定値を書き写さない」は M2 で 1 度踏んでいる) ．
//!
//! # 出力は構造化して受ける — 囲みを剥がす処理を書かないため
//!
//! `output_config.format` に JSON スキーマを渡し，**L0 の本文を 1 つの文字列
//! 欄で受け取る**．こうしないと «説明文が前に付く» «``` で囲む» を毎回
//! 剥がすことになり，剥がし方の癖が道具の側に溜まる．
//!
//! # 温度も種も渡さない — 渡せない (D157)
//!
//! `temperature` ・`top_p` ・`top_k` はこのモデルでは**受け付けない**
//! (送ると 400) ．`thinking` も既定で有効なので触らない．深さを決めるのは
//! `output_config.effort` だけである．
//!
//! # 断りは «壊れた» ではない
//!
//! 応答は HTTP 200 で返り，`stop_reason` が `refusal` になる．
//! **本文を読む前に `stop_reason` を見る** — `content[0]` を無条件に読むと
//! ここで落ちる．断りは作り直しても同じなので**輪を回さずに落とす**
//! ([`crate::error::GenError::Refused`]) ．
//!
//! # 動かない部分を先に固定する
//!
//! システムプロンプト (L0 の書き方) は作り直しの間ずっと同じなので，
//! `cache_control` を置いて使い回す．変わるのは利用者の依頼と前回の助言だけで，
//! **それらは後ろに置く** — 前に置くと毎回キャッシュが崩れる．

use crate::error::{GenError, Result};
use crate::repair::Generator;
use crate::request::{GenKind, GenRequest};

/// 鍵を置く環境変数．**依頼にも素性にも鍵は書かない．**
pub const KEY_VAR: &str = "ANTHROPIC_API_KEY";

/// 送る API の版．
const API_VERSION: &str = "2023-06-01";

/// 断られたときに別のモデルへ回す (Claude API のみ)．
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

/// 非ストリームで安全に受け取れる上限．
const MAX_TOKENS: u32 = 16000;

/// Anthropic Messages API を叩くバックエンド．
pub struct AnthropicGenerator {
    key: String,
    /// L0 が書くべきパレット参照の名前 (道具が先に書いた `.hex`)．
    palette_ref: String,
}

impl AnthropicGenerator {
    /// 環境変数から鍵を読む．
    pub fn from_env(palette_ref: &str) -> Result<Self> {
        let key = std::env::var(KEY_VAR).map_err(|_| GenError::MissingKey {
            var: KEY_VAR.to_string(),
        })?;
        if key.trim().is_empty() {
            return Err(GenError::MissingKey {
                var: KEY_VAR.to_string(),
            });
        }
        Ok(Self {
            key,
            palette_ref: palette_ref.to_string(),
        })
    }
}

/// 添字 → L0 の 1 文字 (`0-9 a-z A-Z`．設計書 4.1 の 62 色)．
fn index_char(i: usize) -> char {
    const TABLE: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    TABLE.get(i).map(|b| *b as char).unwrap_or('?')
}

/// L0 の書き方 — **作り直しの間ずっと同じなので使い回す**．
///
/// > [!warning] **仕様は引いてから書く** (D111) ．
/// > ここを記憶で書いて 1 度外した — L0 には `inline` パレットも `pixels` 配列も
/// > **無い**．パレットは外の `.hex` への参照で，画素は `data` の 1 本の文字列
/// > である．スキーマ (`px_io::l0`) を読んでから書き直した．
fn system_prompt(req: &GenRequest, palette_ref: &str) -> String {
    let map: Vec<String> = req
        .constraints
        .palette
        .iter()
        .enumerate()
        .map(|(i, c)| format!("{} = {i}    # {c}", index_char(i)))
        .collect();
    let chars: String = (0..req.constraints.palette.len()).map(index_char).collect();

    let mut s = String::new();
    s.push_str(
        "あなたはドット絵を L0 テキスト形式で書く．L0 は pxforge の中間形式で TOML である．\n\n",
    );
    s.push_str("**形は次のとおりで，欄を足しても減らしてもいけない**:\n\n");
    s.push_str("[meta]\nformat = 1\nname = \"gen\"\nlayer = \"art\"\n\n");
    s.push_str("[palette]\nref = \"");
    s.push_str(palette_ref);
    s.push_str("\"\n\n[palette.map]\n\".\" = \"transparent\"\n");
    for line in &map {
        s.push_str(line);
        s.push('\n');
    }
    s.push_str("\n[[frame]]\nname = \"gen_0\"\nkind = \"key\"\nduration_ms = 100\n");
    s.push_str("data = '''\n<1 行 1 段の画素．改行で区切る>\n'''\n\n");

    s.push_str("守ること:\n");
    s.push_str("- `data` は**三重引用符の直後に改行**を置き，そこから画素の行を始める\n");
    s.push_str(&format!(
        "- 1 行は**ちょうど {} 文字**，行数は**ちょうど {} 行**\n",
        req.constraints.width, req.constraints.height
    ));
    s.push_str(&format!(
        "- コマは {} 個 (`[[frame]]` をその数だけ書く)\n",
        req.constraints.frames
    ));
    s.push_str(&format!(
        "- 使える文字は `{chars}` と透明の `.` **だけ**．他の文字を書かない\n"
    ));
    s.push_str("- パレットは触らない．`ref` の名前も `[palette.map]` も上のまま写す\n\n");

    s.push_str("ドット絵の作法 (この後の検査に掛かる):\n");
    s.push_str("- 孤立した 1 画素を置かない\n");
    s.push_str("- 同じ色を斜めだけで繋げない (角がぶつかると形が読めなくなる)\n");
    s.push_str("- 縁の階段の段をそろえる (1,1,2,1,1 のような乱れた段にしない)\n");
    s.push_str("- 影は黒へ落とさず，暗い色を使う\n\n");

    s.push_str("出力は `l0` 欄に L0 の本文だけを入れる．説明も囲みも付けない．");
    s
}

/// 応答の形を固定する — **囲みを剥がす処理を書かないため**．
fn output_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "l0": {
                "type": "string",
                "description": "L0 (.px.toml) の本文そのもの．囲みも説明も含めない",
            },
        },
        "required": ["l0"],
        "additionalProperties": false,
    })
}

/// 送る本文を組む．**試験がここを直接見る** — 送っていないことを固定するため．
fn build_body(req: &GenRequest, palette_ref: &str, feedback: Option<&str>) -> serde_json::Value {
    // **動かない部分に印を置く** — 作り直しの間ずっと同じなので使い回す
    let system = serde_json::json!([{
        "type": "text",
        "text": system_prompt(req, palette_ref),
        "cache_control": {"type": "ephemeral"},
    }]);

    // 変わるものは後ろへ．前に置くとキャッシュが毎回崩れる
    let mut user = req.prompt.clone();
    if let Some(fb) = feedback {
        user.push_str("\n\n");
        user.push_str(fb);
    }

    let mut body = serde_json::json!({
        "model": req.backend.model,
        "max_tokens": MAX_TOKENS,
        "system": system,
        "output_config": {
            "effort": req.effort.as_str(),
            "format": {"type": "json_schema", "schema": output_schema()},
        },
        "messages": [{"role": "user", "content": user}],
    });

    // **断られたら別のモデルへ回す** — Claude API でだけ効くので，
    // 宛先が Anthropic のときにしか付けない
    if req.backend.name == "anthropic" {
        body["fallbacks"] = serde_json::json!("default");
    }
    body
}

impl Generator for AnthropicGenerator {
    fn generate(&self, req: &GenRequest, feedback: Option<&str>) -> Result<String> {
        if req.kind != GenKind::Prog {
            return Err(GenError::NotWritten { kind: req.kind });
        }
        let body = build_body(req, &self.palette_ref, feedback);

        let mut request = ureq::post(&req.backend.endpoint)
            .header("content-type", "application/json")
            .header("x-api-key", &self.key)
            .header("anthropic-version", API_VERSION);
        if req.backend.name == "anthropic" {
            request = request.header("anthropic-beta", FALLBACK_BETA);
        }

        let mut response = match request.send_json(&body) {
            Ok(r) => r,
            // **エラー応答でも本文に理由が入っている** — 読めるなら読む
            Err(ureq::Error::StatusCode(code)) => {
                return Err(GenError::Backend {
                    message: format!("HTTP {code}"),
                });
            }
            Err(e) => {
                return Err(GenError::Backend {
                    message: e.to_string(),
                });
            }
        };

        let text = response
            .body_mut()
            .read_to_string()
            .map_err(|e| GenError::Backend {
                message: e.to_string(),
            })?;

        parse_response(&text)
    }

    fn describe(&self) -> String {
        format!("anthropic (鍵は ${KEY_VAR})")
    }
}

/// 応答から L0 の本文を取り出す．
///
/// **`stop_reason` を本文より先に見る** — 断られたときは `content` が空か
/// 途中までしかないので，`content[0]` を無条件に読むとそこで落ちる．
fn parse_response(text: &str) -> Result<String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| GenError::BadResponse {
        message: format!("JSON として読めない: {e}"),
    })?;

    // エラー応答 (HTTP がエラーでなくてもこの形で返ることがある)
    if v.get("type").and_then(|t| t.as_str()) == Some("error") {
        let message = v
            .pointer("/error/message")
            .and_then(|m| m.as_str())
            .unwrap_or("理由の記載が無い");
        return Err(GenError::Backend {
            message: message.to_string(),
        });
    }

    // **断りを本文より先に見る**
    if v.get("stop_reason").and_then(|s| s.as_str()) == Some("refusal") {
        let category = v
            .pointer("/stop_details/category")
            .and_then(|c| c.as_str())
            .unwrap_or("不明");
        return Err(GenError::Refused {
            category: category.to_string(),
        });
    }

    let blocks =
        v.get("content")
            .and_then(|c| c.as_array())
            .ok_or_else(|| GenError::BadResponse {
                message: "content が無い".to_string(),
            })?;

    let raw = blocks
        .iter()
        .find(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
        .and_then(|b| b.get("text"))
        .and_then(|t| t.as_str())
        .ok_or_else(|| GenError::BadResponse {
            message: "text のブロックが 1 つも無い".to_string(),
        })?;

    // 形を固定してあるので JSON で戻る
    let parsed: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| GenError::BadResponse {
            message: format!("構造化出力が JSON になっていない: {e}"),
        })?;
    parsed
        .get("l0")
        .and_then(|l| l.as_str())
        .map(str::to_string)
        .ok_or_else(|| GenError::BadResponse {
            message: "l0 の欄が無い".to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::{Backend, Constraints, Effort};

    fn req() -> GenRequest {
        GenRequest {
            kind: GenKind::Prog,
            backend: Backend::anthropic("claude-opus-5"),
            effort: Effort::High,
            prompt: "宝箱".to_string(),
            constraints: Constraints {
                width: 16,
                height: 16,
                palette: vec!["1a1c2c".to_string(), "f4f4f4".to_string()],
                frames: 1,
            },
            max_attempts: 3,
        }
    }

    /// **壊れると: 断りを «壊れた応答» と読んで作り直しに行く** (同じ結果を繰り返す)．
    #[test]
    fn a_refusal_is_reported_as_a_refusal_not_as_a_broken_response() {
        let body = r#"{"stop_reason":"refusal","stop_details":{"category":"cyber"},"content":[]}"#;
        match parse_response(body) {
            Err(GenError::Refused { category }) => assert_eq!(category, "cyber"),
            other => panic!("断りとして読めていない: {other:?}"),
        }
    }

    /// **壊れると: 断りの本文を読もうとして «content が無い» で落ちる．**
    #[test]
    fn a_refusal_is_detected_before_the_content_is_read() {
        let body = r#"{"stop_reason":"refusal","content":[]}"#;
        assert!(matches!(
            parse_response(body),
            Err(GenError::Refused { .. })
        ));
    }

    /// **壊れると: 構造化出力の欄を取り違える．**
    #[test]
    fn the_l0_body_comes_out_of_the_declared_field() {
        let inner = serde_json::json!({"l0": "[meta]\nformat = 1\n"}).to_string();
        let body = serde_json::json!({
            "stop_reason": "end_turn",
            "content": [{"type": "text", "text": inner}],
        })
        .to_string();
        let l0 = parse_response(&body).expect("読めるはず");
        assert!(l0.starts_with("[meta]"), "本文が取れていない: {l0}");
    }

    /// **壊れると: 思考ブロックを本文と取り違える．**
    #[test]
    fn a_thinking_block_is_not_mistaken_for_the_answer() {
        let inner = serde_json::json!({"l0": "ok"}).to_string();
        let body = serde_json::json!({
            "stop_reason": "end_turn",
            "content": [
                {"type": "thinking", "thinking": ""},
                {"type": "text", "text": inner},
            ],
        })
        .to_string();
        assert_eq!(parse_response(&body).unwrap(), "ok");
    }

    /// **壊れると: エラー応答を «中身が変» として報告し，理由が消える．**
    #[test]
    fn an_error_response_keeps_the_reason() {
        let body = r#"{"type":"error","error":{"type":"invalid_request_error","message":"max_tokens too large"}}"#;
        match parse_response(body) {
            Err(GenError::Backend { message }) => assert!(message.contains("max_tokens")),
            other => panic!("理由が残っていない: {other:?}"),
        }
    }

    /// **壊れると: 温度や種を送って 400 になる** (D157)．
    #[test]
    fn the_request_body_never_carries_sampling_parameters() {
        let text = build_body(&req(), "t.px.hex", None).to_string();
        for banned in ["temperature", "top_p", "top_k", "\"seed\""] {
            assert!(!text.contains(banned), "{banned} を送っている");
        }
    }

    /// **壊れると: 動かない部分に印が付かず，作り直しのたびに前置きを払う．**
    #[test]
    fn the_stable_prefix_is_marked_for_reuse() {
        let body = build_body(&req(), "t.px.hex", None);
        assert_eq!(
            body.pointer("/system/0/cache_control/type").unwrap(),
            "ephemeral"
        );
    }

    /// **壊れると: 助言が前に入り，動かない部分が毎回変わる．**
    #[test]
    fn feedback_goes_after_the_stable_prefix_not_into_it() {
        let a = build_body(&req(), "t.px.hex", None);
        let b = build_body(&req(), "t.px.hex", Some("直すこと"));
        assert_eq!(
            a.pointer("/system/0/text"),
            b.pointer("/system/0/text"),
            "助言でシステム側が変わっている"
        );
        let user = b.pointer("/messages/0/content").unwrap().as_str().unwrap();
        assert!(user.contains("直すこと"), "助言が届いていない");
    }

    /// **壊れると: 宣言した制約がプロンプトに出ない (守らせようがない)．**
    #[test]
    fn the_declared_constraints_reach_the_prompt() {
        let s = system_prompt(&req(), "t.px.hex");
        assert!(s.contains("16 文字"), "幅が無い");
        assert!(s.contains("16 行"), "高さが無い");
        assert!(s.contains("1a1c2c"), "パレットが無い");
        assert!(s.contains("t.px.hex"), "パレット参照が無い");
        assert!(s.contains("コマは 1 個"), "コマ数が無い");
    }

    /// **壊れると: モデルが使える文字とパレットの並びがずれる．**
    #[test]
    fn the_index_characters_follow_the_l0_alphabet() {
        assert_eq!(index_char(0), '0');
        assert_eq!(index_char(9), '9');
        assert_eq!(index_char(10), 'a');
        assert_eq!(index_char(36), 'A');
        assert_eq!(index_char(61), 'Z');
    }
}
