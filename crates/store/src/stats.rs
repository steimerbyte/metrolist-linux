//! Listening statistics: the aggregation behind a future Stats/Wrapped screen.
//!
//! Two tables feed it. `event` is the per-play log (`song_id`, `timestamp`,
//! `play_time`), so every windowed aggregate filters it by timestamp — it is
//! also what `clear_history` empties. `play_count` is the durable
//! `(song, year, month)` tally that survives that clear, so
//! [`Store::plays_per_month`] reads the tally instead of the log.
//!
//! Windows are inclusive on both ends: a range built by [`StatsPeriod::range`]
//! ends at `now`, so a play recorded at exactly `now` counts. Every bound is
//! computed with saturating `i64` arithmetic over the pure civil-calendar
//! helpers in [`crate::history`]; the crate never reads a clock itself.

use ml_core::{Album, ArtistItem, MlResult, Song};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::history::{civil_from_days, days_from_civil, MS_PER_DAY};
use crate::rows::{album_from_row, hydrate_songs, song_at, store_error, ALBUM_COLS, SONG_COLS};
use crate::Store;

/// Epoch day of an event timestamp: floor division of milliseconds by a day,
/// spelled as truncating division plus a sign correction because SQLite's `/`
/// truncates toward zero. The literal is [`MS_PER_DAY`], which SQL cannot bind
/// into a reused expression.
const DAY_INDEX_SQL: &str = "(event.timestamp / 86400000 - (event.timestamp % 86400000 < 0))";

/// An inclusive time window in epoch milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatsRange {
    pub start_ms: i64,
    pub end_ms: i64,
}

impl StatsRange {
    /// Every timestamp a database row can hold.
    pub const ALL: Self = Self {
        start_ms: i64::MIN,
        end_ms: i64::MAX,
    };

    pub fn contains(&self, timestamp_ms: i64) -> bool {
        self.start_ms <= timestamp_ms && timestamp_ms <= self.end_ms
    }
}

impl Default for StatsRange {
    fn default() -> Self {
        Self::ALL
    }
}

/// Which window a stats query asks about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatsPeriod {
    Week,
    Month,
    Year,
    All,
}

impl StatsPeriod {
    /// The window ending at `now_ms`, inclusive at both ends.
    ///
    /// Month and Year are *calendar* windows (one / twelve months back, the
    /// day clamped to the target month's length), matching upstream's
    /// `LocalDateTime.minusMonths`, not fixed 30/365-day spans.
    pub fn range(self, now_ms: i64) -> StatsRange {
        match self {
            Self::Week => StatsRange {
                start_ms: now_ms.saturating_sub(7 * MS_PER_DAY),
                end_ms: now_ms,
            },
            Self::Month => StatsRange {
                start_ms: sub_months(now_ms, 1),
                end_ms: now_ms,
            },
            Self::Year => StatsRange {
                start_ms: sub_months(now_ms, 12),
                end_ms: now_ms,
            },
            Self::All => StatsRange::ALL,
        }
    }
}

/// Days in a Gregorian month. `month` is `1..=12`; `13` reads as January of
/// the following year, which is what the day-count arithmetic already means.
pub fn days_in_month(year: i64, month: u32) -> u32 {
    (days_from_civil(year, month + 1, 1) - days_from_civil(year, month, 1)) as u32
}

/// `months` calendar months before `ms`, keeping the time of day and clamping
/// the day-of-month: `2026-03-31` minus one month is `2026-02-28`.
///
/// Pure and total. Exact for every timestamp a clock or a row can hold;
/// within about a month of `i64::MIN` the clamping saturates instead of
/// overflowing, which no real play timestamp can reach.
pub fn sub_months(ms: i64, months: u32) -> i64 {
    let day = ms.div_euclid(MS_PER_DAY);
    let of_day = ms.rem_euclid(MS_PER_DAY);
    let (year, month, day_of_month) = civil_from_days(day);
    let total = year
        .saturating_mul(12)
        .saturating_add(month as i64 - 1)
        .saturating_sub(i64::from(months));
    let year = total.div_euclid(12);
    let month = total.rem_euclid(12) as u32 + 1;
    let day_of_month = day_of_month.min(days_in_month(year, month));
    days_from_civil(year, month, day_of_month)
        .saturating_mul(MS_PER_DAY)
        .saturating_add(of_day)
}

