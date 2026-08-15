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
//!
//! # 流で受ける — 黙って受け取ると 60 秒で切れる (D162)
//!
//! **一括で受けると «思考している間ずっと 1 バイトも流れない»** ので，
//! 経路のどこかが遊んでいる接続と見て畳む．実測で**60.07 秒ちょうど**に
//! 3 回とも切れ，`ureq` は «peer closed connection without TLS close_notify»
//! としか言えなかった — **依頼は受理されていたのに，応答を 1 度も見られない．**
//!
//! 上限の話ではない ・宛先の話でもない: 60 秒未満の依頼は 12 / 12 通り，
//! **バイトが流れ続ける接続は 200 秒でも切れない**．`stream: true` にすると
//! 前置きと ping が絶えず流れるので，**176 秒 ・245 秒の応答が通った**．
//!
//! 公式の手引きも «長い出力や大きな `max_tokens` では既定で流にすること —
//! 依頼の時間切れを防ぐ» と書いている．**流すのが既定の正しい書き方である．**

use std::io::{BufRead, BufReader, Read};

use crate::error::{GenError, Result};
use crate::repair::Generator;
use crate::request::{GenKind, GenRequest};

/// 鍵を置く環境変数．**依頼にも素性にも鍵は書かない．**
pub const KEY_VAR: &str = "ANTHROPIC_API_KEY";

/// 送る API の版．
const API_VERSION: &str = "2023-06-01";

/// 断られたときに別のモデルへ回す (Claude API のみ)．
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

/// 出力の上限．**思考と本文の合算に掛かる** (D160)．
///
/// 流で受けるので時間切れを気にせず取れる — 手引きの «流すなら 64000» に合わせる
/// (一括で受けていたころの 16000 は «時間切れしない上限» であって，
/// 絵に要る量ではなかった) ．上限であって目標ではないので，上げても高くならない．
///
/// **依頼ごとに変えられる** (`GenRequest::max_tokens` ・`--max-tokens`．D165) —
/// 固定したままだと [`GenError::Truncated`] の道が**偽バックエンドの試験でしか
/// 通らない**．本物の応答で 1 度も通っていない道は «ある» とは言えない
/// (D80 ・D145 ・D162 ・D164 と同じ側)．
pub const DEFAULT_MAX_TOKENS: u32 = 64000;

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
    s.push_str("- 影は黒へ落とさず，暗い色を使う\n");

    // **コマが 2 つ以上あるときだけ，列の作法も書く** (D166)．
    //
    // この節は «この後の検査に掛かる» と名乗っているのに，**フレーム間の
    // 5 つ (22 〜 26 が blocking) を 1 つも書いていなかった**．
    // D165 で輪が列を検査するようにしたので，**書かないと «知らされていない
    // 規則で落とす»** ことになる — モデルが下手なのではなく，こちらが
    // 言っていないだけである．
    //
    // **規則の側から引く** (名前ではなく «何を見ているか» から書く．D111 ・D162)．
    // 1 コマの依頼には足さない — 掛からない検査を書いても守りようがない．
    if req.constraints.frames > 1 {
        s.push_str("\nコマを並べるときの作法 (フレーム間の検査に掛かる):\n");
        s.push_str("- 動く部分は**平行移動で動かす**．コマごとに輪郭を描き直して線を揺らさない\n");
        s.push_str("- ディザ (市松) は**物体と一緒に動かす**．画布に貼り付けたままにしない\n");
        s.push_str("- 動かした跡に**幅 1 の列や行を取り残さない**\n");
        s.push_str("- `kind = \"inbetween\"` と書いたコマでは，**穴の数も部品の数も変えない**\n");
    }
    s.push('\n');

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
        "max_tokens": req.max_tokens,
        // **流で受ける** (D162) — 一括で受けると思考の間 1 バイトも流れず，
        // 60 秒で経路に畳まれる．依頼の中身ではなく «受け取り方» の欄である
        "stream": true,
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

/// 組み立てた依頼をそのまま読める形で返す (`--dry-run` 用)．
///
/// > [!note] **鍵は入らない．** 鍵はヘッダにしか無く，本文には 1 度も現れない
/// > ので，これをそのまま画面へ出しても漏れない．
///
/// **手引きが約束していることを道具にさせるために足した** — 引き継ぎは
/// «通らないときは `--dry-run` で組み立てた依頼を読め» と書いていたのに，
/// 実際に出るのは宛先 ・寸法 ・鍵 ・パレットの行だけだった (D162)．
/// **鍵を使わずに «送る側» を確かめられる唯一の道である．**
pub fn preview_request(req: &GenRequest, palette_ref: &str) -> String {
    serde_json::to_string_pretty(&build_body(req, palette_ref, None))
        .unwrap_or_else(|e| format!("依頼を組み立てられない: {e}"))
}

