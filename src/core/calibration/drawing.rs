// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use crate::gpu::drawing::*;

// Ported from OpenCV: https://github.com/opencv/opencv/blob/4.x/modules/calib3d/src/calibinit.cpp#L2078
pub fn draw_chessboard_corners(org_width: usize, org_height: usize, w: usize, h: usize, drawing: &mut DrawCanvas, pattern_size: (usize, usize), corners: &[(f32, f32)], found: bool, inverted: bool) {
    const LINE_COLORS: &[Color] = &[
        Color::Red,     // #ff0000
        Color::Blue2,   // #0080ff
        Color::Yellow2, // #C8C800
        Color::Green,   // #00ff00
        Color::Blue3,   // #00C8C8
        Color::Blue,    // #0000ff
        Color::Magenta  // #ff00ff
    ];

    let ratio_w = w as f32 / org_width as f32;
    let ratio_h = h as f32 / org_height as f32;
    let r = 10.0 * ratio_w;
    if !found {
        let color = Color::Red;
        for x in corners {
            let mut pt = ((x.0 * ratio_w).round(), (x.1 * ratio_h).round());
            if inverted {
                pt.1 = h as f32 - pt.1;
            }
            line(drawing, (pt.0 - r, pt.1 - r), (pt.0 + r, pt.1 + r), color);
            line(drawing, (pt.0 - r, pt.1 + r), (pt.0 + r, pt.1 - r), color);
            circle(drawing, pt, r + 1.0, color);
        }
    } else {
        let mut prev_pt = (0.0, 0.0);
        let mut i = 0;
        for y in 0..pattern_size.1 {
            let color = LINE_COLORS[y % LINE_COLORS.len()];
            for _x in 0..pattern_size.0 {
                let pt = corners[i];
                let mut pt = ((pt.0 * ratio_w).round(), (pt.1 * ratio_h).round());
                if inverted {
                    pt.1 = h as f32 - pt.1;
                }
                if i != 0 {
                    line(drawing, prev_pt, pt, color);
                }
                line(drawing, (pt.0 - r, pt.1 - r), (pt.0 + r, pt.1 + r), color);
                line(drawing, (pt.0 - r, pt.1 + r), (pt.0 + r, pt.1 - r), color);
                circle(drawing, pt, r + 1.0, color);
                prev_pt = pt;
                i += 1;
            }
        }
    }
}

fn line(drawing: &mut DrawCanvas, p1: (f32, f32), p2: (f32, f32), color: Color) {
    let points = line_drawing::Bresenham::new((p1.0 as isize, p1.1 as isize), (p2.0 as isize, p2.1 as isize));
    draw_pixels(drawing, color, points);
}
fn circle(drawing: &mut DrawCanvas, center: (f32, f32), radius: f32, color: Color) {
    let points = line_drawing::BresenhamCircle::new(center.0 as isize, center.1 as isize, radius as isize);
    draw_pixels(drawing, color, points);
}
fn draw_pixels(drawing: &mut DrawCanvas, color: Color, points: impl Iterator<Item = line_drawing::Point<isize>>) {
    for point in points {
        drawing.put_pixel(point.0 as i32, point.1 as i32, color, Alpha::Alpha100, Stage::OnInput, false, 1);
    }
}
