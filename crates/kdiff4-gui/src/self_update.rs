use std::ffi::OsString;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use crate::update_check::UpdateError;

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const MAX_DOWNLOAD_BYTES: u64 = 512 * 1024 * 1024;
const CHUNK_BYTES: usize = 64 * 1024;


/// `<name>.new` is where the download lands; `<name>.old` is the running executable moved aside during the swap.
pub fn staging_paths(exe: &Path) -> (PathBuf, PathBuf) {
    (with_suffix(exe, ".new"), with_suffix(exe, ".old"))
}


fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}


/// Removes the executable a previous update moved aside. Windows can't delete a running executable, so this runs at the next start.
pub fn clean_up_previous_update() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let (new, old) = staging_paths(&exe);
    for leftover in [old, new] {
        if leftover.exists() {
            match fs::remove_file(&leftover) {
                Ok(()) => log::info!("removed {}", leftover.display()),
                Err(error) => log::warn!("cannot remove {}: {error}", leftover.display()),
            }
        }
    }
}


fn has_executable_magic(head: &[u8]) -> bool {
    if cfg!(windows) {
        head.starts_with(b"MZ")
    } else if cfg!(target_os = "macos") {
        head.starts_with(&[0xCF, 0xFA, 0xED, 0xFE]) || head.starts_with(&[0xCA, 0xFE, 0xBA, 0xBE])
    } else {
        head.starts_with(b"\x7FELF")
    }
}


/// Streams `url` into `dest`, then checks that the file is complete and looks like an executable of this platform. A failed download leaves no file behind.
pub fn download_to(url: &str, dest: &Path, expected_size: u64, downloaded: &AtomicU64, total: &AtomicU64, cancel: &AtomicBool) -> Result<(), UpdateError> {
    let result = stream_to_file(url, dest, expected_size, downloaded, total, cancel);
    if result.is_err() {
        let _ = fs::remove_file(dest);
    }
    result
}


fn stream_to_file(url: &str, dest: &Path, expected_size: u64, downloaded: &AtomicU64, total: &AtomicU64, cancel: &AtomicBool) -> Result<(), UpdateError> {
    let download_error = |error: &dyn std::fmt::Display| UpdateError::Download(error.to_string());
    if cancel.load(Ordering::Relaxed) {
        return Err(UpdateError::Canceled);
    }
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(DOWNLOAD_TIMEOUT))
        .user_agent(format!("kdiff4/{}", crate::update_check::current_version()))
        .build();
    let agent: ureq::Agent = config.into();
    let mut response = agent.get(url).call().map_err(|error| download_error(&error))?;
    let announced = response.body().content_length().unwrap_or(expected_size);
    total.store(announced, Ordering::Relaxed);
    downloaded.store(0, Ordering::Relaxed);
    let mut reader = response.body_mut().with_config().limit(MAX_DOWNLOAD_BYTES).reader();
    let mut file = fs::File::create(dest).map_err(|error| download_error(&error))?;
    let mut buffer = vec![0u8; CHUNK_BYTES];
    let mut head = Vec::new();
    let mut received = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(UpdateError::Canceled);
        }
        let read = reader.read(&mut buffer).map_err(|error| download_error(&error))?;
        if read == 0 {
            break;
        }
        if head.len() < 4 {
            head.extend_from_slice(&buffer[..read.min(4 - head.len())]);
        }
        file.write_all(&buffer[..read]).map_err(|error| download_error(&error))?;
        received += read as u64;
        downloaded.store(received, Ordering::Relaxed);
    }
    file.flush().map_err(|error| download_error(&error))?;
    drop(file);
    if received == 0 || (announced > 0 && received != announced) {
        return Err(UpdateError::Download(format!("received {received} of {announced} bytes")));
    }
    if !has_executable_magic(&head) {
        return Err(UpdateError::Download("the file is not an executable for this platform".to_string()));
    }
    Ok(())
}


