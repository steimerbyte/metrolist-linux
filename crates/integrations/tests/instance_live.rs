//! Two-process single-instance test.
//!
//! Re-executes this test binary as a child so the primaries/forwarders really
//! are separate processes. Run with:
//! `ML_INTEGRATION_LIVE=1 cargo test -p ml-integrations --test instance_live -- --ignored --nocapture`

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use ml_integrations::InstanceGuard;

const APP_ID: &str = "org.metrolist.Metrolist.integration-test";
const CHILD_ENV: &str = "ML_INSTANCE_CHILD";

fn live_enabled() -> bool {
    std::env::var_os("ML_INTEGRATION_LIVE").is_some()
}

/// In the child: become primary, print the forwarded argv as JSON, exit.
#[test]
#[ignore = "child helper for instance_two_processes"]
fn child_primary() {
    if std::env::var_os(CHILD_ENV).is_none() {
        eprintln!("child helper: set ML_INSTANCE_CHILD=1 (driven by instance_two_processes)");
        return;
    }
    let guard = match InstanceGuard::acquire(APP_ID).expect("acquire") {
        Some(guard) => guard,
        None => {
            println!("CHILD_FORWARDED");
            return;
        }
    };
    println!("CHILD_PRIMARY");
    // Wait for one forwarded request, then exit.
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        match guard.requests().recv_timeout(Duration::from_millis(100)) {
            Ok(argv) => {
                println!("RECEIVED {argv:?}");
                return;
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
            Err(error) => {
                println!("CHANNEL {error}");
                return;
            }
        }
    }
    println!("TIMEOUT");
}

/// In the child: a second process must not become primary.
#[test]
#[ignore = "child helper for instance_two_processes"]
fn child_secondary() {
    if std::env::var_os(CHILD_ENV).is_none() {
        eprintln!("child helper: set ML_INSTANCE_CHILD=1 (driven by instance_two_processes)");
        return;
    }
    match InstanceGuard::acquire(APP_ID) {
        Ok(None) => println!("CHILD_FORWARDED"),
        Ok(Some(_)) => println!("CHILD_PRIMARY"),
        Err(error) => println!("CHILD_ERROR {error}"),
    }
}

fn spawn_child(test_name: &str, extra_args: &[&str]) -> std::process::Child {
    let exe = std::env::current_exe().expect("test binary path");
    let mut command = Command::new(exe);
    command
        .arg("--exact")
        .arg(test_name)
        .arg("--ignored")
        .arg("--nocapture")
        .args(extra_args)
        .env(CHILD_ENV, "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command.spawn().expect("spawn child")
}

#[test]
#[ignore = "spawns processes; run with ML_INTEGRATION_LIVE=1"]
fn two_processes_forward_argv_and_exit() {
    if !live_enabled() {
        eprintln!("skipping: ML_INTEGRATION_LIVE is not set");
        return;
    }

    // Primary occupies the lock and waits for one forwarded payload.
    let primary = spawn_child("child_primary", &[]);
    let start = Instant::now();
    let mut primary_out = String::new();
    // Give it a moment to take the lock before the forwarder starts.
    while start.elapsed() < Duration::from_secs(5) {
        if std::path::Path::new(&format!(
            "{}/{APP_ID}.sock",
            std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/run/user/1000".into())
        ))
        .exists()
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    // The forwarder must exit without becoming primary.
    let secondary = spawn_child("child_secondary", &["--", "metrolist://play?v=dQw4w9WgXcQ"]);
    let secondary = secondary.wait_with_output().expect("secondary exits");
    primary_out.push_str(&String::from_utf8_lossy(&secondary.stdout));
    println!("secondary output:\n{primary_out}");
    assert!(
        primary_out.contains("CHILD_FORWARDED"),
        "second process did not forward: {primary_out}"
    );

    let output = primary.wait_with_output().expect("primary exits");
    let text = String::from_utf8_lossy(&output.stdout);
    println!("primary output:\n{text}");
    assert!(
        text.contains("CHILD_PRIMARY"),
        "first process must be primary: {text}"
    );
    assert!(
        text.contains("RECEIVED"),
        "primary never received argv: {text}"
    );
}
