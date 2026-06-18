//! 跨平台进程辅助：屏蔽 sh/cmd 与进程组差异，保证 Linux/macOS/Windows 行为一致。
//!
//! 纯 Rust（零 Tauri 依赖），符合「后端独立化」约束。业务代码不要再直接
//! `Command::new("sh")` 或 `.process_group(0)`——前者在 Windows 不存在，后者是
//! unix-only API 会导致 Windows 直接编译失败。统一走本模块的两个出口。

use tokio::process::Command;

/// 通过平台默认 shell 执行脚本字符串。
///
/// - unix（Linux/macOS）：`sh -lc <script>`（`-l` 走登录 shell，加载用户 PATH）
/// - windows：`cmd /C <script>`
///
/// 返回未 spawn 的 [`Command`]，调用方可继续链式设置 `current_dir`/`env`/stdio。
pub fn shell(script: &str) -> Command {
    #[cfg(windows)]
    {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(script);
        cmd
    }
    #[cfg(not(windows))]
    {
        let mut cmd = Command::new("sh");
        cmd.arg("-lc").arg(script);
        cmd
    }
}

/// 让子进程独立成新进程组，便于整树终止（避免只杀 shell 留下 npm/node 残留），
/// 在 unix 上同时隔离子进程对父进程的信号传递。
///
/// - unix：`setpgid(0, 0)`（进程组 id = 子进程 pid，可对 `-pid` 整组发信号）
/// - windows：`CREATE_NEW_PROCESS_GROUP` creation flag
///
/// 必须在 `spawn()` 之前调用。
pub fn detach_process_group(cmd: &mut Command) {
    #[cfg(unix)]
    {
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        // winbase.h: CREATE_NEW_PROCESS_GROUP. `creation_flags` is an inherent
        // method on tokio's Command under cfg(windows) (mirrors unix
        // `process_group`), so no std `CommandExt` trait import is needed.
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP);
    }
}

/// 在 `PATH` 中查找可执行程序，返回首个匹配的完整路径。
///
/// 关键跨平台差异：Windows 的 `PATH` 用 `;` 分隔（unix 用 `:`），且可执行文件
/// 带 `PATHEXT` 后缀（`.EXE`/`.CMD`/`.BAT`…）。`std::env::split_paths` 自动按平台
/// 选分隔符；Windows 上额外尝试 `PATHEXT` 后缀，从而能定位 `npm.cmd`/`claude.cmd`
/// 这类 `Command::new` 无法直接解析的 shim（CreateProcess 仅补 `.exe`）。
pub fn find_executable(name: &str) -> Option<std::path::PathBuf> {
    use std::path::{Path, PathBuf};

    // Windows：候选后缀（含空串表示名字本身可能已带扩展名）。unix：仅原名。
    #[cfg(windows)]
    let exts: Vec<String> = {
        let mut v: Vec<String> = vec![String::new()];
        v.extend(
            std::env::var("PATHEXT")
                .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
                .split(';')
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string()),
        );
        v
    };
    #[cfg(not(windows))]
    let exts: Vec<String> = vec![String::new()];

    let probe = |base: &Path| -> Option<PathBuf> {
        for ext in &exts {
            let cand = if ext.is_empty() {
                base.to_path_buf()
            } else {
                let mut s = base.as_os_str().to_owned();
                s.push(ext);
                PathBuf::from(s)
            };
            if cand.is_file() {
                return Some(cand);
            }
        }
        None
    };

    // 已含路径分隔符 → 当作相对/绝对路径，不查 PATH。
    if name.contains('/') || name.contains('\\') {
        return probe(Path::new(name));
    }

    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        if let Some(found) = probe(&dir.join(name)) {
            return Some(found);
        }
    }
    None
}

/// 程序是否在 `PATH` 中可用（跨平台，替代手写的 `split(':')` 判断）。
pub fn has_executable(name: &str) -> bool {
    find_executable(name).is_some()
}

/// 把程序名解析成可直接交给 `Command::new` 的值：能在 `PATH` 中定位则返回完整
/// 路径（解决 Windows 无法直接定位 `npm.cmd`/`claude.cmd` 等 shim 的问题），
/// 否则原样返回交给系统在 spawn 时解析（与改造前行为一致，无回归）。
pub fn program(name: &str) -> std::ffi::OsString {
    find_executable(name)
        .map(|p| p.into_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_real_binary_on_path() {
        // `cargo` is always present in the test environment (real .exe on Windows).
        assert!(has_executable("cargo"), "cargo should be discoverable on PATH");
        let p = find_executable("cargo").expect("cargo path");
        assert!(p.is_file());
    }

    #[test]
    fn missing_binary_is_none_but_program_falls_back_to_name() {
        let bogus = "definitely-not-a-real-binary-xyz-123";
        assert!(!has_executable(bogus));
        assert!(find_executable(bogus).is_none());
        // program() never loses the name — keeps prior behavior (system resolves at spawn).
        assert_eq!(program(bogus), std::ffi::OsString::from(bogus));
    }

    #[test]
    fn shell_wraps_script_per_platform() {
        let c = shell("echo hi");
        let prog = c.as_std().get_program().to_string_lossy().to_string();
        #[cfg(windows)]
        assert_eq!(prog, "cmd");
        #[cfg(not(windows))]
        assert_eq!(prog, "sh");
    }
}
