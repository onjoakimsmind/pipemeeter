use dioxus_desktop::tao::window::Icon;

pub(super) fn app_icon() -> Option<Icon> {
    let size = 128_u32;
    let mut rgba = vec![0_u8; (size * size * 4) as usize];
    let scale = size as f32 / 512.0;
    let inset = 64.0 * scale;

    fill_rounded_rect(
        &mut rgba,
        size,
        0.0,
        0.0,
        size as f32,
        size as f32,
        64.0 * scale,
        [15, 23, 42, 255],
    );

    for (y, color) in [
        (96.0, [51, 65, 85, 255]),
        (160.0, [56, 189, 248, 255]),
        (224.0, [74, 222, 128, 255]),
        (288.0, [51, 65, 85, 255]),
    ] {
        fill_horizontal_capsule(
            &mut rgba,
            size,
            inset + (64.0 * scale),
            inset + (320.0 * scale),
            inset + (y * scale),
            24.0 * scale,
            color,
        );
    }

    fill_convex_quad(
        &mut rgba,
        size,
        [
            (inset + (220.0 * scale), inset + (48.0 * scale)),
            (inset + (320.0 * scale), inset + (48.0 * scale)),
            (inset + (240.0 * scale), inset + (336.0 * scale)),
            (inset + (140.0 * scale), inset + (336.0 * scale)),
        ],
        [248, 250, 252, 255],
    );

    fill_circle(
        &mut rgba,
        size,
        inset + (205.0 * scale),
        inset + (160.0 * scale),
        16.0 * scale,
        [14, 165, 233, 255],
    );
    fill_circle(
        &mut rgba,
        size,
        inset + (190.0 * scale),
        inset + (224.0 * scale),
        16.0 * scale,
        [34, 197, 94, 255],
    );

    Icon::from_rgba(rgba, size, size).ok()
}

fn fill_rounded_rect(
    rgba: &mut [u8],
    size: u32,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
    color: [u8; 4],
) {
    paint_shape(rgba, size, color, |px, py| {
        let left = x;
        let right = x + width;
        let top = y;
        let bottom = y + height;

        if px < left || px > right || py < top || py > bottom {
            return false;
        }

        let inner_left = left + radius;
        let inner_right = right - radius;
        let inner_top = top + radius;
        let inner_bottom = bottom - radius;

        (px >= inner_left && px <= inner_right)
            || (py >= inner_top && py <= inner_bottom)
            || circle_contains(px, py, inner_left, inner_top, radius)
            || circle_contains(px, py, inner_right, inner_top, radius)
            || circle_contains(px, py, inner_left, inner_bottom, radius)
            || circle_contains(px, py, inner_right, inner_bottom, radius)
    });
}

fn fill_horizontal_capsule(
    rgba: &mut [u8],
    size: u32,
    x1: f32,
    x2: f32,
    y: f32,
    thickness: f32,
    color: [u8; 4],
) {
    let radius = thickness / 2.0;
    paint_shape(rgba, size, color, |px, py| {
        (px >= x1 && px <= x2 && py >= y - radius && py <= y + radius)
            || circle_contains(px, py, x1, y, radius)
            || circle_contains(px, py, x2, y, radius)
    });
}

fn fill_circle(rgba: &mut [u8], size: u32, cx: f32, cy: f32, radius: f32, color: [u8; 4]) {
    paint_shape(rgba, size, color, |px, py| {
        circle_contains(px, py, cx, cy, radius)
    });
}

fn fill_convex_quad(rgba: &mut [u8], size: u32, points: [(f32, f32); 4], color: [u8; 4]) {
    paint_shape(rgba, size, color, |px, py| {
        point_in_convex_polygon(px, py, &points)
    });
}

fn paint_shape<F>(rgba: &mut [u8], size: u32, color: [u8; 4], contains: F)
where
    F: Fn(f32, f32) -> bool,
{
    for y in 0..size {
        for x in 0..size {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            if contains(px, py) {
                let index = ((y * size + x) * 4) as usize;
                rgba[index..index + 4].copy_from_slice(&color);
            }
        }
    }
}

fn circle_contains(px: f32, py: f32, cx: f32, cy: f32, radius: f32) -> bool {
    let dx = px - cx;
    let dy = py - cy;
    (dx * dx) + (dy * dy) <= radius * radius
}

fn point_in_convex_polygon(px: f32, py: f32, points: &[(f32, f32)]) -> bool {
    let mut previous_sign = 0.0_f32;

    for index in 0..points.len() {
        let (x1, y1) = points[index];
        let (x2, y2) = points[(index + 1) % points.len()];
        let cross = (x2 - x1) * (py - y1) - (y2 - y1) * (px - x1);
        if cross.abs() < f32::EPSILON {
            continue;
        }
        if previous_sign == 0.0 {
            previous_sign = cross.signum();
            continue;
        }
        if cross.signum() != previous_sign {
            return false;
        }
    }

    true
}
