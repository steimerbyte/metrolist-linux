//! Listen history, play tallies and the period bucketing that collapses
//! repeated plays.
//!
//! History shows at most one row per song per displayed period (today,
//! yesterday, this week, last week, then one bucket per calendar month), the
//! newest event of each. Upstream does this with a `CASE` inside
//! `GROUP BY songId`; the same precedence lives here as [`PeriodBucket`], a
//! total function of the timestamp plus [`PeriodBounds`], so the collapse is
//! testable without a clock and the SQL only mirrors it.
//!
//! All calendar arithmetic is pure `i64`/`u32` integer math on epoch days: no
//! `chrono`, no indexing, no panics.

use ml_core::{MlResult, Song};
use rusqlite::params;

use crate::rows::{hydrate_songs, song_at, store_error, PlayCount, SONG_COLS};
use crate::Store;

pub(crate) const MS_PER_DAY: i64 = 86_400_000;

const fn i64_to_i128(value: i64) -> i128 {
    value as i128
}

/// Saturating narrowing: exact for every day inside the `i64` epoch range, and
/// clamped (never panicking) within a week of its ends.
const fn i128_to_i64(value: i128) -> i64 {
    if value < i64::MIN as i128 {
        i64::MIN
    } else if value > i64::MAX as i128 {
        i64::MAX
    } else {
        value as i64
    }
}

/// Which displayed section a play belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeriodBucket {
    /// Future timestamps (clock skew) and this week's Monday-to-yesterday plays
    /// share a bucket, exactly as upstream's `tomorrowStart` branch does.
    ThisWeek,
    Today,
    Yesterday,
    LastWeek,
    /// A calendar month, `month` in `1..=12`.
    Month {
        year: i64,
        month: u32,
    },
}

impl PeriodBucket {
    /// The label upstream's SQL produces. Months yield `YYYY-MM`.
    pub fn label(&self) -> String {
        match self {
            Self::ThisWeek => "this_week".to_string(),
            Self::Today => "today".to_string(),
            Self::Yesterday => "yesterday".to_string(),
            Self::LastWeek => "last_week".to_string(),
            Self::Month { year, month } => format!("{year:04}-{month:02}"),
        }
    }
}

/// Midnight boundaries separating the displayed periods, in epoch
/// milliseconds.
///
/// Invariants for every `now` at least one week inside the `i64` range — that
/// excludes nothing a clock or a database row can hold, since one week of
/// milliseconds is ~6e8 out of ~9e18. Tested here, and for the twin in
/// `proofs/`:
///
/// ```text
/// last_monday_start + 7 days == this_monday_start
/// last_monday_start < this_monday_start <= yesterday_start < today_start < tomorrow_start
/// today_start <= now < tomorrow_start
/// ```
///
/// Within a week of `i64::MIN` the day starts clamp to `i64::MIN`, so the
/// orderings degrade to equalities. Nothing panics and no arithmetic overflows
/// anywhere: the day math runs in `i128`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeriodBounds {
    pub tomorrow_start: i64,
    pub today_start: i64,
    pub yesterday_start: i64,
    pub this_monday_start: i64,
    pub last_monday_start: i64,
}

impl PeriodBounds {
    /// Boundaries for `now_ms` read as UTC.
    pub fn from_millis(now_ms: i64) -> Self {
        Self::from_millis_with_offset(now_ms, 0)
    }

    /// Boundaries for `now_ms` read in a zone `utc_offset_ms` ahead of UTC
    /// (negative west of it). `div_euclid`/`rem_euclid` keep it correct for
    /// negative timestamps and offsets too.
    pub fn from_millis_with_offset(now_ms: i64, utc_offset_ms: i64) -> Self {
        // i128 throughout: `i64` day arithmetic overflows long before the
        // extremes of the epoch-millisecond range, and this keeps the
        // conversion exact for every plausible clock value (and total — no
        // panic — anywhere).
        const DAY: i128 = MS_PER_DAY as i128;
        let local = i64_to_i128(now_ms) + i64_to_i128(utc_offset_ms);
        let today = local.div_euclid(DAY);
        // 1970-01-01 was a Thursday: weekday index 3 with Monday = 0.
        let weekday = (today + 3).rem_euclid(7);
        let this_monday = today - weekday;
        let day_start = |index: i128| i128_to_i64(index * DAY - i64_to_i128(utc_offset_ms));
        Self {
            tomorrow_start: day_start(today + 1),
            today_start: day_start(today),
            yesterday_start: day_start(today - 1),
            this_monday_start: day_start(this_monday),
            last_monday_start: day_start(this_monday - 7),
        }
    }

