//! 色覚の多様性に対するパレット検査 (`pxsmith palette cvd`，D183・D184)．
//!
//! Machado, Oliveira & Fernandes (2009) の severity 1.0 行列で P 型・D 型・
//! T 型の見え方を写し，通常の見え方では離れている色が近づく組だけを数える．
//! 色距離は新しく定義せず，[`crate::color::delta_e`] に統一する (D12・D110)．

use crate::color::{Rgba8, delta_e};

/// W3C CSS Color 4 が deltaEOK の 1 JND とする値．
pub const DEFAULT_THRESHOLD: f32 = 0.02;

/// シミュレーションする色覚の型．
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Deficiency {
    Protan,
    Deutan,
    Tritan,
}

impl Deficiency {
    pub const ALL: [Self; 3] = [Self::Protan, Self::Deutan, Self::Tritan];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Protan => "protan",
            Self::Deutan => "deutan",
            Self::Tritan => "tritan",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "protan" => Some(Self::Protan),
            "deutan" => Some(Self::Deutan),
            "tritan" => Some(Self::Tritan),
            _ => None,
        }
    }
}

const PROTAN: [[f32; 3]; 3] = [
    [0.152286, 1.052583, -0.204868],
    [0.114503, 0.786281, 0.099216],
    [-0.003882, -0.048116, 1.051998],
];

const DEUTAN: [[f32; 3]; 3] = [
    [0.367322, 0.860646, -0.227968],
    [0.280085, 0.672501, 0.047413],
    [-0.011820, 0.042940, 0.968881],
];

const TRITAN: [[f32; 3]; 3] = [
    [1.255528, -0.076749, -0.178779],
    [-0.078411, 0.930809, 0.147602],
    [0.004733, 0.691367, 0.303900],
];

fn matrix(deficiency: Deficiency) -> [[f32; 3]; 3] {
    match deficiency {
        Deficiency::Protan => PROTAN,
        Deficiency::Deutan => DEUTAN,
        Deficiency::Tritan => TRITAN,
    }
}

fn decode_srgb(encoded: u8) -> f32 {
    let encoded = encoded as f32 / 255.0;
    if encoded <= 0.040_45 {
        encoded / 12.92
    } else {
        ((encoded + 0.055) / 1.055).powf(2.4)
    }
}

