use std::ffi::OsString;
use std::fs;
use std::os::unix::{ffi::OsStringExt, fs::MetadataExt};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub(super) struct DesktopProcess {
    pub pid: u32,
    pub executable: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: Option<PathBuf>,
    pub is_main: bool,
    start_time: String,
}

impl DesktopProcess {
    pub fn is_current(&self) -> bool {
        let root = PathBuf::from(format!("/proc/{}", self.pid));
        fs::read_link(root.join("exe")).ok().as_ref() == Some(&self.executable)
            && fs::read_to_string(root.join("stat"))
                .ok()
                .and_then(|stat| start_time(&stat).map(str::to_owned))
                .as_ref()
                == Some(&self.start_time)
    }
}

// Require Electron assets next to the binary: Codex CLI and app-server have
// the same executable name but must never be treated as the desktop process.
pub(super) fn is_desktop_executable(path: &Path) -> bool {
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    if !name.eq_ignore_ascii_case("chatgpt") && !name.eq_ignore_ascii_case("codex") {
        return false;
    }
    let Some(parent) = path.parent() else {
        return false;
    };
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.mode() & 0o111 != 0)
        && parent.join("chrome_100_percent.pak").is_file()
        && (parent.join("resources/app.asar").is_file()
            || parent.join("resources/app/package.json").is_file())
}

fn start_time(stat: &str) -> Option<&str> {
    // comm is parenthesized and may itself contain spaces or parentheses.
    stat.rsplit_once(") ")?.1.split_whitespace().nth(19)
}

fn arguments(cmdline: &[u8]) -> Vec<OsString> {
    cmdline
        .split(|b| *b == 0)
        .skip(1)
        .filter(|s| !s.is_empty())
        .map(|s| OsString::from_vec(s.to_vec()))
        .collect()
}

pub(super) fn processes() -> Result<Vec<DesktopProcess>, String> {
    let uid = fs::metadata("/proc/self").map_err(|e| e.to_string())?.uid();
    let entries = fs::read_dir("/proc").map_err(|e| format!("读取 Linux 进程失败: {e}"))?;
    let mut result = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        let root = entry.path();
        if !fs::metadata(&root).is_ok_and(|m| m.uid() == uid) {
            continue;
        }
        let Ok(executable) = fs::read_link(root.join("exe")) else {
            continue;
        };
        if !is_desktop_executable(&executable) {
            continue;
        }
        let Ok(cmdline) = fs::read(root.join("cmdline")) else {
            continue;
        };
        let Ok(stat) = fs::read_to_string(root.join("stat")) else {
            continue;
        };
        let Some(start_time) = start_time(&stat) else {
            continue;
        };
        let args = arguments(&cmdline);
        let is_main = !args
            .iter()
            .any(|arg| arg.to_string_lossy().starts_with("--type="));
        result.push(DesktopProcess {
            pid,
            executable,
            args,
            cwd: fs::read_link(root.join("cwd")).ok(),
            is_main,
            start_time: start_time.to_owned(),
        });
    }
    result.sort_by_key(|p| (!p.is_main, p.pid));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn arguments_preserve_spaces_and_non_utf8() {
        assert_eq!(
            arguments(b"/opt/Chat GPT/ChatGPT\0--user-data-dir=/tmp/my profile\0\xff\0"),
            vec![
                OsString::from("--user-data-dir=/tmp/my profile"),
                OsString::from_vec(vec![255])
            ]
        );
    }

    #[test]
    fn stat_handles_spaces_and_parentheses_in_comm() {
        let stat = format!(
            "123 (Chat GPT)) S {}",
            (4..=22)
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        );
        assert_eq!(start_time(&stat), Some("22"));
        assert_eq!(start_time("invalid"), None);
    }

    #[test]
    fn desktop_assets_distinguish_gui_from_cli() {
        let root = std::env::temp_dir().join(format!("ccc-desktop-test-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let executable = root.join("codex");
        fs::write(&executable, b"fixture").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!is_desktop_executable(&executable));
        fs::create_dir(root.join("resources")).unwrap();
        fs::write(root.join("chrome_100_percent.pak"), b"").unwrap();
        fs::write(root.join("resources/app.asar"), b"").unwrap();
        assert!(is_desktop_executable(&executable));
        let cli = root.join("resources/codex");
        fs::copy(&executable, &cli).unwrap();
        assert!(!is_desktop_executable(&cli));
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!is_desktop_executable(&executable));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn live_snapshot_has_absolute_paths_and_excludes_cli() {
        for p in processes().unwrap() {
            assert!(p.executable.is_absolute());
            assert!(is_desktop_executable(&p.executable));
            assert!(!p.args.iter().any(|arg| arg == "app-server"));
        }
    }
}
