//! Server clock estimation, a direct port of the Android `ServerClock`.
//!
//! Pongs estimate the offset between the server wall clock and the local
//! monotonic clock. Samples with a low network round trip get weight 0.25,
//! noisier samples 0.05 of the same exponential filter; the best RTT is
//! tracked to classify samples.

use std::cmp::max;

/// Kotlin `MAX_SAMPLE_AGE_MS`.
pub const MAX_SAMPLE_AGE_MS: i64 = 60_000;
/// Kotlin `GOOD_SAMPLE_MARGIN_MS`.
pub const GOOD_SAMPLE_MARGIN_MS: i64 = 50;
const GOOD_SAMPLE_WEIGHT: f64 = 0.25;
const POOR_SAMPLE_WEIGHT: f64 = 0.05;

/// Maps server wall time onto local monotonic (`elapsedRealtime`) milliseconds.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerClock {
    server_offset_ms: Option<f64>,
    best_round_trip_ms: u64,
}

impl Default for ServerClock {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerClock {
    pub fn new() -> Self {
        Self {
            server_offset_ms: None,
            best_round_trip_ms: u64::MAX,
        }
    }

    pub fn reset(&mut self) {
        self.server_offset_ms = None;
        self.best_round_trip_ms = u64::MAX;
    }

    pub fn has_offset(&self) -> bool {
        self.server_offset_ms.is_some()
    }

    pub fn server_offset_ms(&self) -> Option<f64> {
        self.server_offset_ms
    }

    pub fn best_round_trip_ms(&self) -> Option<u64> {
        (self.best_round_trip_ms != u64::MAX).then_some(self.best_round_trip_ms)
    }

    /// Feed a pong sample. `received_at` is the local monotonic time when the
    /// pong arrived. Returns `true` when this was the first accepted sample.
    ///
    /// Rejects samples with a non-positive or future client time, samples
    /// older than [`MAX_SAMPLE_AGE_MS`], non-positive server receive times and
    /// server send times before the receive time.
    pub fn record_pong(
        &mut self,
        client_time: i64,
        server_receive_time: i64,
        server_send_time: i64,
        received_at: i64,
    ) -> bool {
        if client_time <= 0
            || client_time > received_at
            || received_at - client_time > MAX_SAMPLE_AGE_MS
        {
            return false;
        }
        if server_receive_time <= 0 || server_send_time < server_receive_time {
            return false;
        }

        let round_trip = received_at - client_time;
        let server_processing = server_send_time - server_receive_time;
        let network_round_trip = max(0, round_trip - server_processing);
        let sample_offset =
            server_send_time as f64 + network_round_trip as f64 / 2.0 - received_at as f64;
        let previous_offset = self.server_offset_ms;

        if (network_round_trip as u64) < self.best_round_trip_ms {
            self.best_round_trip_ms = network_round_trip as u64;
        }
        let weight = if network_round_trip <= self.best_round_trip_ms as i64 + GOOD_SAMPLE_MARGIN_MS
        {
            GOOD_SAMPLE_WEIGHT
        } else {
            POOR_SAMPLE_WEIGHT
        };
        self.server_offset_ms = Some(match previous_offset {
            Some(previous) => previous + weight * (sample_offset - previous),
            None => sample_offset,
        });
        previous_offset.is_none()
    }

    /// Server time at the given local monotonic timestamp.
    pub fn now_at(&self, elapsed_realtime: i64) -> Option<i64> {
        self.server_offset_ms
            .map(|offset| (elapsed_realtime as f64 + offset) as i64)
    }

