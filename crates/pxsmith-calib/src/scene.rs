//! Tiled (`.tmx`) の地図を**元絵の解像度で**描き出す．
//!
//! 単体のタイルを拡大した正例は揃ったが，実運用で `pxsmith conform` に渡される絵は
//! 「タイルを並べて組んだ画面」である．CC0 素材には作者が組んだ見本地図が付いてくる
//! ので，それを**元絵の解像度で**描き出せば，中身も配置も本物の正例が作れる．
//!
//! > [!warning] 配布されている `Sample.png` は使えない
//! > 同じ画面の紹介用レンダ (918x515) が同梱されているが，**非整数倍で拡大され補間も
//! > 掛かっている**．実測した 18 件のうち 17 件は元絵を復元できなかった (色数が
//! > 8,186 ・41,846 と滑らかにされている件もある) ．紹介用の絵ではなく，こちらで
//! > 描き出すこと．
//!
//! # 対応している範囲
//!
//! 見本地図に必要なぶんだけである．
//!
//! | 項目 | 対応 |
//! | --- | --- |
//! | 向き | `orthogonal` のみ |
//! | レイヤ | `csv` と `base64` (無圧縮 ・zlib ・gzip)．上から順に重ねる |
//! | タイルセット | 外部 (`.tsx`) ・埋め込みの両方．複数可 (`firstgid` で選ぶ) |
//! | 反転 | 水平 ・垂直 ・対角の 3 ビットを解釈する |
//! | 未対応 | オブジェクト層 ・無限マップ ・`zstd` |
//!
//! XML は手で読む．見本地図はどれも Tiled が書いた素直な形で，属性の取り出しだけで
//! 足りる — このためだけに依存を増やさない．

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use pxsmith_core::{Rgba8, RgbaCanvas};

/// 反転ビット (Tiled の規約)．
const FLIP_H: u32 = 0x8000_0000;
const FLIP_V: u32 = 0x4000_0000;
const FLIP_D: u32 = 0x2000_0000;
const GID_MASK: u32 = 0x1FFF_FFFF;

/// 描き出した画面．
#[derive(Clone, Debug)]
pub struct Scene {
    pub canvas: RgbaCanvas,
    /// 重ねたレイヤの数．
    pub layers: usize,
    /// 実際に描いたタイル数 (横, 縦)．`max_tiles` で切ることがある．
    pub tiles: (u32, u32),
    /// 地図が本来持っているタイル数．
    pub full: (u32, u32),
}

struct Tileset {
    first_gid: u32,
    tile: (u32, u32),
    spacing: u32,
    margin: u32,
    columns: u32,
    image: RgbaCanvas,
}

/// `<name ...>` の属性を取り出す．
///
/// **前に空白を要求する** — `width` が `tilewidth` に当たらないようにするため．
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(" {name}=\"");
    let start = tag.find(&key)? + key.len();
    let rest = &tag[start..];
    Some(&rest[..rest.find('"')?])
}

fn attr_u32(tag: &str, name: &str) -> Option<u32> {
    attr(tag, name)?.parse().ok()
}

/// `from` 以降の最初の `<name ...>` を返す (タグ本体と，その `>` の次の位置)．
fn find_tag<'a>(xml: &'a str, name: &str, from: usize) -> Option<(&'a str, usize)> {
    let head = format!("<{name}");
    let start = xml[from..].find(&head)? + from;
    // `<layer` が `<layerx` に当たらないこと
    let after = xml[start + head.len()..].chars().next()?;
    if after.is_alphanumeric() {
        return find_tag(xml, name, start + head.len());
    }
    let end = xml[start..].find('>')? + start;
    Some((&xml[start..end], end + 1))
}

