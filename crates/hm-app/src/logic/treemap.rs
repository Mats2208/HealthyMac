//! Squarified treemap layout (Bruls, Huizing and van Wijk).

/// A rectangle in the layout's own coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn area(&self) -> f32 {
        self.w * self.h
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// Lays out `sizes` inside `bounds` so each rectangle's area is proportional to its size and the
/// rectangles stay close to square. The result is index-aligned with `sizes`. Zero-sized items get
/// an empty rectangle, and they never affect the others.
pub fn squarify(sizes: &[u64], bounds: Rect) -> Vec<Rect> {
    let mut result = vec![Rect::new(bounds.x, bounds.y, 0.0, 0.0); sizes.len()];
    let total: f64 = sizes.iter().map(|&size| size as f64).sum();
    if total <= 0.0 || bounds.w <= 0.0 || bounds.h <= 0.0 {
        return result;
    }

    // Work in f64 because the sizes can span many orders of magnitude.
    let scale = bounds.w as f64 * bounds.h as f64 / total;
    let mut order: Vec<usize> = (0..sizes.len()).filter(|&ix| sizes[ix] > 0).collect();
    order.sort_by(|&a, &b| sizes[b].cmp(&sizes[a]).then(a.cmp(&b)));
    let areas: Vec<f64> = order.iter().map(|&ix| sizes[ix] as f64 * scale).collect();

    let mut free = (
        bounds.x as f64,
        bounds.y as f64,
        bounds.w as f64,
        bounds.h as f64,
    );
    let mut start = 0;
    while start < areas.len() {
        let side = free.2.min(free.3);
        let mut end = start + 1;
        let mut row_area = areas[start];
        let mut worst = worst_ratio(areas[start], areas[start], row_area, side);
        while end < areas.len() {
            let next_area = row_area + areas[end];
            let next_worst = worst_ratio(areas[start], areas[end], next_area, side);
            if next_worst > worst {
                break;
            }
            row_area = next_area;
            worst = next_worst;
            end += 1;
        }

        let is_last_row = end == areas.len();
        if free.2 >= free.3 {
            // A column on the left of the free space.
            let width = if is_last_row {
                free.2
            } else {
                row_area / free.3
            };
            let mut y = free.1;
            for position in start..end {
                let height = if position + 1 == end {
                    free.1 + free.3 - y
                } else {
                    areas[position] / width
                };
                result[order[position]] =
                    Rect::new(free.0 as f32, y as f32, width as f32, height as f32);
                y += height;
            }
            free.0 += width;
            free.2 = (free.2 - width).max(0.0);
        } else {
            // A row along the top of the free space.
            let height = if is_last_row {
                free.3
            } else {
                row_area / free.2
            };
            let mut x = free.0;
            for position in start..end {
                let width = if position + 1 == end {
                    free.0 + free.2 - x
                } else {
                    areas[position] / height
                };
                result[order[position]] =
                    Rect::new(x as f32, free.1 as f32, width as f32, height as f32);
                x += width;
            }
            free.1 += height;
            free.3 = (free.3 - height).max(0.0);
        }
        start = end;
    }
    result
}