    /// Extrapolate a playback position captured at `effective_at_server_time`.
    /// Returns `position` unchanged while paused, without an anchor, or before
    /// the first pong.
    pub fn position_at(
        &self,
        position: i64,
        effective_at_server_time: Option<i64>,
        is_playing: bool,
        elapsed_realtime: i64,
    ) -> i64 {
        let Some(effective_at) = effective_at_server_time.filter(|at| *at > 0) else {
            return position;
        };
        if !is_playing {
            return position;
        }
        let Some(server_now) = self.now_at(elapsed_realtime) else {
            return position;
        };
        position.saturating_add(server_now.saturating_sub(effective_at).max(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_server_wall_time_onto_local_monotonic_time() {
        let mut clock = ServerClock::new();
        assert_eq!(clock.now_at(1_000), None);

        assert!(clock.record_pong(1_000, 10_000, 10_010, 1_120));
        assert_eq!(clock.now_at(1_120), Some(10_065));
        assert_eq!(clock.position_at(500, Some(10_000), true, 1_120), 565);

        assert_eq!(clock.now_at(1_220), Some(10_165));
        assert_eq!(clock.position_at(500, Some(10_000), true, 1_220), 665);
        assert_eq!(clock.position_at(500, Some(10_000), false, 1_220), 500);
    }

    #[test]
    fn rejects_invalid_and_stale_samples() {
        let mut clock = ServerClock::new();
        let elapsed_realtime = 100_000;

        assert!(!clock.record_pong(0, 1_000, 1_001, elapsed_realtime));
        assert!(!clock.record_pong(100_001, 1_000, 1_001, elapsed_realtime));
        assert!(!clock.record_pong(100_000, 1_001, 1_000, elapsed_realtime));
        assert!(!clock.record_pong(100_000, 1_000, 1_001, 200_001));
        assert_eq!(clock.now_at(200_001), None);

        // Server receive time may not be zero.
        assert!(!clock.record_pong(99_000, 0, 1, elapsed_realtime));
        assert!(!clock.has_offset());
    }

    #[test]
    fn reset_removes_the_server_clock_mapping() {
        let mut elapsed_realtime = 1_100;
        let mut clock = ServerClock::new();
        clock.record_pong(1_000, 10_000, 10_000, elapsed_realtime);

        clock.reset();
        elapsed_realtime += 100;

        assert_eq!(clock.now_at(elapsed_realtime), None);
        assert_eq!(
            clock.position_at(500, Some(10_000), true, elapsed_realtime),
            500
        );
    }

    #[test]
    fn noisier_samples_are_filtered_with_less_weight() {
        // First sample: perfect 100 ms RTT -> best. Offset 8_950.
        let mut clock = ServerClock::new();
        assert!(clock.record_pong(1_000, 10_000, 10_000, 1_100));
        assert_eq!(clock.server_offset_ms(), Some(8_950.0));
        assert_eq!(clock.best_round_trip_ms(), Some(100));

        // Second sample with 1 000 ms RTT: sample offset 10_500 + 1_000/2 - 2_000 = 9_000.
        // 1_000 > best (100) + 50 -> weight 0.05: 8_950 + 0.05 * 50 = 8_952.5.
        assert!(!clock.record_pong(1_000, 10_500, 10_500, 2_000));
        assert_eq!(clock.server_offset_ms(), Some(8_952.5));

        // Third sample: 120 ms RTT, 60 ms server processing -> 60 ms network
        // RTT, inside the good margin -> weight 0.25.
        assert!(!clock.record_pong(20_000, 19_940, 20_000, 20_120));
        let expected = 8_952.5 + 0.25 * ((20_000.0 + 30.0 - 20_120.0) - 8_952.5);
        assert_eq!(clock.server_offset_ms(), Some(expected));
    }

    #[test]
    fn position_extrapolation_clamps_negative_deltas_and_requires_anchor() {
        let mut clock = ServerClock::new();
        clock.record_pong(1_000, 10_000, 10_000, 1_100);

        // Server clock says 10_050 but the anchor is in the future -> no time travel.
        assert_eq!(clock.position_at(500, Some(20_000), true, 1_100), 500);
        // Zero / absent anchors are returned unchanged.
        assert_eq!(clock.position_at(500, Some(0), true, 1_100), 500);
        assert_eq!(clock.position_at(500, None, true, 1_100), 500);
        // Without any accepted pong the local playback position is authoritative.
        let empty = ServerClock::new();
        assert_eq!(empty.position_at(500, Some(10_000), true, 1_100), 500);
    }

    #[test]
    fn zero_or_negative_network_round_trip_still_estimates() {
        let mut clock = ServerClock::new();
        // Server processing "longer" than the observed RTT -> network RTT clamps to 0.
        assert!(clock.record_pong(1_000, 1_000, 5_000, 1_100));
        assert_eq!(clock.server_offset_ms(), Some(5_000.0 + 0.0 - 1_100.0));
        assert_eq!(clock.best_round_trip_ms(), Some(0));
    }
}
