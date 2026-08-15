# ライブラリ

[English](library.md) | **日本語**

[← README へ戻る](https://github.com/akitenkrad/rs-pxsmith/blob/main/README.ja.md)

## クレート

| クレート | 中身 |
| --- | --- |
| [`pxsmith-core`](https://crates.io/crates/pxsmith-core) | データモデル・幾何基盤・純関数のアルゴリズム．I/O は持たない |
| [`pxsmith-io`](https://crates.io/crates/pxsmith-io) | 保持層 (`Document`)・`.aseprite`・パレット・L0 テキストの入出力 |
| [`pxsmith-lint`](https://crates.io/crates/pxsmith-lint) | 27 の品質ルールと閾値．21 は 1 枚に，6 はコマの列に掛かる |
| [`pxsmith-recipe`](https://crates.io/crates/pxsmith-recipe) | 制限された式評価器・依存グラフ・ステップキー・キャッシュ |
| [`pxsmith-macro`](https://crates.io/crates/pxsmith-macro) | Rust にスプライトを埋め込む `pixels!` proc-macro |
| [`pxsmith-gen`](https://crates.io/crates/pxsmith-gen) | 生成のループ．依頼・素性・検証と作り直し |

ワークスペースに含まれる残り 2 つのクレートは公開していません．`pxsmith-view` は端末
プレビューを担当しますが，`viuer` を経由して `ansi_colours` (LGPL-3.0-or-later) に
到達しており，CLI である `pxsmith` はこれに依存しています．ビルド済みバイナリを配布
すると LGPL の再リンク義務が生じるため，バイナリはソースから建てる形にしました．
`pxsmith-calib` は閾値を決定するための測定用ハーネスであり，利用者に使ってもらうことを
想定していません．

ライブラリ名はアンダースコア形になるため，取り込みは `use pxsmith_core::…` と記述します．

## コンパイル時にスプライトを埋め込む

```rust
use pxsmith_macro::pixels;

let frames = pixels!("sprites/hero_body.px.toml");
```

行の長さが揃っていない場合はコンパイルエラーになります．また参照しているパレットを編集
すると再ビルドが走ります．マクロが `.hex` ファイルを追跡しているため，パレットを変更した
にもかかわらず建て直しを忘れるという事態が起こりません．

## export クレートを設けなかった理由

書き出し先 (Tiled・スプライトシート・正規 JSON) は独自のアルゴリズムを持たない出力
アダプタです．そのため直列化する対象のデータと同じ `pxsmith-core` に置き，CLI 側で
繋いでいます．ここにクレートの境界を引くと，直列化器とその対象の型を何の利点もなく
引き離すことになるためです．