/// The worst aspect ratio in a row, given its largest and smallest members.
fn worst_ratio(largest: f64, smallest: f64, row_area: f64, side: f64) -> f64 {
    let side_squared = side * side;
    let row_squared = row_area * row_area;
    (side_squared * largest / row_squared).max(row_squared / (side_squared * smallest))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f32 = 0.05;

    fn assert_valid(sizes: &[u64], bounds: Rect, rects: &[Rect]) {
        assert_eq!(rects.len(), sizes.len());
        let total: f64 = sizes.iter().map(|&s| s as f64).sum();
        let area = bounds.area() as f64;
        for (ix, rect) in rects.iter().enumerate() {
            assert!(rect.w >= 0.0 && rect.h >= 0.0, "{rect:?}");
            if sizes[ix] == 0 {
                assert_eq!(rect.area(), 0.0, "zero-size item got area");
                continue;
            }
            // Stays in bounds.
            assert!(rect.x >= bounds.x - EPSILON, "{rect:?}");
            assert!(rect.y >= bounds.y - EPSILON, "{rect:?}");
            assert!(rect.x + rect.w <= bounds.x + bounds.w + EPSILON, "{rect:?}");
            assert!(rect.y + rect.h <= bounds.y + bounds.h + EPSILON, "{rect:?}");
            // Area proportional to size.
            let expected = sizes[ix] as f64 / total * area;
            let actual = rect.area() as f64;
            assert!(
                (expected - actual).abs() <= (expected * 0.01).max(1.0),
                "item {ix}: expected area {expected}, got {actual}"
            );
        }
        // No overlaps.
        for (a_ix, a) in rects.iter().enumerate() {
            for b in rects.iter().skip(a_ix + 1) {
                if a.area() == 0.0 || b.area() == 0.0 {
                    continue;
                }
                let overlap_w = (a.x + a.w).min(b.x + b.w) - a.x.max(b.x);
                let overlap_h = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
                assert!(
                    overlap_w <= EPSILON || overlap_h <= EPSILON,
                    "overlap between {a:?} and {b:?}"
                );
            }
        }
    }

    #[test]
    fn typical_layout_is_valid_and_fills_the_bounds() {
        let sizes = [600, 600, 400, 300, 300, 200, 100, 40, 10];
        let bounds = Rect::new(10.0, 20.0, 800.0, 500.0);
        let rects = squarify(&sizes, bounds);
        assert_valid(&sizes, bounds, &rects);
        let covered: f32 = rects.iter().map(Rect::area).sum();
        assert!((covered - bounds.area()).abs() < 1.0);
    }

    #[test]
    fn rectangles_stay_reasonably_square() {
        let sizes = [500, 433, 312, 280, 190, 176, 150, 120, 80, 77, 50];
        let rects = squarify(&sizes, Rect::new(0.0, 0.0, 900.0, 600.0));
        for rect in &rects {
            let ratio = (rect.w / rect.h).max(rect.h / rect.w);
            assert!(ratio < 4.0, "{rect:?} has aspect ratio {ratio}");
        }
    }

    #[test]
    fn zero_size_items_are_ignored_but_stay_aligned() {
        let sizes = [0, 100, 0, 300, 0];
        let bounds = Rect::new(0.0, 0.0, 100.0, 100.0);
        let rects = squarify(&sizes, bounds);
        assert_valid(&sizes, bounds, &rects);
        assert_eq!(rects[0].area(), 0.0);
        assert!(rects[3].area() > rects[1].area());
    }

    #[test]
    fn huge_ratios_do_not_break_the_layout() {
        let sizes = [8_000_000_000_000, 1_000, 1, 1, 700_000_000];
        let bounds = Rect::new(0.0, 0.0, 1000.0, 700.0);
        let rects = squarify(&sizes, bounds);
        for rect in &rects {
            assert!(rect.x.is_finite() && rect.y.is_finite());
            assert!(rect.w.is_finite() && rect.h.is_finite());
            assert!(rect.x + rect.w <= 1000.0 + EPSILON);
            assert!(rect.y + rect.h <= 700.0 + EPSILON);
        }
        assert!(
            rects[0].area() > 699_000.0,
            "the giant takes nearly all of it"
        );
    }

    #[test]
    fn near_u64_max_sizes_do_not_overflow() {
        let sizes = [u64::MAX, u64::MAX, u64::MAX / 2];
        let bounds = Rect::new(0.0, 0.0, 100.0, 100.0);
        let rects = squarify(&sizes, bounds);
        assert_valid(&sizes, bounds, &rects);
    }

    #[test]
    fn degenerate_inputs() {
        let bounds = Rect::new(0.0, 0.0, 100.0, 100.0);
        assert!(squarify(&[], bounds).is_empty());
        assert_eq!(squarify(&[0, 0], bounds)[1].area(), 0.0);
        assert_eq!(
            squarify(&[5], Rect::new(0.0, 0.0, 0.0, 10.0))[0].area(),
            0.0
        );
        let single = squarify(&[5], bounds);
        assert_eq!(single[0], bounds);
    }

    #[test]
    fn many_items_in_a_narrow_strip() {
        let sizes: Vec<u64> = (1..=500).rev().collect();
        let bounds = Rect::new(0.0, 0.0, 2000.0, 40.0);
        let rects = squarify(&sizes, bounds);
        assert_valid(&sizes, bounds, &rects);
    }

    #[test]
    fn contains_is_half_open() {
        let rect = Rect::new(10.0, 10.0, 5.0, 5.0);
        assert!(rect.contains(10.0, 10.0));
        assert!(!rect.contains(15.0, 10.0));
    }
}
