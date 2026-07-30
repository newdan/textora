//! 通用平滑滚动插值器，供 shell 与应用层共用。

const SNAP_THRESHOLD: f32 = 0.5;
const LERP_FACTOR: f32 = 0.35;

pub struct SmoothScroll {
    offset: f32,
    target: f32,
}

impl SmoothScroll {
    pub fn new() -> Self {
        Self { offset: 0.0, target: 0.0 }
    }
    pub fn current(&self) -> f32 {
        self.offset
    }
    pub fn target(&self) -> f32 {
        self.target
    }
    pub fn set_target(&mut self, t: f32) {
        self.target = t;
    }

    /// 每帧调用。返回 true 表示还在动画中。
    pub fn tick(&mut self) -> bool {
        let diff = self.target - self.offset;
        if diff.abs() < SNAP_THRESHOLD {
            self.offset = self.target;
            return false;
        }
        self.offset += diff * LERP_FACTOR;
        true
    }

    pub fn is_animating(&self) -> bool {
        (self.target - self.offset).abs() >= SNAP_THRESHOLD
    }
}

impl Default for SmoothScroll {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::SmoothScroll;

    const MAX_CONVERGENCE_TICKS: usize = 100;

    #[test]
    fn new_scroll_starts_at_rest() {
        let scroll = SmoothScroll::new();

        assert_eq!(scroll.current(), 0.0);
        assert_eq!(scroll.target(), 0.0);
        assert!(!scroll.is_animating());
    }

    #[test]
    fn tick_converges_and_snaps_to_target() {
        let mut scroll = SmoothScroll::new();
        let expected_target = 100.0;
        scroll.set_target(expected_target);

        for _ in 0..MAX_CONVERGENCE_TICKS {
            if !scroll.tick() {
                break;
            }
        }

        assert_eq!(scroll.current(), expected_target);
        assert_eq!(scroll.target(), expected_target);
        assert!(!scroll.is_animating());
    }

    #[test]
    fn default_matches_new_for_complete_scroll_state() {
        let new_scroll = SmoothScroll::new();
        let default_scroll = SmoothScroll::default();

        assert_eq!(default_scroll.current(), new_scroll.current());
        assert_eq!(default_scroll.target(), new_scroll.target());
        assert_eq!(default_scroll.is_animating(), new_scroll.is_animating());
    }
}
