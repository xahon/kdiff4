use std::ffi::OsString;
use std::fs::{self, File, TryLockError};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::process::{self, Child, Stdio};
use std::sync::{Arc, OnceLock, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use kdiff4_gui::{OpenCompletion, OpenDocument, OpenRequest};
use kdiff4_protocol::{Document, Labels, OpenFiles};
use serde::{Deserialize, Serialize};

pub const SERVER_ENV: &str = "KDIFF4_WINDOW_DIRECTORY";
const POLL_INTERVAL: Duration = Duration::from_millis(100);
const START_TIMEOUT: Duration = Duration::from_secs(15);
const REQUEST: &str = "request.json";
const CLAIMED: &str = "claimed.json";
const RESULT: &str = "result";

type Waker = Arc<OnceLock<Box<dyn Fn() + Send + Sync>>>;


#[derive(Debug, Serialize, Deserialize)]
struct Invocation {
    document: LaunchFiles,
    form: Option<FormSubmission>,
}


#[derive(Debug, Serialize, Deserialize)]
struct FormSubmission {
    last_dir: Option<OsString>,
}


/// Serde's OS strings preserve paths that contain invalid Unicode.
#[derive(Debug, Serialize, Deserialize)]
enum LaunchFiles {
    Open { a: OsString, b: OsString, c: Option<OsString>, output: Option<OsString>, labels: Labels },
    Prefill(Vec<(Document, OsString)>, Labels),
}


impl LaunchFiles {
    fn new(launch: OpenDocument) -> io::Result<Self> {
        fn absolute(path: PathBuf) -> io::Result<OsString> {
            std::path::absolute(path).map(PathBuf::into_os_string)
        }
        match launch {
            OpenDocument::Open(files) => Ok(Self::Open {
                a: absolute(files.a)?,
                b: absolute(files.b)?,
                c: files.c.map(absolute).transpose()?,
                output: files.output.map(absolute).transpose()?,
                labels: files.labels,
            }),
            OpenDocument::Prefill(paths, labels) => {
                let paths = paths.into_iter().map(|(document, path)| Ok((document, absolute(path)?))).collect::<io::Result<_>>()?;
                Ok(Self::Prefill(paths, labels))
            }
        }
    }

    fn document(self) -> OpenDocument {
        match self {
            Self::Open { a, b, c, output, labels } => OpenDocument::Open(OpenFiles {
                a: a.into(),
                b: b.into(),
                c: c.map(PathBuf::from),
                output: output.map(PathBuf::from),
                labels,
            }),
            Self::Prefill(paths, labels) => OpenDocument::Prefill(paths.into_iter().map(|(document, path)| (document, path.into())).collect(), labels),
        }
    }
}


/// Keeps election and request publication in order. The window holds its own liveness lock.
pub fn launch(launch: OpenDocument) -> anyhow::Result<bool> {
    submit(launch, None)
}


pub fn open_from_form(files: OpenFiles, last_dir: Option<PathBuf>) -> anyhow::Result<bool> {
    submit(OpenDocument::Open(files), Some(FormSubmission {
        last_dir: last_dir.map(PathBuf::into_os_string),
    }))
}


fn submit(launch: OpenDocument, form: Option<FormSubmission>) -> anyhow::Result<bool> {
    let document = LaunchFiles::new(launch).context("cannot resolve launch paths")?;
    let request = Invocation { document, form };
    let json = serde_json::to_vec(&request).context("cannot encode launch request")?;
    let cache = dirs::cache_dir().context("no per-user cache directory for the running window")?;
    let root = cache.join("kdiff4").join("instance-v2");
    private_dir(&root).context("cannot create the window directory")?;
    for _ in 0..2 {
        let gate = lock_file(&root.join("election")).context("cannot open the window election lock")?;
        gate.lock().context("cannot acquire the window election lock")?;
        let (window, pending) = match current_window(&root).context("cannot find the running window")? {
            Some(window) => match PendingRequest::publish(&window, &json) {
                Ok(pending) => (window, pending),
                Err(error) if error.kind() == ErrorKind::NotFound && !live(&window)? => continue,
                Err(error) => return Err(error).context("cannot send files to the window"),
            },
            None => start_window(&root, &json).context("cannot start the Kdiff4 window")?,
        };
        drop(gate);
        if let Some(success) = pending.wait(&window).context("cannot receive the tab result")? {
            return Ok(success);
        }
    }
    anyhow::bail!("the window closed before it could open the files");
}


fn private_dir(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}


fn lock_file(path: &Path) -> io::Result<File> {
    File::options().read(true).write(true).create(true).truncate(false).open(path)
}


fn live(window: &Path) -> io::Result<bool> {
    let lock = match File::options().read(true).write(true).open(window.join("live")) {
        Ok(lock) => lock,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    match lock.try_lock() {
        Ok(()) => Ok(false),
        Err(TryLockError::WouldBlock) => Ok(true),
        Err(TryLockError::Error(error)) => Err(error),
    }
}


fn current_window(root: &Path) -> io::Result<Option<PathBuf>> {
    let name = match fs::read_to_string(root.join("current")) {
        Ok(name) => name,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !name.strip_prefix("window-").is_some_and(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit() || byte == b'-')) {
        return Ok(None);
    }
    let window = root.join(name);
    Ok(live(&window)?.then_some(window))
}


fn unique_name(prefix: &str) -> String {
    let time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    format!("{prefix}-{time}-{}", process::id())
}


fn start_window(root: &Path, json: &[u8]) -> anyhow::Result<(PathBuf, PendingRequest)> {
    let name = unique_name("window");
    let window = root.join(&name);
    fs::create_dir(&window)?;
    let started = (|| {
        let pending = PendingRequest::publish(&window, json)?;
        let mut child = process::Command::new(std::env::current_exe()?)
            .env(SERVER_ENV, &window)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        if let Err(error) = await_window(&window, &mut child).and_then(|()| Ok(fs::write(root.join("current"), name)?)) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        Ok(pending)
    })();
    if started.is_err() {
        let _ = fs::remove_file(window.join("live"));
        let _ = fs::remove_dir(&window);
    }
    Ok((window, started?))
}


fn await_window(window: &Path, child: &mut Child) -> anyhow::Result<()> {
    let start = Instant::now();
    while !live(window)? {
        if let Some(status) = child.try_wait()? {
            anyhow::bail!("the window process exited during startup ({status})");
        }
        anyhow::ensure!(start.elapsed() < START_TIMEOUT, "the window process did not start in time");
        thread::sleep(POLL_INTERVAL);
    }
    Ok(())
}


struct PendingRequest(PathBuf);


impl PendingRequest {
    fn publish(window: &Path, json: &[u8]) -> io::Result<Self> {
        let path = window.join(unique_name("open"));
        fs::create_dir(&path)?;
        let pending = Self(path);
        fs::write(pending.0.join("writing"), json)?;
        fs::rename(pending.0.join("writing"), pending.0.join(REQUEST))?;
        Ok(pending)
    }

    fn result(&self) -> io::Result<Option<bool>> {
        match fs::read(self.0.join(RESULT)) {
            Ok(bytes) => match bytes.as_slice() {
                b"1" => Ok(Some(true)),
                b"0" => Ok(Some(false)),
                _ => Err(io::Error::new(ErrorKind::InvalidData, "invalid tab result")),
            },
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn wait(&self, window: &Path) -> io::Result<Option<bool>> {
        loop {
            if let Some(success) = self.result()? {
                return Ok(Some(success));
            }
            if !live(window)? {
                return match self.result()? {
                    Some(success) => Ok(Some(success)),
                    None if self.0.join(REQUEST).try_exists()? => Ok(None),
                    None => Err(io::Error::other("the window stopped before the tab finished")),
                };
            }
            if !self.0.join(REQUEST).try_exists()? && !self.0.join(CLAIMED).try_exists()? {
                return self.result()?.map(Some).ok_or_else(|| io::Error::other("the tab result was lost"));
            }
            thread::sleep(POLL_INTERVAL);
        }
    }
}


impl Drop for PendingRequest {
    fn drop(&mut self) {
        for name in ["writing", REQUEST, CLAIMED, RESULT] {
            let _ = fs::remove_file(self.0.join(name));
        }
        let _ = fs::remove_dir(&self.0);
        if let Some(window) = self.0.parent() {
            let _ = fs::remove_dir(window);
        }
    }
}


/// Watches the launch inbox and writes per-tab results off the UI thread.
pub struct Server {
    stop: mpsc::Sender<()>,
    worker: Option<JoinHandle<()>>,
}


impl Server {
    pub fn start(window: PathBuf) -> io::Result<(Self, mpsc::Receiver<OpenRequest>, Waker)> {
        let (stop, stopped) = mpsc::channel();
        let (incoming, requests) = mpsc::channel();
        let waker: Waker = Arc::default();
        let wake = waker.clone();
        let lock = lock_file(&window.join("live"))?;
        lock.lock()?;
        // The first request is published before the process starts, so the GUI can open it before drawing.
        let mut pending = Vec::new();
        read_requests(&window, &incoming, &wake, &mut pending)?;
        let worker = thread::Builder::new().name("kdiff4-launches".to_string()).spawn(move || {
            loop {
                if let Err(error) = read_requests(&window, &incoming, &wake, &mut pending) {
                    log::error!("cannot read launch requests: {error}");
                }
                write_results(&mut pending);
                if stopped.recv_timeout(POLL_INTERVAL) != Err(mpsc::RecvTimeoutError::Timeout) {
                    break;
                }
            }
            write_results(&mut pending);
            drop(lock);
            let _ = fs::remove_file(window.join("live"));
            let _ = fs::remove_dir(window);
        })?;
        Ok((Self { stop, worker: Some(worker) }, requests, waker))
    }
}


impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() && worker.join().is_err() {
            log::error!("launch worker panicked");
        }
    }
}


fn read_requests(
    window: &Path,
    incoming: &mpsc::Sender<OpenRequest>,
    wake: &Waker,
    pending: &mut Vec<(PathBuf, mpsc::Receiver<bool>)>,
) -> io::Result<()> {
    let mut requests = fs::read_dir(window)?.collect::<io::Result<Vec<_>>>()?;
    requests.sort_by_key(|entry| entry.file_name());
    for entry in requests {
        if !entry.file_name().to_string_lossy().starts_with("open-") {
            continue;
        }
        let path = entry.path();
        let claimed = path.join(CLAIMED);
        match fs::rename(path.join(REQUEST), &claimed) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        }
        let (completion, completed) = mpsc::channel();
        match read_invocation(&claimed) {
            Ok(Invocation { document, form }) => {
                let document = document.document();
                log::info!("launch: {document:?}");
                let completion = match form {
                    Some(form) => OpenCompletion::Dispatched { result: completion, last_dir: form.last_dir.map(PathBuf::from) },
                    None => OpenCompletion::TabClosed(completion),
                };
                if incoming.send(OpenRequest { document, completion }).is_err() {
                    fs::rename(claimed, path.join(REQUEST))?;
                    break;
                }
                if let Some(wake) = wake.get() {
                    wake();
                }
            }
            Err(error) => log::error!("cannot open launch request: {error:#}"),
        }
        pending.push((path, completed));
    }
    Ok(())
}


fn read_invocation(path: &Path) -> anyhow::Result<Invocation> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}