/// Longest run of consecutive epoch days in an ascending list of day indices.
///
/// Pure and total; duplicate days collapse, an empty list is `0`.
pub fn longest_run(days: &[i64]) -> u32 {
    let mut longest = 0u32;
    let mut current = 0u32;
    let mut previous: Option<i64> = None;
    for &day in days {
        match previous {
            Some(previous) if day == previous => continue,
            Some(previous) if previous.checked_add(1) == Some(day) => current += 1,
            _ => current = 1,
        }
        previous = Some(day);
        longest = longest.max(current);
    }
    longest
}

/// Totals for one window, plus first/last play and the longest daily streak
/// inside it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatsSummary {
    pub range: StatsRange,
    pub total_ms: i64,
    pub event_count: u64,
    pub unique_songs: u64,
    pub unique_artists: u64,
    pub unique_albums: u64,
    pub first_play_ms: Option<i64>,
    pub last_play_ms: Option<i64>,
    /// Longest run of consecutive days with at least one play in the window.
    pub longest_streak_days: u32,
}

/// One song's listening in a window.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SongStat {
    pub song: Song,
    pub plays: u64,
    pub ms: i64,
}

/// One artist's listening in a window.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtistStat {
    pub artist: ArtistItem,
    pub plays: u64,
    pub ms: i64,
}

/// One album's listening in a window.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AlbumStat {
    pub album: Album,
    pub plays: u64,
    pub ms: i64,
}

/// One calendar month of the durable `play_count` tally.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonthPlays {
    pub year: i64,
    /// `1..=12`.
    pub month: u32,
    pub plays: u64,
    /// Distinct songs with a tally that month.
    pub songs: u64,
}

/// Everything a Stats/Wrapped screen shows for one period, in one value.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StatsReport {
    pub summary: StatsSummary,
    /// Lifetime trend from `play_count`, oldest month first. Not clipped to the
    /// window: Wrapped needs the whole history.
    pub monthly: Vec<MonthPlays>,
    pub top_songs: Vec<SongStat>,
    pub top_artists: Vec<ArtistStat>,
    pub top_albums: Vec<AlbumStat>,
}