fn encode_srgb(linear: f32) -> u8 {
    let encoded = if linear <= 0.003_130_8 {
        12.92 * linear
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    };
    (encoded.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Machado et al. (2009) の severity 1.0 行列で色を写す．
///
/// 論文の掲載ページは行列を掛ける色空間を明記していない．DaltonLens の検証では
/// sRGB の復号を省くと結果が大きく変わるため，ここでは sRGB EOTF で線形 RGB に
/// 戻してから行列を掛け，再び sRGB に符号化する (D183)．アルファは変更しない．
pub fn simulate(c: Rgba8, deficiency: Deficiency) -> Rgba8 {
    let input = [decode_srgb(c.r), decode_srgb(c.g), decode_srgb(c.b)];
    let matrix = matrix(deficiency);
    let mut output = [0.0f32; 3];
    for row in 0..3 {
        output[row] = matrix[row]
            .iter()
            .zip(input)
            .map(|(coefficient, channel)| coefficient * channel)
            .sum::<f32>()
            .clamp(0.0, 1.0);
    }
    Rgba8::new(
        encode_srgb(output[0]),
        encode_srgb(output[1]),
        encode_srgb(output[2]),
        c.a,
    )
}

/// パレット検査の設定．
#[derive(Clone, Debug, PartialEq)]
pub struct CvdOptions {
    pub deficiencies: Vec<Deficiency>,
    pub threshold: f32,
    /// ゲーム上の意味を持ち，混同したら blocking とする色の組．順序は問わない．
    pub blocking_pairs: Vec<(Rgba8, Rgba8)>,
}

impl Default for CvdOptions {
    fn default() -> Self {
        Self {
            deficiencies: vec![Deficiency::Protan, Deficiency::Deutan],
            threshold: DEFAULT_THRESHOLD,
            blocking_pairs: Vec::new(),
        }
    }
}

/// ある型で写した後の色距離．
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SimulatedDistance {
    pub deficiency: Deficiency,
    pub distance: f32,
}

/// 1 組の色と，通常時・シミュレーション後の距離．
#[derive(Clone, Debug, PartialEq)]
pub struct CvdPair {
    pub a_index: usize,
    pub b_index: usize,
    pub a: Rgba8,
    pub b: Rgba8,
    pub normal_distance: f32,
    pub simulated: Vec<SimulatedDistance>,
    pub blocking: bool,
}

impl CvdPair {
    pub fn distance(&self, deficiency: Deficiency) -> Option<f32> {
        self.simulated
            .iter()
            .find(|d| d.deficiency == deficiency)
            .map(|d| d.distance)
    }
}

/// 1 つの型で見分けられなくなる組．
#[derive(Clone, Debug, PartialEq)]
pub struct DeficiencyReport {
    pub deficiency: Deficiency,
    /// シミュレーション後の距離の昇順．同値なら添字順．
    pub confusions: Vec<CvdPair>,
}

/// パレット全体の検査結果．
#[derive(Clone, Debug, PartialEq)]
pub struct CvdReport {
    pub palette_len: usize,
    pub opaque: usize,
    pub threshold: f32,
    /// 通常の見え方でも閾値未満だった組．色覚による指摘には含めない．
    pub originally_indistinguishable: Vec<CvdPair>,
    pub by_deficiency: Vec<DeficiencyReport>,
    /// blocking 指定に現れたが，パレットに無かった色．
    pub missing_blocking_colours: Vec<Rgba8>,
}

impl CvdReport {
    pub fn has_blocking(&self) -> bool {
        self.by_deficiency
            .iter()
            .flat_map(|report| &report.confusions)
            .any(|pair| pair.blocking)
    }
}

fn is_blocking_pair(a: Rgba8, b: Rgba8, pairs: &[(Rgba8, Rgba8)]) -> bool {
    fn same_rgb(a: Rgba8, b: Rgba8) -> bool {
        (a.r, a.g, a.b) == (b.r, b.g, b.b)
    }
    pairs
        .iter()
        .any(|&(x, y)| (same_rgb(a, x) && same_rgb(b, y)) || (same_rgb(a, y) && same_rgb(b, x)))
}

fn comparison(
    palette: &[Rgba8],
    simulations: &[Vec<Rgba8>],
    options: &CvdOptions,
    a_index: usize,
    b_index: usize,
    normal_distance: f32,
) -> CvdPair {
    let simulated = options
        .deficiencies
        .iter()
        .enumerate()
        .map(|(type_index, &deficiency)| SimulatedDistance {
            deficiency,
            distance: delta_e(
                simulations[type_index][a_index].to_oklab(),
                simulations[type_index][b_index].to_oklab(),
            ),
        })
        .collect();
    CvdPair {
        a_index,
        b_index,
        a: palette[a_index],
        b: palette[b_index],
        normal_distance,
        simulated,
        blocking: is_blocking_pair(palette[a_index], palette[b_index], &options.blocking_pairs),
    }
}

/// 不透明な色の全組を検査する．
///
/// 通常の見え方ですでに `threshold` 未満の組は別に数え，色覚による指摘から外す．
pub fn check(palette: &[Rgba8], options: &CvdOptions) -> CvdReport {
    let opaque_indices: Vec<usize> = palette
        .iter()
        .enumerate()
        .filter_map(|(index, colour)| (colour.a != 0).then_some(index))
        .collect();
    let simulations: Vec<Vec<Rgba8>> = options
        .deficiencies
        .iter()
        .map(|&deficiency| {
            palette
                .iter()
                .map(|&colour| simulate(colour, deficiency))
                .collect()
        })
        .collect();
    let mut originally_indistinguishable = Vec::new();
    let mut by_deficiency: Vec<DeficiencyReport> = options
        .deficiencies
        .iter()
        .map(|&deficiency| DeficiencyReport {
            deficiency,
            confusions: Vec::new(),
        })
        .collect();

    for (position, &a_index) in opaque_indices.iter().enumerate() {
        for &b_index in opaque_indices.iter().skip(position + 1) {
            let normal_distance = delta_e(palette[a_index].to_oklab(), palette[b_index].to_oklab());
            let pair = comparison(
                palette,
                &simulations,
                options,
                a_index,
                b_index,
                normal_distance,
            );
            if normal_distance < options.threshold {
                originally_indistinguishable.push(pair);
                continue;
            }
            for report in &mut by_deficiency {
                if pair
                    .distance(report.deficiency)
                    .is_some_and(|distance| distance < options.threshold)
                {
                    report.confusions.push(pair.clone());
                }
            }
        }
    }

    originally_indistinguishable.sort_by(|a, b| {
        a.normal_distance
            .total_cmp(&b.normal_distance)
            .then(a.a_index.cmp(&b.a_index))
            .then(a.b_index.cmp(&b.b_index))
    });
    for report in &mut by_deficiency {
        report.confusions.sort_by(|a, b| {
            a.distance(report.deficiency)
                .unwrap_or(f32::INFINITY)
                .total_cmp(&b.distance(report.deficiency).unwrap_or(f32::INFINITY))
                .then(a.a_index.cmp(&b.a_index))
                .then(a.b_index.cmp(&b.b_index))
        });
    }

    let mut missing_blocking_colours = Vec::new();
    for colour in options.blocking_pairs.iter().flat_map(|&(a, b)| [a, b]) {
        let present = palette
            .iter()
            .any(|entry| (entry.r, entry.g, entry.b) == (colour.r, colour.g, colour.b));
        if !present && !missing_blocking_colours.contains(&colour) {
            missing_blocking_colours.push(colour);
        }
    }

    CvdReport {
        palette_len: palette.len(),
        opaque: opaque_indices.len(),
        threshold: options.threshold,
        originally_indistinguishable,
        by_deficiency,
        missing_blocking_colours,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{Rgba8, delta_e};

    const EXPECTED_MATRICES: [[[f32; 3]; 3]; 3] = [
        [
            [0.152286, 1.052583, -0.204868],
            [0.114503, 0.786281, 0.099216],
            [-0.003882, -0.048116, 1.051998],
        ],
        [
            [0.367322, 0.860646, -0.227968],
            [0.280085, 0.672501, 0.047413],
            [-0.011820, 0.042940, 0.968881],
        ],
        [
            [1.255528, -0.076749, -0.178779],
            [-0.078411, 0.930809, 0.147602],
            [0.004733, 0.691367, 0.303900],
        ],
    ];

    fn encode(linear: f32) -> u8 {
        let encoded = if linear <= 0.003_130_8 {
            12.92 * linear
        } else {
            1.055 * linear.powf(1.0 / 2.4) - 0.055
        };
        (encoded.clamp(0.0, 1.0) * 255.0).round() as u8
    }

    fn without_srgb_decoding(c: Rgba8, deficiency: Deficiency) -> Rgba8 {
        let input = [c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0];
        let matrix = matrix(deficiency);
        let mut output = [0.0; 3];
        for row in 0..3 {
            output[row] = matrix[row]
                .iter()
                .zip(input)
                .map(|(coefficient, channel)| coefficient * channel)
                .sum::<f32>()
                .clamp(0.0, 1.0);
        }
        Rgba8::new(
            (output[0] * 255.0).round() as u8,
            (output[1] * 255.0).round() as u8,
            (output[2] * 255.0).round() as u8,
            c.a,
        )
    }

    #[test]
    fn matrices_match_machado_severity_one_table() {
        for (deficiency, expected) in Deficiency::ALL.into_iter().zip(EXPECTED_MATRICES) {
            assert_eq!(matrix(deficiency), expected);
        }
    }

    #[test]
    fn matrix_rows_preserve_grays() {
        for deficiency in Deficiency::ALL {
            for row in matrix(deficiency) {
                assert!((row.iter().sum::<f32>() - 1.0).abs() < 1e-5);
            }
            for value in [0u8, 128, 255] {
                let simulated = simulate(Rgba8::rgb(value, value, value), deficiency);
                for channel in [simulated.r, simulated.g, simulated.b] {
                    assert!(channel.abs_diff(value) <= 1);
                }
            }
        }
    }

    #[test]
    fn simulation_applies_the_matrix_in_linear_rgb() {
        let colour = Rgba8::rgb(0xb1, 0x3e, 0x53);
        let linear = simulate(colour, Deficiency::Deutan);
        let encoded = without_srgb_decoding(colour, Deficiency::Deutan);
        assert_ne!(linear, encoded);
        assert!(
            linear.r.abs_diff(encoded.r) > 10
                || linear.g.abs_diff(encoded.g) > 10
                || linear.b.abs_diff(encoded.b) > 10
        );
    }

    #[test]
    fn a_protan_confusion_line_remains_separate_for_tritan() {
        let m = matrix(Deficiency::Protan);
        // A rank-two matrix's null direction is the cross product of two independent rows.
        // Moving two linear-RGB colours along that direction therefore leaves their Protan
        // projections nearly unchanged; this derives the pair instead of baking in outputs.
        let direction = [
            m[0][1] * m[1][2] - m[0][2] * m[1][1],
            m[0][2] * m[1][0] - m[0][0] * m[1][2],
            m[0][0] * m[1][1] - m[0][1] * m[1][0],
        ];
        let scale = 0.28;
        let a = Rgba8::rgb(
            encode(0.5 - scale * direction[0]),
            encode(0.5 - scale * direction[1]),
            encode(0.5 - scale * direction[2]),
        );
        let b = Rgba8::rgb(
            encode(0.5 + scale * direction[0]),
            encode(0.5 + scale * direction[1]),
            encode(0.5 + scale * direction[2]),
        );

        let protan = delta_e(
            simulate(a, Deficiency::Protan).to_oklab(),
            simulate(b, Deficiency::Protan).to_oklab(),
        );
        let tritan = delta_e(
            simulate(a, Deficiency::Tritan).to_oklab(),
            simulate(b, Deficiency::Tritan).to_oklab(),
        );
        assert!(protan < 0.02, "Protan distance was {protan}");
        assert!(tritan >= 0.02, "Tritan distance was {tritan}");
    }

    #[test]
    fn an_already_indistinguishable_pair_is_not_a_cvd_confusion() {
        let palette = [Rgba8::rgb(128, 128, 128), Rgba8::rgb(129, 128, 128)];
        let report = check(&palette, &CvdOptions::default());
        assert_eq!(report.originally_indistinguishable.len(), 1);
        assert!(report.by_deficiency.iter().all(|r| r.confusions.is_empty()));
    }
}
