//! Seamless screen-edge KVM state machine (Deskflow / Barrier / Mouse without Borders style).
//!
//! Tracks whether input belongs to the **local** PC or the **remote** PC:
//! - In local mode, cursor movement passes through until the pointer crosses the configured
//!   [`ScreenEdge`] (outside a 16 px corner dead-zone), switching to remote mode.
//! - In remote mode, local pointer/button/keyboard events are swallowed so local windows
//!   are untouched while relative deltas drive the remote PC. Moving the virtual remote
//!   cursor back across the opposite edge smoothly returns control to the local PC.

use serde::{Deserialize, Serialize};

const DEAD_CORNER_PX: i32 = 16;
const ENTRY_MARGIN_PX: i32 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ScreenEdge {
    #[default]
    Right,
    Left,
    Top,
    Bottom,
    /// Immediately capture and forward without waiting for a screen edge.
    Immediate,
}

impl ScreenEdge {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "left" | "gauche" => Self::Left,
            "top" | "haut" => Self::Top,
            "bottom" | "bas" => Self::Bottom,
            "immediate" | "direct" => Self::Immediate,
            _ => Self::Right,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveDecision {
    /// Keep event local (`CallNextHookEx`).
    PassLocal,
    /// Transition Local -> Remote; park local cursor at `(park_x, park_y)` and swallow.
    EnterRemote { park_x: i32, park_y: i32 },
    /// Swallow local movement and forward `(dx, dy)` to the remote peer.
    SendRemoteDelta { dx: i32, dy: i32 },
    /// Swallow zero-delta tick while remote is active.
    SwallowNoop,
    /// Transition Remote -> Local; place local cursor at `(local_x, local_y)`.
    ReturnLocal { local_x: i32, local_y: i32 },
}

#[derive(Debug, Clone)]
pub struct EdgeSwitchState {
    pub edge: ScreenEdge,
    pub width: i32,
    pub height: i32,
    pub remote_active: bool,
    pub cursor_locked: bool,
    pub rx: i32,
    pub ry: i32,
}

impl EdgeSwitchState {
    pub fn new(edge: ScreenEdge, width: i32, height: i32) -> Self {
        let w = width.max(320);
        let h = height.max(240);
        let immediate = edge == ScreenEdge::Immediate;
        Self {
            edge,
            width: w,
            height: h,
            remote_active: immediate,
            cursor_locked: false,
            rx: w / 2,
            ry: h / 2,
        }
    }

    fn in_corner(&self, x: i32, y: i32) -> bool {
        let near_x = x < DEAD_CORNER_PX || x >= self.width - DEAD_CORNER_PX;
        let near_y = y < DEAD_CORNER_PX || y >= self.height - DEAD_CORNER_PX;
        near_x && near_y
    }

