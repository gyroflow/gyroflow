// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

// Ported from OpenCV: https://github.com/opencv/opencv/blob/4.x/modules/calib3d/src/calibinit.cpp#L2078
pub fn draw_chessboard_corners(org_width: usize, w: u32, _h: u32, s: usize, pixels: &mut [u8], pattern_size: (usize, usize), corners: &[(f32, f32)], found: bool) {
    const LINE_COLORS: &[(u8, u8, u8)] = &[
        (0, 0, 255),
        (0, 128, 255),
        (0, 200, 200),
        (0, 255, 0),
        (200, 200, 0),
        (255, 0, 0),
        (255, 0, 255)
    ];
    let ratio = w as f32 / org_width as f32;
    let r = 14.0 * ratio;
    if !found {
        let color = (0, 0, 255);
        for x in corners {
            let pt = ((x.0 * ratio).round(), (x.1 * ratio).round());
            line(s, pixels, (pt.0 - r, pt.1 - r), (pt.0 + r, pt.1 + r), color);
            line(s, pixels, (pt.0 - r, pt.1 + r), (pt.0 + r, pt.1 - r), color);
            circle(s, pixels, pt, r + 1.0, color);
        }
    } else {
        let mut prev_pt = (0.0, 0.0);
        let mut i = 0;
        for y in 0..pattern_size.1 {
            let color = LINE_COLORS[y % LINE_COLORS.len()];
            for _x in 0..pattern_size.0 {
                let pt = corners[i];
                let pt = ((pt.0 * ratio).round(), (pt.1 * ratio).round());
                if i != 0 {
                    line(s, pixels, prev_pt, pt, color);
                }
                line(s, pixels, (pt.0 - r, pt.1 - r), (pt.0 + r, pt.1 + r), color);
                line(s, pixels, (pt.0 - r, pt.1 + r), (pt.0 + r, pt.1 - r), color);
                circle(s, pixels, pt, r + 1.0, color);
                prev_pt = pt;
                i += 1;
            }
        }
    }
}

fn line(s: usize, pixels: &mut [u8], p1: (f32, f32), p2: (f32, f32), color: (u8, u8, u8)) {
    let points = line_drawing::Bresenham::new((p1.0 as isize, p1.1 as isize), (p2.0 as isize, p2.1 as isize));
    draw_pixels(s, pixels, color, points);
}
fn circle(s: usize, pixels: &mut [u8], center: (f32, f32), radius: f32, color: (u8, u8, u8)) {
    let points = line_drawing::BresenhamCircle::new(center.0 as isize, center.1 as isize, radius as isize); 
    draw_pixels(s, pixels, color, points);
}
fn draw_pixels(s: usize, pixels: &mut [u8], color: (u8, u8, u8), points: impl Iterator<Item = line_drawing::Point<isize>>) {
    for point in points {
        let pos = (point.1 * s as isize + point.0 * 4) as usize;
        if pixels.len() > pos + 2 { 
            pixels[pos + 0] = color.0; // R
            pixels[pos + 1] = color.1; // G
            pixels[pos + 2] = color.2; // B
        }
    }
}
