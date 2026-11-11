#![windows_subsystem = "windows"]

mod console;
mod instance;
mod logging;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use clap::Parser;
use kdiff4_gui::{OpenDocument, clean_path};
use kdiff4_protocol::{Client, Document, Labels, OpenFiles};

/// Compares and merges two or three text files.
#[derive(Debug, Parser)]
#[command(name = "kdiff4", version, about)]
struct Cli {
    /// Input files: A B C (3-way), or A B (2-way). With --base, these are B and C.
    #[arg(value_name = "FILE")]
    files: Vec<PathBuf>,
    /// The base file, A; the positional files then become B and C.
    #[arg(short = 'b', long = "base", value_name = "FILE")]
    base: Option<PathBuf>,
    /// The merge output, D. Without it a 3-way merge keeps D in memory until it is saved.
    #[arg(short = 'o', long = "output", visible_alias = "out", value_name = "FILE")]
    output: Option<PathBuf>,
    /// Display label of A.
    #[arg(long = "L1", value_name = "LABEL")]
    label_a: Option<String>,
    /// Display label of B.
    #[arg(long = "L2", value_name = "LABEL")]
    label_b: Option<String>,
    /// Display label of C.
    #[arg(long = "L3", value_name = "LABEL")]
    label_c: Option<String>,
    /// Label for the next input that has no --L1/--L2/--L3 label. Repeatable.
    #[arg(short = 'L', long = "fname", value_name = "LABEL")]
    fname: Vec<String>,
}


#[derive(Debug, PartialEq, Eq)]
enum Launch {
    OpenWindow,
    Request(OpenDocument),
}


/// Maps the arguments the way kdiff3 does: three inputs are A B C, two are A B, and `--base` comes first. Paths are cleaned of stray line breaks and quotes; one that is blank after that is dropped.
fn plan_launch(cli: Cli) -> Result<Launch, String> {
    let inputs: Vec<PathBuf> = cli.base.into_iter().chain(cli.files).filter_map(|path| clean_path(&path)).collect();
    let output = cli.output.and_then(|path| clean_path(&path));
    if inputs.len() > 3 {
        return Err(format!("too many input files: {} given, at most 3 allowed", inputs.len()));
    }
    let labels = assign_labels([cli.label_a, cli.label_b, cli.label_c], cli.fname);
    let mut inputs = inputs.into_iter();
    match (inputs.next(), inputs.next(), inputs.next()) {
        (Some(a), Some(b), c) => Ok(Launch::Request(OpenDocument::Open(OpenFiles { a, b, c, output, labels }))),
        (None, _, _) if output.is_none() && labels == Labels::default() => Ok(Launch::OpenWindow),
        (a, _, _) => {
            let paths = [(Document::A, a), (Document::D, output)]
                .into_iter()
                .filter_map(|(document, path)| path.map(|path| (document, path)))
                .collect();
            Ok(Launch::Request(OpenDocument::Prefill(paths, labels)))
        }
    }
}

/// `--L1`..`--L3` label their document; each `-L` label goes to the next document still without one. Extra `-L` labels are ignored.
fn assign_labels(explicit: [Option<String>; 3], mut next: Vec<String>) -> Labels {
    next.reverse();
    let [a, b, c] = explicit.map(|label| label.or_else(|| next.pop()));
    Labels { a, b, c }
}

fn start_client() -> std::io::Result<Client> {
    let (client, endpoint) = kdiff4_protocol::in_process();
    kdiff4_core::spawn_host(endpoint)?;
    Ok(client)
}

fn run_window(directory: PathBuf) -> anyhow::Result<bool> {
    let logger = logging::init();
    let result = (|| {
        let client = start_client().context("cannot start the engine host")?;
        let (server, incoming, waker) = instance::Server::start(directory).context("cannot receive launch requests")?;
        let restart = kdiff4_gui::run(client, start_client, incoming, move |wake| {
            let _ = waker.set(wake);
        }, spawn_open_window).context("GUI failed");
        drop(server);
        if let Some(exe) = restart? {
            restart_updated_executable(&exe)?;
        }
        Ok(true)
    })();
    if let Err(error) = &result {
        log::error!("{error:#}");
    }
    log::info!("kdiff4 exits");
    drop(logger);
    result
}


fn spawn_open_window() -> std::io::Result<()> {
    std::process::Command::new(std::env::current_exe()?)
        .env_remove(instance::SERVER_ENV)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}


fn restart_updated_executable(exe: &std::path::Path) -> anyhow::Result<()> {
    std::process::Command::new(exe).env_remove(instance::SERVER_ENV).spawn()
        .with_context(|| format!("cannot restart {}", exe.display()))?;
    Ok(())
}


fn run_open_window() -> anyhow::Result<bool> {
    let check_for_updates = !instance::has_window().unwrap_or(false);
    let _logger = check_for_updates.then(logging::init);
    let restart = kdiff4_gui::run_open_window(check_for_updates, |files, last_dir| {
        match instance::open_from_form(files, last_dir) {
            Ok(true) => Ok(()),
            Ok(false) => Err("Kdiff4 could not accept the files.".into()),
            Err(error) => Err(format!("{error:#}")),
        }
    }).context("cannot show the Open window")?;
    if let Some(exe) = restart {
        restart_updated_executable(&exe)?;
    }
    Ok(true)
}


