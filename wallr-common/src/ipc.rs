use serde::{Deserialize, Serialize};

/// Maximum IPC request size. Prevents unbounded allocations from malformed
/// or malicious clients. 64 KiB is far above any legitimate command
/// (a path + effect params is typically < 2 KiB).
pub const MAX_IPC_BYTES: usize = 64 * 1024;
/// Maximum wallpaper path length accepted over IPC.
pub const MAX_IPC_PATH_LEN: usize = 8192;
/// Maximum monitor name length accepted over IPC.
pub const MAX_MONITOR_LEN: usize = 256;
/// Maximum transition duration accepted over IPC (60s).
pub const MAX_DURATION_MS: u32 = 60_000;
/// Maximum requested video presentation rate.
pub const MAX_VIDEO_FPS: u32 = 240;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum IpcCommand {
    Pause {
        #[serde(default)]
        monitor: Option<String>,
    },
    Resume {
        #[serde(default)]
        monitor: Option<String>,
    },
    Reload,
    Seek {
        timestamp_ms: u64,
        #[serde(default)]
        monitor: Option<String>,
    },
    Preview {
        path: String,
        effect: Option<crate::effect::Effect>,
        duration_ms: Option<u32>,
        #[serde(default)]
        fps: Option<u32>,
        #[serde(default)]
        no_theme: bool,
        #[serde(default)]
        theme_override: Option<crate::types::ThemeProvider>,
        #[serde(default)]
        monitor: Option<String>,
        #[serde(default)]
        scaling_mode: Option<crate::types::ScalingMode>,
    },
    Stop,
    Status,
    Info {
        #[serde(default)]
        monitor: Option<String>,
    },
    MonitorList,
    MonitorCurrent,
    Blank {
        #[serde(default)]
        monitor: Option<String>,
        #[serde(default)]
        effect: Option<crate::effect::Effect>,
        #[serde(default)]
        duration_ms: Option<u32>,
    },
    Restore {
        #[serde(default)]
        monitor: Option<String>,
        #[serde(default)]
        effect: Option<crate::effect::Effect>,
        #[serde(default)]
        duration_ms: Option<u32>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcResponse {
    pub success: bool,
    pub message: Option<String>,
}

impl IpcResponse {
    pub fn ok() -> Self {
        Self {
            success: true,
            message: None,
        }
    }

    pub fn err(message: impl Into<String>) -> Self {
        Self {
            success: false,
            message: Some(message.into()),
        }
    }
}

/// Validates an IPC command before the daemon acts on it. Returns `Ok(())`
/// when the command is well-formed, or a human-readable reason when it must
/// be rejected. This runs before any filesystem, GPU, or theme work.
pub fn validate_command(cmd: &IpcCommand) -> Result<(), String> {
    fn check_monitor(monitor: &Option<String>) -> Result<(), String> {
        if let Some(name) = monitor {
            if name.is_empty() {
                return Err("monitor name cannot be empty".to_string());
            }
            if name.len() > MAX_MONITOR_LEN {
                return Err(format!("monitor name exceeds {MAX_MONITOR_LEN} bytes"));
            }
            if name.bytes().any(|b| b < 0x20 || b == 0x7f) {
                return Err("monitor name contains control characters".to_string());
            }
            if name.contains('\n') || name.contains('\0') {
                return Err("monitor name contains invalid characters".to_string());
            }
        }
        Ok(())
    }

    fn check_duration(duration_ms: &Option<u32>) -> Result<(), String> {
        if let Some(ms) = duration_ms
            && *ms > MAX_DURATION_MS
        {
            return Err(format!(
                "duration {ms}ms exceeds maximum {MAX_DURATION_MS}ms"
            ));
        }
        Ok(())
    }

    fn check_effect(effect: &Option<crate::effect::Effect>) -> Result<(), String> {
        if let Some(effect) = effect {
            for value in effect_param_values(effect) {
                if !value.is_finite() {
                    return Err("effect parameter must be finite".to_string());
                }
                if value.abs() > 1_000_000.0 {
                    return Err("effect parameter out of range".to_string());
                }
            }
        }
        Ok(())
    }

    fn check_path(path: &str) -> Result<(), String> {
        if path.is_empty() {
            return Err("wallpaper path cannot be empty".to_string());
        }
        if path.len() > MAX_IPC_PATH_LEN {
            return Err(format!("wallpaper path exceeds {MAX_IPC_PATH_LEN} bytes"));
        }
        if path.bytes().any(|b| b == 0) {
            return Err("wallpaper path contains NUL".to_string());
        }
        Ok(())
    }

    match cmd {
        IpcCommand::Pause { monitor } | IpcCommand::Resume { monitor } => check_monitor(monitor),
        IpcCommand::Reload
        | IpcCommand::Stop
        | IpcCommand::Status
        | IpcCommand::MonitorList
        | IpcCommand::MonitorCurrent => Ok(()),
        IpcCommand::Seek {
            timestamp_ms,
            monitor,
        } => {
            check_monitor(monitor)?;
            if *timestamp_ms > 24 * 3600 * 1000 {
                return Err("seek timestamp exceeds 24h".to_string());
            }
            Ok(())
        }
        IpcCommand::Preview {
            path,
            effect,
            duration_ms,
            fps,
            monitor,
            ..
        } => {
            check_path(path)?;
            check_monitor(monitor)?;
            check_duration(duration_ms)?;
            check_effect(effect)?;
            if let Some(fps) = fps
                && (*fps == 0 || *fps > MAX_VIDEO_FPS)
            {
                return Err(format!("video fps must be between 1 and {MAX_VIDEO_FPS}"));
            }
            Ok(())
        }
        IpcCommand::Info { monitor } => check_monitor(monitor),
        IpcCommand::Blank {
            monitor,
            effect,
            duration_ms,
        }
        | IpcCommand::Restore {
            monitor,
            effect,
            duration_ms,
        } => {
            check_monitor(monitor)?;
            check_duration(duration_ms)?;
            check_effect(effect)?;
            Ok(())
        }
    }
}

fn effect_param_values(effect: &crate::effect::Effect) -> Vec<f32> {
    use crate::effect::{Effect, Origin};

    fn origin_values(origin: &Origin, values: &mut Vec<f32>) {
        if let Origin::Custom(x, y) = origin {
            values.push(*x);
            values.push(*y);
        }
    }

    let mut values = Vec::new();
    match effect {
        Effect::Fade(p) => values.extend([p.from, p.to]),
        Effect::Wipe(p) => {
            values.push(p.softness);
            if let Some(angle) = p.angle {
                values.push(angle);
            }
        }
        Effect::Slide(_) => {}
        Effect::Wave(p) => {
            values.extend([p.frequency, p.amplitude]);
            if let Some(angle) = p.angle {
                values.push(angle);
            }
            origin_values(&p.origin, &mut values);
        }
        Effect::Grow(p) => origin_values(&p.origin, &mut values),
        Effect::Outer(p) => origin_values(&p.origin, &mut values),
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preview(path: &str) -> IpcCommand {
        IpcCommand::Preview {
            path: path.to_string(),
            effect: None,
            duration_ms: None,
            fps: None,
            no_theme: true,
            theme_override: None,
            monitor: None,
            scaling_mode: None,
        }
    }

    #[test]
    fn accepts_well_formed_commands() {
        assert!(validate_command(&preview("/tmp/wall.jpg")).is_ok());
        assert!(validate_command(&IpcCommand::Status).is_ok());
        assert!(
            validate_command(&IpcCommand::Seek {
                timestamp_ms: 1000,
                monitor: Some("DP-1".to_string()),
            })
            .is_ok()
        );
    }

    #[test]
    fn rejects_empty_path_and_oversized_requests() {
        assert!(validate_command(&preview("")).is_err());
        assert!(validate_command(&preview(&"a".repeat(MAX_IPC_PATH_LEN + 1))).is_err());
        assert!(
            validate_command(&IpcCommand::Preview {
                path: "/tmp/w.jpg".to_string(),
                effect: None,
                duration_ms: Some(MAX_DURATION_MS + 1),
                fps: None,
                no_theme: true,
                theme_override: None,
                monitor: None,
                scaling_mode: None,
            })
            .is_err()
        );
    }

    #[test]
    fn rejects_bad_monitor_names() {
        assert!(
            validate_command(&IpcCommand::Pause {
                monitor: Some("".to_string())
            })
            .is_err()
        );
        assert!(
            validate_command(&IpcCommand::Pause {
                monitor: Some("a".repeat(MAX_MONITOR_LEN + 1))
            })
            .is_err()
        );
        assert!(
            validate_command(&IpcCommand::Pause {
                monitor: Some("DP-1\n".to_string())
            })
            .is_err()
        );
        // Normal output names are accepted.
        for name in ["DP-1", "HDMI-A-1", "eDP-1", "output-42"] {
            assert!(
                validate_command(&IpcCommand::Pause {
                    monitor: Some(name.to_string())
                })
                .is_ok(),
                "should accept {name}"
            );
        }
    }

    #[test]
    fn rejects_non_finite_effect_params() {
        let effect = crate::effect::Effect::Fade(crate::effect::FadeParams {
            from: f32::NAN,
            to: 1.0,
            easing: crate::effect::Easing::Linear,
        });
        assert!(
            validate_command(&IpcCommand::Preview {
                path: "/tmp/w.jpg".to_string(),
                effect: Some(effect),
                duration_ms: None,
                fps: None,
                no_theme: true,
                theme_override: None,
                monitor: None,
                scaling_mode: None,
            })
            .is_err()
        );
    }

    #[test]
    fn rejects_non_finite_effect_origins() {
        for origin in [
            crate::effect::Origin::Custom(f32::NAN, 0.5),
            crate::effect::Origin::Custom(0.5, f32::INFINITY),
        ] {
            let effect = crate::effect::Effect::Grow(crate::effect::GrowParams {
                origin,
                easing: crate::effect::Easing::Linear,
            });
            assert!(
                validate_command(&IpcCommand::Preview {
                    path: "/tmp/w.jpg".to_string(),
                    effect: Some(effect),
                    duration_ms: None,
                    fps: None,
                    no_theme: true,
                    theme_override: None,
                    monitor: None,
                    scaling_mode: None,
                })
                .is_err()
            );
        }
    }

    #[test]
    fn rejects_excessive_seek() {
        assert!(
            validate_command(&IpcCommand::Seek {
                timestamp_ms: 25 * 3600 * 1000,
                monitor: None,
            })
            .is_err()
        );
    }
}
