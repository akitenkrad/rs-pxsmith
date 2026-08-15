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
| `generated/sample.aseprite` | `cargo run -p pxsmith-io --example make_sample` の出力 | 自作 | CC0 (自作) |
| `../palettes/sweetie-16.hex` | Sweetie 16 パレット | GrafxKid — https://lospec.com/palette-list/sweetie-16 | CC0 |
| `grid-eval/real/render/*.png` (25 件) | 実データ枠の `render` 区分．球と床のレイトレーシングを縮小し，合成データと同じ劣化をかけたもの | 自作 — `cargo run -p pxsmith-calib -- render` | CC0 (自作) |
| `grid-eval/seeds/*.png` (64 件) | **合成データセットの元絵**．Kenney Tiny Dungeon 16x16 (32 件) と Dungeon Crawl 32x32 (32 件) ．**実データ枠の 48 件とは重ならない**ものを選んである | 上と同じ 2 パック | CC0．原文は `grid-eval/real/other/LICENSE-*.txt` |
| `grid-eval/real/screenshot/000-008.png` (9 件) | 実データ枠の**正例 (組んだ画面)**．Tiled の見本地図を `pxsmith-calib scene` で元絵の解像度に描き出し，倍率 2 で拡大して劣化を通したもの．1 件 (`008`) は `Sample.png` が 418x211 の厳密な 3 倍だったので元絵を復元して作った | Kenney 各パックの `Tiled/*.tmx` ・`Map/*.tmx` | CC0．原文は `grid-eval/real/screenshot/LICENSE-kenney.txt` |
| `grid-eval/real/screenshot/neg-*.png` (10 件) | 実データ枠の**負例**．Kenney の紹介用レンダ (918x515)．**非整数倍で拡大され補間も掛かっている**ので整数の格子が無い | Kenney 各パックの `Sample*.png` | 同上 |
| `grid-eval/real/other/000-023.png` (24 件) | 実データ枠の**正例**．16x16 の元絵をこちらが決めた倍率 (2〜12) で拡大し，合成データと同じ劣化を通したもの | Kenney — Tiny Dungeon https://kenney.nl/assets/tiny-dungeon | CC0．原文は `grid-eval/real/other/LICENSE-kenney-tiny-dungeon.txt` |
| `grid-eval/real/other/024-047.png` (24 件) | 同上．元絵は 32x32 | Dungeon Crawl Stone Soup — https://opengameart.org/content/dungeon-crawl-32x32-tiles | CC0．原文は `grid-eval/real/other/LICENSE-dungeon-crawl.txt` |
| `grid-eval/real/ai-output/NNN.png` (28 件) | 実データ枠の**正例**．下の 28 枚を周期 32 で縮小し，こちらが決めた倍率 (2〜12) で拡大し直したもの．正解は拡大倍率 | 同下 | 同下 |
| `grid-eval/real/ai-output/neg-*.png` (28 件) | 実データ枠の**負例**．ドット絵風だが整数の格子が無い画像．元画像の中央 256 画素角を切り出しただけで再標本化はしていない | 自作 — ChatGPT で生成 (2026-08-11) ．原寸は `grid-eval/real/local/_sources/ai-output/` (追跡しない) | CC0 (自作)．OpenAI 規約により出力の権利は利用者に属する |

## 未調達のもの

