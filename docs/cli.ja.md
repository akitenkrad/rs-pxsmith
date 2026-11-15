# コマンドライン

[English](cli.md) | **日本語**

[← README へ戻る](https://github.com/akitenkrad/rs-pxsmith/blob/main/README.ja.md)

以下の例はインストール済みの `pxsmith` を実行する形で記載しています．リポジトリの
チェックアウトから実行する場合は，先頭に `cargo run -p pxsmith --` を付けてください．

`.px.toml` は L0 と呼ぶテキスト形式です．スプライトを文字の並びとして記述し，パレットは
別の `.hex` ファイルが保持します．`.aseprite` はバイト単位で往復するため，pxsmith は
既存の Aseprite による作業の途中に配置することができ，ファイルの所有権を奪うことは
ありません．

`--light` は反転で作った方向に掛けるルール 7 の光源を宣言します．`--reshade` のときは
陰影の塗り直しにも同じ光源を使い，`pxsmith shade --light` と同じ計算で塗ります．
`--light` が無ければ，塗り直しは `--preset` の既定の光源で行います．

フレームの列を書き出すコマンド (`anim` の各コマンド・`compose`・`direction`・`scale`・
`rotate` など) は，出力の名前で形式を選びます．`.px.toml` なら L0 を書いて隣にパレットを
置き，`.gif` ならアニメーション GIF を，`.aseprite` なら Aseprite のファイルを書きます．
GIF は等倍で，無限ループで書きます．透明な画素は GIF の透明色になり，各フレームの色は
そのフレームの局所カラーテーブルへ完全一致で写すため，減色し直すことはありません．
透明を含めて 256 色を超えるフレームはエラーです．GIF の表示時間は 10 ms 刻みなので丸め，
丸めたフレームの数と丸めた値を報告します．GIF は書き出し専用で，読み込むコマンドは
ありません．時間は GIF を書く前に `anim ease` で付けてください．

L0・PNG・GIF は各フレームに可視レイヤを 1 枚しか持てません．`compose` などが複数レイヤを
作った場合，これらの形式では下から上へ平坦化し，その旨を 1 行で報告します．`.aseprite` は
レイヤを保ちます．L0 の平坦化は既存の添字色だけを正確に写し，混色を新しく作りません．
半透明レイヤや通常以外のブレンドモードは，パレットに無い色を黙って足す代わりに明示的な
エラーにします．

L0 の `[meta]` に書いたアンカーは，全フレームに共通する既定値です．`[[frame]]` の中に
`anchors` を書くと，そのフレームだけ名前ごとに上書きでき，残りの名前は既定値のまま
使われます．

```toml
[meta]
format = 1
name = "body"
anchors = { head = [8, 3], feet = [8, 15] }

[[frame]]
name = "step"
anchors = { head = [8, 2] }
data = '''
0
'''
```

この例では，`step` の `head` は `[8, 2]` になり，`feet` は既定値の `[8, 15]` のままです．
`anchors` を持たないフレームの正規形はバイト単位で従来と変わらず，L0 の形式の版も 1 の
ままです．

## 基本

```sh
# 端末が画素を正確に出せるか確かめる (Kitty / iTerm2 / Sixel)
pxsmith verify terminal

# スプライトのレイヤを編集できるテキストにして，戻す
pxsmith text export sprite.aseprite hero.px.toml --palette pal.hex
pxsmith text import hero.px.toml sprite.aseprite

# 保存を見張って描き直す
pxsmith watch hero.px.toml --zoom 8

# 2 枚の間でどの画素が変わったかを出す
pxsmith diff before.px.toml after.px.toml

# パレットを見る・変換する
pxsmith palette info palettes/sweetie-16.hex
pxsmith palette convert input.gpl output.hex
pxsmith palette convert output.hex output.gpl

# `.aseprite` の読み書きがバイト一致するか確かめる
pxsmith verify roundtrip sprite.aseprite --via-frame
```

パレットの入力は `.hex`・`.gpl`・`.pal`・`.act` です．変換の出力形式は拡張子で選び，
`.hex` は 1 行 1 色の正規形，`.gpl` は GIMP Palette を書きます．ほかの出力拡張子は
エラーです．

## 絵を導出する

陰影は塗るのではなく，シルエットから導出します．入力の色は捨てるため，反転・中割り・
色替えを行っても光の向きが壊れることはありません．

幾何光源を受け取るすべての `--light` で，`dir:x,y` は光が進む向きです．x は右，
y は下を正とします．`dir:-0.6,0.8` は右上から，`dir:0.6,0.8` は左上からの光で，
既定は `dir:-0.6,0.8` です．`point:x,y` の x・y は向きではなく光源の位置です．
例外の `atmos --light` は方向ではなく，光源色のプリセットを受け取ります．

```sh
# シルエットから陰影を導出する (入力の色は捨てる)
pxsmith shade hero.png hero.px.toml --base 8A6A4A --light dir:-0.6,0.8

# ジャギーを正規化し，アンチエイリアスを付け，縁取りを描く
pxsmith smooth hero.px.toml smoothed.px.toml
pxsmith aa smoothed.px.toml aa.px.toml
pxsmith outline aa.px.toml outlined.px.toml --style tinted

# アニメ: 中割り・タイミング・周期・スメア・スクワッシュ・サブピクセル・残像
pxsmith anim tween out.px.toml --from a.px.toml --to b.px.toml --base 8A6A4A
pxsmith anim ease walk.px.toml eased.px.toml --fps 30 --hold 2,1,1,1,2
pxsmith anim smear out.px.toml --from a.px.toml --to b.px.toml --base 8A6A4A
pxsmith anim squash in.px.toml out.px.toml --amount -0.3
pxsmith anim subpixel in.px.toml out.px.toml --method tangent

# スコアの表示のように数字を回す (下の «数字を回す» を参照)
pxsmith anim roll out/misaki.pxfont.toml --from 0129 --to 0131 --frames 4 \
    out/score-roll.px.toml --palette palettes/sweetie-16.hex --ink 12

# 出力先の制約に照らす (違反があれば非零で終了する)
pxsmith lint out.px.toml
pxsmith validate out.px.toml --target gb
```

### 色数を減らす

```sh
pxsmith quantize photo.png indexed.png --colors 16 --method kmeans
pxsmith clean indexed.png cleaned.png
pxsmith conform upscaled.png native.png
```

`conform` は，拡大されさらに JPEG 圧縮を経ているかもしれない画像から元の格子を復元し，
等倍へ戻します．格子が一様でない場合は推測を行わず拒否します．非一様な格子は決定論的に
復元することができないため，そこは人の判断に委ねるという方針を採りました．

## 合成・タイルセット・投影

```sh
# 胴体の各フレームで解決したアンカーの位置を取り出し，別の L0 へ流し込む
pxsmith anchor track body.px.toml --anchor head -o head-track.json
pxsmith anchor apply head-track.json target.px.toml -o tracked.px.toml --anchor head

# パーツを合成し，n・ne・e・se・s を描いて残り 3 方向を反転で作る
pxsmith compose out.px.toml --part body.px.toml --part head.px.toml
pxsmith direction 'out/${dir}.px.toml' \
    --from n=hero_n.px.toml --from ne=hero_ne.px.toml --from e=hero_e.px.toml \
    --from se=hero_se.px.toml --from s=hero_s.px.toml \
    --light dir:-0.6,0.8 --reshade --base 8A6A4A

# シートをタイルに切り，重複を畳み，47 枚のオートタイルを組む
# (入力はインデックスカラーのみ — ここで量子化するとタイルの同一性に
#  こちらの都合が紛れ込む)
pxsmith tileset extract sheet.aseprite tiles.aseprite --tile 16 --map map.json
pxsmith tileset autotile quadrants.px.toml auto.aseprite

# 奥行きを作る: 遠景を空色へ寄せ，視差の速さを記録する
pxsmith atmos 'out/${name}.px.toml' --input fg.px.toml --input bg.px.toml \
    --sky 41a6f6 --haze background=0.6 --scroll-meta out/scene.scroll.json

# 真上から見た絵を等角の床へ投影し，合うガイドを引く
pxsmith project in.px.toml iso.px.toml --to iso --from top --facing right
pxsmith guide g.png --projection iso --from top --cell 16 --size 256x256
```

`direction` が作れるのは鏡像の相手だけです．n・ne・e・se・s を描くと，sw・w・nw を
作ります．反転で作れない方向は報告して飛ばし，1 方向から残り 7 方向を作ることは
ありません．

`project` は `--from` と `--facing` を必須の引数としています．どの面を倒すのか，また
どちらを向いているのかは画素からは読み取れないため，推測に任せると外れた場合にのみ
静かに壊れることになるからです．

`anchor track` は，各フレームで解決したアンカー (フレームの上書きを先に，無ければ
`[meta]` の既定値) を形式の版 1 の JSON として書き出します．`--anchor` を省けば全ての
名前を昇順で，`-o` を省けば標準出力へ書きます．`anchor apply` はトラックと対象の
フレーム数が一致することを要求し，対象の各フレームへ値を書き込みます．`[meta]` は
変更しません．フレーム数が違うときに切り詰めたり繰り返したりしないのは，どちらに
合わせるべきかが絵からは決まらないためです．アンカーを持てるのは L0 (`.px.toml`)
だけなので，`anchor` 系のコマンドは L0 以外の入力を受け付けません．

合成では，ずれを出力のフレームごとに求めます．各パーツのアンカーは，そのパーツが
そのフレームで表示しているコマから取ります．`--part-delay` で遅らせたパーツは形だけが
遅れ，位置は基準のパーツがいま表示しているコマに付いていきます．画布と原点は，
全パーツ・全フレームの矩形の和として 1 度だけ決めます．アンカーが動いても出力の
フレームの大きさが揃うので，`sheet pack` の升がずれません．ずれがフレームごとに
変わる場合，報告には第 0 フレームのずれと x・y の範囲を表示します．ずれが一定なら
従来どおり 1 つのずれだけを表示します．

`compose` の出力が `.px.toml`・`.png`・`.gif` の場合，パーツの宣言順にレイヤを平坦化します．
後のパーツが前のパーツを覆い，透明画素では下のパーツが見えます．結果は，レイヤを持つ
`.aseprite` 出力を等倍で表示した画素と一致します．`.aseprite` 出力は従来どおりパーツごとの
レイヤを保ちます．

## 拡縮と回転

```sh
pxsmith scale in.px.toml out.px.toml --factor 4          # 既定は nearest (厳密)
pxsmith rotate in.px.toml out.px.toml --degrees 30 --algo cleanedge
```

整数倍の拡大と 90 度の倍数の回転は，標本の丸めに任せず添字の置き換えとして実装して
います．そのため 4 回まわすと元の絵に完全に戻ります．`cleanedge` が効果を発揮するのは
拡大を伴う回転であり，等倍では既定の `nearest` の方が良い結果になります．この点は
CLI が実行時に説明します．

## フォントと文字

ドット絵のフォントは L0 ではなく，専用のテキスト形式 `.pxfont.toml` で持ちます．既存の
BDF・TTF・OTF・TTC のフォントを 1 度取り込んだら，あとはゲームが実際に使う文言に照らして
検査し，文字列を絵に焼き込み，ゲームエンジン向けに AngelCode BMFont として書き出します．

```sh
# BDF を取り込む．文言ファイルに現れる字だけを取り込むこともできる
pxsmith font import testdata/fonts/misaki/misaki_gothic.bdf out/misaki.pxfont.toml
pxsmith font import testdata/fonts/misaki/misaki_gothic.bdf out/ui.pxfont.toml \
    --chars-file script.txt

# TTF/OTF/TTC を画素数を指定して取り込む
pxsmith font import testdata/fonts/misaki/misaki_gothic.ttf out/misaki-ttf.pxfont.toml --size 8
pxsmith font import testdata/fonts/synthetic/pxsynth-gpos.ttf out/gpos.pxfont.toml --size 8

# 文言に必要な字がフォントにそろっているか確かめる (不足があれば終了コード 1)
pxsmith font check out/ui.pxfont.toml --text-file script.txt
pxsmith font check out/misaki.pxfont.toml --text-file script.txt menu.txt
pxsmith font check out/misaki.pxfont.toml --text-file menu.txt --vertical

# 文字列を焼き込み，フォントをエンジン向けに書き出す
pxsmith font render out/misaki.pxfont.toml "SCORE 0120" out/score.px.toml \
    --palette palettes/sweetie-16.hex --ink 12
pxsmith font bmfont out/misaki.pxfont.toml out/misaki.fnt
```

### `.pxfont.toml` の形式

1 つのフォントは 1 つのファイルです．各字は «字送りの幅 × フォントの行の高さ» の升ごと
描くので，基線の位置は升で決まり，オフセットを手で計算する必要がありません．墨の外接矩形への
切り詰めは書き出す側が行います．

```toml
[meta]
format = 1
name = "tiny"
line_height = 3
baseline = 2
default_advance = 3

[[glyph]]
char = "U+0020"
data = '''
..
..
..
'''

[[glyph]]
char = "A"
data = '''
.#.
###
#.#
'''

[[glyph]]
char = "ー"
variant = "vert"
data = '''
.#.
.#.
.#.
'''

[[kern]]
pair = "AV"
amount = -1
```

- `[meta]` は `format` (常に 1)・`name`・`line_height` (升の高さ．px)・`baseline` (升の上端から
  基線までの px．`line_height` 以下)・`default_advance` を持ちます．`source` と `copyright` は
  任意で，フォントの出所を記録します．取り込みのコマンドが埋めます．
- 字は既定で単色です．`#` が墨，`.` が透明で，墨の色は焼き込むときに決めます．多色のフォントは
  `[meta]` に `mode = "color"` と書き，L0 と同じ `[palette]` の表 (`.hex` を指す `ref` と，
  1 文字の記号の `map`．`.` は透明) を持ちます．1 つのフォントの中で 2 つの方式は混ぜません．
- `char` は 1 文字か `U+XXXX` です．正規形では空白類と制御文字だけを `U+XXXX` で書き，
  それ以外は字そのものを書きます．
- `data` はちょうど `line_height` 行で，各行の幅はそろっていなければなりません．`advance` は
  任意で，省いた字は `data` の幅を字送りにします．幅の狭い字は狭く描けばそれで足ります．
  `default_advance` は `data` が空の字にだけ使います．正規形では，`advance` がその値と違うときだけ
  書きます．
- `variant = "vert"` は縦書きで使う字形です．同じ字と同じ変種の組は 1 度しか書けません．
- `[[kern]]` は，ちょうど 2 文字の組の横方向の字送りを `amount` px だけ調整します．同じ組は
  1 度しか書けません．
- 正規形では，字を符号位置の順に (基の字形を変種より先に)，カーニングの組を符号位置の順に
  並べます．`mode = "mono"` は読めますが書き出しません．

### フォントを取り込む

`font import` は入力の拡張子 (`.bdf`・`.ttf`・`.otf`・`.ttc`) で読み手を選び，取り込んだ字・
飛ばした字・切った字の数を報告します．フォントの著作権表示は `copyright` へ写し，画面にも
表示します．

- **BDF 2.1．** `line_height` と `baseline` は `FONT_ASCENT` と `FONT_DESCENT` から決め，各字は
  `BBX` に従って升の中へ置きます．`ISO10646` で登録されたフォントはそのまま読み，`JISX0208`
  のフォントは `pxsmith-io` に同梱した Unicode Consortium の対応表 `JIS0208.TXT` で Unicode へ
  写します．符号位置の無い字 (`ENCODING -1`) と，対応表で写せない字は飛ばして数えます．
- **TTF・OTF・TTC．** `--size` (1 em の画素数) が必須で，`--face N` でコレクションの中の face を
  選びます (既定 0)．ちょうどその大きさの 1 ビットの埋め込みビットマップがあればそのまま使います．
  無ければ輪郭をアンチエイリアス無しでラスタライズします．画素の中心が非零巻き数規則で
  輪郭の内側にあれば墨とするため，格子に沿って描かれたドット絵フォントは 1 ドットも違わずに
  戻ります．`--outline` を付けると，埋め込みがあっても輪郭から作ります．輪郭の点が画素の格子に
  乗らない字の割合を報告し，5% を超えると助言を出します．ドット絵用でない輪郭は，この
  ラスタライズでつぶれるためです．
- GSUB の `vert`／`vrt2` の単一置換は `variant = "vert"` の字になります (両方が当たるときは
  `vrt2` を採ります)．GPOS の組のカーニングは画素に丸めて `[[kern]]` にします．`kern` 表は，
  GPOS にカーニングが無いときだけ使います．
- 升の外に墨がはみ出す字があると，取り込み全体を失敗させてその字を列挙します．`--clip` を
  付けたときだけ升で切り，切った字を数えます．
- `--chars-file FILE` を付けると，そのファイルに現れる字だけを取り込みます (改行は無視します)．

美咲ゴシックの TTF は 8 ppem のビットマップと輪郭の両方を持っており，どちらの経路でも
BDF からの取り込みと 7,171 字すべてが一致します．

### 文言に照らしてフォントを検査する

`font check` は 1 つ以上の文言ファイルを読み，そこで使われている字がフォントに無ければ，
その字を列挙して終了コード 1 で終わります．`--vertical` を付けると，縦書きの変種が要る字
(`Vertical_Orientation` が Tu か Tr) に `variant = "vert"` の字形が無い場合も失敗にします．

形についての助言も出しますが，終了コードは変えません．対象は，右端の列に墨がある字
(次の字の左端の列にも墨があるとくっつく．つながるように作られた罫線・ブロック要素と，
縦書きの変種は除く)，墨が 1 画素も無い字 (空白類は除く)，字送りが `data` の幅より小さい字です．

### 文字列を焼き込む

`font render` は文字列を組版し，透明な背景の上に `.px.toml`・`.aseprite`・`.png` で書きます．
文字列は出力の前の位置引数として渡すか，`--text-file` で UTF-8 のファイルとして渡します．
後者のときの位置引数は出力だけです．

```sh
# 台詞を折り返し，中央に寄せ，句読点をぶら下げる
pxsmith font render out/misaki.pxfont.toml --text-file script.txt out/script.png \
    --palette palettes/sweetie-16.hex --ink 12 --wrap 64 --hang --line-spacing 2 --align center

# 高さ 64 px の列で縦書きにし，2 桁の数字を縦中横にする
pxsmith font render out/misaki.pxfont.toml --text-file script.txt out/script-v.png \
    --palette palettes/sweetie-16.hex --ink 12 --vertical --wrap 64 --tcy 2

# カーニング: GPOS の組 AV が 1 px 詰まる (--no-kern と見比べられる)
pxsmith font render out/gpos.pxfont.toml AV out/av.png --palette palettes/sweetie-16.hex --ink 12
```

- 単色のフォントには `--palette` と，墨に使う添字 `--ink` が必要です．多色のフォントは字の色を
  そのまま使い，`--ink` を受け付けません．`--palette` を省くとフォント自身のパレットを使い，
  与えた場合は，実際に描く字の色がすべてそのパレットに含まれていることを求めます (無い色は
  列挙します)．パレットに透明の色が無ければ 1 つ足します．
- `--letter-spacing` と `--line-spacing` は字の間と行の間に px を足します．
  `--align left|center|right` は，いちばん長い行の幅の中で各行を寄せます．
- 横書きでは `[[kern]]` のカーニングを当てます．`--no-kern` で切れます．
- フォントに無い字はエラーで，そのような字をすべて列挙します．`--missing C` (1 文字か
  `U+XXXX`) を付けると C で置き換え，置き換えた数を報告します．
- 文字列の中の改行はそのまま使います．`--wrap N` を付けると N px で折り返します．折り返して
  よい位置はクレート `unicode-linebreak` を通して UAX #14 (Unicode の行分割アルゴリズム) で
  決めるので，日本語の行頭禁則・行末禁則や，単語・数字の途中で切らないことは，手で書いた表では
  なく Unicode の分類に従います．行より長い 1 語は字の間で切り，その数を報告します．
- `--kinsoku normal` (既定) では，小書きの仮名と長音符 (UAX #14 の CJ 類) が行頭に来ることを
  許します．`--kinsoku strict` では許しません (JLReq の厳格な処理)．
- `--hang` を付けると，行末の句読点 (`、。，．` と半角形) 1 字を行幅の外へぶら下げます．
  付けなければ直前の字ごと次の行へ送ります．

`--vertical` を付けると，字を上から下へ，列を右から左へ並べる縦書きになり，`--wrap` は列の
高さになります．折り返し・`--kinsoku`・`--hang` は横書きと同じに働き，句読点は列の下端の外へ
ぶら下がります．`--align left|center|right` は各列を上・中央・下に寄せます．各字の向きは
Unicode の `Vertical_Orientation` (UAX #50) で決めます．

| 値 | `vert` の字形があるとき | 無いとき |
| --- | --- | --- |
| U | `vert` の字形 | 正立 |
| R | `vert` の字形 | 時計回りに 90° 回す |
| Tu | `vert` の字形 | 正立にして数える |
| Tr | `vert` の字形 | 時計回りに 90° 回して数える |

90° の回転は画素の置き換えなので正確です．正立の字は幅が字送り・高さが行の高さで，回した字は
その逆になります．`--tcy N` (縦書きのみ) は N 桁以下の半角数字の並びを 1 字分へ横に並べ，
列の幅に収まらなければ数を報告して正立の縦並びに戻します．カーニングは横書きのための表なので，
縦書きでは当てません．

### BMFont を書き出す

`font bmfont` は AngelCode BMFont のテキスト形式と，RGBA のページ画像を書きます．

```sh
pxsmith font bmfont out/misaki.pxfont.toml out/misaki.fnt
pxsmith font bmfont out/misaki.pxfont.toml out/ui.fnt --chars-file script.txt --page-size 128
```

- 出力は `.fnt` でなければなりません．ページはその隣に `<語幹>_0.png`・`<語幹>_1.png`… の
  名前で書きます．
- 各字は墨の外接矩形に切り詰め，高さの降順に棚詰めします．字と字の間は 1 px 空け，ページは
  一辺 `--page-size` px の正方形 (2 の冪．既定 256) です．同じ入力からは常に同じ出力が出ます．
  `xoffset` と `yoffset` は升の左上から測るので，切り詰めても升の位置関係は保たれます．空白類は
  字送りだけを持つ 0×0 の字になります．
- 単色のフォントは透明の上に白で書きます．色はエンジン側で付けます (Godot なら `modulate`
  など)．多色のフォントは字の色のまま書きます．
- `[[kern]]` の組は `kerning` 行になります．縦書きの変種は書きません (この形式に縦書きの
  情報が無いため)．書かなかった数は報告します．
- `--chars-file` を付けると，そのファイルに現れる字だけを書きます．フォントに無い字があれば
  エラーです．カーニングの組は両方の字を書くときだけ残します．
- 最後にフォントの `copyright` を表示します．道具はフォントのライセンス上の義務を肩代わり
  しませんが，持ち主の表示を途中で落とすこともしません．

[`examples/godot-ui/`](../examples/godot-ui/) では，こうして書いた 2 つのフォントを Godot 4.7 の
`Label` で表示しています．そこで撮影した画像は，カーニングも含めて `font render` と画素単位で
一致します．

### 数字を回す

`anim roll` は，スコアの表示のように，数字の列を別の数字の列へ回して変えます．

```sh
pxsmith anim roll out/misaki.pxfont.toml --from 0129 --to 0131 --frames 4 \
    out/score-roll.px.toml --palette palettes/sweetie-16.hex --ink 12
pxsmith anim ease out/score-roll.px.toml out/score-roll.gif --fps 60 --hold 1,1,2,3,4,1,1,2,3
```

- 変わる桁は自分の升 (その桁に現れる字の最大の字送りの幅) の中だけで縦にずれるので，隣の桁へ
  はみ出しません．既定の `--direction up` では古い字が上へ抜け，数字は 9 の次が 0 になる向きに
  増えます．`--direction down` ではどちらも逆になります．
- 1 回の字の切り替えに `--frames` 区間を使います．何度も変わる桁は途中の数字を順に通り，
  変わる回数の少ない桁は遅れて動き始めるので，すべての桁が同時に止まります．`0129` から
  `0131` へ `--frames 4` で回すと 9 フレームになり，1 の位は 9・0・1 と進み，10 の位は後半だけ
  動きます．
- `--from` と `--to` の字数は同じでなければなりません (ゼロ詰めは呼び出し側で行います)．
  数字でない字があってもよいですが，変わってはいけません．
- 最初と最後のフレームはキーフレームで，2 つの文字列を `font render` で焼いたものと一致します．
  間は中割りです．フレームは時間を持たないので，ほかの `anim` のコマンドと同じく `anim ease`
  で付けます．
- `--palette`・`--ink`・`--letter-spacing` は `font render` と同じに働きます．

## UI 部品

### 9-slice の宣言

L0 の `[meta]` に 9-slice の四辺を宣言できます．

```toml
[meta]
format = 1
name = "window"
nine_slice = { left = 4, top = 4, right = 4, bottom = 4, stretch = "tile" }
```

`stretch` は `"stretch"` (既定)・`"tile"`・`"tile_fit"` のいずれかで，Godot の
`AxisStretchMode` と同じ 3 種です．縦横で分けるときは代わりに `stretch_x` と `stretch_y` を
書きます．四辺を除いた中央が縦横とも 1 px 以上残らなければなりません．`nine_slice` を持たない
ファイルの正規形はバイト単位で変わらず，L0 の形式の版も 1 のままです．`.aseprite` には宣言の
置き場所が無いため，元の L0 が宣言を持つときは `text import` が «失われる» と警告します．

```sh
pxsmith nineslice preview examples/godot-ui/window.px.toml out/window.png --size 64x32 --size 120x48
```

`nineslice preview` は先頭のフレームを各 `--size WxH` へ伸ばして PNG を書きます．大きさが
1 つなら出力の名前をそのまま使い，複数なら語幹に大きさを付けます (`window-64x32.png`・
`window-120x48.png`)．伸ばし方は Godot 4.7 の nine-patch の写像を `f32` のまま移したもので，
画素の中心で評価します．Godot 4.7.2 の `NinePatchRect` を撮影した画像と，3 つの方式すべてで
1 画素も違いません．タイルで並べる帯の両端の画素が違うときは，継ぎ目が見えると助言します．
描画先が四辺より小さい場合と，`nine_slice` を持たない L0 を渡した場合はエラーです．

### Godot の StyleBoxTexture を書き出す

`sheet pack` は各入力の `nine_slice` をシートメタの升へ写すので，書き出す側は元の絵を読まずに
済みます．`export godot` は 9-slice の升ごとに `StyleBoxTexture` のリソースを 1 つ書きます．

```sh
pxsmith sheet pack out/ui.png --input examples/godot-ui/window.px.toml \
    --input examples/sprites/hero_body.px.toml --meta out/ui.sheet.json
pxsmith export godot --sheet out/ui.sheet.json --out-dir out/godot --res-dir res://ui
```

- `nine_slice` を持つ升ごとに，`--out-dir` へ `<升の名前>.stylebox.tres` を Godot 4.7 の
  テキストリソース形式で書きます．中身は，`--res-dir` (`res://` で始めること) の下のシート画像を
  指す `ExtResource`・テクスチャの余白・2 軸の伸縮方式・`region_rect` です．`region_rect` は
  升の左上と，元の絵自身の幅と高さから作ります．升の矩形をそのまま使うと，一様格子の余白が
  9-slice の内側に入ってしまうためです．
- `uid` は書きません．Godot が取り込み時に振ります．`.import` ファイルも書かないので，
  最近傍のフィルタは Godot のプロジェクト側で設定してください．
- `nine_slice` を持たない升と Theme のリソースは書かず，書かなかった数を報告します．
  パレットスワップ (形式 2) のシートも書き出しますが，テクスチャが添字を持つためシェーダが
  要る旨を警告します．

[`examples/godot-ui/`](../examples/godot-ui/) では，書き出したリソースでスタイルを付けた
パネルを Godot 4.7.2 で表示し，3 つの伸縮方式すべてで `nineslice preview` の出力と 1 画素も
違わないことを確かめています．

## 書き出す

```sh
pxsmith sheet pack out/sheet.png --input a.px.toml --input b.px.toml --meta out/sheet.sheet.json
pxsmith export tiled map.json --tsx out/sheet.tsx --sheet out/sheet.sheet.json
```

`export godot` は 9-slice の升から Godot の `StyleBoxTexture` を書きます．[UI 部品](#ui-部品)
を参照してください．Godot の terrain set は書き出しません．

`sheet pack` は通常，全入力が使っている色を 1 つのパレットへ併合します．併合したパレットが
256 色を超える場合や，色をゲーム側のシェーダで選ばせたい場合は，パレットスワップ用の出力を
使います．

```sh
pxsmith sheet pack out/sheet.png \
  --input hero.px.toml --input enemy.px.toml \
  --palette-swap \
  --variant palettes/hero.hex=palettes/hero-red.hex \
  --variant palettes/hero.hex=palettes/hero-blue.hex
```

このとき RGBA8 の PNG を 2 枚と，形式の版 2 のシートメタを書き出します．

- `sheet.png` は添字テクスチャです．R チャンネルが元の絵の添字そのもので，G と B は 0，
  A は 255 です．元の絵で透明な画素 (パレットの色の A が 0 の添字)，升の隙間・外周・空き升は
  `(0, 0, 0, 0)` になります．色を併合しないので，添字は付け替えられません．
- `sheet.palettes.png` はパレットアトラスです．入力に現れた順に，相異なるパレットを 1 行ずつ
  並べます．項目が完全に一致するパレットどうしは行を共有します．色違いの行はその後ろに
  コマンドラインの順で並びます．幅は最も長いパレットの長さで，短い行の余りは透明です．
- `sheet.sheet.json` は `format: 2` です．各升は自分の基になるパレット行を持ち，
  `palettes.rows` には行の名前と，色違いの行が基にしている行を記録します．従来の併合した
  パレットの出力は形式の版 1 のままです．版を上げたのは，0.1 の読み手が添字テクスチャを
  色の絵として黙って読まないようにするためです．

`--variant BASE.hex=VARIANT.hex` は，BASE と完全に一致する行が入力の中にあることを要求します．
VARIANT は BASE と同じ長さで，透明の位置も同じでなければなりません．透明の位置が変わると
絵の形が変わってしまうためです．行の名前は VARIANT のファイル名の語幹から取り，重複は
許しません．パレットスワップ用の出力は PNG だけで，`.aseprite` には書き出せません．

シェーダが行うのと同じ引き当てを CPU で行い，通常の RGBA の絵を作るには `sheet resolve` を
使います．

```sh
pxsmith sheet resolve out/sheet.png --meta out/sheet.sheet.json -o out/resolved.png
pxsmith sheet resolve out/sheet.png --meta out/sheet.sheet.json \
  --variant hero-red -o out/resolved-red.png
```

`--variant` を省くと，各升は自分の基の行を使います．色違いの名前を渡すと，その色違いが
基にしている行を使う升だけが色を変えます．`--palette-swap` 無しで梱包した絵と，
`--palette-swap` で梱包して `resolve` した絵は全画素で一致します (実素材 64 枚で確かめて
います)．`export tiled` は版 2 のメタでも升の位置と大きさをそのまま使いますが，Tiled には
引き当て後の色ではなく添字テクスチャがそのまま表示される旨を警告します．

Godot 4.7 で色違いを表示する最小例 (canvas_item シェーダ・テクスチャの取り込み設定・
3 つの行の切り替え・コマンドラインからの撮影) は
[`examples/godot-palette-swap/`](../examples/godot-palette-swap/) にあります．

## パレットを強制する

```sh
pxsmith palette apply walk.px.toml walk-dark.px.toml --palette palettes/dark.hex
pxsmith palette apply walk.aseprite walk-dark.aseprite \
  --palette palettes/dark.hex --dither ordered
```

`palette apply` は全フレーム・全レイヤを処理します．L0 と Aseprite ではフレーム数・表示時間・
レイヤのメタデータを保ち，L0 では文書とフレームの名前・`kind`・既定およびフレームごとの
アンカー・その他の L0 メタデータも保ちます．完全に透明な画素は `none`・`ordered`・`error` の
どのディザ方式でも透明のままで，誤差拡散の対象にもなりません．強制先のパレットに透明色が
無ければ出力パレットの末尾に足し，不透明な画素だけを強制先の色へ写します．

## 調べる

```sh
pxsmith view walk.px.toml --frame 2 --onion 2   # オニオンスキン．輪郭のみ
pxsmith palette report hero.px.toml --top 12    # どの色が面積を担っているか
pxsmith palette cvd palettes/sweetie-16.hex     # P 型 ・D 型の色覚で見分けられるか
```

`palette report` は単一の割合ではなく 4 通りの閾値を並べ，さらに「その添字の合計面積」と
「1 つながりの塊としての最大面積」を分けて報告します．広い範囲に撒かれた色は主要な色とは
言えませんが，合計面積だけを見ると主要な色として読めてしまうためです．

### 色覚の多様性に対する検査

`palette cvd` は `.hex` のパレット，または `.px.toml`・`.aseprite`・`.png` の絵を受け取ります．
絵の場合は実際に使われている色だけを検査し，完全に透明な色は対象から外します．既定で
検査するのは P 型 (1 型) と D 型 (2 型) で，カンマ区切りで別の型を指定できます．

```sh
pxsmith palette cvd hero.px.toml --type protan,deutan,tritan
pxsmith palette cvd hero.px.toml --pair ffcd75,a7f070 --json
```

シミュレーションには Machado, Oliveira, Fernandes の «A Physiologically-based Model for
Simulation of Color Vision Deficiency» (IEEE TVCG 15(6), 2009) の severity 1.0 の行列を使います．
行列は線形 RGB に掛けます．つまり sRGB を復号してから行列を掛け，結果を再び符号化します．
組の距離には，pxsmith の他の色の道具と同じ OKLab のユークリッド距離を使います．

既定の `--threshold 0.02` は，W3C の CSS Color Module Level 4 §14.2.1 が OkLCh の 1 JND
(丁度可知差異) と定める値です．シミュレーションをしなくても閾値を下回る組は別に数えます．
それらが見分けにくいのは色覚のせいではないからです．

`--pair` を指定しなければ指摘は助言にとどまり，終了コードは 0 です．ゲーム上で意味を持つ
色の組を `--pair RRGGBB,RRGGBB` で (繰り返して) 指定すると，その組が見分けられなくなったときに
blocking として報告し，終了コード 1 で終わります．指定した色が検査対象のパレットに無い場合は
入力の誤りとして終了コード 2 を返します．引数の書式の誤りも 2，それ以外の入出力や処理の
誤りは 1 です．

## チェックアウトからのビルド

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

`cargo-make` のタスクは `Makefile.toml` に定義しています．

```sh
cargo make format-all   # taplo + clippy + rustfmt
cargo make test
```