fn write_results(pending: &mut Vec<(PathBuf, mpsc::Receiver<bool>)>) {
    pending.retain(|(path, completion)| {
        let success = match completion.try_recv() {
            Ok(success) => success,
            Err(mpsc::TryRecvError::Disconnected) => false,
            Err(mpsc::TryRecvError::Empty) => return true,
        };
        let writing = path.join("writing");
        if let Err(error) = fs::write(&writing, if success { b"1" } else { b"0" }).and_then(|()| fs::rename(&writing, path.join(RESULT))) {
            log::error!("cannot write the tab result: {error}");
            let _ = fs::remove_file(path.join(CLAIMED));
        }
        false
    });
}


#[cfg(test)]
mod tests {
    use super::*;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(unique_name("kd4"));
            private_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn merge_request(output: &str) -> Invocation {
        let document = LaunchFiles::new(OpenDocument::Open(OpenFiles {
            a: "test_data/merge/whitespace_rs/base.rs".into(),
            b: "test_data/merge/whitespace_rs/left.rs".into(),
            c: Some("test_data/merge/whitespace_rs/right.rs".into()),
            output: Some(output.into()),
            labels: Labels { a: Some("Base".into()), b: Some("Local".into()), c: Some("Remote".into()) },
        })).unwrap();
        Invocation { document, form: None }
    }