| 用途 | 必要量 | 置き場所 | ゲートするもの |
| --- | --- | --- | --- |
| **独立した `.aseprite` 素材** | 数点．**最新版 Aseprite が書いた，未知チャンクを含みうるもの** | `aseprite/independent/` | R3 の残り (下記) |
| 合成データの種 | 済 (64 件．CC0 のドット絵) ．**差し替えで検証セットの成績が 88.8% → 67.5% に落ち，実データと一致した** | `grid-eval/seeds/` | M2 の格子推定の補正 |
| 実データ (格子推定) — `render` | 済 (25 件，自作レンダ．正例 9 ・負例 16) | `grid-eval/real/render/` | — |
| 実データ (格子推定) — `ai-output` | 済 (28 枚から正例 28 ・負例 28) ．拾った時点では全件に格子が無く，正例は縮小 → 既知倍率で拡大して作った | `grid-eval/real/ai-output/` | M2 の完了条件 |
| 実データ (格子推定) — `other` (CC0 のドット絵) | 済 (48 件．元絵 16x16 ・32x32 をこちらが決めた倍率で拡大) ．**中身が本物のドット絵である同梱可能な正例** | `grid-eval/real/other/` | M2 の完了条件 |
| 実データ (格子推定) — `screenshot` | 済 (正例 9 ・負例 10) ．CC0 素材の見本地図を元絵の解像度で描き出したもの | `grid-eval/real/screenshot/` | M2 の完了条件 |

### 再配布できない素材の扱い

同梱できない素材 (再配布を禁じる規約のもの) は `grid-eval/real/local/` へ置き，
**追跡しない** (`.gitignore`) ．入手元と規約だけをここに記録する．評価はその環境でしか
再現できないので，調査記録にもその旨を書く．

| 入手元 | 規約 | 状態 |
| --- | --- | --- |
| ゲームまてりあるず — https://game-materials.com/userpolicy/ | 商用可・加工可・**素材としての再配布は禁止**．クレジットは任意 | 47 件 (前回の 22 件を含む) ．**正例にはならず，負例として `local/screenshot/` に置いた** (下記) |
| DOT ILLUST — https://dot-illust.net/terms/ | 加工可・クレジット不要・商用は 1 制作物 30 点まで無料．**素材そのものも加工したものも再配布・販売は禁止**．他に商標登録 ・NFT 化 ・ジェネレーター利用等を禁止 | 23 件．**本物のドット絵だが 22 件は非整数倍で拡大されて配られている** (下記) ．`local/other/` に置いた |

`local/` の中身 (追跡しない)．

| パス | 中身 |
| --- | --- |
| `local/screenshot/neg-*.png` (47 件) | ゲームまてりあるずの**負例**．原寸のまま (1920x1080) ．47 件すべてが正しく棄却されている |
| `local/other/neg-*.png` (22 件) | DOT ILLUST の**負例**．原寸のまま (幅 500) ．本物のドット絵だが幅 500 へ非整数倍で拡大されており，整数の格子が存在しない．22 件すべてが正しく棄却されている |
| `local/other/NNN.png` (23 件) | DOT ILLUST の**正例**．元絵 (12x14〜39x19) を厳密に復元し，こちらが決めた倍率 (2〜12) で拡大し直したもの．**中身は本物のドット絵**である |
| `local/manifest.json` | その目録．`cargo run -p pxsmith-calib --release -- real --dir testdata/grid-eval/real/local` で採点する |
| `local/_sources/screenshot/` (47 件) | 上記の元ファイル |
| `local/_sources/other/` (23 件) | 同上 |
| `local/_sources/ai-output/` (28 件) | 同梱した AI 出力の負例の**原寸**．切り出すと消える誤受理があるため残してある (1024 画素角以上でのみ現れる) |

> [!warning] 「ドット絵風」は目視では判定できない
> 縮小表示ではブロックが並んで見えるが，拡大すると縁が滑らかで境界が無い．
> **画風であって格子ではない**．隣接行の近似一致率も当てにならない — 平坦な領域が
> 広いと 90% を超える．`pxsmith-calib ingest` の判定 (縁が境界へ集中しているか) で
> 確かめること．
>
> 検査した 75 件 (配布素材 47 ・AI 出力 28) は，**隣接行の完全一致率が 1 件も 0 を
> 超えなかった**．最近傍で $s$ 倍した画像なら $(s-1)/s$ になる量である．
> 詳細は `docs/investigations/grid-calibration.md`．
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