fn count(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

/// Distinct epoch days with a play inside `range`, ascending.
fn play_days(conn: &Connection, range: StatsRange) -> MlResult<Vec<i64>> {
    let sql = format!(
        "SELECT DISTINCT {DAY_INDEX_SQL} FROM event \
         WHERE timestamp >= ?1 AND timestamp <= ?2 ORDER BY 1"
    );
    let mut stmt = conn.prepare(&sql).map_err(store_error)?;
    let days = stmt
        .query_map(params![range.start_ms, range.end_ms], |row| row.get(0))
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(store_error)?;
    Ok(days)
}

impl Store {
    /// Totals and the longest daily streak for one period.
    pub fn stats_summary(&self, period: StatsPeriod, now_ms: i64) -> MlResult<StatsSummary> {
        self.stats_summary_in(period.range(now_ms))
    }

    /// [`Store::stats_summary`] against an explicit window (no period math).
    pub fn stats_summary_in(&self, range: StatsRange) -> MlResult<StatsSummary> {
        self.with_conn(|conn| {
            let (events, total_ms, songs, first, last) = conn
                .query_row(
                    "SELECT COUNT(*), COALESCE(SUM(play_time), 0), COUNT(DISTINCT song_id), \
                            MIN(timestamp), MAX(timestamp) \
                     FROM event WHERE timestamp >= ?1 AND timestamp <= ?2",
                    params![range.start_ms, range.end_ms],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, Option<i64>>(3)?,
                            row.get::<_, Option<i64>>(4)?,
                        ))
                    },
                )
                .map_err(store_error)?;
            let artists: i64 = conn
                .query_row(
                    "SELECT COUNT(DISTINCT m.artist_id) FROM event e \
                     JOIN song_artist_map m ON m.song_id = e.song_id \
                     WHERE e.timestamp >= ?1 AND e.timestamp <= ?2",
                    params![range.start_ms, range.end_ms],
                    |row| row.get(0),
                )
                .map_err(store_error)?;
            let albums: i64 = conn
                .query_row(
                    "SELECT COUNT(DISTINCT s.album_id) FROM event e \
                     JOIN song s ON s.id = e.song_id \
                     WHERE e.timestamp >= ?1 AND e.timestamp <= ?2 AND s.album_id IS NOT NULL",
                    params![range.start_ms, range.end_ms],
                    |row| row.get(0),
                )
                .map_err(store_error)?;
            let days = play_days(conn, range)?;
            Ok(StatsSummary {
                range,
                total_ms,
                event_count: count(events),
                unique_songs: count(songs),
                unique_artists: count(artists),
                unique_albums: count(albums),
                first_play_ms: first,
                last_play_ms: last,
                longest_streak_days: longest_run(&days),
            })
        })
    }

    /// Most-listened songs in the window, listening time first; ties break on
    /// play count, then folded title, then id, so the order is total.
    pub fn top_songs(
        &self,
        period: StatsPeriod,
        now_ms: i64,
        limit: u32,
    ) -> MlResult<Vec<SongStat>> {
        self.top_songs_in(period.range(now_ms), limit)
    }

    /// [`Store::top_songs`] against an explicit window.
    pub fn top_songs_in(&self, range: StatsRange, limit: u32) -> MlResult<Vec<SongStat>> {
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT {SONG_COLS}, COUNT(*) AS plays, \
                        COALESCE(SUM(e.play_time), 0) AS listened_ms \
                 FROM event e JOIN song ON song.id = e.song_id \
                 WHERE e.timestamp >= ?1 AND e.timestamp <= ?2 \
                 GROUP BY song.id \
                 ORDER BY listened_ms DESC, plays DESC, song.title COLLATE NOCASE, song.id \
                 LIMIT ?3"
            );
            let mut stmt = conn.prepare(&sql).map_err(store_error)?;
            let mut stats = stmt
                .query_map(
                    params![range.start_ms, range.end_ms, i64::from(limit)],
                    |row| {
                        Ok(SongStat {
                            song: song_at(row, 0)?,
                            plays: count(row.get(16)?),
                            ms: row.get(17)?,
                        })
                    },
                )
                .map_err(store_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(store_error)?;
            let mut songs: Vec<Song> = stats.iter().map(|stat| stat.song.clone()).collect();
            hydrate_songs(conn, &mut songs)?;
            for (stat, song) in stats.iter_mut().zip(songs) {
                stat.song = song;
            }
            Ok(stats)
        })
    }

    /// Most-listened artists in the window. A song credited to two artists
    /// counts for both, as upstream's join does.
    pub fn top_artists(
        &self,
        period: StatsPeriod,
        now_ms: i64,
        limit: u32,
    ) -> MlResult<Vec<ArtistStat>> {
        self.top_artists_in(period.range(now_ms), limit)
    }

    /// [`Store::top_artists`] against an explicit window.
    pub fn top_artists_in(&self, range: StatsRange, limit: u32) -> MlResult<Vec<ArtistStat>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT a.id, a.name, a.thumbnail_url, a.channel_id, a.subscribers, \
                            COUNT(*) AS plays, COALESCE(SUM(e.play_time), 0) AS listened_ms \
                     FROM event e \
                     JOIN song_artist_map m ON m.song_id = e.song_id \
                     JOIN artist a ON a.id = m.artist_id \
                     WHERE e.timestamp >= ?1 AND e.timestamp <= ?2 \
                     GROUP BY a.id \
                     ORDER BY listened_ms DESC, plays DESC, a.name COLLATE NOCASE, a.id \
                     LIMIT ?3",
                )
                .map_err(store_error)?;
            let stats = stmt
                .query_map(
                    params![range.start_ms, range.end_ms, i64::from(limit)],
                    |row| {
                        Ok(ArtistStat {
                            artist: ArtistItem {
                                id: row.get(0)?,
                                name: row.get(1)?,
                                thumbnail_url: row.get(2)?,
                                channel_id: row.get(3)?,
                                subscribers: row.get(4)?,
                            },
                            plays: count(row.get(5)?),
                            ms: row.get(6)?,
                        })
                    },
                )
                .map_err(store_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(store_error)?;
            Ok(stats)
        })
    }

    /// Most-listened albums in the window, grouped by the album id carried on
    /// the song row (albums with no stored row are skipped).
    pub fn top_albums(
        &self,
        period: StatsPeriod,
        now_ms: i64,
        limit: u32,
    ) -> MlResult<Vec<AlbumStat>> {
        self.top_albums_in(period.range(now_ms), limit)
    }

    /// [`Store::top_albums`] against an explicit window.
    pub fn top_albums_in(&self, range: StatsRange, limit: u32) -> MlResult<Vec<AlbumStat>> {
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT {ALBUM_COLS}, COUNT(*) AS plays, \
                        COALESCE(SUM(e.play_time), 0) AS listened_ms \
                 FROM event e JOIN song s ON s.id = e.song_id \
                 JOIN album ON album.id = s.album_id \
                 WHERE e.timestamp >= ?1 AND e.timestamp <= ?2 \
                 GROUP BY album.id \
                 ORDER BY listened_ms DESC, plays DESC, album.title COLLATE NOCASE, album.id \
                 LIMIT ?3"
            );
            let mut stmt = conn.prepare(&sql).map_err(store_error)?;
            let stats = stmt
                .query_map(
                    params![range.start_ms, range.end_ms, i64::from(limit)],
                    |row| {
                        Ok(AlbumStat {
                            album: album_from_row(row)?,
                            plays: count(row.get(13)?),
                            ms: row.get(14)?,
                        })
                    },
                )
                .map_err(store_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(store_error)?;
            Ok(stats)
        })
    }

    /// Plays per calendar month from the durable `play_count` tally, oldest
    /// first, clipped to the most recent `months` buckets (`0` = every month).
    ///
    /// The tally is used rather than `event` because it survives
    /// `clear_history`; it holds counts only, so there is no time column here.
    pub fn plays_per_month(&self, months: u32) -> MlResult<Vec<MonthPlays>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT year, month, SUM(count) AS plays, \
                            COUNT(DISTINCT song_id) AS songs \
                     FROM play_count GROUP BY year, month ORDER BY year, month",
                )
                .map_err(store_error)?;
            let mut buckets = stmt
                .query_map([], |row| {
                    Ok(MonthPlays {
                        year: row.get(0)?,
                        month: row.get(1)?,
                        plays: count(row.get(2)?),
                        songs: count(row.get(3)?),
                    })
                })
                .map_err(store_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(store_error)?;
            if months > 0 && buckets.len() > months as usize {
                buckets.drain(..buckets.len() - months as usize);
            }
            Ok(buckets)
        })
    }

    /// Everything a Stats screen shows for one period.
    pub fn stats_report(
        &self,
        period: StatsPeriod,
        now_ms: i64,
        limit: u32,
    ) -> MlResult<StatsReport> {
        Ok(StatsReport {
            summary: self.stats_summary(period, now_ms)?,
            monthly: self.plays_per_month(0)?,
            top_songs: self.top_songs(period, now_ms, limit)?,
            top_artists: self.top_artists(period, now_ms, limit)?,
            top_albums: self.top_albums(period, now_ms, limit)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sub_months_is_the_calendar_month_with_a_clamped_day() {
        let midnight = |y, m, d| days_from_civil(y, m, d) * MS_PER_DAY;
        assert_eq!(sub_months(midnight(2026, 3, 31), 1), midnight(2026, 2, 28));
        assert_eq!(sub_months(midnight(2028, 3, 31), 1), midnight(2028, 2, 29));
        assert_eq!(sub_months(midnight(2026, 1, 15), 1), midnight(2025, 12, 15));
        assert_eq!(sub_months(midnight(2026, 8, 26), 12), midnight(2025, 8, 26));
        assert_eq!(sub_months(midnight(2026, 8, 26), 0), midnight(2026, 8, 26));
        // The time of day is preserved, including in the clamped month.
        assert_eq!(
            sub_months(midnight(2026, 3, 31) + 3_600_000, 1),
            midnight(2026, 2, 28) + 3_600_000
        );
        // Total at the extremes: saturating, never panicking.
        for ms in [i64::MIN, i64::MIN + 1, i64::MAX, i64::MAX - 1, 0] {
            for months in [0, 1, 12, u32::MAX] {
                let _ = sub_months(ms, months);
            }
        }
    }

    #[test]
    fn days_in_month_matches_the_gregorian_calendar() {
        assert_eq!(days_in_month(2026, 2), 28);
        assert_eq!(days_in_month(2028, 2), 29);
        assert_eq!(days_in_month(2000, 2), 29, "divisible by 400");
        assert_eq!(days_in_month(1900, 2), 28, "divisible by 100 but not 400");
        assert_eq!(days_in_month(2026, 4), 30);
        assert_eq!(days_in_month(2026, 12), 31);
        // 13 is January of the next year: 31 days, as the arithmetic means.
        assert_eq!(days_in_month(2026, 13), 31);
    }

    #[test]
    fn longest_run_counts_consecutive_days_once() {
        assert_eq!(longest_run(&[]), 0);
        assert_eq!(longest_run(&[0]), 1);
        assert_eq!(longest_run(&[0, 0, 0]), 1, "duplicates collapse");
        assert_eq!(longest_run(&[1, 2, 3, 4]), 4);
        assert_eq!(longest_run(&[1, 2, 3, 9, 10]), 3);
        assert_eq!(
            longest_run(&[7, 1, 2, 3]),
            3,
            "order does not matter per run"
        );
        assert_eq!(longest_run(&[i64::MIN, i64::MAX]), 1);
        assert_eq!(longest_run(&[i64::MAX - 1, i64::MAX]), 2);
        assert_eq!(longest_run(&[5, 6, 6, 7]), 3);
    }

    #[test]
    fn periods_are_inclusive_and_ordered() {
        let now = day_ms_local(2026, 8, 26, 12);
        for period in [StatsPeriod::Week, StatsPeriod::Month, StatsPeriod::Year] {
            let range = period.range(now);
            assert_eq!(range.end_ms, now);
            assert!(range.start_ms < range.end_ms);
            assert!(range.contains(now));
            assert!(range.contains(range.start_ms));
            assert!(!range.contains(range.start_ms - 1));
        }
        assert_eq!(StatsPeriod::All.range(now), StatsRange::ALL);
        assert!(StatsRange::ALL.contains(i64::MIN));
        assert!(StatsRange::ALL.contains(i64::MAX));
        // A one-week window is exactly 7 days of milliseconds.
        assert_eq!(StatsPeriod::Week.range(now).start_ms, now - 7 * MS_PER_DAY);
    }

    fn day_ms_local(year: i64, month: u32, day: u32, hour: u32) -> i64 {
        days_from_civil(year, month, day) * MS_PER_DAY + i64::from(hour) * 3_600_000
    }
}
