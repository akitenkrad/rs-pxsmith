//! 色と色距離 (設計書 3.2 / 6.6)．
//!
//! 色距離は OKLab に統一する (D12)．明度の重み $w_L$ は `px palette apply` の
//! 経路でのみ可変で，`px quantize` は 1.0 固定．

use crate::error::{CoreError, Result};

/// 8 ビット RGBA．アルファは 2 値のみを想定する (D4)．
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Rgba8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

/// OKLab 座標．[`Palette`](crate::palette::Palette) では `entries` から導出される
/// キャッシュであり，個別に書き換える API は公開しない．
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Oklab {
    pub l: f32,
    pub a: f32,
    pub b: f32,
}

impl Rgba8 {
    pub const TRANSPARENT: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };

    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// 不透明色を作る．
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn is_opaque(self) -> bool {
        self.a == 255
    }

    /// アルファが 2 値かどうか (不変条件，設計書 3.2)．
    pub const fn has_binary_alpha(self) -> bool {
        self.a == 0 || self.a == 255
    }

    /// `RRGGBB` の 6 桁 16 進を読む (設計書 4.5)．先頭の `#` は付けない規約だが，
    /// 外部資産の取り込みを妨げないよう受け付けはする．
    pub fn from_hex_str(s: &str) -> Result<Self> {
        let t = s.trim().trim_start_matches('#');
        if t.len() != 6 || !t.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(CoreError::InvalidHexColor(s.to_string()));
        }
        let v =
            u32::from_str_radix(t, 16).map_err(|_| CoreError::InvalidHexColor(s.to_string()))?;
        Ok(Self::rgb(
            ((v >> 16) & 0xff) as u8,
            ((v >> 8) & 0xff) as u8,
            (v & 0xff) as u8,
        ))
    }

    /// `.hex` 形式で書き出す小文字 6 桁．アルファは落ちる (設計書 4.5)．
    pub fn to_hex_string(self) -> String {
        format!("{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// 決定論的な全順序のキー (設計書 6.15 規則 1)．
    pub const fn sort_key(self) -> (u8, u8, u8, u8) {
        (self.r, self.g, self.b, self.a)
    }

    pub fn to_oklab(self) -> Oklab {
        oklab_of(self)
    }
}

impl Oklab {
    pub const fn new(l: f32, a: f32, b: f32) -> Self {
        Self { l, a, b }
    }

    /// 彩度 (chroma)．彩度カーブの検査に使う (lint ルール 5)．
    pub fn chroma(self) -> f32 {
        (self.a * self.a + self.b * self.b).sqrt()
    }
}

/// sRGB から OKLab へ変換する．アルファは無視する．
pub fn oklab_of(c: Rgba8) -> Oklab {
    use ::palette::{FromColor, Srgb};
    let srgb: Srgb<f32> = Srgb::new(c.r, c.g, c.b).into_format();
    let lab = ::palette::Oklab::from_color(srgb);
    Oklab::new(lab.l, lab.a, lab.b)
}

/// 重み付き OKLab 距離の 2 乗 (設計書 6.6)．
///
/// $d^2 = w_L (L_1 - L_2)^2 + (a_1 - a_2)^2 + (b_1 - b_2)^2$
pub fn distance_sq(x: Oklab, y: Oklab, w_l: f32) -> f32 {
    let dl = x.l - y.l;
    let da = x.a - y.a;
    let db = x.b - y.b;
    w_l * dl * dl + da * da + db * db
}

/// 色差 $\Delta E_{\mathrm{OKLab}}$ ($w_L = 1$)．格子推定の再構成検査で使う (設計書 6.1)．
pub fn delta_e(x: Oklab, y: Oklab) -> f32 {
    distance_sq(x, y, 1.0).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        for s in ["1a1c2c", "5d275d", "ffffff", "000000"] {
            let c = Rgba8::from_hex_str(s).unwrap();
            assert_eq!(c.to_hex_string(), s);
            assert!(c.is_opaque());
        }
    }

    #[test]
    fn hex_rejects_malformed() {
        for s in ["12345", "1234567", "gggggg", ""] {
            assert!(Rgba8::from_hex_str(s).is_err(), "受理してはいけない: {s}");
        }
    }

    #[test]
    fn oklab_lightness_is_monotone_on_grays() {
        let mut prev = f32::NEG_INFINITY;
        for v in [0u8, 32, 64, 128, 192, 255] {
            let l = oklab_of(Rgba8::rgb(v, v, v)).l;
            assert!(l > prev, "灰色の明度は単調増加のはず");
            prev = l;
        }
        assert!((oklab_of(Rgba8::rgb(0, 0, 0)).l).abs() < 1e-4);
        assert!((oklab_of(Rgba8::rgb(255, 255, 255)).l - 1.0).abs() < 1e-3);
    }

    #[test]
    fn gray_has_no_chroma() {
        assert!(oklab_of(Rgba8::rgb(128, 128, 128)).chroma() < 1e-4);
    }

    #[test]
    fn weight_scales_lightness_term_only() {
        let x = oklab_of(Rgba8::rgb(0, 0, 0));
        let y = oklab_of(Rgba8::rgb(255, 255, 255));
        let d1 = distance_sq(x, y, 1.0);
        let d2 = distance_sq(x, y, 4.0);
        assert!((d2 - 4.0 * d1).abs() < 1e-3);
    }
}
