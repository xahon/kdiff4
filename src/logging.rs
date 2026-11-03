use std::path::{Path, PathBuf};

use flexi_logger::{Cleanup, Criterion, Duplicate, FileSpec, FlexiLoggerError, Logger, LoggerHandle, Naming};


/// Environment variable that sets the level of kdiff4's own messages for one run.
pub const LEVEL_VARIABLE: &str = "KDIFF4_LOG";
const DEFAULT_LEVEL: &str = "info";
const LEVELS: [&str; 6] = ["off", "error", "warn", "info", "debug", "trace"];
/// Crates whose messages follow the chosen level; everything else logs warnings and errors only.
const OWN_CRATES: [&str; 4] = ["kdiff4", "kdiff4_core", "kdiff4_gui", "kdiff4_protocol"];
const ROTATE_SIZE: u64 = 5 * 1024 * 1024;
const KEPT_FILES: usize = 5;
const BASENAME: &str = "kdiff4";


/// The log specification for `level` (the value of `KDIFF4_LOG`, if set). An unknown level falls back to info and is returned as the second value, so it can be reported once logging runs.
pub fn log_spec(level: Option<&str>) -> (String, Option<String>) {
    let requested = level
        .map(str::trim)
        .filter(|level| !level.is_empty())
        .map(str::to_ascii_lowercase);
    let (level, rejected) = match requested {
        Some(level) if LEVELS.contains(&level.as_str()) => (level, None),
        Some(level) => (DEFAULT_LEVEL.to_string(), Some(level)),
        None => (DEFAULT_LEVEL.to_string(), None),
    };
    let own: Vec<String> = OWN_CRATES
        .iter()
        .map(|krate| format!("{krate}={level}"))
        .collect();
    (format!("warn, {}", own.join(", ")), rejected)
}


/// The `logs` folder next to the executable.
pub fn log_directory() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("logs")))
}


/// Logs to rotating files in `directory` and duplicates every record to stdout.
pub fn start_with_files(directory: &Path, spec: &str) -> Result<LoggerHandle, FlexiLoggerError> {
    Logger::try_with_str(spec)?
        .log_to_file(FileSpec::default().directory(directory).basename(BASENAME).suppress_timestamp())
        .rotate(Criterion::Size(ROTATE_SIZE), Naming::Numbers, Cleanup::KeepLogFiles(KEPT_FILES))
        .append()
        .duplicate_to_stdout(Duplicate::All)
        .format_for_files(flexi_logger::opt_format)
        .format_for_stdout(flexi_logger::opt_format)
        .start()
}


fn start_stdout_only(spec: &str) -> Result<LoggerHandle, FlexiLoggerError> {
    Logger::try_with_str(spec)?
        .log_to_stdout()
        .format(flexi_logger::opt_format)
        .start()
}


/// Starts logging for the whole program and logs panics. Keep the handle alive until the program ends; dropping it flushes the files. Falls back to stdout only if the log folder can't be used, and to no logging (with a message on stderr) if even that fails.
pub fn init() -> Option<LoggerHandle> {
    let (spec, rejected) = log_spec(std::env::var(LEVEL_VARIABLE).ok().as_deref());
    let directory = log_directory();
    let file_result = match directory.as_deref() {
        Some(directory) => start_with_files(directory, &spec).map_err(|error| error.to_string()),
        None => Err("cannot find the executable's folder".to_string()),
    };
    let handle = match file_result {
        Ok(handle) => handle,
        Err(file_error) => match start_stdout_only(&spec) {
            Ok(handle) => {
                log::warn!("logging to stdout only; cannot log to files: {file_error}");
                handle
            }
            Err(error) => {
                eprintln!("kdiff4: logging is off: {error}");
                return None;
            }
        },
    };
    if let Some(rejected) = rejected {
        log::warn!("{LEVEL_VARIABLE}={rejected:?} is not a level ({}); using {DEFAULT_LEVEL}", LEVELS.join(", "));
    }
    install_panic_hook();
    log::info!("kdiff4 {} started; log folder: {}", env!("CARGO_PKG_VERSION"), directory.as_deref().map(|dir| dir.display().to_string()).unwrap_or_else(|| "none".to_string()));
    Some(handle)
}


/// Logs a panic before the default hook prints it, so crashes end up in the log file.
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("panic: {info}");
        default_hook(info);
    }));
}


#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn default_level_is_info_for_own_crates_and_warn_for_the_rest() {
        let (spec, rejected) = log_spec(None);
        assert_eq!(spec, "warn, kdiff4=info, kdiff4_core=info, kdiff4_gui=info, kdiff4_protocol=info");
        assert_eq!(rejected, None);
    }


    #[test]
    fn environment_level_sets_own_crates() {
        let (spec, rejected) = log_spec(Some(" DEBUG "));
        assert_eq!(spec, "warn, kdiff4=debug, kdiff4_core=debug, kdiff4_gui=debug, kdiff4_protocol=debug");
        assert_eq!(rejected, None);
        assert!(log_spec(Some("off")).0.contains("kdiff4_gui=off"));
    }


    #[test]
    fn unknown_or_empty_level_falls_back_to_info() {
        let (spec, rejected) = log_spec(Some("loud"));
        assert!(spec.contains("kdiff4=info"));
        assert_eq!(rejected.as_deref(), Some("loud"));
        assert_eq!(log_spec(Some("  ")), log_spec(None));
    }


    #[test]
    fn every_spec_is_accepted_by_flexi_logger() {
        for level in LEVELS {
            let (spec, _) = log_spec(Some(level));
            assert!(flexi_logger::LogSpecification::parse(&spec).is_ok(), "{spec}");
        }
    }


    #[test]
    fn log_directory_is_next_to_the_executable() {
        let directory = log_directory().unwrap();
        assert_eq!(directory.file_name().unwrap(), "logs");
        assert_eq!(directory.parent(), std::env::current_exe().unwrap().parent());
    }


    /// The only test that installs the global logger, so it can't clash with another one.
    #[test]
    fn file_logging_writes_the_current_file() {
        let directory = std::env::temp_dir().join(format!("kdiff4-logging-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let handle = start_with_files(&directory, "info").unwrap();
        log::info!("hello from the logging test");
        handle.flush();
        let text = std::fs::read_to_string(directory.join("kdiff4_rCURRENT.log")).unwrap();
        assert!(text.contains("INFO"), "{text}");
        assert!(text.contains("hello from the logging test"), "{text}");
        drop(handle);
        let _ = std::fs::remove_dir_all(&directory);
    }
}
