//! Read-only stats over the real library database. Run with `--ignored`.

use ml_core::now_millis;
use ml_store::{StatsPeriod, Store};
use ml_ui::settings_store::SettingsPaths;

#[test]
#[ignore = "reads the operator's real library database"]
fn live_stats_report_counts() {
    let paths = SettingsPaths::from_env();
    let store = Store::open(&paths.database_file()).expect("open library database");
    for period in [
        StatsPeriod::Week,
        StatsPeriod::Month,
        StatsPeriod::Year,
        StatsPeriod::All,
    ] {
        let report = store
            .stats_report(period, now_millis() as i64, 10)
            .expect("stats report");
        println!(
            "{:?}: plays={} total_ms={} songs={} artists={} albums={} streak={} top_song={:?}",
            period,
            report.summary.event_count,
            report.summary.total_ms,
            report.summary.unique_songs,
            report.summary.unique_artists,
            report.summary.unique_albums,
            report.summary.longest_streak_days,
            report
                .top_songs
                .first()
                .map(|stat| stat.song.title.as_str())
        );
    }
}