/// 相対参照を解決する (`../Tilemap/tilemap.png` など)．
fn resolve(base: &Path, rel: &str) -> PathBuf {
    let joined = base.parent().unwrap_or(Path::new(".")).join(rel);
    // `..` を潰す — そのままでも開けるが，エラーメッセージが読みにくくなる
    let mut out = PathBuf::new();
    for part in joined.components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// タイルセット 1 つを読む．
fn read_tileset(tmx: &Path, xml: &str, tag: &str, after: usize) -> Result<Tileset> {
    let first_gid = attr_u32(tag, "firstgid").context("firstgid が無い")?;

    // 外部 (.tsx) なら開き直す．埋め込みならこの tmx の続きを読む
    let (owner, def, image_tag) = match attr(tag, "source") {
        Some(src) => {
            let path = resolve(tmx, src);
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("{} を読めない", path.display()))?;
            let (def, next) = find_tag(&text, "tileset", 0)
                .map(|(d, n)| (d.to_string(), n))
                .context("tsx に <tileset> が無い")?;
            let (image, _) = find_tag(&text, "image", next)
                .map(|(i, n)| (i.to_string(), n))
                .context("tsx に <image> が無い")?;
            (path, def, image)
        }
        None => {
            let (image, _) = find_tag(xml, "image", after).context("<image> が無い")?;
            (tmx.to_path_buf(), tag.to_string(), image.to_string())
        }
    };

    let tile = (
        attr_u32(&def, "tilewidth").context("tilewidth が無い")?,
        attr_u32(&def, "tileheight").context("tileheight が無い")?,
    );
    let spacing = attr_u32(&def, "spacing").unwrap_or(0);
    let margin = attr_u32(&def, "margin").unwrap_or(0);

    let src = attr(&image_tag, "source").context("<image source> が無い")?;
    let path = resolve(&owner, src);
    let image = pxsmith_io::png::read_rgba(&path)
        .with_context(|| format!("タイル画像 {} を読めない", path.display()))?;

    // `columns` を書かない tsx がある (埋め込みは特に)．画像の幅から求める
    let columns = attr_u32(&def, "columns").unwrap_or_else(|| {
        (image.width().saturating_sub(2 * margin) + spacing) / (tile.0 + spacing)
    });
    anyhow::ensure!(columns > 0, "タイルの列数が 0 になった");

    Ok(Tileset {
        first_gid,
        tile,
        spacing,
        margin,
        columns,
        image,
    })
}

/// base64 を復号する．**この 1 用途のために依存を増やさない** — 表は 64 文字である．
fn base64(text: &str) -> Result<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for b in text.bytes() {
        if b.is_ascii_whitespace() || b == b'=' {
            continue;
        }
        let v = TABLE
            .iter()
            .position(|c| *c == b)
            .with_context(|| format!("base64 に使えない文字 {:?}", b as char))?;
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok(out)
}

/// レイヤの升目を読む．
fn read_layer(xml: &str, after: usize) -> Result<(Vec<u32>, usize)> {
    use std::io::Read;

    let (data_tag, body) = find_tag(xml, "data", after).context("<data> が無い")?;
    let end = xml[body..].find("</data>").context("</data> が無い")? + body;
    let text = &xml[body..end];

    let gids = match attr(data_tag, "encoding") {
        Some("csv") => text
            .split(',')
            .filter_map(|s| s.trim().parse::<u32>().ok())
            .collect(),
        Some("base64") => {
            let raw = base64(text)?;
            // 圧縮を解く．**gid は 4 バイトずつのリトルエンディアン**である
            let bytes = match attr(data_tag, "compression") {
                None | Some("") => raw,
                Some(kind @ ("zlib" | "gzip")) => {
                    let mut buf = Vec::new();
                    let read: &mut dyn Read = &mut match kind {
                        "zlib" => {
                            Box::new(flate2::read::ZlibDecoder::new(&raw[..])) as Box<dyn Read>
                        }
                        _ => Box::new(flate2::read::GzDecoder::new(&raw[..])),
                    };
                    read.read_to_end(&mut buf)
                        .with_context(|| format!("{kind} を展開できない"))?;
                    buf
                }
                Some(other) => bail!("読めない圧縮形式 ({other})"),
            };
            bytes
                .chunks_exact(4)
                .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect()
        }
        other => bail!(
            "読めないレイヤ形式 (encoding = {})",
            other.unwrap_or("なし")
        ),
    };
    Ok((gids, end))
}

