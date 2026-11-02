//! `metrolist` binary: logging, CLI modes, settings paths and the eframe app.

use std::path::PathBuf;

use ml_ui::{DemoMode, RunMode, RunOptions};

fn main() -> eframe::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let options = match parse_args(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}\n\n{}", usage());
            std::process::exit(2);
        }
    };
    ml_ui::run(options)
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<RunOptions, String> {
    let mut args = args.peekable();
    let mut options = RunOptions {
        mode: RunMode::Normal,
        width: 1100,
        height: 760,
        demo: None,
    };
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--help" | "-h" => {
                println!("{}", usage());
                std::process::exit(0);
            }
            "--smoke" => {
                let frames = args
                    .next()
                    .and_then(|value| value.parse::<u32>().ok())
                    .unwrap_or(120)
                    .max(1);
                options.mode = RunMode::Smoke { frames };
                options.width = 1024;
                options.height = 720;
            }
            "--screenshot" => {
                let path = args
                    .next()
                    .map(PathBuf::from)
                    .ok_or_else(|| "--screenshot needs a path".to_string())?;
                let mut size = |fallback| {
                    let parsed = args.peek().and_then(|value| value.parse::<u32>().ok());
                    if parsed.is_some() {
                        args.next();
                    }
                    parsed.unwrap_or(fallback)
                };
                let width = size(405);
                let height = size(900);
                options.mode = RunMode::Screenshot { path };
                options.width = width;
                options.height = height;
            }
            "--demo" => {
                let value = args.next().ok_or_else(|| {
                    "--demo needs player|queue|lyrics|library|settings|settings-audio|history|liked|downloads|account|charts|moods|new-releases|podcasts|stats|wrapped|recognize|recognize-listening|recognize-success|recognize-error|integrations|integrations-connected|integrations-error|together|together-connected|together-guest"
                        .to_string()
                })?;
                options.demo = Some(
                    DemoMode::parse(&value)
                        .ok_or_else(|| format!("unknown demo screen: {value}"))?,
                );
            }
            other if other.starts_with("metrolist://") => {
                // Forwarded by the single-instance guard on the next launch.
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(options)
}

fn usage() -> &'static str {
    "Metrolist — YouTube Music client for Linux

Usage:
  metrolist
  metrolist --smoke [FRAMES]           # CI smoke test, default 120 frames
  metrolist --screenshot PATH [WIDTH] [HEIGHT]   # default 405x900
  metrolist --screenshot PATH --demo player|queue|lyrics|library|settings|settings-audio|history|liked|downloads|account|charts|moods|new-releases|podcasts|stats|wrapped|recognize|recognize-listening|recognize-success|recognize-error|integrations|integrations-connected|integrations-error|together|together-connected|together-guest
  metrolist --demo together           # Listen Together lobby
  metrolist --demo player               # interactive sample playback
  metrolist --help

Environment:
  RUST_LOG=info|debug                  # tracing filter"
}