fn exit_code(result: anyhow::Result<bool>) -> ExitCode {
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("kdiff4: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// Usage errors exit with 1, like kdiff3, not clap's 2.
fn main() -> ExitCode {
    if let Some(directory) = std::env::var_os(instance::SERVER_ENV) {
        return exit_code(run_window(PathBuf::from(directory)));
    }
    console::attach_to_parent();
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let _ = error.print();
            return if error.use_stderr() { ExitCode::FAILURE } else { ExitCode::SUCCESS };
        }
    };
    let launch = match plan_launch(cli) {
        Ok(launch) => launch,
        Err(message) => {
            eprintln!("kdiff4: {message}");
            return ExitCode::FAILURE;
        }
    };
    match launch {
        Launch::OpenWindow => exit_code(run_open_window()),
        Launch::Request(document) => exit_code(instance::launch(document)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launch(args: &[&str]) -> Result<Launch, String> {
        let cli = Cli::try_parse_from(std::iter::once("kdiff4").chain(args.iter().copied())).map_err(|error| error.to_string())?;
        plan_launch(cli)
    }

    fn open(a: &str, b: &str, c: Option<&str>, output: Option<&str>, labels: Labels) -> Launch {
        Launch::Request(OpenDocument::Open(OpenFiles {
            a: PathBuf::from(a),
            b: PathBuf::from(b),
            c: c.map(PathBuf::from),
            output: output.map(PathBuf::from),
            labels,
        }))
    }

    fn label(text: &str) -> Option<String> {
        Some(text.to_string())
    }

    #[test]
    fn two_files_are_a_and_b() {
        assert_eq!(launch(&["a", "b"]), Ok(open("a", "b", None, None, Labels::default())));
    }

    #[test]
    fn stray_line_breaks_and_blank_paths_are_cleaned() {
        assert_eq!(
            launch(&["a\n", " b ", "c\r\n", "-o", "out\n"]),
            Ok(open("a", "b", Some("c"), Some("out"), Labels::default()))
        );
        assert_eq!(launch(&["a", "\n", "b"]), Ok(open("a", "b", None, None, Labels::default())));
    }

    #[test]
    fn three_files_are_a_b_c() {
        assert_eq!(launch(&["a", "b", "c"]), Ok(open("a", "b", Some("c"), None, Labels::default())));
    }

    #[test]
    fn git_mergetool_three_way_call() {
        let args = ["--L1", "Base", "--L2", "Local", "--L3", "Remote", "base", "local", "remote", "-o", "merged"];
        let labels = Labels {
            a: label("Base"),
            b: label("Local"),
            c: label("Remote"),
        };
        assert_eq!(launch(&args), Ok(open("base", "local", Some("remote"), Some("merged"), labels)));
    }

    #[test]
    fn git_mergetool_two_way_call() {
        let args = ["--L1", "Local", "--L2", "Remote", "local", "remote", "-o", "merged"];
        let labels = Labels {
            a: label("Local"),
            b: label("Remote"),
            c: None,
        };
        assert_eq!(launch(&args), Ok(open("local", "remote", None, Some("merged"), labels)));
    }

    #[test]
    fn base_option_makes_positionals_b_and_c() {
        assert_eq!(
            launch(&["--base", "base", "left", "right", "--out", "out"]),
            Ok(open("base", "left", Some("right"), Some("out"), Labels::default()))
        );
        assert_eq!(launch(&["-b", "base", "left"]), Ok(open("base", "left", None, None, Labels::default())));
        assert_eq!(
            launch(&["left", "-b", "base", "right", "--output", "out"]),
            Ok(open("base", "left", Some("right"), Some("out"), Labels::default()))
        );
    }

    #[test]
    fn fname_labels_fill_unlabeled_inputs_in_order() {
        let args = ["--L2", "Two", "-L", "First", "--fname", "Third", "-L", "Extra", "a", "b", "c"];
        let labels = Labels {
            a: label("First"),
            b: label("Two"),
            c: label("Third"),
        };
        assert_eq!(launch(&args), Ok(open("a", "b", Some("c"), None, labels)));
    }

    #[test]
    fn no_arguments_start_a_standalone_open_window() {
        assert_eq!(launch(&[]), Ok(Launch::OpenWindow));
    }

    #[test]
    fn too_few_files_prefill_the_form_in_the_main_window() {
        assert_eq!(
            launch(&["a", "-o", "out"]),
            Ok(Launch::Request(OpenDocument::Prefill(
                vec![(Document::A, PathBuf::from("a")), (Document::D, PathBuf::from("out"))],
                Labels::default()
            )))
        );
        assert_eq!(launch(&["-b", "base"]), Ok(Launch::Request(OpenDocument::Prefill(vec![(Document::A, PathBuf::from("base"))], Labels::default()))));
    }

    #[test]
    fn too_many_files_is_a_usage_error() {
        assert!(launch(&["a", "b", "c", "d"]).unwrap_err().contains("too many input files"));
        assert!(launch(&["-b", "base", "a", "b", "c"]).is_err());
    }

    #[test]
    fn unknown_options_are_rejected() {
        assert!(launch(&["--frobnicate", "a", "b"]).is_err());
    }

}
