//! OS daemon management: install / uninstall / status for the HTTP MCP server,
//! so Tono runs in the background and survives logout/reboot.
//!
//! Detects the OS: macOS uses a per-user **launchd** LaunchAgent
//! (`~/Library/LaunchAgents/com.tono.server.plist`, KeepAlive); Linux uses
//! a **systemd --user** unit (`~/.config/systemd/user/tono.service`,
//! Restart=on-failure). Logs land in `~/.tono/logs/`.

use std::path::PathBuf;
use std::process::Command;

const LABEL: &str = "com.tono.server";
const DEFAULT_BIND: &str = "127.0.0.1:8787";

/// Entry point for `tono service <install|uninstall|status> [--bind ADDR]
/// [--workdir DIR]`. Returns a process exit code.
pub fn run(args: &[String]) -> i32 {
    let cmd = args.first().map(|s| s.as_str());
    let bind = flag_value(args, "--bind").unwrap_or_else(|| DEFAULT_BIND.to_string());
    let workdir = flag_value(args, "--workdir")
        .map(PathBuf::from)
        .unwrap_or_else(default_workdir);
    match cmd {
        Some("install") => install(&bind, &workdir),
        Some("uninstall") => uninstall(),
        Some("status") => status(),
        _ => {
            eprintln!(
                "usage: tono service <install|uninstall|status> [--bind ADDR] [--workdir DIR]\n\
                 \n\
                 install    set up + start the background service (launchd / systemd --user)\n\
                 uninstall  stop + remove the service\n\
                 status     show whether the service is running and where logs live"
            );
            2
        }
    }
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn default_workdir() -> PathBuf {
    home()
        .map(|h| h.join(".tono").join("sounds"))
        .unwrap_or_else(|| std::env::temp_dir().join("tono"))
}

fn log_dir() -> PathBuf {
    home()
        .map(|h| h.join(".tono").join("logs"))
        .unwrap_or_else(|| std::env::temp_dir().join("tono_logs"))
}

fn current_uid() -> String {
    Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "501".to_string())
}

/// The launchd LaunchAgent plist (macOS).
fn launchd_plist(bin: &str, bind: &str, workdir: &str, logs: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{bin}</string>
        <string>--http</string>
        <string>{bind}</string>
    </array>
    <key>EnvironmentVariables</key>
    <dict>
        <key>TONO_WORKDIR</key><string>{workdir}</string>
    </dict>
    <key>RunAtLoad</key><true/>
    <key>KeepAlive</key><true/>
    <key>StandardOutPath</key><string>{logs}/tono.out.log</string>
    <key>StandardErrorPath</key><string>{logs}/tono.err.log</string>
</dict>
</plist>
"#
    )
}

/// The systemd --user unit (Linux).
fn systemd_unit(bin: &str, bind: &str, workdir: &str) -> String {
    format!(
        "[Unit]\n\
         Description=Tono MCP audio server\n\
         After=network.target\n\
         \n\
         [Service]\n\
         ExecStart={bin} --http {bind}\n\
         Environment=TONO_WORKDIR={workdir}\n\
         Restart=on-failure\n\
         RestartSec=2\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n"
    )
}

fn install(bind: &str, workdir: &std::path::Path) -> i32 {
    let Ok(bin) = std::env::current_exe() else {
        eprintln!("cannot resolve the tono binary path");
        return 1;
    };
    let bin = bin.to_string_lossy().into_owned();
    let logs = log_dir();
    let _ = std::fs::create_dir_all(&logs);
    let _ = std::fs::create_dir_all(workdir);
    let wd = workdir.to_string_lossy();

    match std::env::consts::OS {
        "macos" => {
            let Some(home) = home() else {
                eprintln!("no HOME directory");
                return 1;
            };
            let agents = home.join("Library").join("LaunchAgents");
            let _ = std::fs::create_dir_all(&agents);
            let plist_path = agents.join(format!("{LABEL}.plist"));
            let plist = launchd_plist(&bin, bind, &wd, &logs.to_string_lossy());
            if let Err(e) = std::fs::write(&plist_path, plist) {
                eprintln!("failed to write {}: {e}", plist_path.display());
                return 1;
            }
            let uid = current_uid();
            // Clear any stale registration, then bootstrap (fall back to the
            // legacy `load -w` on older macOS).
            let _ = Command::new("launchctl")
                .args(["bootout", &format!("gui/{uid}/{LABEL}")])
                .output();
            let ok = Command::new("launchctl")
                .args(["bootstrap", &format!("gui/{uid}")])
                .arg(&plist_path)
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
                || Command::new("launchctl")
                    .args(["load", "-w"])
                    .arg(&plist_path)
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);
            if !ok {
                eprintln!(
                    "launchctl failed to start the service (see logs in {})",
                    logs.display()
                );
                return 1;
            }
            println!(
                "✓ launchd service installed and started\n  label:    {LABEL}\n  plist:    {}\n  endpoint: http://{bind}/mcp\n  workdir:  {wd}\n  logs:     {}/tono.{{out,err}}.log\n\nConnect a client:\n  claude mcp add --transport http tono http://{bind}/mcp",
                plist_path.display(),
                logs.display()
            );
            0
        }
        "linux" => {
            let Some(home) = home() else {
                eprintln!("no HOME directory");
                return 1;
            };
            let unit_dir = home.join(".config").join("systemd").join("user");
            let _ = std::fs::create_dir_all(&unit_dir);
            let unit_path = unit_dir.join("tono.service");
            if let Err(e) = std::fs::write(&unit_path, systemd_unit(&bin, bind, &wd)) {
                eprintln!("failed to write {}: {e}", unit_path.display());
                return 1;
            }
            let reload = Command::new("systemctl")
                .args(["--user", "daemon-reload"])
                .status();
            let enable = Command::new("systemctl")
                .args(["--user", "enable", "--now", "tono"])
                .status();
            if !(reload.map(|s| s.success()).unwrap_or(false)
                && enable.map(|s| s.success()).unwrap_or(false))
            {
                eprintln!("systemctl failed — is this a systemd user session?");
                return 1;
            }
            println!(
                "✓ systemd --user service installed and started\n  unit:     {}\n  endpoint: http://{bind}/mcp\n  workdir:  {wd}\n  logs:     journalctl --user -u tono -f\n\nConnect a client:\n  claude mcp add --transport http tono http://{bind}/mcp",
                unit_path.display()
            );
            0
        }
        other => {
            eprintln!(
                "no native daemon support for '{other}'. Run `tono --http {bind}` under your\n\
                 service manager of choice (e.g. NSSM or Task Scheduler on Windows)."
            );
            1
        }
    }
}

