# 変更履歴

[English](CHANGELOG.md) | **日本語**

このワークスペースのクレートはすべて同じ版番号を使います．書式は
[Keep a Changelog](https://keepachangelog.com/ja/1.1.0/) に，版番号は Cargo が
`0.x` に適用する [Semantic Versioning](https://semver.org/lang/ja/) に従います．
つまり，副版 (minor) が上がると公開 API が壊れることがあります．

## [0.2.0] - 未公開

この版は `pxsmith-core` の公開 API を壊します．L0 テキスト形式は版 1 のままであり，
0.1 が書いた `.px.toml` はすべてそのまま読めます．

### 互換性のない変更

- **`pxsmith_core::compose::Part::anchors` の型が `BTreeMap<String, IVec2>` から
  `Anchors` に変わりました．** `Anchors::default` は全フレームで共有する位置，
  `Anchors::frames` はフレームごとの上書きです．マップを直接組み立てていたコードは
  `Anchors { default: map, frames: vec![] }` へ移せば 0.1 と同じに動きます．
  位置は `Anchors::get(frame, name)` で引きます．
- `compose::Placement` に，合成後のフレームごとの位置を持つ `offsets` が増えました．
  `offset` は 0.1 と同じ意味 (第 0 フレームの位置) のままです．
- `sheet::SheetCell` に `palette` が，`sheet::SheetDoc` に `palettes` が増えました．
  0.1 が書いていた形式 1 のメタではどちらも `None` です．これらの型を構造体リテラルで
  作っているコードはフィールドを足す必要があります．
- `CoreError::ComposeAnchorMissing` に `frame` が増え，`CoreError::SheetVersion` からは
  `expected` が無くなりました (受け付ける形式が 2 つになったため)．`CoreError` は
  `#[non_exhaustive]` なので，下に挙げる新しい列挙子は網羅的な `match` を壊しません．
- `IndexedCanvas::opaque_bbox` は，透明添字を持たない画布に対して `None` ではなく
  画布全体を返すようになりました．透明添字の無い画布は全面が不透明であり，
  `None` は «描くものが無い» と読まれていました．

### 追加

- **L0 のフレームごとのアンカー．** `[[frame]]` は自分の `anchors` 表で `[meta]` の
  アンカーを個別に上書きできます．上書きの無いフレームは 0.1 と同じ正規形のバイト列の
  ままです．合成では各パーツがそのとき表示しているフレームのアンカーを使うので，
  静止したパーツを動く胴体に追従させられます．画布は全フレームから大きさを決めるので，
  詰めた升の大きさは変わりません．
- `pxsmith anchor track` は全フレームの解決済みアンカー位置を JSON に書き出し，
  `pxsmith anchor apply` はその軌跡を別の L0 ファイルへ書き込みます．
- **色覚多様性の検査．** `pxsmith palette cvd` は 1 型・2 型・3 型の見え方を
  模擬し (Machado, Oliveira, Fernandes 2009．線形 RGB で適用)，OKLCh の
  丁度可知差 1 つ分を下回る色の組を報告します．`--pair` で指定した組は
  «遮断» 扱いで，見分けられなくなると終了コード 1 を返します．
  ライブラリ側は `pxsmith_core::cvd` です．
- **パレットスワップ用のシート出力．** `pxsmith sheet pack --palette-swap` は，
  全入力を 256 色のパレット 1 枚へ統合する代わりに，添字テクスチャとパレット
  アトラスを書き出します．`--variant BASE.hex=VARIANT.hex` で色違いの行を足せます．
  メタはシート形式 2 で，既定は形式 1 のままです．`pxsmith sheet resolve` は
  シェーダと同じ引き当てをコマンドラインで行い，`examples/godot-palette-swap/` は
  同じ引き当てを Godot 4.7 で示します．ライブラリ側は
  `pxsmith_core::sheet::{pack_palette_swap, resolve}` です．

### 修正

- `pxsmith quantize` が透明を壊さなくなりました．透明背景の PNG が，`.png` に出すと
  全面不透明に，`.aseprite` に出すと全面透明になっていました．`--colors N` は
  不透明な色の数として扱います．
- `pxsmith sheet pack` は，画布の透明添字が不透明なパレット色を指していても，
  Aseprite と同じくその添字を透明として扱うようになりました．以前は不透明に描いていました．
- 透明を使わない絵を `.aseprite` に書くと全画素が消える不具合を直しました．
  書き出しは未使用の添字を透明添字に選び，空きが無ければ 1 色足し，
  パレットが 256 色で満杯ならエラーを返します．

## [0.1.0] - 2026-08-15

crates.io への最初の公開です: `pxsmith-core`・`pxsmith-io`・`pxsmith-lint`・
`pxsmith-recipe`・`pxsmith-macro`・`pxsmith-gen`・`pxsmith-view` と，
コマンドラインツール `pxsmith`．

[0.2.0]: https://github.com/akitenkrad/rs-pxsmith/compare/067a9b8...HEAD
[0.1.0]: https://github.com/akitenkrad/rs-pxsmith/tree/067a9b81a6efb517e6c47f24af8d0d8b836743bf