    /// Bucket of one play timestamp, with upstream's `CASE` precedence.
    pub fn bucket_of(&self, timestamp_ms: i64) -> PeriodBucket {
        if timestamp_ms >= self.tomorrow_start {
            PeriodBucket::ThisWeek
        } else if timestamp_ms >= self.today_start {
            PeriodBucket::Today
        } else if timestamp_ms >= self.yesterday_start {
            PeriodBucket::Yesterday
        } else if timestamp_ms >= self.this_monday_start {
            PeriodBucket::ThisWeek
        } else if timestamp_ms >= self.last_monday_start {
            PeriodBucket::LastWeek
        } else {
            let (year, month, _) = civil_from_days(timestamp_ms.div_euclid(MS_PER_DAY));
            PeriodBucket::Month { year, month }
        }
    }
}

/// Days since 1970-01-01 for a proleptic-Gregorian date (Hinnant).
///
/// Total for every `i64` year and every `month`/`day` in `1..=12` / `1..=31`,
/// which are the only values the date pipeline produces. No indexing, no
/// division by a caller-controlled value.
pub const fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month = month as i64;
    let day = day as i64;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Inverse of [`days_from_civil`]: `(year, month, day)` for an epoch day.
pub const fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    (year + (month <= 2) as i64, month as u32, day as u32)
}

impl Store {
    /// Record a play. The timestamp is passed in rather than read here so
    /// period-bucket tests (and history imports) are deterministic. Inserts the
    /// event and bumps the `(song, year, month)` tally.
    pub fn record_play(&self, song_id: &str, play_time_ms: i64, timestamp_ms: i64) -> MlResult<()> {
        let (year, month, _) = civil_from_days(timestamp_ms.div_euclid(MS_PER_DAY));
        self.with_conn(|conn| {
            let tx = conn.transaction().map_err(store_error)?;
            tx.execute(
                "INSERT INTO event (song_id, timestamp, play_time) VALUES (?1, ?2, ?3)",
                params![song_id, timestamp_ms, play_time_ms],
            )
            .map_err(store_error)?;
            tx.execute(
                "INSERT INTO play_count (song_id, year, month, count) VALUES (?1, ?2, ?3, 1) \
                 ON CONFLICT(song_id, year, month) DO UPDATE SET count = count + 1",
                params![song_id, year, month],
            )
            .map_err(store_error)?;
            tx.commit().map_err(store_error)
        })
    }

    /// History newest first, collapsing to the newest play per song per period.
    pub fn history(&self, limit: u32, offset: u32) -> MlResult<Vec<crate::HistoryEntry>> {
        self.history_at(crate::rows::now_ms(), limit, offset)
    }

    /// [`Store::history`] with an injected clock.
    pub fn history_at(
        &self,
        now_ms: i64,
        limit: u32,
        offset: u32,
    ) -> MlResult<Vec<crate::HistoryEntry>> {
        self.history_in(PeriodBounds::from_millis(now_ms), limit, offset)
    }