impl Generator for AnthropicGenerator {
    fn generate(&self, req: &GenRequest, feedback: Option<&str>) -> Result<String> {
        if req.kind != GenKind::Prog {
            return Err(GenError::NotWritten { kind: req.kind });
        }
        let body = build_body(req, &self.palette_ref, feedback);

        // **状態番号をエラーにしない** — 既定では 4xx が `Err(StatusCode)` になり，
        // **本文が落ちる**．理由が入っているのは本文の方である (下の実測を見よ)．
        let mut request = ureq::post(&req.backend.endpoint)
            .config()
            .http_status_as_error(false)
            .build()
            .header("content-type", "application/json")
            .header("accept", "text/event-stream")
            .header("x-api-key", &self.key)
            .header("anthropic-version", API_VERSION);
        if req.backend.name == "anthropic" {
            request = request.header("anthropic-beta", FALLBACK_BETA);
        }

        let mut response = match request.send_json(&body) {
            Ok(r) => r,
            Err(e) => {
                return Err(GenError::Backend {
                    message: e.to_string(),
                });
            }
        };

        let status = response.status().as_u16();

        // **弾かれたときは流ではなく普通の JSON が来る** — 本文を捨てない (D159)
        if !(200..300).contains(&status) {
            let text = response
                .body_mut()
                .read_to_string()
                .map_err(|e| GenError::Backend {
                    message: e.to_string(),
                })?;
            return Err(error_from_status(status, &text));
        }

        read_stream(response.body_mut().as_reader())?.finish(req.max_tokens)
    }

    fn describe(&self) -> String {
        format!("anthropic (鍵は ${KEY_VAR})")
    }
}

/// エラー応答から理由を取り出す．**本文を捨てない．**
///
/// > [!warning] **実測で 1 件出た．状態番号だけでは何も読めない．**
/// > 期限切れの鍵で叩いたとき，道具は «HTTP 401» としか言わなかったが，
/// > 同じ応答の本文には «API key is invalid.» と書いてあった．
/// > `output_config` や `fallbacks` が弾かれたときも同じで，**弾かれた欄の
/// > 名前は本文にしか無い**．状態番号は残しつつ本文を併記する．
fn error_from_status(status: u16, text: &str) -> GenError {
    let reason = serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .and_then(|m| m.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| {
            let body = text.trim();
            if body.is_empty() {
                "本文が空".to_string()
            } else {
                // JSON でない本文 (串の HTML など) もそのまま見せる
                body.chars().take(400).collect()
            }
        });
    GenError::Backend {
        message: format!("HTTP {status}: {reason}"),
    }
}

/// 応答から «判断に要るもの» だけを取り出した形．
///
/// **受け取る仕事と判断する仕事を分ける**ため置いてある．[`read_stream`] は
/// «SSE をこの形へ落とす» だけを行い，«何をエラーとするか» は
/// [`Outcome::finish`] だけが決める — 流は `stop_reason` が**本文より後に
/// 届く**ので，読みながら判断すると «先に見る» が崩れるからである．
#[derive(Default, Debug)]
struct Outcome {
    stop_reason: Option<String>,
    refusal_category: Option<String>,
    /// 最初の `text` ブロックの中身 (思考ブロックは入れない)．
    text: Option<String>,
}