    /// Evaluates a proposed cursor move to `(pt_x, pt_y)` given current OS cursor `(cur_x, cur_y)`.
    pub fn on_mouse_move(&mut self, pt_x: i32, pt_y: i32, cur_x: i32, cur_y: i32) -> MoveDecision {
        if !self.remote_active {
            if self.cursor_locked || self.in_corner(pt_x, pt_y) {
                return MoveDecision::PassLocal;
            }
            let entered = match self.edge {
                ScreenEdge::Right if pt_x >= self.width - 2 => {
                    self.rx = ENTRY_MARGIN_PX;
                    self.ry = pt_y.clamp(DEAD_CORNER_PX, self.height - DEAD_CORNER_PX);
                    true
                }
                ScreenEdge::Left if pt_x <= 1 => {
                    self.rx = self.width - ENTRY_MARGIN_PX;
                    self.ry = pt_y.clamp(DEAD_CORNER_PX, self.height - DEAD_CORNER_PX);
                    true
                }
                ScreenEdge::Top if pt_y <= 1 => {
                    self.rx = pt_x.clamp(DEAD_CORNER_PX, self.width - DEAD_CORNER_PX);
                    self.ry = self.height - ENTRY_MARGIN_PX;
                    true
                }
                ScreenEdge::Bottom if pt_y >= self.height - 2 => {
                    self.rx = pt_x.clamp(DEAD_CORNER_PX, self.width - DEAD_CORNER_PX);
                    self.ry = ENTRY_MARGIN_PX;
                    true
                }
                _ => false,
            };
            if entered {
                self.remote_active = true;
                return MoveDecision::EnterRemote {
                    park_x: self.width / 2,
                    park_y: self.height / 2,
                };
            }
            return MoveDecision::PassLocal;
        }

        let dx = pt_x - cur_x;
        let dy = pt_y - cur_y;
        if dx == 0 && dy == 0 {
            return MoveDecision::SwallowNoop;
        }
        self.rx += dx;
        self.ry += dy;

        if !self.cursor_locked {
            match self.edge {
                ScreenEdge::Right if self.rx <= 0 => {
                    self.remote_active = false;
                    return MoveDecision::ReturnLocal {
                        local_x: self.width - ENTRY_MARGIN_PX,
                        local_y: self.ry.clamp(DEAD_CORNER_PX, self.height - DEAD_CORNER_PX),
                    };
                }
                ScreenEdge::Left if self.rx >= self.width - 1 => {
                    self.remote_active = false;
                    return MoveDecision::ReturnLocal {
                        local_x: ENTRY_MARGIN_PX,
                        local_y: self.ry.clamp(DEAD_CORNER_PX, self.height - DEAD_CORNER_PX),
                    };
                }
                ScreenEdge::Top if self.ry >= self.height - 1 => {
                    self.remote_active = false;
                    return MoveDecision::ReturnLocal {
                        local_x: self.rx.clamp(DEAD_CORNER_PX, self.width - DEAD_CORNER_PX),
                        local_y: ENTRY_MARGIN_PX,
                    };
                }
                ScreenEdge::Bottom if self.ry <= 0 => {
                    self.remote_active = false;
                    return MoveDecision::ReturnLocal {
                        local_x: self.rx.clamp(DEAD_CORNER_PX, self.width - DEAD_CORNER_PX),
                        local_y: self.height - ENTRY_MARGIN_PX,
                    };
                }
                _ => {}
            }
        }

        self.rx = self.rx.clamp(0, self.width - 1);
        self.ry = self.ry.clamp(0, self.height - 1);
        MoveDecision::SendRemoteDelta { dx, dy }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn right_edge_switches_to_remote_and_returns_cleanly() {
        let mut s = EdgeSwitchState::new(ScreenEdge::Right, 1920, 1080);
        assert!(!s.remote_active);

        // Normal local movement
        assert_eq!(s.on_mouse_move(500, 500, 490, 500), MoveDecision::PassLocal);
        // Top-right corner dead-zone does not trigger accidental switch
        assert_eq!(s.on_mouse_move(1919, 4, 1910, 4), MoveDecision::PassLocal);

        // Crossing right edge enters remote mode and parks local cursor at (960, 540)
        assert_eq!(
            s.on_mouse_move(1919, 500, 1910, 500),
            MoveDecision::EnterRemote { park_x: 960, park_y: 540 }
        );
        assert!(s.remote_active);

        // Moving right on remote forwards delta while local cursor stays parked at (960, 540)
        assert_eq!(
            s.on_mouse_move(985, 535, 960, 540),
            MoveDecision::SendRemoteDelta { dx: 25, dy: -5 }
        );

        // Moving back past left edge of remote screen returns to local right edge
        assert_eq!(
            s.on_mouse_move(910, 540, 960, 540),
            MoveDecision::ReturnLocal { local_x: 1910, local_y: 495 }
        );
        assert!(!s.remote_active);
    }

    #[test]
    fn cursor_lock_keeps_focus_on_current_side() {
        let mut s = EdgeSwitchState::new(ScreenEdge::Left, 1920, 1080);
        s.cursor_locked = true;
        assert_eq!(s.on_mouse_move(0, 500, 10, 500), MoveDecision::PassLocal);
        s.cursor_locked = false;
        assert!(matches!(s.on_mouse_move(0, 500, 10, 500), MoveDecision::EnterRemote { .. }));
        s.cursor_locked = true;
        // Locked on remote: hitting return edge clamps instead of exiting
        assert!(matches!(
            s.on_mouse_move(1000, 540, 960, 540),
            MoveDecision::SendRemoteDelta { dx: 40, dy: 0 }
        ));
        assert!(s.remote_active);
    }
}
