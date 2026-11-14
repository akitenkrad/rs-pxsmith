# ライブラリ

[English](library.md) | **日本語**

[← README へ戻る](https://github.com/akitenkrad/rs-pxsmith/blob/main/README.ja.md)

## クレート

| クレート | 中身 |
| --- | --- |
| [`pxsmith-core`](https://crates.io/crates/pxsmith-core) | データモデル・幾何基盤・純関数のアルゴリズム．I/O は持たない |
| [`pxsmith-io`](https://crates.io/crates/pxsmith-io) | 保持層 (`Document`)・`.aseprite`・パレット・L0 テキスト・フォント (`.pxfont.toml`・BDF・TTF/OTF/TTC) の入出力 |
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
pxsmith-view = { version = "0.3", default-features = false }
```

ライブラリ名はアンダースコア形になるため，取り込みは `use pxsmith_core::…` と記述します．

## フレームごとの合成アンカー

`pxsmith_core::compose::Part` はアンカーを 1 つの表ではなく `Anchors` に持ちます．
`Anchors::default` は全フレームに共通する位置，`Anchors::frames` はフレームごとの上書きです．
`frames` が空なら全フレームが `default` を使います．空でなければ長さは
`Part::frames.len()` と一致しなければならず，違えば `compose` は
`ComposeAnchorFrameCount` を返します．

`Anchors::get(frame, name)` はフレームの上書きを先に引き，無ければ既定値を返します．
`Anchors::names()` は両方に現れる名前を昇順で返します．合成では，各パーツがその
フレームで表示しているコマのアンカーを使います．`Placement::offset` は従来どおり
第 0 フレームの位置を表し，`Placement::offsets` は合成後のフレームごとの位置を持ちます．

広げた画布は，フレームごとの置き場所すべてから 1 度だけ求めます．そのため
アンカーが動いても出力のフレームはすべて同じ大きさになります．
`ComposeOptions::clip` を指定した場合は，従来どおり基準パーツの画布に切り揃えます．

## フォントと組版

`pxsmith_core::font` は I/O を持たないフォントのモデルです．`Font` は `FontMeta` (行の高さ・
基線・既定の字送り・`FontMode::Mono` か `FontMode::Color` とその `FontPalette`・任意の出所と
著作権表示)，字の並び，`Kern` の組を持ちます．各 `Glyph` は 1 文字・任意の `variant`
(`"vert"` など)・任意の `advance` と，升全体を覆う `IndexedCanvas` を持つので，高さは常に行の
高さです．`Glyph::effective_advance` は個別の指定が無ければ字の画布の幅を返し，
その幅が 0 のときだけフォントの既定の字送りを返します．
単色の字は添字 0 が透明，1 が墨です．`Font::validate` はこれらの不変条件を検査し，
`Font::canonicalize` は字とカーニングの組を正規形の順に並べます．

ファイル形式は `pxsmith-io` にあります．`pxfont::PxFontDocument` は `.pxfont.toml` を読み書きし
(`read`・`parse`・`write`・`to_toml`)，`to_font` と `from_font` でモデルと相互に変換します．
正規形はバイト単位で往復します．`bdf::import_bdf` は BDF 2.1 の本文を，`ttf::import_ttf` は
TrueType／OpenType のデータを受け取ります．どちらも `Font` と，取り込んだ字・飛ばした字・
切った字 (OpenType ではどの経路で字形を作ったかも) の報告を返します．設定
(`BdfImportOptions`・`TtfImportOptions`) は `font import` の引数に対応します．

`font_layout::layout_text(font, text, options)` は文字列を組版し，`FontLayoutResult` を返します．
中身は，フォント自身の添字で描いた透明な背景の `IndexedCanvas` と，行・置き換え・強制改行・
ぶら下げ・縦書きの代替の数をまとめた `FontLayoutReport` です．添字を出力のパレットへ写すのは
呼び出し側の仕事で，CLI の `--ink` はそこで当てています．`FontLayoutOptions::default()` は
横書き・折り返し無し・カーニング有り・`Kinsoku::Normal` です．`WritingDirection::Vertical` で
縦書きになり，`tcy` は縦書きのときだけ効きます．フォントに無い字があると，`missing` で代替字を
指定していない限り `FontLayoutError::MissingGlyphs` を返します．

`vertical::vertical_orientation(code_point)` は Unicode の `Vertical_Orientation` の値
(`U`・`R`・`Tu`・`Tr`) を返します．表は Unicode 18.0.0 の `VerticalOrientation.txt` から生成した
もので，組版と `font check` はこの 1 つの表を共有します．

`bmfont::build_bmfont(font, options)` は AngelCode BMFont のテキスト記述と RGBA のページ画布を
メモリ上に作り，ファイルは書きません．`BmFontOptions::new(page_size, page_file_stem)` でページの
大きさと記述の中のページ名の語幹を決め，`with_characters` で字を絞り，`with_palette` で多色
フォントに要る色を渡します (無ければ `BmFontError::MissingPalette`)．

`roll::roll(font, from, to, options)` は `anim roll` のフレームを，フォントの添字で描いた
`IndexedCanvas` の列として返します．`RollOptions` は 1 回の切り替えの区間数・
`RollDirection`・字間を持ちます．

## 9-slice と Godot への書き出し

`nineslice::NineSlice` は四辺と，軸ごとの `AxisStretch` (`Stretch`・`Tile`・`TileFit`) を
持ちます．逆直列化では共通の `stretch` と，`stretch_x`・`stretch_y` のどちらも受け付け，
直列化では両軸を明示的に書きます．シートメタはこの形を使います．`validate_source` は元の絵に
中央が 1 px 以上残ることを，`validate_target` は描画先に四辺が収まることを求めます．
`render_nine_slice(source, spec, width, height)` は `IndexedCanvas` を Godot 4.7 と同じに伸ばし，
`visible_seams` はタイルで並べる帯の両端の画素が違う軸を返します．

`export::godot_styleboxes(sheet, res_dir)` は `SheetDoc` の 9-slice の升を `GodotStyleboxFile`
(ファイル名と `.tres` の本文) にして `GodotStyleboxExport` に入れて返します．飛ばした升の数と，
パレットスワップのシートかどうかも持ちます．ディスクには何も書きません．

このために 2 つの公開構造体にフィールドが増えており，構造体リテラルで作っているコードは
壊れます．`pxsmith_io::l0::L0Meta` には `nine_slice: Option<NineSlice>` が増えました．
`Default` を実装しているので，`..Default::default()` で新しいフィールドを埋められます．
`pxsmith_core::sheet::SheetCell` にも `nine_slice: Option<NineSlice>` が増えましたが，こちらは
`Default` を実装していないので，リテラルに `nine_slice: None` を足す必要があります．どちらも
`None` のときは直列化した形に現れないため，既存の L0 ファイルとシートメタは変わりません．

## コンパイル時にスプライトを埋め込む

```rust
use pxsmith_macro::pixels;

let frames = pixels!("sprites/hero_body.px.toml");
```

行の長さが揃っていない場合はコンパイルエラーになります．また参照しているパレットを編集
すると再ビルドが走ります．マクロが `.hex` ファイルを追跡しているため，パレットを変更した
にもかかわらず建て直しを忘れるという事態が起こりません．

## export クレートを設けなかった理由

書き出し先 (Tiled・Godot の StyleBox・スプライトシート・正規 JSON) は独自のアルゴリズムを持たない出力
アダプタです．そのため直列化する対象のデータと同じ `pxsmith-core` に置き，CLI 側で
繋いでいます．ここにクレートの境界を引くと，直列化器とその対象の型を何の利点もなく
引き離すことになるためです．