/// Puts `new` at `exe`'s path and moves the old executable to `<name>.old`. If the second move fails the old executable is put back.
pub fn swap_executable(exe: &Path, new: &Path) -> Result<(), UpdateError> {
    let install_error = |what: &str, error: std::io::Error| UpdateError::Install(format!("{what}: {error}"));
    let (_, old) = staging_paths(exe);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(new, fs::Permissions::from_mode(0o755)).map_err(|error| install_error("cannot make the download executable", error))?;
    }
    if old.exists() {
        fs::remove_file(&old).map_err(|error| install_error(&format!("cannot remove {}", old.display()), error))?;
    }
    fs::rename(exe, &old).map_err(|error| install_error(&format!("cannot move {} aside (is the folder writable?)", exe.display()), error))?;
    if let Err(error) = fs::rename(new, exe) {
        let _ = fs::rename(&old, exe);
        return Err(install_error(&format!("cannot put the new version at {}", exe.display()), error));
    }
    Ok(())
}


pub fn relaunch(exe: &Path, args: Vec<OsString>) -> Result<(), UpdateError> {
    std::process::Command::new(exe)
        .args(args)
        .spawn()
        .map(|_| ())
        .map_err(|error| UpdateError::Install(format!("cannot start {}: {error}", exe.display())))
}


/// A download and install running on its own thread. The window shows its progress and does nothing else until it ends.
pub struct UpdateInstall {
    /// Where the running executable was when the install started; the new version is at this path afterwards.
    pub exe: PathBuf,
    pub downloaded: Arc<AtomicU64>,
    pub total: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
    receiver: Receiver<Result<(), UpdateError>>,
}

impl UpdateInstall {
    /// `expected_size` is the asset size from the release, used until the server announces its own. `on_done` wakes the window.
    pub fn start(url: String, expected_size: u64, on_done: impl FnOnce() + Send + 'static) -> Result<UpdateInstall, UpdateError> {
        let exe = std::env::current_exe().map_err(|error| UpdateError::Install(format!("cannot find the running executable: {error}")))?;
        let downloaded = Arc::new(AtomicU64::new(0));
        let total = Arc::new(AtomicU64::new(expected_size));
        let cancel = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        let worker = (downloaded.clone(), total.clone(), cancel.clone());
        let installed_exe = exe.clone();
        std::thread::spawn(move || {
            let (downloaded, total, cancel) = worker;
            let (new, _) = staging_paths(&exe);
            let result = download_to(&url, &new, expected_size, &downloaded, &total, &cancel).and_then(|()| swap_executable(&exe, &new));
            if result.is_err() {
                let _ = fs::remove_file(&new);
            }
            let _ = sender.send(result);
            on_done();
        });
        Ok(UpdateInstall { exe: installed_exe, downloaded, total, cancel, receiver })
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Fraction downloaded, or `None` while the size is unknown.
    pub fn fraction(&self) -> Option<f32> {
        let total = self.total.load(Ordering::Relaxed);
        (total > 0).then(|| (self.downloaded.load(Ordering::Relaxed) as f32 / total as f32).clamp(0.0, 1.0))
    }

    pub fn poll(&self) -> Option<Result<(), UpdateError>> {
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(UpdateError::Stopped)),
        }
    }
}