impl Outcome {
    /// **`stop_reason` を本文より先に見る** — 断られたときや切れたときは
    /// `content` が空か途中までしかないので，本文から読むとそこで落ちて
    /// **本当の理由が消える**．
    /// `max_tokens` は**報告のために渡す** — 依頼ごとに違うので
    /// 定数から読むと «こちらが要求した上限» を言えなくなる (D165)．
    fn finish(self, max_tokens: u32) -> Result<String> {
        // **上限に当たったのを先に見る** — 構造化出力は途中で切れると JSON に
        // ならないので，見ないと «JSON になっていない» と誤診する．原因は
        // «壊れた応答» ではなく «こちらが要求した上限» である (D160)．
        if self.stop_reason.as_deref() == Some("max_tokens") {
            return Err(GenError::Truncated { max_tokens });
        }

        // **断りも本文より先に見る**
        if self.stop_reason.as_deref() == Some("refusal") {
            return Err(GenError::Refused {
                category: self.refusal_category.unwrap_or_else(|| "不明".to_string()),
            });
        }

        let raw = self.text.ok_or_else(|| GenError::BadResponse {
            message: "text のブロックが 1 つも無い".to_string(),
        })?;

        // 形を固定してあるので JSON で戻る
        let parsed: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| GenError::BadResponse {
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
}

/// 流 (SSE) を [`Outcome`] へ落とす．
///
/// > [!note] **一括の応答を読む関数は消した** (D162)．
/// > `stream: true` は常に付くので，一括の JSON を読む道はもう通らない．
/// > 残しておくと «同じ仕事の実装が 2 つ» になり (D110)，しかも**試験だけが
/// > 通り続ける** — D80 ・D145 «補助関数が構造的に空だった» の新しい形である．
/// > 消した側が見ていた誤り (断り ・上限 ・思考ブロック ・エラー本文) は
/// > すべて流の側の試験へ移してある．
///
/// > [!warning] **`stop_reason` は最後に来る．**
/// > 一括の応答では本文と一緒に届くが，流では `message_delta` で**後から**届く．
/// > だから «先に見る» を «先に読む» と実装してはいけない — **ぜんぶ受けてから
/// > [`Outcome::finish`] に渡す**ことで，判断の順序だけを保つ．
///
/// **思考ブロックを本文と取り違えない** — `content_block_start` で始まりの
/// 型を覚え，`text` の添字に来た差分だけを繋ぐ (思考は既定で中身が空だが，
/// «空だから害が無い» は理由にならない．D159 «読めるなら読む» の裏返しで，
/// **区別できるものを区別しないと，中身が入った日に静かに壊れる**) ．
fn read_stream(reader: impl Read) -> Result<Outcome> {
    let mut out = Outcome::default();
    let mut text_indices: Vec<u64> = Vec::new();
    let mut buf = String::new();

    for line in BufReader::new(reader).lines() {
        let line = line.map_err(|e| GenError::Backend {
            message: format!("流が途中で切れた: {e}"),
        })?;
        // SSE は `event:` 行と `data:` 行の対だが，型は本文にも入っている
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(data) {
            Ok(v) => v,
            // **読めない行で落とさない** — ping や将来の欄を知らなくても進む
            Err(_) => continue,
        };

        match v.get("type").and_then(|t| t.as_str()) {
            // 流の途中でも «error» は来る．本文を捨てない (D159)
            Some("error") => {
                return Err(GenError::Backend {
                    message: error_message(&v),
                });
            }
            Some("content_block_start") => {
                let is_text =
                    v.pointer("/content_block/type").and_then(|t| t.as_str()) == Some("text");
                if let (true, Some(i)) = (is_text, v.get("index").and_then(|i| i.as_u64())) {
                    text_indices.push(i);
                }
            }
            Some("content_block_delta") => {
                let index = v.get("index").and_then(|i| i.as_u64());
                let is_text_delta =
                    v.pointer("/delta/type").and_then(|t| t.as_str()) == Some("text_delta");
                if let (Some(i), true) = (index, is_text_delta)
                    && text_indices.contains(&i)
                    && let Some(chunk) = v.pointer("/delta/text").and_then(|t| t.as_str())
                {
                    buf.push_str(chunk);
                }
            }
            // **止まった理由はここで届く** — 本文より «後» だが «先» に効く
            Some("message_delta") => {
                if let Some(s) = v.pointer("/delta/stop_reason").and_then(|s| s.as_str()) {
                    out.stop_reason = Some(s.to_string());
                }
                if let Some(c) = v
                    .pointer("/delta/stop_details/category")
                    .and_then(|c| c.as_str())
                {
                    out.refusal_category = Some(c.to_string());
                }
            }
            _ => {}
        }
    }

    if !buf.is_empty() {
        out.text = Some(buf);
    }
    Ok(out)
}

/// エラー応答の本文から理由を取り出す (流でも一括でも同じ形)．
fn error_message(v: &serde_json::Value) -> String {
    v.pointer("/error/message")
        .and_then(|m| m.as_str())
        .unwrap_or("理由の記載が無い")
        .to_string()
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
            max_tokens: DEFAULT_MAX_TOKENS,
        }
    }

    /// **壊れると: «HTTP 401» としか言わず，何が弾かれたのか読めない．**
    ///
    /// 実測でここを 1 度踏んでいる — 本文には «API key is invalid.» と
    /// 書いてあったのに，道具は状態番号しか出さなかった．
    #[test]
    fn an_error_status_carries_the_body_not_just_the_number() {
        let body = r#"{"type":"error","error":{"type":"authentication_error","message":"API key is invalid."}}"#;
        match error_from_status(401, body) {
            GenError::Backend { message } => {
                assert!(message.contains("401"), "状態番号が消えている: {message}");
                assert!(
                    message.contains("API key is invalid."),
                    "本文が消えている: {message}"
                );
            }
            other => panic!("エラーとして読めていない: {other:?}"),
        }
    }

