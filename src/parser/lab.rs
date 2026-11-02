//! `Lab` to sRGB (ISO 32000-1 §8.6.5.4): L*a*b* against the space's white point, adapted to D65
//! by the Bradford transform, then encoded with the sRGB transfer curve.

const BRADFORD: [[f32; 3]; 3] = [
    [0.8951, 0.2664, -0.1614],
    [-0.7502, 1.7135, 0.0367],
    [0.0389, -0.0685, 1.0296],
];
const BRADFORD_INVERSE: [[f32; 3]; 3] = [
    [0.986_992_9, -0.147_054_3, 0.159_962_7],
    [0.432_305_3, 0.518_360_3, 0.049_291_2],
    [-0.008_528_7, 0.040_042_8, 0.968_486_7],
];
const D65: [f32; 3] = [0.950_47, 1.0, 1.088_83];
const XYZ_TO_LINEAR_SRGB: [[f32; 3]; 3] = [
    [3.240_454_2, -1.537_138_5, -0.498_531_4],
    [-0.969_266, 1.876_010_8, 0.041_556],
    [0.055_643_4, -0.204_025_9, 1.057_225_2],
];

fn apply(matrix: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    matrix.map(|row| row[0] * v[0] + row[1] * v[1] + row[2] * v[2])
}

fn clip(v: f32, lo: f32, hi: f32) -> f32 {
    v.clamp(lo.min(hi), lo.max(hi))
}

fn cube(t: f32) -> f32 {
    const KNEE: f32 = 6.0 / 29.0;
    if t >= KNEE {
        t * t * t
    } else {
        108.0 / 841.0 * (t - 4.0 / 29.0)
    }
}

fn encode(linear: f32) -> f32 {
    let linear = if linear > 0.0 { linear.min(1.0) } else { 0.0 };
    if linear <= 0.003_130_8 {
        12.92 * linear
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    }
}

/// sRGB in `0..=1` for the color `lab` (L* in `0..=100`, a* and b* within `range`, as
/// `[amin, amax, bmin, bmax]`) of the space whose white point is `white`.
pub(crate) fn lab_to_rgb_unit(white: [f32; 3], range: [f32; 4], lab: [f32; 3]) -> [f32; 3] {
    let m = (clip(lab[0], 0.0, 100.0) + 16.0) / 116.0;
    let l = m + clip(lab[1], range[0], range[1]) / 500.0;
    let n = m - clip(lab[2], range[2], range[3]) / 200.0;
    let xyz = [white[0] * cube(l), white[1] * cube(m), white[2] * cube(n)];
    let (from, to, cone) = (
        apply(&BRADFORD, white),
        apply(&BRADFORD, D65),
        apply(&BRADFORD, xyz),
    );
    let adapted = apply(
        &BRADFORD_INVERSE,
        [
            cone[0] * to[0] / from[0],
            cone[1] * to[1] / from[1],
            cone[2] * to[2] / from[2],
        ],
    );
    apply(&XYZ_TO_LINEAR_SRGB, adapted).map(encode)
}

#[cfg(test)]
mod tests {
    use super::*;

    const D50: [f32; 3] = [0.9642, 1.0, 0.8249];
    const WIDE: [f32; 4] = [-100.0, 100.0, -100.0, 100.0];

    fn bytes(white: [f32; 3], lab: [f32; 3]) -> [f32; 3] {
        lab_to_rgb_unit(white, WIDE, lab).map(|c| c * 255.0)
    }

    fn assert_near(got: [f32; 3], want: [f32; 3]) {
        assert!(
            got.iter().zip(want).all(|(g, w)| (g - w).abs() < 0.1),
            "{got:?} {want:?}"
        );
    }

    #[test]
    fn a_d50_color_matches_littlecms() {
        for (lab, want) in [
            ([100.0, 0.0, 0.0], [255.0; 3]),
            ([50.0, 0.0, 0.0], [118.91; 3]),
            ([90.0, 0.0, 0.0], [226.33; 3]),
            ([2.0, 0.0, 0.0], [7.29; 3]),
            ([5.0, 0.0, 0.0], [16.84; 3]),
            ([75.0, -50.0, 60.0], [105.38, 207.24, 58.88]),
            ([30.0, 10.0, 10.0], [89.36, 64.73, 55.59]),
            ([54.29, 80.8, 69.89], [254.98, 0.16, 0.0]),
            ([87.82, -79.29, 80.99], [0.0, 255.0, 0.03]),
        ] {
            assert_near(bytes(D50, lab), want);
        }
    }

    #[test]
    fn a_color_is_adapted_from_the_white_point_of_its_space() {
        let d65 = [0.95047, 1.0, 1.08883];
        assert_near(bytes(d65, [100.0, 0.0, 0.0]), [255.0; 3]);
        assert_near(bytes(d65, [75.0, -50.0, 60.0]), [118.82, 205.73, 62.78]);
        assert_near(bytes(d65, [60.0, 40.0, 30.0]), [220.8, 113.71, 94.04]);
    }

    #[test]
    fn components_are_clamped_to_their_ranges() {
        let narrow = [-50.0, 50.0, -20.0, 20.0];
        let clamped = lab_to_rgb_unit(D50, narrow, [75.0, -200.0, 200.0]);
        assert_eq!(clamped, lab_to_rgb_unit(D50, narrow, [75.0, -50.0, 20.0]));
        assert_eq!(
            lab_to_rgb_unit(D50, [50.0, -50.0, 20.0, -20.0], [75.0, 200.0, -200.0]),
            lab_to_rgb_unit(D50, WIDE, [75.0, 50.0, -20.0])
        );
        assert_eq!(
            lab_to_rgb_unit(D50, WIDE, [250.0, 0.0, 0.0]),
            lab_to_rgb_unit(D50, WIDE, [100.0, 0.0, 0.0])
        );
        assert_eq!(
            lab_to_rgb_unit(D50, WIDE, [-5.0, 40.0, 30.0]),
            lab_to_rgb_unit(D50, WIDE, [0.0, 40.0, 30.0])
        );
    }
}
