//! Enumeration of mpv audio devices (`mpv --audio-device=help`).

/// One output device as reported by mpv.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDevice {
    /// Device id accepted by `--audio-device` / the `audio-device` property.
    pub id: String,
    /// Human-readable description exactly as mpv prints it.
    pub description: String,
    /// mpv marks the auto-selected default per audio backend with "Default".
    pub is_default: bool,
}

/// Parse the output of `mpv --audio-device=help`.
///
/// Lines look like `  'pipewire/<node>' (Built-in Audio Headphones)`. Lines
/// that do not carry a quoted id (the header, warnings) are skipped.
pub(crate) fn parse_devices(output: &str) -> Vec<AudioDevice> {
    let mut devices = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix('\'') else {
            continue;
        };
        let Some((id, tail)) = rest.split_once('\'') else {
            continue;
        };
        if id.is_empty() {
            continue;
        }
        let description = tail
            .trim()
            .strip_prefix('(')
            .and_then(|value| value.strip_suffix(')'))
            .unwrap_or("")
            .trim()
            .to_string();
        devices.push(AudioDevice {
            id: id.to_string(),
            is_default: description.contains("Default"),
            description,
        });
    }
    devices
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELP: &str = "List of detected audio devices:\n  \
        'auto' (Autoselect device)\n  \
        'pipewire' (Default (pipewire))\n  \
        'pipewire/alsa_output.pci-0000_00_1f.3.HiFi__Headphones__sink' (Built-in Audio Headphones)\n  \
        'pulse' (Default (pulse))\n  \
        'alsa/pipewire' (PipeWire Sound Server)\n";

    #[test]
    fn parses_ids_descriptions_and_defaults() {
        let devices = parse_devices(HELP);
        assert_eq!(devices.len(), 5);
        assert_eq!(devices[0].id, "auto");
        assert_eq!(devices[0].description, "Autoselect device");
        assert!(!devices[0].is_default);
        assert_eq!(devices[1].id, "pipewire");
        assert_eq!(devices[1].description, "Default (pipewire)");
        assert!(devices[1].is_default);
        assert_eq!(
            devices[2].id,
            "pipewire/alsa_output.pci-0000_00_1f.3.HiFi__Headphones__sink"
        );
        assert_eq!(devices[2].description, "Built-in Audio Headphones");
        assert!(!devices[2].is_default);
        assert!(devices[3].is_default);
        assert_eq!(devices[4].id, "alsa/pipewire");
    }

    #[test]
    fn ignores_noise_and_malformed_lines() {
        assert!(parse_devices("").is_empty());
        assert!(parse_devices("noise\n  'unterminated\n  (no id)\n").is_empty());
    }

    #[test]
    fn tolerates_missing_description() {
        let devices = parse_devices("  'alsa' \n");
        assert_eq!(
            devices,
            vec![AudioDevice {
                id: "alsa".to_string(),
                description: String::new(),
                is_default: false,
            }]
        );
    }
}
