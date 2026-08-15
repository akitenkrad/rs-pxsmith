# ライブラリ

[English](library.md) | **日本語**

[← README へ戻る](../README.ja.md)

## クレート

| クレート | 中身 |
| --- | --- |
| [`pxsmith-core`](https://crates.io/crates/pxsmith-core) | データモデル・幾何基盤・純関数のアルゴリズム．I/O は持たない |
| [`pxsmith-io`](https://crates.io/crates/pxsmith-io) | 保持層 (`Document`)・`.aseprite`・パレット・L0 テキストの入出力 |
| [`pxsmith-lint`](https://crates.io/crates/pxsmith-lint) | 27 の品質ルールと閾値 — 21 は 1 枚に，6 はコマの列に掛かる |
| [`pxsmith-recipe`](https://crates.io/crates/pxsmith-recipe) | 制限された式評価器・依存グラフ・ステップキー・キャッシュ |
| [`pxsmith-macro`](https://crates.io/crates/pxsmith-macro) | Rust にスプライトを埋め込む `pixels!` proc-macro |
| [`pxsmith-gen`](https://crates.io/crates/pxsmith-gen) | 生成の輪 — 依頼・素性・検証と作り直し |

ワークスペースのうち 2 つは**公開していません**．`pxsmith-view`（端末プレビュー）は
`viuer` 経由で `ansi_colours` (LGPL-3.0-or-later) に届き，`pxsmith`（CLI）が
それに依存します．ビルド済みバイナリを配ると LGPL の再リンク義務が生じるので，
バイナリはソースから建てる形にしてあります．`pxsmith-calib` は閾値を決めるための
測定用ハーネスで，使ってもらうためのものではありません．

ライブラリ名はアンダースコア形なので，取り込みは `use pxsmith_core::…` です．

## コンパイル時にスプライトを埋め込む

```rust
use pxsmith_macro::pixels;

let frames = pixels!("sprites/hero_body.px.toml");
```

行の長さが揃っていなければ**コンパイルエラー**になり，参照しているパレットを
編集すれば再ビルドが走ります — マクロが `.hex` を追跡するので，
**パレットを変えたのに建て直し忘れる**ということが起きません．

## export クレートは無い

書き出し先（Tiled・スプライトシート・正規 JSON）は，独自のアルゴリズムを持たない
**出力アダプタ**です．だから直列化する対象のデータの隣（`pxsmith-core`）に置き，
CLI で繋いでいます．ここにクレートの境界を引くと，直列化器とその対象の型を
何の得もなく引き離すことになります．
