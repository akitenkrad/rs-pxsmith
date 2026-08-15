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

このほかに 2 つのクレートを公開しています．`pxsmith-view` は端末プレビューを担当し，
`pxsmith` はコマンドライン本体で，`cargo install pxsmith` で入ります．公開していないのは
閾値を決定するための測定用ハーネス `pxsmith-calib` だけで，これは利用者に使ってもらうことを
想定していないためです．

LGPL の依存に到達するクレートは `pxsmith-view` だけであり，しかも単一の feature を
経由します．`term::detect` と `term::show` が `viuer` を使い，`viuer` が
`ansi_colours` (LGPL-3.0-or-later) を引き込みますが，この 2 関数はいずれも既定の
`terminal` feature の後ろにあります．`--no-default-features` で取り込めば `viuer` と
`ansi_colours` は依存ツリーから完全に消え，`render`・`diff`・`onion` と `TerminalKind` は
そのまま使えます．

```toml
pxsmith-view = { version = "0.1", default-features = false }
```

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
