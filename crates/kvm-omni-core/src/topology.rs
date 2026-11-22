use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenRect {
    pub id: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub dpi_scale: f32,
}

impl ScreenRect {
    pub fn logical_width(&self) -> f32 {
        self.width as f32 / self.dpi_scale.max(0.25)
    }

    pub fn logical_height(&self) -> f32 {
        self.height as f32 / self.dpi_scale.max(0.25)
    }

    pub fn right(&self) -> i32 {
        self.x + self.width as i32
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.height as i32
    }

    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SwitchTransition {
    pub target_screen: String,
    pub entry_x: i32,
    pub entry_y: i32,
}

#[derive(Debug, Clone)]
pub struct ScreenTopology {
    screens: HashMap<String, ScreenRect>,
    /// Minimum pixel overshoot required across an edge before switching (anti-jitter hysteresis).
    pub hysteresis_px: i32,
    /// Dead-zone margin at corners (pixels) where edge switching is suppressed.
    pub corner_dead_zone_px: i32,
}

impl Default for ScreenTopology {
    fn default() -> Self {
        Self::new(4, 16)
    }
}

impl ScreenTopology {
    pub fn new(hysteresis_px: i32, corner_dead_zone_px: i32) -> Self {
        Self {
            screens: HashMap::new(),
            hysteresis_px,
            corner_dead_zone_px,
        }
    }

    pub fn add_screen(&mut self, rect: ScreenRect) {
        self.screens.insert(rect.id.clone(), rect);
    }

    pub fn screens(&self) -> impl Iterator<Item = &ScreenRect> {
        self.screens.values()
    }

