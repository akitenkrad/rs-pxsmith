# レシピ

[English](recipes.md) | **日本語**

[← README へ戻る](../README.ja.md)

レシピは TOML ファイルです．変数と，制限された式評価器と，直積の `for_each` を
持ちます．**ループも関数定義も I/O もありません．** その制限こそが狙いで，
ステップキーが逐次的に決まるため，**変わっていないステップは走らせずに
「変わっていない」と分かります**．

```toml
[project]
format = 1

[vars]
seeds = ["hero", "slime"]

[[step]]
op = "shade"
input = "src/${s}.png"
output = "out/${s}.px.toml"
base = "8A6A4A"
light = "dir:-0.6,0.8"
for_each = { s = "${seeds}" }

[[step]]
op = "anim.squash"
input = "out/hero.px.toml"
output = "out/squashed.px.toml"
amount = -0.3
```

`op` は CLI と 1 対 1 に対応します — `op = "anim.squash"` は
`pxsmith anim squash` です．引数の名前と順序は**コマンドライン parser から
読み出して**おり，手書きの対応表は持ちません．表を 2 つ持てば必ずずれますが，
parser を読む限りずれようがありません．

## 走らせる

```sh
pxsmith run build.toml --dry-run   # 順序だけ出す．何も走らせない
pxsmith run build.toml --explain   # 各ステップのキーと argv を出す
pxsmith run build.toml --gc        # このレシピが使わなくなったキャッシュを落とす

# ある成果物が «どう出来たか» を GIF にする (系譜をビルド順に)
pxsmith run build.toml --progress how.gif --progress-of out/hero.px.toml

# 外部データからレシピを起こす (1 行につき 1 つの [[step]]．対応関係が保たれる)
pxsmith recipe expand template.toml build.toml --data rows.csv
```

生成過程の GIF は**コマごとに局所カラーテーブル**を書くので，色は入れたとおりに
出ます — 添字は `u8`，アルファは 2 値で，これはちょうど GIF のコマが持てるもの
そのものなので，量子化をやり直す必要がありません．

## キャッシュ

変わっていないレシピを再実行すると，すべて `.pxcache/` から復元されます．
64 枚に対する 128 ステップで **冷 2.66 秒に対し温 0.09 秒**，入力を 1 つ変えると
**それに依存する 2 ステップだけ**が再ビルドされます．

生成物を含むビルドが再現するのも，このキャッシュのおかげです．生成のステップは
決定論的ではありません（モデルが seed を受け付けません）．**ビルドが繰り返せる
のは，結果をキャッシュしてコミットするから**であって，モデルが 2 度同じ答えを
返すからではありません．[生成](generation.ja.md) を見てください．

## 決定論性

スレッド数を変えても出力はバイト単位で同じです — `RAYON_NUM_THREADS` を変えても
何も変わりません．これは主張ではなく**試験で確かめて**います．
