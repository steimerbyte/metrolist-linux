//! Pure timing model for the boot overlay.

/// Seconds one module occupies in the sequence.
pub const SLOT: f32 = 0.16;
/// Loading sub-phase at the start of a slot, in seconds.
pub const LOAD: f32 = 0.05;
/// Handover animation (line enter and fade out), in seconds.
pub const HANDOVER: f32 = 0.16;

/// Total duration of a boot sequence with `count` modules.
pub fn total(count: usize) -> f32 {
    if count == 0 {
        0.0
    } else {
        count as f32 * SLOT + HANDOVER
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Loading,
    Ok,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct State {
    pub index: usize,
    pub phase: Phase,
    pub row_t: f32,
    pub exit_t: f32,
    pub finished: bool,
}

/// Samples the sequence at `elapsed` seconds; clamps and never panics.
pub fn state_at(elapsed: f32, count: usize) -> State {
    if count == 0 {
        return State {
            index: 0,
            phase: Phase::Ok,
            row_t: 1.0,
            exit_t: 1.0,
            finished: true,
        };
    }
    let elapsed = elapsed.max(0.0);
    let index = ((elapsed / SLOT) as usize).min(count - 1);
    let within = elapsed - index as f32 * SLOT;
    let phase = if within >= LOAD {
        Phase::Ok
    } else {
        Phase::Loading
    };
    let row_t = (within / HANDOVER).clamp(0.0, 1.0);
    let exit_t = ((elapsed - count as f32 * SLOT) / HANDOVER).clamp(0.0, 1.0);
    State {
        index,
        phase,
        row_t,
        exit_t,
        finished: elapsed >= total(count),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn total_stays_within_the_spec_bounds() {
        let t = total(10);
        assert!((1.0..=2.5).contains(&t), "total(10) = {t}");
    }

    #[test]
    fn starts_loading_at_zero() {
        let s = state_at(0.0, 10);
        assert_eq!(s.index, 0);
        assert_eq!(s.phase, Phase::Loading);
        assert_eq!(s.row_t, 0.0);
        assert!(!s.finished);
    }

    #[test]
    fn switches_to_ok_after_load_phase() {
        assert_eq!(state_at(LOAD, 10).phase, Phase::Ok);
    }

    #[test]
    fn advances_one_slot_per_module() {
        assert_eq!(state_at(SLOT * 3.5, 10).index, 3);
    }

    #[test]
    fn handover_starts_after_last_slot() {
        let s = state_at(SLOT * 10.0, 10);
        assert_eq!(s.exit_t, 0.0);
        assert!(!s.finished);
    }

    #[test]
    fn finishes_at_total() {
        let s = state_at(total(10), 10);
        assert!(s.finished);
        // f32 rounding leaves exit_t a hair below the exact 1.0 boundary.
        assert!((s.exit_t - 1.0).abs() < 1e-6, "exit_t = {}", s.exit_t);
    }

    #[test]
    fn empty_registry_is_already_done() {
        let s = state_at(0.0, 0);
        let want = State {
            index: 0,
            phase: Phase::Ok,
            row_t: 1.0,
            exit_t: 1.0,
            finished: true,
        };
        assert_eq!(s, want);
        assert_eq!(total(0), 0.0);
    }

    #[test]
    fn negative_elapsed_clamps_to_start() {
        let s = state_at(-1.0, 10);
        assert_eq!(s.index, 0);
        assert_eq!(s.phase, Phase::Loading);
        assert_eq!(s.row_t, 0.0);
    }
}