#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kdiff4-self-update-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn executable_bytes(len: usize) -> Vec<u8> {
        let mut bytes = vec![7u8; len];
        let magic: &[u8] = if cfg!(windows) {
            b"MZ"
        } else if cfg!(target_os = "macos") {
            &[0xCF, 0xFA, 0xED, 0xFE]
        } else {
            b"\x7FELF"
        };
        bytes[..magic.len()].copy_from_slice(magic);
        bytes
    }

    /// Serves `body` once, announcing `announced_length` bytes: fewer sent than announced gives a cut-off download.
    fn serve_once(body: Vec<u8>, announced_length: usize) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                request.push(byte[0]);
            }
            let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {announced_length}\r\nConnection: close\r\n\r\n");
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&body);
        });
        url
    }

    fn counters() -> (AtomicU64, AtomicU64, AtomicBool) {
        (AtomicU64::new(0), AtomicU64::new(0), AtomicBool::new(false))
    }

    #[test]
    fn staging_paths_sit_next_to_the_executable() {
        let (new, old) = staging_paths(Path::new("C:/tools/kdiff4.exe"));
        assert_eq!(new, PathBuf::from("C:/tools/kdiff4.exe.new"));
        assert_eq!(old, PathBuf::from("C:/tools/kdiff4.exe.old"));
    }

    #[test]
    fn download_saves_the_file_and_reports_progress() {
        let dir = temp_dir("download");
        let bytes = executable_bytes(200_000);
        let url = serve_once(bytes.clone(), bytes.len());
        let dest = dir.join("kdiff4.new");
        let (downloaded, total, cancel) = counters();
        download_to(&url, &dest, 0, &downloaded, &total, &cancel).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), bytes);
        assert_eq!(downloaded.load(Ordering::Relaxed), bytes.len() as u64);
        assert_eq!(total.load(Ordering::Relaxed), bytes.len() as u64);
    }

    #[test]
    fn cut_off_download_fails_and_leaves_no_file() {
        let dir = temp_dir("cut-off");
        let bytes = executable_bytes(1000);
        let url = serve_once(bytes, 5000);
        let dest = dir.join("kdiff4.new");
        let (downloaded, total, cancel) = counters();
        assert!(matches!(download_to(&url, &dest, 0, &downloaded, &total, &cancel), Err(UpdateError::Download(_))));
        assert!(!dest.exists());
    }

    #[test]
    fn download_that_is_not_an_executable_is_rejected() {
        let dir = temp_dir("not-executable");
        let bytes = b"<html>404</html>".to_vec();
        let url = serve_once(bytes.clone(), bytes.len());
        let dest = dir.join("kdiff4.new");
        let (downloaded, total, cancel) = counters();
        assert!(matches!(download_to(&url, &dest, 0, &downloaded, &total, &cancel), Err(UpdateError::Download(_))));
        assert!(!dest.exists());
    }

    #[test]
    fn canceled_download_stops_before_writing() {
        let dir = temp_dir("canceled");
        let bytes = executable_bytes(1000);
        let url = serve_once(bytes.clone(), bytes.len());
        let dest = dir.join("kdiff4.new");
        let (downloaded, total, cancel) = counters();
        cancel.store(true, Ordering::Relaxed);
        assert!(matches!(download_to(&url, &dest, 0, &downloaded, &total, &cancel), Err(UpdateError::Canceled)));
        assert!(!dest.exists());
    }

    #[test]
    fn swap_puts_the_new_file_at_the_old_path_and_keeps_the_old_one_aside() {
        let dir = temp_dir("swap");
        let exe = dir.join("kdiff4.exe");
        let (new, old) = staging_paths(&exe);
        fs::write(&exe, b"old build").unwrap();
        fs::write(&new, b"new build").unwrap();
        swap_executable(&exe, &new).unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"new build");
        assert_eq!(fs::read(&old).unwrap(), b"old build");
        assert!(!new.exists());
    }

    #[test]
    fn swap_replaces_a_leftover_old_file() {
        let dir = temp_dir("swap-leftover");
        let exe = dir.join("kdiff4.exe");
        let (new, old) = staging_paths(&exe);
        fs::write(&exe, b"current").unwrap();
        fs::write(&old, b"ancient").unwrap();
        fs::write(&new, b"newest").unwrap();
        swap_executable(&exe, &new).unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"newest");
        assert_eq!(fs::read(&old).unwrap(), b"current");
    }

    #[test]
    fn failed_swap_restores_the_old_executable() {
        let dir = temp_dir("swap-rollback");
        let exe = dir.join("kdiff4.exe");
        let (new, old) = staging_paths(&exe);
        fs::write(&exe, b"current").unwrap();
        assert!(matches!(swap_executable(&exe, &new), Err(UpdateError::Install(_))));
        assert_eq!(fs::read(&exe).unwrap(), b"current");
        assert!(!old.exists());
    }

    #[test]
    fn executable_magic_matches_this_platform() {
        assert!(has_executable_magic(&executable_bytes(16)[..4]));
        assert!(!has_executable_magic(b"<htm"));
        assert!(!has_executable_magic(b""));
    }

    #[test]
    fn fraction_is_unknown_until_a_size_is_known() {
        let install = UpdateInstall {
            exe: PathBuf::new(),
            downloaded: Arc::new(AtomicU64::new(25)),
            total: Arc::new(AtomicU64::new(0)),
            cancel: Arc::new(AtomicBool::new(false)),
            receiver: mpsc::channel().1,
        };
        assert_eq!(install.fraction(), None);
        install.total.store(100, Ordering::Relaxed);
        assert_eq!(install.fraction(), Some(0.25));
    }
}
