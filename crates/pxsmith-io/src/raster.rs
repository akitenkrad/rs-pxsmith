//! 輪郭を線分へ折り，画素中心の nonzero 巻き数で単色画像にする．

use pxsmith_core::IndexedCanvas;

#[derive(Clone, Copy, Debug)]
struct Point {
    x: f64,
    y: f64,
}

#[derive(Debug, Default)]
pub(crate) struct Outline {
    contours: Vec<Vec<Point>>,
    current: Vec<Point>,
    scale: f64,
    off_grid: bool,
}

impl Outline {
    pub(crate) fn new(scale: f64) -> Self {
        Self {
            scale,
            ..Self::default()
        }
    }

    pub(crate) fn is_off_grid(&self) -> bool {
        self.off_grid
    }

    pub(crate) fn rasterize(
        &self,
        width: u32,
        height: u32,
        baseline: u32,
    ) -> (IndexedCanvas, bool) {
        let mut canvas = IndexedCanvas::filled(width, height, 0).with_transparent(Some(0));
        let contours = self
            .contours
            .iter()
            .chain((self.current.len() >= 2).then_some(&self.current));
        let points = contours.clone().flatten().copied().collect::<Vec<_>>();
        let Some(first) = points.first() else {
            return (canvas, false);
        };
        let (mut min_x, mut max_x, mut min_y, mut max_y) = (first.x, first.x, first.y, first.y);
        for point in &points[1..] {
            min_x = min_x.min(point.x);
            max_x = max_x.max(point.x);
            min_y = min_y.min(point.y);
            max_y = max_y.max(point.y);
        }

        let min_pixel_x = (min_x - 0.5).floor() as i32;
        let max_pixel_x = (max_x - 0.5).ceil() as i32;
        let min_pixel_y = (min_y - 0.5).floor() as i32;
        let max_pixel_y = (max_y - 0.5).ceil() as i32;
        let mut overflow = false;
        for pixel_y in min_pixel_y..=max_pixel_y {
            for pixel_x in min_pixel_x..=max_pixel_x {
                let point = Point {
                    // 境界上では，画面座標で右下へわずかに寄せる．これにより左・上を
                    // 含め，右・下を含めない規則を一意にする．
                    x: f64::from(pixel_x) + 0.5 + 1.0e-9,
                    y: f64::from(pixel_y) + 0.5 - 1.0e-9,
                };
                if winding_number(contours.clone(), point) == 0 {
                    continue;
                }
                let canvas_y = i64::from(baseline) - 1 - i64::from(pixel_y);
                if pixel_x >= 0
                    && i64::from(pixel_x) < i64::from(width)
                    && canvas_y >= 0
                    && canvas_y < i64::from(height)
                {
                    let _ = canvas.set(pixel_x, canvas_y as i32, 1);
                } else {
                    overflow = true;
                }
            }
        }
        (canvas, overflow)
    }

    fn point(&self, x: f32, y: f32) -> Point {
        Point {
            x: f64::from(x) * self.scale,
            y: f64::from(y) * self.scale,
        }
    }

    fn endpoint(&mut self, x: f32, y: f32) -> Point {
        let point = self.point(x, y);
        self.off_grid |= (point.x - point.x.round()).abs() > 1.0e-3
            || (point.y - point.y.round()).abs() > 1.0e-3;
        point
    }

    fn finish_contour(&mut self) {
        if self.current.len() >= 2 {
            self.contours.push(std::mem::take(&mut self.current));
        } else {
            self.current.clear();
        }
    }
}

impl ttf_parser::OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.finish_contour();
        let point = self.endpoint(x, y);
        self.current.push(point);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let point = self.endpoint(x, y);
        self.current.push(point);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let Some(&start) = self.current.last() else {
            return;
        };
        let control = self.point(x1, y1);
        let end = self.endpoint(x, y);
        flatten_quadratic(start, control, end, 0, &mut self.current);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let Some(&start) = self.current.last() else {
            return;
        };
        let control1 = self.point(x1, y1);
        let control2 = self.point(x2, y2);
        let end = self.endpoint(x, y);
        flatten_cubic(start, control1, control2, end, 0, &mut self.current);
    }

    fn close(&mut self) {
        if let Some(&first) = self.current.first()
            && self
                .current
                .last()
                .is_some_and(|last| !same_point(*last, first))
        {
            self.current.push(first);
        }
        self.finish_contour();
    }
}

fn same_point(a: Point, b: Point) -> bool {
    a.x == b.x && a.y == b.y
}

fn midpoint(a: Point, b: Point) -> Point {
    Point {
        x: (a.x + b.x) * 0.5,
        y: (a.y + b.y) * 0.5,
    }
}

fn distance_to_line_squared(point: Point, start: Point, end: Point) -> f64 {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let length_squared = dx * dx + dy * dy;
    if length_squared == 0.0 {
        return (point.x - start.x).powi(2) + (point.y - start.y).powi(2);
    }
    let cross = dx * (start.y - point.y) - (start.x - point.x) * dy;
    cross * cross / length_squared
}