    /// Given a cursor move from `active_id` to global virtual coordinates `(vx, vy)`,
    /// determine whether the cursor crossed into a neighboring screen.
    pub fn evaluate_move(&self, active_id: &str, vx: i32, vy: i32) -> Option<SwitchTransition> {
        let current = self.screens.get(active_id)?;
        if current.contains(vx, vy) {
            return None;
        }

        let crossed_edge = if vx < current.x - self.hysteresis_px {
            Edge::Left
        } else if vx >= current.right() + self.hysteresis_px {
            Edge::Right
        } else if vy < current.y - self.hysteresis_px {
            Edge::Top
        } else if vy >= current.bottom() + self.hysteresis_px {
            Edge::Bottom
        } else {
            return None;
        };

        // Corner dead-zone suppression
        if matches!(crossed_edge, Edge::Left | Edge::Right) {
            if vy < current.y + self.corner_dead_zone_px
                || vy >= current.bottom() - self.corner_dead_zone_px
            {
                return None;
            }
        } else if vx < current.x + self.corner_dead_zone_px
            || vx >= current.right() - self.corner_dead_zone_px
        {
            return None;
        }

        let rel_x = ((vx - current.x) as f32 / current.width as f32).clamp(0.0, 1.0);
        let rel_y = ((vy - current.y) as f32 / current.height as f32).clamp(0.0, 1.0);

        // Find best matching neighbor adjacent to the crossed edge
        let mut best: Option<(&ScreenRect, i32)> = None;
        for candidate in self.screens.values() {
            if candidate.id == current.id {
                continue;
            }
            let distance = match crossed_edge {
                Edge::Right if candidate.x >= current.right() => {
                    let y_overlap = overlap_1d(current.y, current.bottom(), candidate.y, candidate.bottom());
                    if y_overlap > 0 { Some(candidate.x - current.right()) } else { None }
                }
                Edge::Left if candidate.right() <= current.x => {
                    let y_overlap = overlap_1d(current.y, current.bottom(), candidate.y, candidate.bottom());
                    if y_overlap > 0 { Some(current.x - candidate.right()) } else { None }
                }
                Edge::Bottom if candidate.y >= current.bottom() => {
                    let x_overlap = overlap_1d(current.x, current.right(), candidate.x, candidate.right());
                    if x_overlap > 0 { Some(candidate.y - current.bottom()) } else { None }
                }
                Edge::Top if candidate.bottom() <= current.y => {
                    let x_overlap = overlap_1d(current.x, current.right(), candidate.x, candidate.right());
                    if x_overlap > 0 { Some(current.y - candidate.bottom()) } else { None }
                }
                _ => None,
            };

            if let Some(d) = distance {
                if best.map_or(true, |(_, bd)| d < bd) {
                    best = Some((candidate, d));
                }
            }
        }

        let (target, _) = best?;
        let (entry_x, entry_y) = match crossed_edge {
            Edge::Right => (
                target.x + 2,
                target.y + (rel_y * (target.height as f32 - 1.0)) as i32,
            ),
            Edge::Left => (
                target.right() - 2,
                target.y + (rel_y * (target.height as f32 - 1.0)) as i32,
            ),
            Edge::Bottom => (
                target.x + (rel_x * (target.width as f32 - 1.0)) as i32,
                target.y + 2,
            ),
            Edge::Top => (
                target.x + (rel_x * (target.width as f32 - 1.0)) as i32,
                target.bottom() - 2,
            ),
        };

        Some(SwitchTransition {
            target_screen: target.id.clone(),
            entry_x,
            entry_y,
        })
    }
}

fn overlap_1d(a0: i32, a1: i32, b0: i32, b1: i32) -> i32 {
    (a1.min(b1) - a0.max(b0)).max(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(id: &str, x: i32, y: i32, w: u32, h: u32, dpi: f32) -> ScreenRect {
        ScreenRect {
            id: id.to_string(),
            x,
            y,
            width: w,
            height: h,
            dpi_scale: dpi,
        }
    }

    #[test]
    fn ten_complex_topologies() {
        // 1. Dual 4K + 1080p horizontal
        let mut t1 = ScreenTopology::default();
        t1.add_screen(rect("main-4k", 0, 0, 3840, 2160, 2.0));
        t1.add_screen(rect("side-fhd", 3840, 540, 1920, 1080, 1.0));
        let sw = t1.evaluate_move("main-4k", 3850, 1080).unwrap();
        assert_eq!(sw.target_screen, "side-fhd");

        // 2. Reverse left crossing
        let sw2 = t1.evaluate_move("side-fhd", 3830, 1080).unwrap();
        assert_eq!(sw2.target_screen, "main-4k");

        // 3. Corner dead-zone suppression
        assert!(t1.evaluate_move("main-4k", 3850, 2).is_none());

        // 4. Hysteresis suppression
        assert!(t1.evaluate_move("main-4k", 3841, 1080).is_none());

        // 5. Vertical stacked (laptop below ultrawide)
        let mut t2 = ScreenTopology::default();
        t2.add_screen(rect("uw", 0, 0, 3440, 1440, 1.0));
        t2.add_screen(rect("laptop", 760, 1440, 1920, 1080, 1.5));
        assert_eq!(
            t2.evaluate_move("uw", 1720, 1450).unwrap().target_screen,
            "laptop"
        );

        // 6. Top crossing back to ultrawide
        assert_eq!(
            t2.evaluate_move("laptop", 1720, 1430).unwrap().target_screen,
            "uw"
        );

        // 7. Three-monitor L-shape with mobile tablet on the right
        let mut t3 = ScreenTopology::default();
        t3.add_screen(rect("left", -1920, 0, 1920, 1080, 1.0));
        t3.add_screen(rect("center", 0, 0, 2560, 1440, 1.25));
        t3.add_screen(rect("tablet", 2560, 200, 1200, 800, 2.0));
        assert_eq!(
            t3.evaluate_move("center", -10, 500).unwrap().target_screen,
            "left"
        );
        assert_eq!(
            t3.evaluate_move("center", 2570, 600).unwrap().target_screen,
            "tablet"
        );

        // 8. Gap between monitors still resolves nearest neighbor
        let mut t4 = ScreenTopology::default();
        t4.add_screen(rect("a", 0, 0, 1920, 1080, 1.0));
        t4.add_screen(rect("b", 2000, 0, 1920, 1080, 1.0));
        assert_eq!(t4.evaluate_move("a", 1930, 500).unwrap().target_screen, "b");

        // 9. No neighbor in crossed direction returns None
        assert!(t4.evaluate_move("a", -20, 500).is_none());

        // 10. Non-overlapping Y band returns None
        let mut t5 = ScreenTopology::default();
        t5.add_screen(rect("high", 0, 0, 1920, 1080, 1.0));
        t5.add_screen(rect("low", 1920, 2000, 1920, 1080, 1.0));
        assert!(t5.evaluate_move("high", 1930, 500).is_none());
    }
}
