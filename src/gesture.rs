use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub enum Phase {
    Started,
    Moved,
    Ended,
    Wheel,
}
#[derive(Default)]
pub struct Gesture {
    distance: f32,
    latched: bool,
    phased: bool,
    last: Option<Instant>,
}
impl Gesture {
    pub fn update(&mut self, x: f32, y: f32, phase: Phase, now: Instant) -> (bool, Option<bool>) {
        if matches!(phase, Phase::Wheel) {
            self.phased = false;
        }
        if matches!(phase, Phase::Started) {
            self.phased = true;
        }
        if matches!(phase, Phase::Started)
            || (!self.phased
                && self.last.is_none_or(|last| {
                    now.saturating_duration_since(last) > Duration::from_millis(220)
                }))
        {
            self.distance = 0.;
            self.latched = false;
        }
        self.last = Some(now);
        let horizontal = x.abs() > 1. && x.abs() > y.abs() * 1.75;
        let mut direction = None;
        if horizontal && !self.latched {
            if self.distance.signum() != x.signum() {
                self.distance = 0.;
            }
            self.distance += x;
            if self.distance.abs() >= 60. {
                self.latched = true;
                direction = Some(self.distance > 0.);
            }
        } else if !horizontal {
            self.distance = 0.;
        }
        if matches!(phase, Phase::Ended) {
            self.distance = 0.;
        }
        (horizontal, direction)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn one_step_survives_momentum_and_unphased_bursts_reset_only_after_idle() {
        let start = Instant::now();
        let mut phased = Gesture::default();
        assert_eq!(phased.update(80., 0., Phase::Started, start).1, Some(true));
        assert_eq!(
            phased
                .update(800., 0., Phase::Ended, start + Duration::from_secs(1))
                .1,
            None
        );
        assert_eq!(
            phased
                .update(800., 0., Phase::Moved, start + Duration::from_secs(2))
                .1,
            None
        );
        assert_eq!(
            phased
                .update(-80., 0., Phase::Started, start + Duration::from_secs(3))
                .1,
            Some(false)
        );
        assert_eq!(
            phased
                .update(80., 0., Phase::Wheel, start + Duration::from_secs(4))
                .1,
            Some(true)
        );
        let mut wheel = Gesture::default();
        assert_eq!(wheel.update(80., 0., Phase::Moved, start).1, Some(true));
        assert_eq!(
            wheel
                .update(-800., 0., Phase::Moved, start + Duration::from_millis(100))
                .1,
            None
        );
        assert_eq!(
            wheel
                .update(-80., 0., Phase::Moved, start + Duration::from_millis(400))
                .1,
            Some(false)
        );
        assert_eq!(
            wheel.update(40., 40., Phase::Moved, start + Duration::from_secs(1)),
            (false, None)
        );
    }
}