    /// [`Store::history`] against explicit period boundaries (no clock read).
    pub fn history_in(
        &self,
        bounds: PeriodBounds,
        limit: u32,
        offset: u32,
    ) -> MlResult<Vec<crate::HistoryEntry>> {
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT e.id, e.timestamp, e.play_time, {SONG_COLS} \
                 FROM event e JOIN song ON song.id = e.song_id \
                 JOIN (SELECT MAX(id) AS id FROM event GROUP BY song_id, \
                        CASE \
                          WHEN timestamp >= :tomorrow THEN 'this_week' \
                          WHEN timestamp >= :today THEN 'today' \
                          WHEN timestamp >= :yesterday THEN 'yesterday' \
                          WHEN timestamp >= :this_monday THEN 'this_week' \
                          WHEN timestamp >= :last_monday THEN 'last_week' \
                          ELSE strftime('%Y-%m', timestamp / 1000, 'unixepoch') \
                        END) latest ON latest.id = e.id \
                 ORDER BY e.id DESC LIMIT :limit OFFSET :offset"
            );
            let mut stmt = conn.prepare(&sql).map_err(store_error)?;
            let mut entries = stmt
                .query_map(
                    rusqlite::named_params! {
                        ":tomorrow": bounds.tomorrow_start,
                        ":today": bounds.today_start,
                        ":yesterday": bounds.yesterday_start,
                        ":this_monday": bounds.this_monday_start,
                        ":last_monday": bounds.last_monday_start,
                        ":limit": i64::from(limit),
                        ":offset": i64::from(offset),
                    },
                    |row| {
                        Ok(crate::HistoryEntry {
                            id: row.get(0)?,
                            timestamp: row.get(1)?,
                            play_time: row.get(2)?,
                            song: song_at(row, 3)?,
                        })
                    },
                )
                .map_err(store_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(store_error)?;
            let mut songs: Vec<Song> = entries.iter().map(|entry| entry.song.clone()).collect();
            hydrate_songs(conn, &mut songs)?;
            for (entry, song) in entries.iter_mut().zip(songs) {
                entry.song = song;
            }
            Ok(entries)
        })
    }

    /// Most recently recorded event, whatever period it belongs to.
    pub fn latest_event(&self) -> MlResult<Option<crate::HistoryEntry>> {
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT e.id, e.timestamp, e.play_time, {SONG_COLS} \
                 FROM event e JOIN song ON song.id = e.song_id ORDER BY e.id DESC LIMIT 1"
            );
            let Some(mut entry) = conn
                .query_row(&sql, [], |row| {
                    Ok(crate::HistoryEntry {
                        id: row.get(0)?,
                        timestamp: row.get(1)?,
                        play_time: row.get(2)?,
                        song: song_at(row, 3)?,
                    })
                })
                .map(Some)
                .or_else(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })
                .map_err(store_error)?
            else {
                return Ok(None);
            };
            hydrate_songs(conn, std::slice::from_mut(&mut entry.song))?;
            Ok(Some(entry))
        })
    }

    /// Number of recorded play events.
    pub fn event_count(&self) -> MlResult<u32> {
        self.with_conn(|conn| {
            conn.query_row("SELECT COUNT(*) FROM event", [], |row| row.get(0))
                .map_err(store_error)
        })
    }

    /// Total listening time in milliseconds.
    pub fn total_play_time(&self) -> MlResult<i64> {
        self.with_conn(|conn| {
            conn.query_row("SELECT COALESCE(SUM(play_time), 0) FROM event", [], |row| {
                row.get(0)
            })
            .map_err(store_error)
        })
    }

    /// Play tallies for one month, most played first.
    pub fn play_counts(&self, year: i64, month: u32) -> MlResult<Vec<PlayCount>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT song_id, count FROM play_count WHERE year = ?1 AND month = ?2 \
                     ORDER BY count DESC, song_id",
                )
                .map_err(store_error)?;
            let counts = stmt
                .query_map(params![year, month], |row| {
                    Ok(PlayCount {
                        song_id: row.get(0)?,
                        count: row.get(1)?,
                    })
                })
                .map_err(store_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(store_error)?;
            Ok(counts)
        })
    }

    /// Recorded plays of one song inside one calendar month.
    pub fn play_count(&self, song_id: &str, year: i64, month: u32) -> MlResult<i64> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT count FROM play_count WHERE song_id = ?1 AND year = ?2 AND month = ?3",
                params![song_id, year, month],
                |row| row.get(0),
            )
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(0),
                other => Err(other),
            })
            .map_err(store_error)
        })
    }

    /// Delete every recorded play. The `play_count` tallies stay, matching
    /// upstream's `clearListenHistory`, which only truncates `event`.
    pub fn clear_history(&self) -> MlResult<()> {
        self.with_conn(|conn| {
            conn.execute("DELETE FROM event", []).map_err(store_error)?;
            Ok(())
        })
    }

    /// Section labels for a batch of timestamps, in input order.
    pub fn period_labels(now_ms: i64, timestamps: &[i64]) -> Vec<String> {
        let bounds = PeriodBounds::from_millis(now_ms);
        timestamps
            .iter()
            .map(|timestamp| bounds.bucket_of(*timestamp).label())
            .collect()
    }
}
