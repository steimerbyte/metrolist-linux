//! Sleep timer, EQ/speed/device plumbing and output-device enumeration.

use std::time::{Duration, Instant};

use ml_audio::SleepTimer;
use ml_core::Settings;
use ml_ui::services::{Services, UiEvent};
use ml_ui::settings_store::SettingsPaths;

#[test]
fn playback_extras_reach_the_controller_and_engine() {
    let ctx = eframe::egui::Context::default();
    let root = std::env::temp_dir().join(format!("metrolist-extras-{}", std::process::id()));
    let paths = SettingsPaths::new(root.join("config"), root.join("data"), root.join("cache"));
    let (services, _notes) = Services::new(&ctx, paths.clone(), &Settings::default());

    services.set_eq(&ml_ui::pure::clamp_eq_bands(&[3.0; 10]));
    services.set_speed(1.25);
    services.set_device(Some("auto".to_string()));
    services.set_crossfade(5, false);
    assert_eq!(
        services.crossfade(),
        (Duration::from_secs(5), false),
        "crossfade setting did not reach the controller"
    );
    assert!(
        services.crossfade_available(),
        "second mpv engine did not start/attach"
    );
    services.set_crossfade(0, true);
    assert_eq!(services.crossfade(), (Duration::ZERO, true));
    services.set_skip_silence(true);
    services.set_normalization(true, Some(-6.0));
    services.set_normalization(false, None);

    services.set_sleep_timer(SleepTimer::Minutes(15));
    assert_eq!(services.sleep_timer(), SleepTimer::Minutes(15));
    services.set_sleep_timer(SleepTimer::EndOfTrack);
    assert_eq!(services.sleep_timer(), SleepTimer::EndOfTrack);
    services.set_sleep_timer(SleepTimer::Off);
    assert_eq!(services.sleep_timer(), SleepTimer::Off);

    assert!(
        services.downloads_available(),
        "download manager should start"
    );
    assert_eq!(
        services.download_dir(),
        paths.data_dir.join("downloads").as_path()
    );
    assert_eq!(services.download_active_limit(), None);
    assert!(services.download_list().is_empty());
    assert_eq!(
        services.download_badge("missing"),
        ml_ui::pure::DownloadBadge::None
    );

    services.list_devices();
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut devices = None;
    while Instant::now() < deadline && devices.is_none() {
        if let Some(event) = services.try_recv() {
            if let UiEvent::Devices(result) = event {
                devices = Some(result);
            }
        } else {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    let devices = devices.expect("device enumeration event");
    let devices = devices.expect("mpv --audio-device=help failed");
    assert!(!devices.is_empty(), "mpv reported no output devices");
    eprintln!(
        "extras verified: sleep timer, EQ/speed/device commands, {} output devices",
        devices.len()
    );
}