    #[test]
    fn invocations_preserve_labels_and_resolve_paths_in_the_launcher() {
        let output = "result folder/merge.txt";
        let json = serde_json::to_vec(&merge_request(output)).unwrap();
        let decoded: Invocation = serde_json::from_slice(&json).unwrap();
        let OpenDocument::Open(files) = decoded.document.document() else { panic!("expected merge request") };
        assert_eq!(files.output, Some(std::path::absolute(output).unwrap()));
        assert_eq!(files.labels.c.as_deref(), Some("Remote"));
        assert!(files.a.is_absolute() && files.b.is_absolute() && files.c.unwrap().is_absolute());
    }

    #[cfg(unix)]
    #[test]
    fn a_path_with_non_unicode_bytes_survives_the_handoff() {
        use std::os::unix::ffi::OsStringExt;

        let path = PathBuf::from(OsString::from_vec(vec![b'/', b'a', 255]));
        let request = LaunchFiles::new(OpenDocument::Prefill(vec![(Document::A, path.clone())], Labels::default())).unwrap();
        let decoded: LaunchFiles = serde_json::from_slice(&serde_json::to_vec(&request).unwrap()).unwrap();
        assert_eq!(decoded.document(), OpenDocument::Prefill(vec![(Document::A, path)], Labels::default()));
    }