fn flatten_quadratic(start: Point, control: Point, end: Point, depth: u8, output: &mut Vec<Point>) {
    const TOLERANCE_SQUARED: f64 = 1.0 / (64.0 * 64.0);
    if depth >= 12 || distance_to_line_squared(control, start, end) <= TOLERANCE_SQUARED {
        output.push(end);
        return;
    }
    let a = midpoint(start, control);
    let b = midpoint(control, end);
    let middle = midpoint(a, b);
    flatten_quadratic(start, a, middle, depth + 1, output);
    flatten_quadratic(middle, b, end, depth + 1, output);
}

fn flatten_cubic(
    start: Point,
    control1: Point,
    control2: Point,
    end: Point,
    depth: u8,
    output: &mut Vec<Point>,
) {
    const TOLERANCE_SQUARED: f64 = 1.0 / (64.0 * 64.0);
    if depth >= 12
        || (distance_to_line_squared(control1, start, end) <= TOLERANCE_SQUARED
            && distance_to_line_squared(control2, start, end) <= TOLERANCE_SQUARED)
    {
        output.push(end);
        return;
    }
    let a = midpoint(start, control1);
    let b = midpoint(control1, control2);
    let c = midpoint(control2, end);
    let d = midpoint(a, b);
    let e = midpoint(b, c);
    let middle = midpoint(d, e);
    flatten_cubic(start, a, d, middle, depth + 1, output);
    flatten_cubic(middle, e, c, end, depth + 1, output);
}

fn winding_number<'a>(contours: impl Iterator<Item = &'a Vec<Point>>, point: Point) -> i32 {
    let mut winding = 0;
    for contour in contours {
        for edge in contour.windows(2) {
            let (a, b) = (edge[0], edge[1]);
            let cross = (b.x - a.x) * (point.y - a.y) - (point.x - a.x) * (b.y - a.y);
            if a.y <= point.y {
                if b.y > point.y && cross > 0.0 {
                    winding += 1;
                }
            } else if b.y <= point.y && cross < 0.0 {
                winding -= 1;
            }
        }
    }
    winding
}

#[cfg(test)]
mod tests {
    use super::*;
    use ttf_parser::OutlineBuilder;

    fn rows(outline: &Outline, width: u32, height: u32, baseline: u32) -> Vec<String> {
        let (canvas, overflow) = outline.rasterize(width, height, baseline);
        assert!(!overflow);
        assert_eq!(canvas.transparent(), Some(0));
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| {
                        if canvas.get(x as i32, y as i32) == Some(1) {
                            '#'
                        } else {
                            '.'
                        }
                    })
                    .collect()
            })
            .collect()
    }

    fn rectangle(outline: &mut Outline, left: f32, bottom: f32, right: f32, top: f32) {
        outline.move_to(left, bottom);
        outline.line_to(left, top);
        outline.line_to(right, top);
        outline.line_to(right, bottom);
        outline.close();
    }

    #[test]
    fn square_uses_pixel_centres_without_antialiasing() {
        let mut outline = Outline::new(1.0);
        rectangle(&mut outline, 1.0, 0.0, 3.0, 2.0);
        assert_eq!(rows(&outline, 4, 3, 2), [".##.", ".##.", "...."]);
        assert!(!outline.is_off_grid());
    }

    #[test]
    fn opposite_contour_makes_a_hole_with_nonzero_winding() {
        let mut outline = Outline::new(1.0);
        rectangle(&mut outline, 0.0, 0.0, 3.0, 3.0);
        outline.move_to(1.0, 1.0);
        outline.line_to(2.0, 1.0);
        outline.line_to(2.0, 2.0);
        outline.line_to(1.0, 2.0);
        outline.close();
        assert_eq!(rows(&outline, 3, 3, 3), ["###", "#.#", "###"]);
    }

    #[test]
    fn cubic_curve_is_flattened_before_centre_testing() {
        let mut outline = Outline::new(1.0);
        outline.move_to(0.0, 0.0);
        outline.curve_to(0.0, 4.0, 4.0, 4.0, 4.0, 0.0);
        outline.close();
        assert_eq!(rows(&outline, 4, 4, 4), ["....", ".##.", "####", "####"]);
    }

    #[test]
    fn points_on_left_and_top_edges_are_inside_but_right_and_bottom_are_outside() {
        let mut outline = Outline::new(1.0);
        rectangle(&mut outline, 0.5, 0.5, 1.5, 1.5);
        assert_eq!(rows(&outline, 2, 2, 2), ["#.", ".."]);
        assert!(outline.is_off_grid());
    }

    #[test]
    fn ink_outside_the_cell_is_reported_without_losing_inside_pixels() {
        let mut outline = Outline::new(1.0);
        rectangle(&mut outline, -1.0, 0.0, 2.0, 1.0);
        let (canvas, overflow) = outline.rasterize(1, 1, 1);
        assert!(overflow);
        assert_eq!(canvas.get(0, 0), Some(1));
    }
}
