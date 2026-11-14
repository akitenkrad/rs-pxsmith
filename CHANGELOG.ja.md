# 変更履歴

[English](CHANGELOG.md) | **日本語**

このワークスペースのクレートはすべて同じ版番号を使います．書式は
[Keep a Changelog](https://keepachangelog.com/ja/1.1.0/) に，版番号は Cargo が
`0.x` に適用する [Semantic Versioning](https://semver.org/lang/ja/) に従います．
つまり，副版 (minor) が上がると公開 API が壊れることがあります．

## [Unreleased]

### 修正

- `compose` の出力が単一レイヤの L0・PNG・GIF の場合，レイヤ順に全パーツを平坦化するように
  しました．L0 が先頭レイヤだけを黙って残すことはありません (MYTASK-3576)．Aseprite 出力は
  従来どおりレイヤを保ちます．
- `palette apply` が全フレーム・全レイヤを処理し，フレーム数・表示時間と，L0 の名前・
  `kind`・アンカーを保つようにしました (MYTASK-3577)．
- `palette apply` で全ディザ方式の透明画素を透明のまま保ち，強制先に透明が無ければ
  出力パレットの末尾へ足すようにしました (MYTASK-3578)．
- `anim smear` で重心を合わせた後も形が切れる場合，揃え方と入力の成分数に応じた
  対処を示すようにしました (MYTASK-3579)．

## [0.3.0] - 2026-11-14

この版はドット絵のフォントと UI 部品を足し，公開構造体にフィールドが増えるため
`pxsmith-core` と `pxsmith-io` の公開 API を壊します．L0 テキスト形式は版 1 のままであり，
0.2 が書いた `.px.toml` はすべてそのまま読めます．新しい `nine_slice` の宣言を使った L0 は
0.2 では読めません．シートメタは形式 1 と 2 のままで，0.2 の読み手は新しい `nine_slice` の
欄を無視します．

### 互換性のない変更

- `pxsmith_io::l0::L0Meta` に `nine_slice: Option<NineSlice>` が増えました．`Default` を
  実装しているので，構造体リテラルは `..Default::default()` で終えられます．そうでなければ
  `nine_slice: None` を足してください．
- `pxsmith_core::sheet::SheetCell` に `nine_slice: Option<NineSlice>` が増えました．
  `SheetCell` は `Default` を実装していないので，構造体リテラルには `nine_slice: None` を
  足す必要があります．
- `pxsmith_io::gif::GifReport` に，表示時間を丸めたフレームの数 `rounded_frames` が
  増えました．`Default` を実装しています．
- `CoreError` に 9-slice・Godot への書き出し・数字の回転の列挙子が，`IoError` に `Font` と
  BDF・TrueType の取り込みの列挙子が増えました．どちらも `#[non_exhaustive]` なので，
  網羅的な `match` は壊れません．
- **フレーム列を書くコマンドは，未知の拡張子を `.aseprite` として扱わなくなりました．**
  `.aseprite` と `.ase` は Aseprite，`.px.toml` は L0，`.gif` は GIF，`.png` はフレームが
  1 枚のときだけ PNG を書きます．2 枚以上の `.png` とそれ以外の拡張子は，受け付ける拡張子を
  示すエラーになります．以前は `anim tween ... out.png` が `.png` の名前で Aseprite のデータを
  黙って書いていました．

### 追加

- **ドット絵フォントの形式．** `.pxfont.toml` は 1 つのフォントを 1 つの TOML ファイルで
  持ちます．各字は升全体 (字送り × 行の高さ) を描き，既定は `#` と `.` の単色，
  `mode = "color"` と L0 と同じ `[palette]` で多色にできます．縦書き用の字形は
  `variant = "vert"`，カーニングは `[[kern]]` で持ちます．ライブラリ側は
  `pxsmith_core::font` と `pxsmith_io::pxfont::PxFontDocument` です．
- `pxsmith font import` は BDF 2.1 (`pxsmith_io::bdf`) と TTF・OTF・TTC
  (`pxsmith_io::ttf`) を `--size` の大きさで取り込みます．その大きさの 1 ビットの埋め込み
  ビットマップがあればそのまま使い，無ければ輪郭を画素の中心の巻き数でアンチエイリアス
  無しにラスタライズします (`--outline` で常にこちら)．JIS X 0208 の BDF は同梱した
  Unicode の `JIS0208.TXT` で写し，GSUB の `vert`／`vrt2` は縦書きの変種に，GPOS または
  `kern` 表の組のカーニングは `[[kern]]` にします．升からはみ出す字は `--clip` が無ければ
  エラーです．`--chars-file` で字を絞り，`--face` で TTC の face を選べます．
- `pxsmith font check` は，文言に使われている字がフォントに無いとき (`--vertical` では
  縦書きの変種が無いときも) 失敗します．右端の列に墨がある字・墨の無い字・字送りが字形より
  狭い字には助言を出します．
- `pxsmith font render` は文字列を L0・`.aseprite`・PNG に焼き込みます．横書きのほか，
  `--vertical` で縦書き (列は右から左，字の向きは UAX #50，90° の回転は画素の置き換えで正確，
  `--tcy` で縦中横) もできます．カーニングを当て，`unicode-linebreak` による UAX #14 の
  行分割で折り返し，`--kinsoku normal|strict`・`--hang`・`--align`・字間・行間・`--missing` を
  持ちます．ライブラリ側は `pxsmith_core::font_layout::layout_text` と
  `pxsmith_core::vertical` です．
- `pxsmith font bmfont` は AngelCode BMFont のテキスト形式と RGBA のページを，決定的な
  棚詰めとカーニングの組つきで書き出します．単色のフォントは色付けのため白で書きます．
  ライブラリ側は `pxsmith_core::bmfont` です．
- **9-slice．** L0 の `[meta]` に任意の `nine_slice` 表 (四辺と，軸ごとの `stretch`・
  `tile`・`tile_fit`) を書けます．`pxsmith nineslice preview` は Godot 4.7 と同じに伸ばし，
  タイルの継ぎ目が見えるときは助言します．`pxsmith sheet pack` は宣言をシートメタへ写し，
  `text import` は `.aseprite` が宣言を持てないことを警告します．ライブラリ側は
  `pxsmith_core::nineslice` です．
- `pxsmith export godot` は，シートの 9-slice の升ごとに Godot 4.7 の `StyleBoxTexture`
  リソース (`.tres`) を，シートメタだけから書き出します．ライブラリ側は
  `pxsmith_core::export::godot_styleboxes` です．
- `pxsmith anim roll` は，オドメーターのように数字の列を別の数字の列へ回します．各桁は
  自分の升の中だけで動きます．時間は `anim ease` で付けます．ライブラリ側は
  `pxsmith_core::roll` です．
- **GIF の出力．** フレームの列を書くすべてのコマンドが，出力が `.gif` ならアニメーション
  GIF を書くようになりました．等倍・無限ループ・透明を保持し，色はフレームごとに完全一致で
  写し，表示時間は 10 ms 刻みに丸めて丸めたフレームを報告します．
- `examples/godot-ui/` は，BMFont の Label・`NinePatchRect`・`StyleBoxTexture` のパネルを
  Godot 4.7.2 で表示し，撮影した画像が `font render` と `nineslice preview` に画素単位で
  一致することを示します．
- 新しい依存: `pxsmith-io` に `ttf-parser` 0.25.1 (MIT OR Apache-2.0)，`pxsmith-core` に
  `unicode-linebreak` 0.1.5 (Apache-2.0)．`pxsmith-io` は Unicode Consortium の対応表
  `JIS0208.TXT` を `data/` に同梱し，同じ場所に置いた Unicode License v3 のもとで配ります．

## [0.2.0] - 2026-11-14

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

[0.3.0]: https://github.com/akitenkrad/rs-pxsmith/compare/98dccb5...3ef788d
[0.2.0]: https://github.com/akitenkrad/rs-pxsmith/compare/067a9b8...98dccb5
[0.1.0]: https://github.com/akitenkrad/rs-pxsmith/tree/067a9b81a6efb517e6c47f24af8d0d8b836743bf