    #[test]
    fn running_window_receives_both_launches_and_reports_their_results_independently() {
        let root = TestDirectory::new();
        let name = unique_name("window");
        let window = root.0.join(&name);
        fs::create_dir(&window).unwrap();
        let (server, incoming, _) = Server::start(window.clone()).unwrap();
        fs::write(root.0.join("current"), name).unwrap();
        assert_eq!(current_window(&root.0).unwrap(), Some(window.clone()));

        let first = PendingRequest::publish(&window, &serde_json::to_vec(&merge_request("first.txt")).unwrap()).unwrap();
        let second = PendingRequest::publish(&window, &serde_json::to_vec(&merge_request("second.txt")).unwrap()).unwrap();
        for _ in 0..2 {
            let OpenRequest { document, completion } = incoming.recv_timeout(Duration::from_secs(5)).unwrap();
            let OpenDocument::Open(files) = document else { panic!("expected merge request") };
            let OpenCompletion::TabClosed(result) = completion else { panic!("expected a tab result") };
            result.send(files.output.unwrap().ends_with("second.txt")).unwrap();
        }
        drop(server);

        assert_eq!(first.wait(&window).unwrap(), Some(false));
        assert_eq!(second.wait(&window).unwrap(), Some(true));
        assert_eq!(current_window(&root.0).unwrap(), None);
    }

    #[test]
    fn startup_request_is_ready_before_the_gui_draws() {
        let window = TestDirectory::new();
        let launch = merge_request("first.txt");
        let _pending = PendingRequest::publish(&window.0, &serde_json::to_vec(&launch).unwrap()).unwrap();
        let (_server, incoming, _) = Server::start(window.0.clone()).unwrap();
        let request = incoming.try_recv().unwrap();
        assert_eq!(request.document, launch.document.document());
        assert!(matches!(request.completion, OpenCompletion::TabClosed(_)));
    }

    #[test]
    fn open_window_request_is_acknowledged_without_waiting_for_the_tab_to_close() {
        let window = TestDirectory::new();
        let (server, incoming, _) = Server::start(window.0.clone()).unwrap();
        let mut launch = merge_request("output.txt");
        let last_dir = window.0.join("files");
        launch.form = Some(FormSubmission { last_dir: Some(last_dir.clone().into_os_string()) });
        let pending = PendingRequest::publish(&window.0, &serde_json::to_vec(&launch).unwrap()).unwrap();
        let request = incoming.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(request.document, launch.document.document());
        let OpenCompletion::Dispatched { result, last_dir: recent } = request.completion else { panic!("expected a dispatch result") };
        assert_eq!(recent, Some(last_dir));
        result.send(true).unwrap();
        drop(server);
        assert_eq!(pending.wait(&window.0).unwrap(), Some(true));
    }

    #[test]
    fn a_request_left_unclaimed_by_a_closed_window_can_be_retried() {
        let window = TestDirectory::new();
        let pending = PendingRequest::publish(&window.0, b"unused").unwrap();
        assert_eq!(pending.wait(&window.0).unwrap(), None);
    }
}
