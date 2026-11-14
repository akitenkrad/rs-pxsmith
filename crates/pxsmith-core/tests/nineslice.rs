use pxsmith_core::IndexedCanvas;
use pxsmith_core::nineslice::{
    AxisStretch, NineSlice, NineSliceAxis, render_nine_slice, visible_seams,
};

fn source() -> IndexedCanvas {
    IndexedCanvas::from_pixels(4, 4, (0..16).collect())
        .expect("4x4 の画素")
        .with_transparent(Some(5))
}

fn spec(stretch: AxisStretch) -> NineSlice {
    NineSlice {
        left: 1,
        top: 1,
        right: 1,
        bottom: 1,
        stretch_x: stretch,
        stretch_y: stretch,
    }
}

fn assert_mapping(stretch: AxisStretch, expected_axis: &[u32; 7]) {
    let output = render_nine_slice(&source(), &spec(stretch), 7, 7).expect("9-slice できる");
    let expected: Vec<u8> = expected_axis
        .iter()
        .flat_map(|&y| expected_axis.iter().map(move |&x| (y * 4 + x) as u8))
        .collect();
    assert_eq!(output.pixels(), expected);
    assert_eq!(output.transparent(), Some(5), "透明添字を引き継ぐ");
}

#[test]
fn stretch_matches_the_hand_calculated_texels() {
    assert_mapping(AxisStretch::Stretch, &[0, 1, 1, 2, 2, 2, 3]);
}

#[test]
fn tile_matches_the_hand_calculated_texels() {
    assert_mapping(AxisStretch::Tile, &[0, 1, 2, 1, 2, 1, 3]);
}

#[test]
fn tile_fit_uses_floor_of_the_repeat_ratio_plus_one_half() {
    // 描画中央 5 px / 元中央 2 px = 2.5 なので，repeat 数は floor(2.5 + 0.5) = 3．
    assert_mapping(AxisStretch::TileFit, &[0, 1, 2, 2, 1, 2, 3]);
}

#[test]
fn margins_leave_a_source_centre_and_fit_inside_the_target() {
    assert!(spec(AxisStretch::Stretch).validate_source(3, 3).is_ok());
    assert!(spec(AxisStretch::Stretch).validate_source(2, 3).is_err());
    assert!(spec(AxisStretch::Stretch).validate_source(3, 2).is_err());
    assert!(spec(AxisStretch::Stretch).validate_target(2, 2).is_ok());
    assert!(spec(AxisStretch::Stretch).validate_target(1, 2).is_err());
    assert!(spec(AxisStretch::Stretch).validate_target(2, 1).is_err());
}

#[test]
fn seam_advice_is_per_axis_and_only_for_repeating_modes() {
    assert_eq!(
        visible_seams(&source(), &spec(AxisStretch::Tile)).expect("検査できる"),
        vec![NineSliceAxis::Horizontal, NineSliceAxis::Vertical]
    );
    assert!(
        visible_seams(&source(), &spec(AxisStretch::Stretch))
            .expect("検査できる")
            .is_empty()
    );

    let seamless = IndexedCanvas::filled(4, 4, 7);
    assert!(
        visible_seams(&seamless, &spec(AxisStretch::TileFit))
            .expect("検査できる")
            .is_empty()
    );
}
