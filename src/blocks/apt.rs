//! Pending updates available for your Debian/Ubuntu based system
//!
//! Behind the scenes this uses `apt`, and in order to run it without root privileges i3status-rust will create its own package database in `/tmp/i3rs-apt/` which may take up several MB or more. If you have a custom apt config then this block may not work as expected - in that case please open an issue.
//!
//! Tip: You can grab the list of available updates using `APT_CONFIG=/tmp/i3rs-apt/apt.conf apt list --upgradable`
//!
//! # Configuration
//!
//! Key | Values | Default
//! ----|--------|--------
//! `interval` | Update interval in seconds. | `600`
//! `format` | A string to customise the output of this block. See below for available placeholders. | `"$count.eng(1)"`
//! `format_singular` | Same as `format`, but for when exactly one update is available. | `"$count.eng(1)"`
//! `format_up_to_date` | Same as `format`, but for when no updates are available. | `"$count.eng(1)"`
//! `warning_updates_regex` | Display block as warning if updates matching regex are available. | `None`
//! `critical_updates_regex` | Display block as critical if updates matching regex are available. | `None`
//! `hide_when_uptodate` | Hides the block when there are no updates available | `false`
//!
//! Key | Value | Type | Unit
//! ----|-------|------|------
//! `count` | Number of updates available | Number | -
//!
//! # Example
//!
//! Update the list of pending updates every thirty minutes (1800 seconds):
//!
//! ```toml
//! [[block]]
//! block = "apt"
//! interval = 1800
//! format = "$count updates available"
//! format_singular = "One update available"
//! format_up_to_date = "system up to date"
//! critical_updates_regex = "(linux|linux-lts|linux-zen)"
//! [[block.click]]
//! # shows dmenu with cached available updates. Any dmenu alternative should also work.
//! button = "left"
//! cmd = "APT_CONFIG=/tmp/i3rs-apt/apt.conf apt list --upgradable | tail -n +2 | rofi -dmenu"
//! ```
//!
//! # Icons Used
//!
//! - `update`

use std::env;

use regex::Regex;

use tokio::fs::{create_dir_all, File};
use tokio::process::Command;

use super::prelude::*;

#[derive(Deserialize, Debug, SmartDefault)]
#[serde(deny_unknown_fields, default)]
struct AptConfig {
    #[default(600.into())]
    interval: Seconds,
    format: FormatConfig,
    format_singular: FormatConfig,
    format_up_to_date: FormatConfig,
    warning_updates_regex: Option<String>,
    critical_updates_regex: Option<String>,
    hide_when_uptodate: bool,
}

pub async fn run(config: toml::Value, mut api: CommonApi) -> Result<()> {
    let config = AptConfig::deserialize(config).config_error()?;
    let mut widget = api.new_widget().with_icon("update")?;

    let format = config.format.with_default("$count.eng(1)")?;
    let format_singular = config.format_singular.with_default("$count.eng(1)")?;
    let format_up_to_date = config.format_up_to_date.with_default("$count.eng(1)")?;

    let warning_updates_regex = config
        .warning_updates_regex
        .as_deref()
        .map(Regex::new)
        .transpose()
        .error("invalid warning updates regex")?;
    let critical_updates_regex = config
        .critical_updates_regex
        .as_deref()
        .map(Regex::new)
        .transpose()
        .error("invalid critical updates regex")?;

    let mut cache_dir = env::temp_dir();
    cache_dir.push("i3rs-apt");
    if !cache_dir.exists() {
        create_dir_all(&cache_dir)
            .await
            .error("Failed to create temp dir")?;
    }

    let apt_config = format!(
        "Dir::State \"{}\";\n
             Dir::State::lists \"lists\";\n
             Dir::Cache \"{}\";\n
             Dir::Cache::srcpkgcache \"srcpkgcache.bin\";\n
             Dir::Cache::pkgcache \"pkgcache.bin\";",
        cache_dir.display(),
        cache_dir.display(),
    );

    let mut config_file = cache_dir;
    config_file.push("apt.conf");
    let mut file = File::create(&config_file)
        .await
        .error("Failed to create config file")?;
    file.write_all(apt_config.as_bytes())
        .await
        .error("Failed to write to config file")?;

    loop {
        let updates = get_updates_list(config_file.to_str().unwrap()).await?;
        let count = get_update_count(&updates);

        if count == 0 && config.hide_when_uptodate {
            api.hide().await?;
        } else {
            widget.set_format(match count {
                0 => format_up_to_date.clone(),
                1 => format_singular.clone(),
                _ => format.clone(),
            });
            widget.set_values(map!("count" => Value::number(count)));

            let warning = warning_updates_regex
                .as_ref()
                .map_or(false, |regex| has_matching_update(&updates, regex));
            let critical = critical_updates_regex
                .as_ref()
                .map_or(false, |regex| has_matching_update(&updates, regex));
            widget.state = match count {
                0 => State::Idle,
                _ => {
                    if critical {
                        State::Critical
                    } else if warning {
                        State::Warning
                    } else {
                        State::Info
                    }
                }
            };

            api.set_widget(&widget).await?;
        }

        loop {
            select! {
                _ = sleep(config.interval.0) => break,
                event = api.event() => match event {
                    UpdateRequest => break,
                    Click(click) => {
                        if click.button == MouseButton::Left {
                            break;
                        }
                    }
                }
            }
        }
    }
}

async fn get_updates_list(config_path: &str) -> Result<String> {
    Command::new("sh")
        .env("APT_CONFIG", config_path)
        .args(["-c", "apt update"])
        .spawn()
        .error("Failed to ren `apt update` command")?
        .wait()
        .await
        .error("Failed to run `apt update` command")?;
    let stdout = Command::new("sh")
        .env("APT_CONFIG", config_path)
        .args(&["-c", "apt list --upgradable"])
        .output()
        .await
        .error("Problem running apt command")?
        .stdout;
    String::from_utf8(stdout).error("apt produced non-UTF8 output")
}

fn get_update_count(updates: &str) -> usize {
    updates
        .lines()
        .filter(|line| line.contains("[upgradable"))
        .count()
}

fn has_matching_update(updates: &str, regex: &Regex) -> bool {
    updates.lines().any(|line| regex.is_match(line))
}
