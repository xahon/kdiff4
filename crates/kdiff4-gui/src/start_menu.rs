use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use thiserror::Error;

const SHORTCUT_NAME: &str = "Kdiff4.lnk";
const SHORTCUT_ENV: &str = "KDIFF4_SHORTCUT";
const TARGET_ENV: &str = "KDIFF4_TARGET";
const WORKING_DIR_ENV: &str = "KDIFF4_WORKING_DIR";
const SCRIPT: &str = "$ErrorActionPreference = 'Stop'; $link = (New-Object -ComObject WScript.Shell).CreateShortcut($env:KDIFF4_SHORTCUT); $link.TargetPath = $env:KDIFF4_TARGET; $link.WorkingDirectory = $env:KDIFF4_WORKING_DIR; $link.Description = 'Kdiff4: diff and merge tool'; $link.Save()";

#[derive(Debug, Error)]
pub enum ShortcutError {
    #[error("Start menu shortcuts are only supported on Windows")]
    Unsupported,
    #[error("cannot find the Start menu folder of this user")]
    NoStartMenu,
    #[error("cannot find the Kdiff4 executable: {0}")]
    NoExecutable(std::io::Error),
    #[error("cannot create {}: {source}", path.display())]
    Folder { path: PathBuf, source: std::io::Error },
    #[error("cannot run PowerShell: {0}")]
    Launch(std::io::Error),
    #[error("PowerShell failed to create the shortcut: {0}")]
    Script(String),
    #[error("PowerShell finished but {} does not exist", path.display())]
    Missing { path: PathBuf },
    #[error("the shortcut creation stopped before it finished")]
    Stopped,
}


/// `%APPDATA%\Microsoft\Windows\Start Menu\Programs`: the current user's Start menu, writable without administrator rights.
pub fn programs_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("Microsoft").join("Windows").join("Start Menu").join("Programs"))
}


pub fn shortcut_path(programs_dir: &Path) -> PathBuf {
    programs_dir.join(SHORTCUT_NAME)
}


/// The PowerShell call that writes `link` pointing at `target`. Paths go through environment variables, so no quoting of them is needed.
fn powershell_command(link: &Path, target: &Path) -> Command {
    let working_dir = target.parent().unwrap_or(Path::new(""));
    let mut command = Command::new("powershell");
    command.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", SCRIPT]);
    command.env(SHORTCUT_ENV, link).env(TARGET_ENV, target).env(WORKING_DIR_ENV, working_dir);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}


/// Writes (or overwrites) the shortcut in `programs_dir` pointing at `target`, and returns its path.
pub fn create_shortcut(programs_dir: &Path, target: &Path) -> Result<PathBuf, ShortcutError> {
    if !cfg!(windows) {
        return Err(ShortcutError::Unsupported);
    }
    std::fs::create_dir_all(programs_dir).map_err(|source| ShortcutError::Folder { path: programs_dir.to_path_buf(), source })?;
    let link = shortcut_path(programs_dir);
    let output = powershell_command(&link, target).output().map_err(ShortcutError::Launch)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let message = if stderr.is_empty() { output.status.to_string() } else { stderr };
        return Err(ShortcutError::Script(message));
    }
    if !link.is_file() {
        return Err(ShortcutError::Missing { path: link });
    }
    Ok(link)
}


/// The shortcut for the running executable in the current user's Start menu.
pub fn install_for_current_exe() -> Result<PathBuf, ShortcutError> {
    let programs_dir = programs_dir().ok_or(ShortcutError::NoStartMenu)?;
    let exe = std::env::current_exe().map_err(ShortcutError::NoExecutable)?;
    create_shortcut(&programs_dir, &exe)
}


/// An installation running on its own thread, so the window never waits for PowerShell.
pub struct ShortcutInstall {
    receiver: Receiver<Result<PathBuf, ShortcutError>>,
}

impl ShortcutInstall {
    /// `on_done` runs on the worker thread when the result is ready, to wake the window.
    pub fn start(on_done: impl FnOnce() + Send + 'static) -> ShortcutInstall {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(install_for_current_exe());
            on_done();
        });
        ShortcutInstall { receiver }
    }

    pub fn poll(&self) -> Option<Result<PathBuf, ShortcutError>> {
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(ShortcutError::Stopped)),
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shortcut_is_named_after_the_program() {
        assert_eq!(shortcut_path(Path::new("programs")), Path::new("programs").join("Kdiff4.lnk"));
    }

    #[test]
    fn the_start_menu_is_under_the_roaming_app_data() {
        let Some(dir) = programs_dir() else {
            return;
        };
        assert!(dir.ends_with(Path::new("Microsoft").join("Windows").join("Start Menu").join("Programs")));
    }

    #[test]
    fn paths_reach_powershell_through_the_environment() {
        let command = powershell_command(Path::new("C:\\menu\\Kdiff4.lnk"), Path::new("C:\\app's dir\\kdiff4.exe"));
        let envs: Vec<_> = command.get_envs().map(|(key, value)| (key.to_os_string(), value.map(|value| value.to_os_string()))).collect();
        assert!(envs.contains(&(SHORTCUT_ENV.into(), Some("C:\\menu\\Kdiff4.lnk".into()))));
        assert!(envs.contains(&(TARGET_ENV.into(), Some("C:\\app's dir\\kdiff4.exe".into()))));
        assert!(envs.contains(&(WORKING_DIR_ENV.into(), Some("C:\\app's dir".into()))));
        assert!(command.get_args().all(|arg| !arg.to_string_lossy().contains("app's dir")));
    }

    #[cfg(windows)]
    #[test]
    fn a_shortcut_is_written_into_the_given_folder_and_replaced_on_reinstall() {
        let dir = std::env::temp_dir().join(format!("kdiff4-start-menu-test-{}", std::process::id()));
        let programs = dir.join("Start Menu's Programs");
        let exe = std::env::current_exe().expect("Test: current exe");
        let link = create_shortcut(&programs, &exe).expect("Test: first install");
        assert_eq!(link, programs.join("Kdiff4.lnk"));
        assert!(link.is_file());
        let again = create_shortcut(&programs, &exe).expect("Test: second install");
        assert_eq!(again, link);
        std::fs::remove_dir_all(&dir).expect("Test: cleanup");
    }

    #[cfg(not(windows))]
    #[test]
    fn other_platforms_are_refused() {
        assert!(matches!(create_shortcut(Path::new("programs"), Path::new("kdiff4")), Err(ShortcutError::Unsupported)));
    }
}
