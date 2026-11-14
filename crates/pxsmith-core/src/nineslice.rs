//! 9-slice の宣言と，Godot 4.7 と同じ画素中心での伸縮．

use serde::de::Error as _;
use serde::ser::SerializeStruct as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::canvas::IndexedCanvas;
use crate::error::{CoreError, Result};

/// Godot `NinePatchRect.AxisStretchMode` と同じ軸方向の伸縮方式．
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AxisStretch {
    /// 中央の帯を描画先へ引き伸ばす．
    #[default]
    Stretch,
    /// 中央の帯を元の大きさのまま繰り返す．
    Tile,
    /// 端数が出ない整数回の繰り返しへ合わせる．
    TileFit,
}

/// 9-slice の四辺と軸ごとの伸縮方式．
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct NineSlice {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
    pub stretch_x: AxisStretch,
    pub stretch_y: AxisStretch,
}

impl Serialize for NineSlice {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // SheetCell の JSON では，省略や共通指定に畳まず軸ごとの値を明示する．
        let mut state = serializer.serialize_struct("NineSlice", 6)?;
        state.serialize_field("left", &self.left)?;
        state.serialize_field("top", &self.top)?;
        state.serialize_field("right", &self.right)?;
        state.serialize_field("bottom", &self.bottom)?;
        state.serialize_field("stretch_x", &self.stretch_x)?;
        state.serialize_field("stretch_y", &self.stretch_y)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for NineSlice {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawNineSlice {
            left: u32,
            top: u32,
            right: u32,
            bottom: u32,
            #[serde(default)]
            stretch: Option<AxisStretch>,
            #[serde(default)]
            stretch_x: Option<AxisStretch>,
            #[serde(default)]
            stretch_y: Option<AxisStretch>,
        }

        let raw = RawNineSlice::deserialize(deserializer)?;
        if raw.stretch.is_some() && (raw.stretch_x.is_some() || raw.stretch_y.is_some()) {
            return Err(D::Error::custom(
                "stretch と stretch_x / stretch_y は同時に指定できない",
            ));
        }
        let (stretch_x, stretch_y) = match raw.stretch {
            Some(stretch) => (stretch, stretch),
            None => (
                raw.stretch_x.unwrap_or_default(),
                raw.stretch_y.unwrap_or_default(),
            ),
        };
        Ok(Self {
            left: raw.left,
            top: raw.top,
            right: raw.right,
            bottom: raw.bottom,
            stretch_x,
            stretch_y,
        })
    }
}

impl NineSlice {
    /// 元画像に中央が縦横とも 1 px 以上残ることを確かめる．
    pub fn validate_source(&self, width: u32, height: u32) -> Result<()> {
        if u64::from(self.left) + u64::from(self.right) >= u64::from(width)
            || u64::from(self.top) + u64::from(self.bottom) >= u64::from(height)
        {
            return Err(CoreError::NineSliceMargins {
                width,
                height,
                left: self.left,
                top: self.top,
                right: self.right,
                bottom: self.bottom,
            });
        }
        Ok(())
    }

    /// 描画先に四辺を切らずに置けることを確かめる．中央は 0 px でもよい．
    pub fn validate_target(&self, width: u32, height: u32) -> Result<()> {
        if u64::from(self.left) + u64::from(self.right) > u64::from(width)
            || u64::from(self.top) + u64::from(self.bottom) > u64::from(height)
        {
            return Err(CoreError::NineSliceTarget {
                width,
                height,
                left: self.left,
                top: self.top,
                right: self.right,
                bottom: self.bottom,
            });
        }
        Ok(())
    }
}

/// 継ぎ目の可能性を報告する軸．
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NineSliceAxis {
    Horizontal,
    Vertical,
}

