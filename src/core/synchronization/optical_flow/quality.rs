// SPDX-License-Identifier: GPL-3.0-or-later

//! Bidirectional and photometric checks shared by dense optical-flow backends.
//! Coordinates always refer to the active image, excluding decoder row padding.

use super::OpticalFlowPair;

pub fn filter_matches(
    first: &image::GrayImage,
    second: &image::GrayImage,
    size: (u32, u32),
    pairs: OpticalFlowPair,
    backward: impl Fn((f32, f32)) -> Option<(f32, f32)>,
) -> OpticalFlowPair {
    let (a, b) = pairs?;
    let mut source = Vec::new();
    let mut target = Vec::new();
    for (p, q) in a.into_iter().zip(b) {
        let Some(back) = backward(q) else {
            continue;
        };
        let distance = (q.0 - p.0).hypot(q.1 - p.1);
        let cycle_error = (back.0 - p.0).hypot(back.1 - p.1);
        if !cycle_error.is_finite() || cycle_error > 0.75 + 0.03 * distance {
            continue;
        }
        if patch_correlation(first, second, size, p, q).is_some_and(|v| v >= 0.6) {
            source.push(p);
            target.push(q);
        }
    }
    (source.len() >= 10).then_some((source, target))
}

fn patch_correlation(
    a: &image::GrayImage,
    b: &image::GrayImage,
    size: (u32, u32),
    p: (f32, f32),
    q: (f32, f32),
) -> Option<f32> {
    fn sample(image: &image::GrayImage, size: (u32, u32), x: f32, y: f32) -> Option<f32> {
        if !x.is_finite()
            || !y.is_finite()
            || x < 0.0
            || y < 0.0
            || x + 1.0 >= size.0 as f32
            || y + 1.0 >= size.1 as f32
            || size.0 > image.width()
            || size.1 > image.height()
        {
            return None;
        }
        let (ix, iy) = (x as u32, y as u32);
        let (ax, ay) = (x - ix as f32, y - iy as f32);
        let at = |dx, dy| image.get_pixel(ix + dx, iy + dy).0[0] as f32;
        Some(
            (at(0, 0) * (1.0 - ax) + at(1, 0) * ax) * (1.0 - ay)
                + (at(0, 1) * (1.0 - ax) + at(1, 1) * ax) * ay,
        )
    }
    let (mut sa, mut sb, mut aa, mut bb, mut ab) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for y in -2..=2 {
        for x in -2..=2 {
            let va = sample(a, size, p.0 + x as f32, p.1 + y as f32)?;
            let vb = sample(b, size, q.0 + x as f32, q.1 + y as f32)?;
            sa += va;
            sb += vb;
            aa += va * va;
            bb += vb * vb;
            ab += va * vb;
        }
    }
    let va = aa - sa * sa / 25.0;
    let vb = bb - sb * sb / 25.0;
    if va < 25.0 * 3.0 || vb < 25.0 * 3.0 {
        return None;
    }
    Some((ab - sa * sb / 25.0) / (va * vb).sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shifted_tracks_survive_exposure_change_but_not_occlusion_or_bad_cycles() {
        let a = image::GrayImage::from_fn(80, 60, |x, y| {
            image::Luma([((x * 17 + y * 11 + x * y) % 100 + 40) as u8])
        });
        let b = image::GrayImage::from_fn(80, 60, |x, y| {
            image::Luma([if x >= 4 {
                a.get_pixel(x - 4, y).0[0] + 15
            } else {
                0
            }])
        });
        let p: Vec<_> = (0..20)
            .map(|i| (10.0 + (i % 5) as f32 * 10.0, 10.0 + (i / 5) as f32 * 10.0))
            .collect();
        let pairs = Some((p.clone(), p.iter().map(|&(x, y)| (x + 4.0, y)).collect()));
        assert_eq!(
            filter_matches(&a, &b, (80, 60), pairs.clone(), |(x, y)| Some((x - 4.0, y)))
                .unwrap()
                .0
                .len(),
            20
        );
        assert!(filter_matches(&a, &b, (80, 60), pairs.clone(), |(x, y)| Some((x, y))).is_none());
        assert!(
            filter_matches(
                &a,
                &image::GrayImage::new(80, 60),
                (80, 60),
                pairs,
                |(x, y)| Some((x - 4.0, y))
            )
            .is_none()
        );
    }
}