    /// **壊れると: JSON でない本文 (串の HTML など) を黙って捨てる．**
    #[test]
    fn a_non_json_error_body_is_still_shown() {
        match error_from_status(502, "<html>Bad Gateway</html>") {
            GenError::Backend { message } => {
                assert!(message.contains("502"));
                assert!(
                    message.contains("Bad Gateway"),
                    "本文が消えている: {message}"
                );
            }
            other => panic!("エラーとして読めていない: {other:?}"),
        }
    }

    /// **壊れると: 空の本文で «本文が空» ではなく状態番号だけになる．**
    #[test]
    fn an_empty_error_body_says_so() {
        match error_from_status(500, "   ") {
            GenError::Backend { message } => assert!(message.contains("本文が空")),
            other => panic!("エラーとして読めていない: {other:?}"),
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

    /// **壊れると: 知らせていない規則で列を落とす** (D166)．
    ///
    /// D165 で輪がフレーム間のルールを検査するようにしたので，**掛ける規則は
    /// 依頼にも書いてなければならない**．書かなければモデルが下手なのではなく，
    /// こちらが言っていないだけである．
    #[test]
    fn a_multi_frame_request_is_told_the_sequence_rules() {
        let mut r = req();
        r.constraints.frames = 4;
        let s = system_prompt(&r, "t.px.hex");
        assert!(s.contains("フレーム間の検査"), "列の作法が無い: {s}");
        assert!(s.contains("平行移動"), "揺れる線 (23) の作法が無い");
        assert!(s.contains("ディザ"), "ディザの位相 (24) の作法が無い");
        assert!(s.contains("幅 1 の列"), "孤立列 (25) の作法が無い");
        assert!(s.contains("inbetween"), "トポロジー (22) の作法が無い");
    }

    /// **壊れると: 1 コマの依頼に掛かりもしない検査の作法が混ざる．**
    ///
    /// 掛からない規則を書いても守りようがなく，**«検査に掛かる» という
    /// 見出しが嘘になる**．
    #[test]
    fn a_single_frame_request_is_not_told_about_sequences() {
        let s = system_prompt(&req(), "t.px.hex");
        assert!(
            !s.contains("フレーム間の検査"),
            "1 コマなのに列の作法が入っている: {s}"
        );
    }

    /// 台本を SSE の形に組む (`event:` 行も本物と同じように付ける)．
    fn sse(events: &[serde_json::Value]) -> String {
        let mut s = String::new();
        for e in events {
            let name = e.get("type").and_then(|t| t.as_str()).unwrap_or("message");
            s.push_str(&format!("event: {name}\ndata: {e}\n\n"));
        }
        s
    }

    fn text_start(index: u64) -> serde_json::Value {
        serde_json::json!({
            "type": "content_block_start", "index": index,
            "content_block": {"type": "text", "text": ""},
        })
    }

    fn text_delta(index: u64, text: &str) -> serde_json::Value {
        serde_json::json!({
            "type": "content_block_delta", "index": index,
            "delta": {"type": "text_delta", "text": text},
        })
    }

    /// **壊れると: 依頼が一括で飛び，60 秒を超える応答が 1 度も受け取れない** (D162)．
    #[test]
    fn the_request_asks_for_a_stream() {
        let body = build_body(&req(), "t.px.hex", None);
        assert_eq!(
            body.get("stream").and_then(|s| s.as_bool()),
            Some(true),
            "流を頼んでいない — 思考の間 1 バイトも流れず経路に畳まれる"
        );
    }

    /// **壊れると: 流で来た本文を繋げられない．**
    #[test]
    fn a_streamed_body_is_reassembled_from_its_deltas() {
        let inner = serde_json::json!({"l0": "[meta]\nformat = 1\n"}).to_string();
        let (a, b) = inner.split_at(7);
        let body = sse(&[
            serde_json::json!({"type": "message_start"}),
            text_start(0),
            text_delta(0, a),
            text_delta(0, b),
            serde_json::json!({"type": "content_block_stop", "index": 0}),
            serde_json::json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}}),
            serde_json::json!({"type": "message_stop"}),
        ]);
        let l0 = read_stream(body.as_bytes())
            .unwrap()
            .finish(DEFAULT_MAX_TOKENS)
            .unwrap();
        assert!(l0.starts_with("[meta]"), "本文が繋がっていない: {l0}");
    }

    /// **壊れると: 思考ブロックを本文と取り違える** (一括の側と同じ不変条件)．
    #[test]
    fn a_streamed_thinking_block_is_not_mistaken_for_the_answer() {
        let inner = serde_json::json!({"l0": "ok"}).to_string();
        let body = sse(&[
            serde_json::json!({
                "type": "content_block_start", "index": 0,
                "content_block": {"type": "thinking", "thinking": ""},
            }),
            serde_json::json!({
                "type": "content_block_delta", "index": 0,
                "delta": {"type": "thinking_delta", "thinking": "ぐるぐる"},
            }),
            text_start(1),
            text_delta(1, &inner),
            serde_json::json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}}),
        ]);
        assert_eq!(
            read_stream(body.as_bytes())
                .unwrap()
                .finish(DEFAULT_MAX_TOKENS)
                .unwrap(),
            "ok"
        );
    }

    /// **壊れると: 流の断りを «壊れた応答» と読んで作り直しに行く．**
    ///
    /// 流では `stop_reason` が**本文より後に届く**ので，«先に見る» を
    /// «先に読む» と書くとここで崩れる．
    #[test]
    fn a_streamed_refusal_is_reported_as_a_refusal_even_though_it_arrives_last() {
        let body = sse(&[
            text_start(0),
            text_delta(0, "途中まで書いた"),
            serde_json::json!({
                "type": "message_delta",
                "delta": {"stop_reason": "refusal", "stop_details": {"category": "cyber"}},
            }),
        ]);
        match read_stream(body.as_bytes())
            .unwrap()
            .finish(DEFAULT_MAX_TOKENS)
        {
            Err(GenError::Refused { category }) => assert_eq!(category, "cyber"),
            other => panic!("断りとして読めていない: {other:?}"),
        }
    }

    /// **壊れると: 切れた流を «JSON になっていない» と誤診する** (D160)．
    #[test]
    fn a_streamed_truncation_is_reported_as_truncation_not_as_broken_json() {
        let body = sse(&[
            text_start(0),
            text_delta(0, "{\"l0\": \"[meta]"),
            serde_json::json!({"type": "message_delta", "delta": {"stop_reason": "max_tokens"}}),
        ]);
        match read_stream(body.as_bytes())
            .unwrap()
            .finish(DEFAULT_MAX_TOKENS)
        {
            Err(GenError::Truncated { max_tokens }) => assert_eq!(max_tokens, DEFAULT_MAX_TOKENS),
            other => panic!("上限として読めていない: {other:?}"),
        }
    }

    /// **壊れると: 流の途中で来たエラーの理由が消える** (D159 と同じ側)．
    #[test]
    fn a_mid_stream_error_keeps_its_reason() {
        let body = sse(&[
            text_start(0),
            serde_json::json!({
                "type": "error",
                "error": {"type": "overloaded_error", "message": "Overloaded"},
            }),
        ]);
        match read_stream(body.as_bytes()) {
            Err(GenError::Backend { message }) => assert!(
                message.contains("Overloaded"),
                "理由が消えている: {message}"
            ),
            other => panic!("エラーとして読めていない: {other:?}"),
        }
    }

    /// **壊れると: 知らない行 (ping や新しい欄) で輪ごと落ちる．**
    #[test]
    fn unknown_lines_do_not_stop_the_stream() {
        let inner = serde_json::json!({"l0": "ok"}).to_string();
        let mut body =
            String::from(": これはコメント行\n\nevent: ping\ndata: {\"type\":\"ping\"}\n\n");
        body.push_str("data: これは JSON ではない\n\n");
        body.push_str(&sse(&[
            text_start(0),
            text_delta(0, &inner),
            serde_json::json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}}),
        ]));
        assert_eq!(
            read_stream(body.as_bytes())
                .unwrap()
                .finish(DEFAULT_MAX_TOKENS)
                .unwrap(),
            "ok"
        );
    }

    /// **壊れると: `--dry-run` が «組み立てた依頼» を見せない — 鍵を使わずに
    /// 送る側を確かめる道が無くなる** (D162)．
    #[test]
    fn the_preview_shows_the_assembled_request_and_never_the_key() {
        let text = preview_request(&req(), "t.px.hex");
        for field in ["output_config", "effort", "stream", "system", "max_tokens"] {
            assert!(text.contains(field), "{field} が依頼の写しに無い");
        }
        assert!(!text.contains("x-api-key"), "鍵の欄が本文に混ざっている");
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