fn uninstall() -> i32 {
    match std::env::consts::OS {
        "macos" => {
            let Some(home) = home() else {
                eprintln!("no HOME directory");
                return 1;
            };
            let plist_path = home
                .join("Library")
                .join("LaunchAgents")
                .join(format!("{LABEL}.plist"));
            let uid = current_uid();
            let _ = Command::new("launchctl")
                .args(["bootout", &format!("gui/{uid}/{LABEL}")])
                .output();
            let _ = std::fs::remove_file(&plist_path);
            println!(
                "✓ launchd service stopped and removed ({})",
                plist_path.display()
            );
            0
        }
        "linux" => {
            let Some(home) = home() else {
                eprintln!("no HOME directory");
                return 1;
            };
            let unit_path = home
                .join(".config")
                .join("systemd")
                .join("user")
                .join("tono.service");
            let _ = Command::new("systemctl")
                .args(["--user", "disable", "--now", "tono"])
                .output();
            let _ = std::fs::remove_file(&unit_path);
            let _ = Command::new("systemctl")
                .args(["--user", "daemon-reload"])
                .output();
            println!(
                "✓ systemd service stopped and removed ({})",
                unit_path.display()
            );
            0
        }
        other => {
            eprintln!("no native daemon support for '{other}'");
            1
        }
    }
}

fn status() -> i32 {
    let logs = log_dir();
    match std::env::consts::OS {
        "macos" => {
            let uid = current_uid();
            let out = Command::new("launchctl")
                .args(["print", &format!("gui/{uid}/{LABEL}")])
                .output();
            match out {
                Ok(o) if o.status.success() => {
                    let text = String::from_utf8_lossy(&o.stdout);
                    let state = text
                        .lines()
                        .find(|l| l.trim_start().starts_with("state ="))
                        .map(|l| l.trim().to_string())
                        .unwrap_or_else(|| "state = unknown".into());
                    println!(
                        "● {LABEL}: loaded ({state})\n  logs: {}/tono.{{out,err}}.log",
                        logs.display()
                    );
                    0
                }
                _ => {
                    println!("○ {LABEL}: not installed (run `tono service install`)");
                    1
                }
            }
        }
        "linux" => {
            let st = Command::new("systemctl")
                .args(["--user", "status", "tono", "--no-pager"])
                .status();
            st.map(|s| if s.success() { 0 } else { 1 }).unwrap_or(1)
        }
        other => {
            eprintln!("no native daemon support for '{other}'");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The daemon templates are string contracts: the binary path, --http flag,
    // env var name, and label must stay in lockstep with main.rs and the docs.
    #[test]
    fn launchd_plist_wires_the_http_server() {
        let p = launchd_plist("/usr/local/bin/tono", "127.0.0.1:9000", "/wd", "/logs");
        assert!(p.contains("<string>com.tono.server</string>"));
        assert!(p.contains("<string>--http</string>"));
        assert!(p.contains("<string>127.0.0.1:9000</string>"));
        assert!(p.contains("<key>TONO_WORKDIR</key><string>/wd</string>"));
        assert!(p.contains("/logs/tono.out.log"));
        assert!(p.contains("<key>KeepAlive</key><true/>"));
    }

    #[test]
    fn systemd_unit_wires_the_http_server() {
        let u = systemd_unit("/usr/bin/tono", "127.0.0.1:8787", "/wd");
        assert!(u.contains("ExecStart=/usr/bin/tono --http 127.0.0.1:8787"));
        assert!(u.contains("Environment=TONO_WORKDIR=/wd"));
        assert!(u.contains("Restart=on-failure"));
    }
}
