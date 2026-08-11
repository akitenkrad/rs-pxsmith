//! キャンバス (設計書 3.2)．
//!
//! インデックスカラーを第一級とする (中心となる設計判断 1)．RGBA は生成 AI 出力や
//! 格子復元前の中間表現としてのみ扱う．

use crate::color::Rgba8;
use crate::error::{CoreError, Result};
use crate::math::{IRect, IVec2, UVec2, clip_pair, ivec2, uvec2};

/// インデックスカラーのキャンバス．
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexedCanvas {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    transparent: Option<u8>,
}

/// RGBA のキャンバス．`conform` の入力など，添字化する前の表現．
#[derive(Clone, Debug, PartialEq)]
pub struct RgbaCanvas {
    width: u32,
    height: u32,
    pixels: Vec<Rgba8>,
}

impl IndexedCanvas {
    /// 単一の添字で埋めたキャンバスを作る．
    pub fn filled(width: u32, height: u32, index: u8) -> Self {
        Self {
            width,
            height,
            pixels: vec![index; uvec2(width, height).area()],
            transparent: None,
        }
    }

    /// 画素列からキャンバスを作る．長さが `width * height` と一致しなければエラー．
    pub fn from_pixels(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self> {
        let expected = uvec2(width, height).area();
        if pixels.len() != expected {
            return Err(CoreError::PixelCountMismatch {
                width,
                height,
                expected,
                actual: pixels.len(),
            });
        }
        Ok(Self {
            width,
            height,
            pixels,
            transparent: None,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn size(&self) -> UVec2 {
        uvec2(self.width, self.height)
    }

    pub fn bounds(&self) -> IRect {
        IRect::new(0, 0, self.width, self.height)
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn pixels_mut(&mut self) -> &mut [u8] {
        &mut self.pixels
    }

    /// 透明を表す添字 (D3)．`None` は「透明色を持たない」．
    pub fn transparent(&self) -> Option<u8> {
        self.transparent
    }

    pub fn set_transparent(&mut self, index: Option<u8>) {
        self.transparent = index;
    }

    pub fn with_transparent(mut self, index: Option<u8>) -> Self {
        self.transparent = index;
        self
    }

    fn offset(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x as i64 >= self.width as i64 || y as i64 >= self.height as i64 {
            None
        } else {
            Some(y as usize * self.width as usize + x as usize)
        }
    }

    /// 範囲外は `None`．端の処理を呼び出し側で書かせないための唯一の入口．
    pub fn get(&self, x: i32, y: i32) -> Option<u8> {
        self.offset(x, y).map(|i| self.pixels[i])
    }

    pub fn get_at(&self, p: IVec2) -> Option<u8> {
        self.get(p.x, p.y)
    }

    /// 書き込めたら `true`．範囲外は黙って捨てる．
    pub fn set(&mut self, x: i32, y: i32, index: u8) -> bool {
        match self.offset(x, y) {
            Some(i) => {
                self.pixels[i] = index;
                true
            }
            None => false,
        }
    }

    pub fn set_at(&mut self, p: IVec2, index: u8) -> bool {
        self.set(p.x, p.y, index)
    }

    /// 透明添字と一致するか．透明色を持たない場合は常に `false`．
    pub fn is_transparent_at(&self, p: IVec2) -> bool {
        match (self.transparent, self.get_at(p)) {
            (Some(t), Some(v)) => t == v,
            _ => false,
        }
    }

    /// 添字を置換する．`map[old] = new`．
    ///
    /// パレットの明度順正規化 (D50) に伴い全画素の添字を張り替えるために使う．
    /// 透明添字も同じ表で張り替える．
    pub fn remap(&mut self, map: &[u8]) -> Result<()> {
        if map.len() > 256 {
            return Err(CoreError::PaletteTooLarge(map.len()));
        }
        for p in &mut self.pixels {
            if let Some(&n) = map.get(*p as usize) {
                *p = n;
            }
        }
        if let Some(t) = self.transparent
            && let Some(&n) = map.get(t as usize)
        {
            self.transparent = Some(n);
        }
        Ok(())
    }

    /// 透明でない画素を囲う最小の矩形．全て透明なら `None`．
    pub fn opaque_bbox(&self) -> Option<IRect> {
        let t = self.transparent?;
        let (mut x0, mut y0) = (i32::MAX, i32::MAX);
        let (mut x1, mut y1) = (i32::MIN, i32::MIN);
        for y in 0..self.height as i32 {
            for x in 0..self.width as i32 {
                if self.pixels[y as usize * self.width as usize + x as usize] != t {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
            }
        }
        if x0 > x1 {
            None
        } else {
            Some(IRect::new(
                x0,
                y0,
                (x1 - x0 + 1) as u32,
                (y1 - y0 + 1) as u32,
            ))
        }
    }

    /// 矩形を切り出す．範囲外の画素は `fill` で埋める．
    pub fn crop(&self, rect: IRect, fill: u8) -> Self {
        let mut out = Self::filled(rect.w, rect.h, fill);
        out.transparent = self.transparent;
        for dy in 0..rect.h as i32 {
            for dx in 0..rect.w as i32 {
                if let Some(v) = self.get(rect.x + dx, rect.y + dy) {
                    out.set(dx, dy, v);
                }
            }
        }
        out
    }

    /// `src` を `at` に転送する．`skip_transparent` が真なら `src` の透明画素は書かない．
    pub fn blit(&mut self, src: &Self, at: IVec2, skip_transparent: bool) {
        let Some((d, s)) = clip_pair(self.size(), src.size(), at) else {
            return;
        };
        for dy in 0..d.h as i32 {
            for dx in 0..d.w as i32 {
                let sp = ivec2(s.x + dx, s.y + dy);
                if skip_transparent && src.is_transparent_at(sp) {
                    continue;
                }
                if let Some(v) = src.get_at(sp) {
                    self.set(d.x + dx, d.y + dy, v);
                }
            }
        }
    }
}

impl RgbaCanvas {
    pub fn filled(width: u32, height: u32, color: Rgba8) -> Self {
        Self {
            width,
            height,
            pixels: vec![color; uvec2(width, height).area()],
        }
    }

    pub fn from_pixels(width: u32, height: u32, pixels: Vec<Rgba8>) -> Result<Self> {
        let expected = uvec2(width, height).area();
        if pixels.len() != expected {
            return Err(CoreError::PixelCountMismatch {
                width,
                height,
                expected,
                actual: pixels.len(),
            });
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn size(&self) -> UVec2 {
        uvec2(self.width, self.height)
    }

    pub fn pixels(&self) -> &[Rgba8] {
        &self.pixels
    }

    pub fn pixels_mut(&mut self) -> &mut [Rgba8] {
        &mut self.pixels
    }

    pub fn get(&self, x: i32, y: i32) -> Option<Rgba8> {
        if x < 0 || y < 0 || x as i64 >= self.width as i64 || y as i64 >= self.height as i64 {
            None
        } else {
            Some(self.pixels[y as usize * self.width as usize + x as usize])
        }
    }

    pub fn set(&mut self, x: i32, y: i32, color: Rgba8) -> bool {
        if x < 0 || y < 0 || x as i64 >= self.width as i64 || y as i64 >= self.height as i64 {
            false
        } else {
            self.pixels[y as usize * self.width as usize + x as usize] = color;
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> IndexedCanvas {
        IndexedCanvas::from_pixels(3, 2, vec![0, 1, 2, 3, 4, 5])
            .unwrap()
            .with_transparent(Some(0))
    }

    #[test]
    fn from_pixels_rejects_length_mismatch() {
        assert!(IndexedCanvas::from_pixels(3, 2, vec![0; 5]).is_err());
    }

    #[test]
    fn get_returns_none_outside() {
        let c = sample();
        assert_eq!(c.get(0, 0), Some(0));
        assert_eq!(c.get(2, 1), Some(5));
        assert_eq!(c.get(-1, 0), None);
        assert_eq!(c.get(3, 0), None);
        assert_eq!(c.get(0, 2), None);
    }

    #[test]
    fn remap_also_moves_transparent_index() {
        let mut c = sample();
        // 添字を全て 1 つずらす
        let map: Vec<u8> = (0..6u8).map(|i| i + 10).collect();
        c.remap(&map).unwrap();
        assert_eq!(c.pixels(), &[10, 11, 12, 13, 14, 15]);
        assert_eq!(c.transparent(), Some(10));
    }

    #[test]
    fn opaque_bbox_excludes_transparent_border() {
        let c = IndexedCanvas::from_pixels(4, 3, vec![0, 0, 0, 0, 0, 7, 7, 0, 0, 0, 0, 0])
            .unwrap()
            .with_transparent(Some(0));
        assert_eq!(c.opaque_bbox(), Some(IRect::new(1, 1, 2, 1)));
    }

    #[test]
    fn opaque_bbox_is_none_when_fully_transparent() {
        let c = IndexedCanvas::filled(4, 4, 0).with_transparent(Some(0));
        assert_eq!(c.opaque_bbox(), None);
    }

    #[test]
    fn blit_clips_at_edges() {
        let mut dst = IndexedCanvas::filled(4, 4, 0);
        let src = IndexedCanvas::filled(3, 3, 9);
        dst.blit(&src, ivec2(3, 3), false);
        assert_eq!(dst.get(3, 3), Some(9));
        assert_eq!(dst.get(2, 2), Some(0));
    }

    #[test]
    fn blit_can_skip_transparent_source_pixels() {
        let mut dst = IndexedCanvas::filled(2, 1, 5);
        let src = IndexedCanvas::from_pixels(2, 1, vec![0, 8])
            .unwrap()
            .with_transparent(Some(0));
        dst.blit(&src, ivec2(0, 0), true);
        assert_eq!(dst.pixels(), &[5, 8]);
    }

    #[test]
    fn crop_round_trips_through_blit() {
        let c = sample();
        let r = IRect::new(1, 0, 2, 2);
        let part = c.crop(r, 0);
        let mut back = IndexedCanvas::filled(3, 2, 0).with_transparent(Some(0));
        back.blit(&c.crop(IRect::new(0, 0, 1, 2), 0), ivec2(0, 0), false);
        back.blit(&part, ivec2(1, 0), false);
        assert_eq!(back.pixels(), c.pixels());
    }
}