/// 1 画素を上に重ねる (source-over)．
fn over(dst: Rgba8, src: Rgba8) -> Rgba8 {
    if src.a == 0 {
        return dst;
    }
    if src.a == 255 {
        return src;
    }
    let a = u32::from(src.a);
    let inv = 255 - a;
    let mix = |s: u8, d: u8| ((u32::from(s) * a + u32::from(d) * inv) / 255) as u8;
    Rgba8::new(
        mix(src.r, dst.r),
        mix(src.g, dst.g),
        mix(src.b, dst.b),
        (a + u32::from(dst.a) * inv / 255).min(255) as u8,
    )
}

/// 地図を描き出す．`max_tiles` を超える地図は左上から切り取る．
pub fn render(tmx: &Path, max_tiles: (u32, u32)) -> Result<Scene> {
    let xml =
        std::fs::read_to_string(tmx).with_context(|| format!("{} を読めない", tmx.display()))?;

    let (map, mut cursor) = find_tag(&xml, "map", 0).context("<map> が無い")?;
    match attr(map, "orientation") {
        Some("orthogonal") => {}
        other => bail!("orthogonal 以外は読めない ({})", other.unwrap_or("なし")),
    }
    let full = (
        attr_u32(map, "width").context("width が無い")?,
        attr_u32(map, "height").context("height が無い")?,
    );
    let tile = (
        attr_u32(map, "tilewidth").context("tilewidth が無い")?,
        attr_u32(map, "tileheight").context("tileheight が無い")?,
    );

    // タイルセットは firstgid の大きい順に見て，最初に届いたものを使う
    let mut sets = Vec::new();
    while let Some((tag, next)) = find_tag(&xml, "tileset", cursor) {
        sets.push(read_tileset(tmx, &xml, tag, next)?);
        cursor = next;
    }
    anyhow::ensure!(!sets.is_empty(), "<tileset> が無い");
    sets.sort_by_key(|s| std::cmp::Reverse(s.first_gid));

    let tiles = (
        full.0.min(max_tiles.0).max(1),
        full.1.min(max_tiles.1).max(1),
    );
    let (w, h) = (tiles.0 * tile.0, tiles.1 * tile.1);
    let mut canvas = RgbaCanvas::filled(w, h, Rgba8::TRANSPARENT);

    let mut cursor = 0usize;
    let mut layers = 0usize;
    while let Some((_, after)) = find_tag(&xml, "layer", cursor) {
        let (gids, end) = read_layer(&xml, after)?;
        anyhow::ensure!(
            gids.len() as u32 >= full.0 * full.1,
            "レイヤの升目が足りない ({} < {})",
            gids.len(),
            full.0 * full.1
        );
        for ty in 0..tiles.1 {
            for tx in 0..tiles.0 {
                let raw = gids[(ty * full.0 + tx) as usize];
                let gid = raw & GID_MASK;
                if gid == 0 {
                    continue;
                }
                let Some(set) = sets.iter().find(|s| s.first_gid <= gid) else {
                    continue;
                };
                let id = gid - set.first_gid;
                let (tw, th) = set.tile;
                let sx0 = set.margin + (id % set.columns) * (tw + set.spacing);
                let sy0 = set.margin + (id / set.columns) * (th + set.spacing);
                let (fh, fv, fd) = (raw & FLIP_H != 0, raw & FLIP_V != 0, raw & FLIP_D != 0);
                for y in 0..th {
                    for x in 0..tw {
                        // 対角 → 水平 → 垂直 の順 (Tiled の規約)．対角は正方形のみ
                        let (mut sx, mut sy) = (x, y);
                        if fd && tw == th {
                            std::mem::swap(&mut sx, &mut sy);
                        }
                        if fh {
                            sx = tw - 1 - sx;
                        }
                        if fv {
                            sy = th - 1 - sy;
                        }
                        let Some(src) = set.image.get((sx0 + sx) as i32, (sy0 + sy) as i32) else {
                            continue;
                        };
                        let (dx, dy) = ((tx * tile.0 + x) as i32, (ty * tile.1 + y) as i32);
                        if let Some(dst) = canvas.get(dx, dy) {
                            canvas.set(dx, dy, over(dst, src));
                        }
                    }
                }
            }
        }
        layers += 1;
        cursor = end;
    }
    anyhow::ensure!(layers > 0, "<layer> が無い");

    Ok(Scene {
        canvas,
        layers,
        tiles,
        full,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2x1 タイルの地図を組み立てて，描き出しを確かめる．
    fn fixture(dir: &Path, spacing: u32) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        // タイル 0 = 赤，タイル 1 = 左半分だけ緑 (反転が効いたか分かるように)
        let (tw, th) = (4u32, 4u32);
        let w = tw * 2 + spacing;
        let mut sheet = RgbaCanvas::filled(w, th, Rgba8::TRANSPARENT);
        for y in 0..th as i32 {
            for x in 0..tw as i32 {
                sheet.set(x, y, Rgba8::rgb(200, 0, 0));
                let c = if x < 2 {
                    Rgba8::rgb(0, 200, 0)
                } else {
                    Rgba8::TRANSPARENT
                };
                sheet.set((tw + spacing) as i32 + x, y, c);
            }
        }
        pxsmith_io::png::write_rgba(dir.join("sheet.png"), &sheet).unwrap();

        std::fs::write(
            dir.join("map.tsx"),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<tileset version="1.8" name="t" tilewidth="4" tileheight="4" spacing="{spacing}" tilecount="2" columns="2">
 <image source="sheet.png" width="{w}" height="4"/>
</tileset>"#
            ),
        )
        .unwrap();

        // 升目: [赤, 緑] / [緑を水平反転, 空]
        let flipped = 2 | FLIP_H;
        std::fs::write(
            dir.join("map.tmx"),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<map version="1.8" orientation="orthogonal" renderorder="right-down" width="2" height="2" tilewidth="4" tileheight="4">
 <tileset firstgid="1" source="map.tsx"/>
 <layer id="1" name="L" width="2" height="2">
  <data encoding="csv">
1,2,
{flipped},0
</data>
 </layer>
</map>"#
            ),
        )
        .unwrap();
        dir.join("map.tmx")
    }

    fn temp(case: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pxforge-scene-{case}"));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_map_is_drawn_at_the_native_resolution() {
        let dir = temp("native");
        let tmx = fixture(&dir, 0);
        let scene = render(&tmx, (100, 100)).unwrap();
        assert_eq!((scene.canvas.width(), scene.canvas.height()), (8, 8));
        assert_eq!((scene.layers, scene.tiles, scene.full), (1, (2, 2), (2, 2)));
        // 左上は赤
        assert_eq!(scene.canvas.get(0, 0), Some(Rgba8::rgb(200, 0, 0)));
        // 右上のタイルは左半分だけ緑，右半分は透明のまま
        assert_eq!(scene.canvas.get(4, 0), Some(Rgba8::rgb(0, 200, 0)));
        assert_eq!(scene.canvas.get(7, 0), Some(Rgba8::TRANSPARENT));
        // 空の升目 (gid 0) は描かない
        assert_eq!(scene.canvas.get(4, 4), Some(Rgba8::TRANSPARENT));
    }

    #[test]
    fn the_flip_bits_are_honoured() {
        let dir = temp("flip");
        let tmx = fixture(&dir, 0);
        let scene = render(&tmx, (100, 100)).unwrap();
        // 左下は「左半分が緑」を水平反転したもの — 右半分が緑になる
        assert_eq!(scene.canvas.get(0, 4), Some(Rgba8::TRANSPARENT));
        assert_eq!(scene.canvas.get(3, 4), Some(Rgba8::rgb(0, 200, 0)));
    }

    #[test]
    fn the_spacing_between_tiles_is_skipped() {
        // 余白つきのシートでも同じ絵になること (Kenney の tilemap.png は spacing 1)
        let plain = render(&fixture(&temp("gap0"), 0), (100, 100)).unwrap();
        let gapped = render(&fixture(&temp("gap1"), 1), (100, 100)).unwrap();
        assert_eq!(plain.canvas.pixels(), gapped.canvas.pixels());
    }

    #[test]
    fn a_large_map_is_cut_down() {
        let dir = temp("cut");
        let tmx = fixture(&dir, 0);
        let scene = render(&tmx, (1, 2)).unwrap();
        assert_eq!((scene.canvas.width(), scene.canvas.height()), (4, 8));
        assert_eq!((scene.tiles, scene.full), ((1, 2), (2, 2)));
        // 切っても左端の列は同じ (行の読み飛ばしを間違えていないこと)
        assert_eq!(scene.canvas.get(0, 4), Some(Rgba8::TRANSPARENT));
        assert_eq!(scene.canvas.get(3, 4), Some(Rgba8::rgb(0, 200, 0)));
    }

    #[test]
    fn base64_layers_give_the_same_grid_as_csv() {
        // 同じ升目を csv ・base64 (無圧縮) ・base64+zlib で書いて，結果が一致すること
        use std::io::Write;
        let gids: Vec<u32> = vec![1, 2, 2 | FLIP_H, 0];
        let raw: Vec<u8> = gids.iter().flat_map(|g| g.to_le_bytes()).collect();
        let plain = {
            const T: &[u8; 64] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut s = String::new();
            for c in raw.chunks(3) {
                let mut n = 0u32;
                for (i, b) in c.iter().enumerate() {
                    n |= u32::from(*b) << (16 - 8 * i);
                }
                for i in 0..=c.len() {
                    s.push(T[((n >> (18 - 6 * i)) & 0x3F) as usize] as char);
                }
                for _ in c.len()..3 {
                    s.push('=');
                }
            }
            s
        };
        assert_eq!(base64(&plain).unwrap(), raw);

        let zipped = {
            let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
            e.write_all(&raw).unwrap();
            e.finish().unwrap()
        };
        // zlib を通しても同じ升目が戻ること (base64 化は上の実装を信頼して省く)
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(&zipped[..]), &mut buf)
            .unwrap();
        assert_eq!(buf, raw);

        let xml = format!(r#"<data encoding="base64">{plain}</data>"#);
        assert_eq!(read_layer(&xml, 0).unwrap().0, gids);
        let csv = format!(
            r#"<data encoding="csv">{}</data>"#,
            gids.iter()
                .map(|g| g.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        assert_eq!(read_layer(&csv, 0).unwrap().0, gids);
    }

    #[test]
    fn an_attribute_is_matched_whole() {
        // `width` が `tilewidth` を拾わないこと
        let tag = r#"<map width="32" height="20" tilewidth="16" tileheight="16""#;
        assert_eq!(attr_u32(tag, "width"), Some(32));
        assert_eq!(attr_u32(tag, "tilewidth"), Some(16));
        assert_eq!(attr(tag, "orientation"), None);
    }

    #[test]
    fn a_tag_name_is_matched_whole() {
        // `<layer` が `<layerx` を拾わないこと
        let xml = r#"<layerx a="1"/><layer b="2"/>"#;
        let (tag, _) = find_tag(xml, "layer", 0).unwrap();
        assert_eq!(attr_u32(tag, "b"), Some(2));
    }
}
