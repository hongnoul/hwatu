// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! Share sheet (roadmap H36): hand the current page URL to
//! user-configured commands, `~/.config/hwatu/share.conf`:
//!
//! ```text
//! # share.conf — <name> <command with %s for the URL>
//! mpv       mpv %s
//! yt-dlp    yt-dlp -P ~/Videos %s
//! wallabag  wallabag-add %s
//! email     xdg-email --body %s
//! ```
//!
//! The palette lists each entry as "share: <name>"; running one
//! spawns the command detached with `%s` replaced by the (shell-
//! escaped-free — no shell involved) URL. Missing file = no entries,
//! zero cost. Read per invocation, same contract as search.conf.

use std::path::PathBuf;

/// One share target.
#[derive(Debug, Clone, PartialEq)]
pub struct ShareTarget {
    pub name: String,
    /// argv template; one element contains `%s`.
    pub argv: Vec<String>,
}

fn conf_file() -> PathBuf {
    glib::user_config_dir().join("hwatu").join("share.conf")
}

/// Parse share.conf content.
pub fn parse(conf: &str) -> Vec<ShareTarget> {
    let mut out = Vec::new();
    for line in conf.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((name, command)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let argv: Vec<String> = command.split_whitespace().map(str::to_string).collect();
        // The command must reference the URL somewhere.
        if argv.is_empty() || !argv.iter().any(|a| a.contains("%s")) {
            continue;
        }
        out.push(ShareTarget {
            name: name.to_string(),
            argv,
        });
    }
    out
}

/// Load the user's share targets.
pub fn targets() -> Vec<ShareTarget> {
    std::fs::read_to_string(conf_file())
        .map(|s| parse(&s))
        .unwrap_or_default()
}

/// Run one target against `url`, detached. No shell: `%s` is replaced
/// inside argv elements, so URLs with metacharacters cannot inject.
pub fn run(target: &ShareTarget, url: &str) -> Result<(), String> {
    let argv: Vec<String> = target.argv.iter().map(|a| a.replace("%s", url)).collect();
    let (bin, args) = argv.split_first().ok_or("empty command")?;
    std::process::Command::new(bin)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                format!("{bin} not installed")
            } else {
                format!("{bin}: {e}")
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_lines_and_skips_junk() {
        let conf = "# comment\n\
                    mpv mpv %s\n\
                    ytdlp yt-dlp -P /tmp %s\n\
                    nourl echo hello\n\
                    \n\
                    justname\n";
        let targets = parse(conf);
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].name, "mpv");
        assert_eq!(targets[0].argv, vec!["mpv", "%s"]);
        assert_eq!(targets[1].argv, vec!["yt-dlp", "-P", "/tmp", "%s"]);
    }

    #[test]
    fn url_substitution_is_argv_level_no_shell() {
        let target = ShareTarget {
            name: "t".into(),
            argv: vec!["echo".into(), "%s".into()],
        };
        // A URL full of shell metacharacters is one argv element;
        // nothing can inject because no shell ever parses it.
        let evil = "https://example.com/?q=$(rm -rf /)&x=;boom";
        let argv: Vec<String> = target.argv.iter().map(|a| a.replace("%s", evil)).collect();
        assert_eq!(argv[1], evil);
    }
}