/// Godot 4.7 の `map_ninepatch_axis` と同じ `f32` の計算で 9-slice する．
pub fn render_nine_slice(
    source: &IndexedCanvas,
    spec: &NineSlice,
    width: u32,
    height: u32,
) -> Result<IndexedCanvas> {
    spec.validate_source(source.width(), source.height())?;
    spec.validate_target(width, height)?;

    let xs: Vec<i32> = (0..width)
        .map(|x| {
            mapped_texel(
                x,
                width,
                source.width(),
                spec.left,
                spec.right,
                spec.stretch_x,
            )
        })
        .collect();
    let ys: Vec<i32> = (0..height)
        .map(|y| {
            mapped_texel(
                y,
                height,
                source.height(),
                spec.top,
                spec.bottom,
                spec.stretch_y,
            )
        })
        .collect();
    let pixels = ys
        .iter()
        .flat_map(|&sy| {
            xs.iter()
                .map(move |&sx| source.get(sx, sy).expect("写像は元画像の内側"))
        })
        .collect();
    Ok(IndexedCanvas::from_pixels(width, height, pixels)?.with_transparent(source.transparent()))
}

fn mapped_texel(
    coordinate: u32,
    draw_size: u32,
    texture_size: u32,
    margin_begin: u32,
    margin_end: u32,
    repeat: AxisStretch,
) -> i32 {
    let pixel = coordinate as f32 + 0.5;
    let draw_size = draw_size as f32;
    let tex_pixel_size = 1.0f32 / texture_size as f32;
    let tex_size = 1.0f32 / tex_pixel_size;
    let margin_begin = margin_begin as f32;
    let margin_end = margin_end as f32;

    let uv = if pixel < margin_begin {
        pixel * tex_pixel_size
    } else if pixel >= draw_size - margin_end {
        (tex_size - (draw_size - pixel)) * tex_pixel_size
    } else {
        match repeat {
            AxisStretch::Stretch => {
                let ratio = (pixel - margin_begin) / (draw_size - margin_begin - margin_end);
                (margin_begin + ratio * (tex_size - margin_begin - margin_end)) * tex_pixel_size
            }
            AxisStretch::Tile => {
                let size = tex_size - margin_begin - margin_end;
                let offset = glsl_mod(pixel - margin_begin, size);
                (margin_begin + offset) * tex_pixel_size
            }
            AxisStretch::TileFit => {
                let src_area = draw_size - margin_begin - margin_end;
                let dst_area = tex_size - margin_begin - margin_end;
                let scale = 1.0f32.max((src_area / dst_area.max(0.0000001) + 0.5).floor());
                let ratio = (pixel - margin_begin) / src_area;
                let ratio = glsl_mod(ratio * scale, 1.0);
                (margin_begin + ratio * dst_area) * tex_pixel_size
            }
        }
    };
    (uv * texture_size as f32)
        .floor()
        .clamp(0.0, texture_size.saturating_sub(1) as f32) as i32
}

fn glsl_mod(x: f32, y: f32) -> f32 {
    x - y * (x / y).floor()
}

/// 繰り返す中央帯の先頭と末尾が違う軸を返す．
pub fn visible_seams(source: &IndexedCanvas, spec: &NineSlice) -> Result<Vec<NineSliceAxis>> {
    spec.validate_source(source.width(), source.height())?;
    let mut seams = Vec::new();
    if spec.stretch_x != AxisStretch::Stretch {
        let first = spec.left as i32;
        let last = (source.width() - spec.right - 1) as i32;
        if (0..source.height() as i32).any(|y| source.get(first, y) != source.get(last, y)) {
            seams.push(NineSliceAxis::Horizontal);
        }
    }
    if spec.stretch_y != AxisStretch::Stretch {
        let first = spec.top as i32;
        let last = (source.height() - spec.bottom - 1) as i32;
        if (0..source.width() as i32).any(|x| source.get(x, first) != source.get(x, last)) {
            seams.push(NineSliceAxis::Vertical);
        }
    }
    Ok(seams)
}
