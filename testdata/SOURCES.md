# 検証素材の出典

`testdata/` の素材はリポジトリにコミットされるため **CC0 を原則とする**
(実装計画書 5 章)．CC-BY / CC-BY-SA / GPL は除外する．

> [!note] MIT の例外
> **著作権表示を同梱すれば再配布できるライセンス (MIT) は受け入れる**．CC0 限定と
> したのは帰属義務を負わないためであり，表示の同梱で足りるものまで排除する必要は
> ない．受け入れる場合は原文の LICENSE を素材と同じディレクトリに置き，下の表に
> 記録する．CC-BY / CC-BY-SA / GPL の除外は変わらない (成果物側に条件が伝播するため)．

## 一覧

| パス | 内容 | 出典 | ライセンス |
| --- | --- | --- | --- |
| `aseprite/aseprite-tests/*.aseprite` (19 件) | Aseprite 本体のテストスプライト．タイルマップ・リンクセル・グループ・スライス・タグ・ユーザデータ props を含む | Aseprite 公式 — https://github.com/aseprite/aseprite/tree/main/tests/sprites (`aseprite-io` 0.2.0 の `tests/fixtures/` 経由で取得) | MIT (Igara Studio S.A. / David Capello)．原文は同ディレクトリの `LICENSE` |
| `generated/sample.aseprite` | `cargo run -p px-io --example make_sample` の出力 | 自作 | CC0 (自作) |
| `../palettes/sweetie-16.hex` | Sweetie 16 パレット | GrafxKid — https://lospec.com/palette-list/sweetie-16 | CC0 |
| `grid-eval/real/render/*.png` (25 件) | 実データ枠の `render` 区分．球と床のレイトレーシングを縮小し，合成データと同じ劣化をかけたもの | 自作 — `cargo run -p px-calib -- render` | CC0 (自作) |

## 未調達のもの

| 用途 | 必要量 | 置き場所 | ゲートするもの |
| --- | --- | --- | --- |
| **独立した `.aseprite` 素材** | 数点．**最新版 Aseprite が書いた，未知チャンクを含みうるもの** | `aseprite/independent/` | R3 の残り (下記) |
| 合成データの種 | きれいなドット絵．ここから 500 件を合成する | `grid-eval/seeds/` | M2 の格子推定の補正 |
| 実データ (格子推定) — `render` | 済 (25 件，自作レンダ) | `grid-eval/real/render/` | — |
| 実データ (格子推定) — `ai-output` | 未 | `grid-eval/real/ai-output/` | M2 の完了条件 |
| 実データ (格子推定) — `screenshot` | 未．CC0 素材で組んだ画面 | `grid-eval/real/screenshot/` | M2 の完了条件 |

### 再配布できない素材の扱い

同梱できない素材 (再配布を禁じる規約のもの) は `grid-eval/real/local/` へ置き，
**追跡しない** (`.gitignore`) ．入手元と規約だけをここに記録する．評価はその環境でしか
再現できないので，調査記録にもその旨を書く．

| 入手元 | 規約 | 状態 |
| --- | --- | --- |
| ゲームまてりあるず — https://game-materials.com/userpolicy/ | 商用可・加工可・**素材としての再配布は禁止**．クレジットは任意 | 22 件を検討したが**格子が無く不採用** (下記) |

> [!warning] 「ドット絵風」は目視では判定できない
> 上の 22 件は縮小表示ではブロックが並んで見えるが，拡大すると縁が滑らかで境界が
> 無かった．**画風であって格子ではない**．隣接行の近似一致率も当てにならない —
> 平坦な領域が広いと 90% を超える．`px-calib ingest` の判定 (縁が境界へ集中して
> いるか) で確かめること．
| lint 正例 | CC0 の良質なドット絵 | `lint-cases/positive/` | M2・M3 の閾値決定 (誤検出率の測定) |
| lint 負例 | 自作の失敗例 (ジャギー・バンディング・pillow shading・単色影・揺れる線) | `lint-cases/negative/` | M2・M3 の閾値決定 (検出率の測定) |

**書籍の図版は使えない** (著作物である)．記述されている失敗パターンを自分で再現して作る．
Obsidian の `本棚/pdfs/` にある参考書籍 2 冊 (`Pixel Logic` / `ULTIMATE PIXEL CREW REPORT`)
は**実装時の参照にのみ使い，図版を `testdata/` へ持ち込まない**．

商用ゲームのスクリーンショットは CC0 ではないので実データ枠に入れない．

### R3 の残り

`aseprite/aseprite-tests/` は `aseprite-io` 自身のテスト用 fixtures でもあるため，
**`aseprite-io` の忠実性を独立に検証したことにはならない** (向こうも同じファイルで
CI を回している)．我々が書いた `Document` ・射影・`merge_back` の検証としては本物だが，
R3 (「`.aseprite` 仕様の非公開挙動」) を完全に潰すには，**別系統の素材**が要る．
最新版 Aseprite で自作したファイルを `aseprite/independent/` へ置くのが最も確実である．

## 入手先

- Kenney — https://kenney.nl (全て CC0)
- OpenGameArt の CC0 コレクション
- itch.io の CC0 タグ
- Lospec のパレット (個別にライセンス表記を確認する)
