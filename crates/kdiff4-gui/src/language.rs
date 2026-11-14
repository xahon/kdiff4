use std::ops::Range;
use std::path::{Path, PathBuf};

use egui::{Image, PopupCloseBehavior, Ui, Vec2, vec2};

use crate::bottom_controls::{bottom_switch, SwitchPopup};
use crate::mirror::Mirror;

const PLAIN_TEXT: &str = "Plain text";
/// Endings that editors, patch tools and merge tools append to a file's name; they say nothing about its language.
const BACKUP_SUFFIXES: &[&str] = &["orig", "bak", "old", "new", "tmp", "rej"];
/// Starts of a first line, compared in lower case, that name a language without a shebang.
const FIRST_LINE_MARKERS: &[(&str, &str)] = &[("<?xml", "XML"), ("<?php", "PHP"), ("<!doctype html", "HTML"), ("<html", "HTML")];
const SELECTOR_HEIGHT: f32 = 400.0;
const ICON_SIZE: f32 = 16.0;
const SELECTOR_HINT: &str = "Language of the documents";

/// How one language is recognized: by file name, by extension, or by the program in a shebang line. Names are compared without regard to case.
struct Rule {
    name: &'static str,
    extensions: &'static [&'static str],
    file_names: &'static [&'static str],
    interpreters: &'static [&'static str],
}

const fn rule(name: &'static str, extensions: &'static [&'static str], file_names: &'static [&'static str], interpreters: &'static [&'static str]) -> Rule {
    Rule {
        name,
        extensions,
        file_names,
        interpreters,
    }
}

const RULES: &[Rule] = &[
    rule("Assembly", &["asm", "s", "nasm", "masm"], &[], &[]),
    rule("Batch", &["bat", "cmd"], &[], &[]),
    rule("C", &["c", "h"], &[], &[]),
    rule("C#", &["cs", "csx"], &[], &[]),
    rule("C++", &["cpp", "cc", "cxx", "c++", "hpp", "hh", "hxx", "h++", "ipp", "inl", "tpp"], &[], &[]),
    rule("Clojure", &["clj", "cljs", "cljc", "edn"], &[], &[]),
    rule("CMake", &["cmake"], &["CMakeLists.txt"], &[]),
    rule("CSS", &["css"], &[], &[]),
    rule("CUDA", &["cu", "cuh"], &[], &[]),
    rule("Dart", &["dart"], &[], &[]),
    rule("Diff", &["diff", "patch"], &[], &[]),
    rule("Dockerfile", &["dockerfile"], &["Dockerfile", "Containerfile"], &[]),
    rule("Elixir", &["ex", "exs"], &[], &["elixir"]),
    rule("Erlang", &["erl", "hrl"], &[], &["escript"]),
    rule("F#", &["fs", "fsx", "fsi"], &[], &[]),
    rule("Fortran", &["f", "for", "f77", "f90", "f95", "f03", "f08"], &[], &[]),
    rule("Gettext", &["po", "pot"], &[], &[]),
    rule("GLSL", &["glsl", "vert", "frag", "geom", "tesc", "tese", "comp"], &[], &[]),
    rule("Go", &["go"], &[], &[]),
    rule("GraphQL", &["graphql", "gql"], &[], &[]),
    rule("Groovy", &["groovy", "gradle", "gvy"], &["Jenkinsfile"], &["groovy"]),
    rule("Haskell", &["hs", "lhs"], &[], &["runhaskell", "runghc"]),
    rule("HCL", &["hcl", "tf", "tfvars"], &[], &[]),
    rule("HLSL", &["hlsl", "hlsli", "fx", "fxh"], &[], &[]),
    rule("HTML", &["html", "htm", "xhtml"], &[], &[]),
    rule("INI", &["ini", "cfg", "inf"], &[".gitconfig", ".gitmodules", ".editorconfig"], &[]),
    rule("Java", &["java"], &[], &[]),
    rule("JavaScript", &["js", "mjs", "cjs", "jsx"], &[], &["node", "nodejs", "deno", "bun"]),
    rule("JSON", &["json", "jsonc", "json5", "geojson"], &[], &[]),
    rule("Julia", &["jl"], &[], &["julia"]),
    rule("Kotlin", &["kt", "kts"], &[], &["kotlin"]),
    rule("LaTeX", &["tex", "sty", "cls"], &[], &[]),
    rule("Less", &["less"], &[], &[]),
    rule("Lisp", &["lisp", "lsp", "el"], &[], &["sbcl", "clisp"]),
    rule("Lua", &["lua"], &[], &["lua", "luajit"]),
    rule("Makefile", &["mk", "mak"], &["Makefile", "GNUmakefile"], &["make"]),
    rule("Markdown", &["md", "markdown", "mdx"], &[], &[]),
    rule("Objective-C", &["m", "mm"], &[], &[]),
    rule("OCaml", &["ml", "mli"], &[], &["ocaml"]),
    rule("Pascal", &["pas", "dpr"], &[], &[]),
    rule("Perl", &["pl", "pm", "t"], &[], &["perl"]),
    rule("PHP", &["php", "phtml", "phps"], &[], &["php"]),
    rule("PowerShell", &["ps1", "psm1", "psd1"], &[], &["pwsh", "powershell"]),
    rule("Properties", &["properties"], &[], &[]),
    rule("Protocol Buffers", &["proto"], &[], &[]),
    rule("Python", &["py", "pyw", "pyi"], &["SConstruct", "SConscript"], &["python", "pypy"]),
    rule("QML", &["qml"], &[], &["qmlscene"]),
    rule("R", &["r"], &[], &["rscript"]),
    rule("reStructuredText", &["rst"], &[], &[]),
    rule(
        "Ruby",
        &["rb", "rake", "gemspec", "ru"],
        &["Rakefile", "Gemfile", "Guardfile", "Vagrantfile", "Podfile", "Brewfile", "Capfile"],
        &["ruby"],
    ),
    rule("Rust", &["rs"], &[], &[]),
    rule("Scala", &["scala", "sbt", "sc"], &[], &["scala"]),
    rule("SCSS", &["scss"], &[], &[]),
    rule(
        "Shell",
        &["sh", "bash", "zsh", "ksh"],
        &[
            ".bashrc",
            ".bash_profile",
            ".bash_aliases",
            ".bash_login",
            ".bash_logout",
            ".profile",
            ".zshrc",
            ".zshenv",
            ".zprofile",
            "PKGBUILD",
        ],
        &["sh", "bash", "zsh", "ksh", "dash", "ash"],
    ),
    rule("SQL", &["sql"], &[], &[]),
    rule("Swift", &["swift"], &[], &["swift"]),
    rule("TOML", &["toml"], &["Cargo.lock", "Pipfile"], &[]),
    rule("TypeScript", &["ts", "tsx", "mts", "cts"], &[], &["ts-node"]),
    rule("Visual Basic", &["vb", "vbs", "bas"], &[], &[]),
    rule("WGSL", &["wgsl"], &[], &[]),
    rule(
        "XML",
        &[
            "xml", "xsd", "xsl", "xslt", "svg", "plist", "csproj", "vcxproj", "props", "targets", "resx", "xaml", "ui", "qrc", "wxs", "rss", "atom",
        ],
        &[],
        &[],
    ),
    rule("YAML", &["yaml", "yml"], &[".clang-format", ".clang-tidy"], &[]),
    rule("Zig", &["zig"], &[], &[]),
];

/// The logo of one language: an SVG from the vscode-icons set (MIT, see `LICENSE-vscode-icons` next to the files).
struct Icon {
    language: &'static str,
    svg: &'static [u8],
}

const fn icon(language: &'static str, svg: &'static [u8]) -> Icon {
    Icon { language, svg }
}

macro_rules! svg {
    ($file:literal) => {
        include_bytes!(concat!("../assets/language_icons/", $file, ".svg"))
    };
}

const ICONS: &[Icon] = &[
    icon(PLAIN_TEXT, svg!("text")),
    icon("Assembly", svg!("assembly")),
    icon("Batch", svg!("bat")),
    icon("C", svg!("c")),
    icon("C#", svg!("csharp")),
    icon("C++", svg!("cpp")),
    icon("Clojure", svg!("clojure")),
    icon("CMake", svg!("cmake")),
    icon("CSS", svg!("css")),
    icon("CUDA", svg!("cuda")),
    icon("Dart", svg!("dartlang")),
    icon("Diff", svg!("diff")),
    icon("Dockerfile", svg!("docker")),
    icon("Elixir", svg!("elixir")),
    icon("Erlang", svg!("erlang")),
    icon("F#", svg!("fsharp")),
    icon("Fortran", svg!("fortran")),
    icon("Gettext", svg!("poedit")),
    icon("GLSL", svg!("glsl")),
    icon("Go", svg!("go")),
    icon("GraphQL", svg!("graphql")),
    icon("Groovy", svg!("groovy")),
    icon("Haskell", svg!("haskell")),
    icon("HCL", svg!("terraform")),
    icon("HLSL", svg!("hlsl")),
    icon("HTML", svg!("html")),
    icon("INI", svg!("ini")),
    icon("Java", svg!("java")),
    icon("JavaScript", svg!("js_official")),
    icon("JSON", svg!("json")),
    icon("Julia", svg!("julia")),
    icon("Kotlin", svg!("kotlin")),
    icon("LaTeX", svg!("tex")),
    icon("Less", svg!("less")),
    icon("Lisp", svg!("lisp")),
    icon("Lua", svg!("lua")),
    icon("Makefile", svg!("config")),
    icon("Markdown", svg!("markdown")),
    icon("Objective-C", svg!("objectivec")),
    icon("OCaml", svg!("ocaml")),
    icon("Pascal", svg!("delphi")),
    icon("Perl", svg!("perl")),
    icon("PHP", svg!("php")),
    icon("PowerShell", svg!("powershell")),
    icon("Properties", svg!("config")),
    icon("Protocol Buffers", svg!("protobuf")),
    icon("Python", svg!("python")),
    icon("QML", svg!("qml")),
    icon("R", svg!("r")),
    icon("reStructuredText", svg!("rest")),
    icon("Ruby", svg!("ruby")),
    icon("Rust", svg!("rust")),
    icon("Scala", svg!("scala")),
    icon("SCSS", svg!("scss")),
    icon("Shell", svg!("shell")),
    icon("SQL", svg!("sql")),
    icon("Swift", svg!("swift")),
    icon("TOML", svg!("toml")),
    icon("TypeScript", svg!("typescript_official")),
    icon("Visual Basic", svg!("vb")),
    icon("WGSL", svg!("wgsl")),
    icon("XML", svg!("xml")),
    icon("YAML", svg!("yaml_official")),
    icon("Zig", svg!("zig")),
];

/// What the text of the documents is: a programming, markup or configuration language, or plain text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Language {
    name: &'static str,
}

impl Language {
    pub fn plain_text() -> Language {
        Language { name: PLAIN_TEXT }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    /// The language's logo, a square of `ICON_SIZE`. Needs the image loaders of `egui_extras` to be installed.
    pub fn icon(&self) -> Image<'static> {
        let position = ICONS.iter().position(|icon| icon.language == self.name);
        let svg = position.map_or(&[][..], |position| ICONS[position].svg);
        Image::from_bytes(format!("bytes://language_icons/{}.svg", position.unwrap_or(0)), svg)
            .fit_to_exact_size(vec2(ICON_SIZE, ICON_SIZE))
            .alt_text(format!("{} icon", self.name))
    }

    /// Plain text, then every other language by name.
    pub fn all() -> Vec<Language> {
        let mut names: Vec<&'static str> = RULES.iter().map(|rule| rule.name).collect();
        names.sort_by_key(|name| name.to_ascii_lowercase());
        std::iter::once(PLAIN_TEXT).chain(names).map(|name| Language { name }).collect()
    }
}

impl Default for Language {
    fn default() -> Language {
        Language::plain_text()
    }
}

/// What detection looks at in one document: the path it was read from, and its first line.
pub struct Candidate<'a> {
    pub path: Option<&'a Path>,
    pub first_line: &'a str,
}

/// The language of the first candidate that names one, by its path or else by its first line; plain text if none does.
pub fn detect(candidates: &[Candidate<'_>]) -> Language {
    let found = candidates
        .iter()
        .find_map(|candidate| candidate.path.and_then(rule_for_path).or_else(|| rule_for_first_line(candidate.first_line)));
    found.map_or_else(Language::plain_text, |rule| Language { name: rule.name })
}

/// Detects the language of a session from A, then B, C and D. A is the source of truth: when the documents are named for different languages, A's wins, and a later document counts only when the ones before it name none.
pub fn detect_in(mirror: &Mirror) -> Language {
    let sources = mirror.documents().iter().map(|document| Candidate {
        path: Some(document.path.as_path()),
        first_line: first_line(&document.text, &document.lines),
    });
    let output = mirror.output().map(|output| Candidate {
        path: output.path.as_deref(),
        first_line: first_line(&output.text, &output.lines),
    });
    let candidates: Vec<Candidate<'_>> = sources.chain(output).collect();
    detect(&candidates)
}

fn first_line<'a>(text: &'a str, lines: &[Range<usize>]) -> &'a str {
    lines.first().and_then(|range| text.get(range.clone())).unwrap_or("")
}

fn rule_for_path(path: &Path) -> Option<&'static Rule> {
    let mut name = path.file_name()?.to_str()?;
    loop {
        let by_file_name = RULES.iter().find(|rule| rule.file_names.iter().any(|file_name| file_name.eq_ignore_ascii_case(name)));
        if by_file_name.is_some() {
            return by_file_name;
        }
        let (stem, extension) = name.rsplit_once('.')?;
        let extension = extension.to_ascii_lowercase();
        let by_extension = RULES.iter().find(|rule| rule.extensions.contains(&extension.as_str()));
        if by_extension.is_some() {
            return by_extension;
        }
        if stem.is_empty() || !BACKUP_SUFFIXES.contains(&extension.as_str()) {
            return None;
        }
        name = stem;
    }
}

fn rule_for_first_line(line: &str) -> Option<&'static Rule> {
    let line = line.trim_start_matches('\u{feff}').trim();
    if let Some(command) = line.strip_prefix("#!") {
        return rule_for_interpreter(command);
    }
    let lowered = line.to_ascii_lowercase();
    let (_, name) = FIRST_LINE_MARKERS.iter().find(|(prefix, _)| lowered.starts_with(*prefix))?;
    RULES.iter().find(|rule| rule.name == *name)
}

/// The rule for the program a shebang line starts: `env` is looked through, and a version after the name (`python3.11`) is ignored.
fn rule_for_interpreter(command: &str) -> Option<&'static Rule> {
    let mut words = command.split_whitespace();
    let mut program = words.next()?;
    if base_name(program) == "env" {
        program = words.find(|word| !word.starts_with('-') && !word.contains('='))?;
    }
    let name = base_name(program)
        .trim_end_matches(|character: char| character.is_ascii_digit() || character == '.')
        .to_ascii_lowercase();
    RULES.iter().find(|rule| rule.interpreters.contains(&name.as_str()))
}

fn base_name(program: &str) -> &str {
    program.rsplit(['/', '\\']).next().unwrap_or(program)
}

/// The session's language: detected from its documents, or picked by the user.
#[derive(Debug, Clone, Default)]
pub struct LanguageState {
    current: Language,
    /// The source paths the user picked `current` for.
    picked_for: Option<Vec<PathBuf>>,
}

impl LanguageState {
    pub fn current(&self) -> &Language {
        &self.current
    }

    /// For every new snapshot. A picked language stays while the session has the same source files; otherwise the language is detected again, so opening other files or reloading one from another path starts afresh.
    pub fn follow(&mut self, mirror: &Mirror) {
        if self.picked_for.as_ref() == Some(&source_paths(mirror)) {
            return;
        }
        self.current = detect_in(mirror);
        self.picked_for = None;
    }

    pub fn pick(&mut self, language: Language, mirror: &Mirror) {
        self.current = language;
        self.picked_for = Some(source_paths(mirror));
    }
}

fn source_paths(mirror: &Mirror) -> Vec<PathBuf> {
    mirror.documents().iter().map(|document| document.path.clone()).collect()
}

/// The drop-down for the status bar. Returns the language the user picked this frame.
pub fn show_selector(ui: &mut Ui, current: &Language) -> Option<Language> {
    let mut picked = None;
    let entries = std::iter::once(PLAIN_TEXT).chain(RULES.iter().map(|rule| rule.name));
    let icon = Some((current.icon(), Vec2::splat(ICON_SIZE)));
    let popup = SwitchPopup { title: Some("Language"), close_behavior: PopupCloseBehavior::CloseOnClick, max_height: Some(SELECTOR_HEIGHT) };
    bottom_switch(ui, SELECTOR_HINT, current.name().into(), entries, icon, popup, |ui| {
        for language in Language::all() {
            if ui.selectable_label(language == *current, (language.icon(), language.name())).clicked() {
                picked = Some(language);
            }
        }
    });
    picked
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use egui::vec2;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use kdiff4_protocol::engine::{Comparison, ComparisonOptions, EngineCapabilities};
    use kdiff4_protocol::{Document, DocumentText, EolChoice, EolStyle, OutputDocument, Revision, Snapshot};

    use super::*;

    fn name_of_path(path: &str) -> &'static str {
        detect(&[Candidate {
            path: Some(Path::new(path)),
            first_line: "",
        }])
        .name()
    }

    fn name_of_line(line: &str) -> &'static str {
        detect(&[Candidate { path: None, first_line: line }]).name()
    }

    #[test]
    fn extensions_name_languages() {
        assert_eq!(name_of_path("src/main.rs"), "Rust");
        assert_eq!(name_of_path("a/b/config.JSON"), "JSON");
        assert_eq!(name_of_path("E:\\work\\widget.cpp"), "C++");
        assert_eq!(name_of_path("deploy.yml"), "YAML");
        assert_eq!(name_of_path("deploy.yaml"), "YAML");
        assert_eq!(name_of_path("lib.TS"), "TypeScript");
        assert_eq!(name_of_path("README.MD"), "Markdown");
        assert_eq!(name_of_path("shader.wgsl"), "WGSL");
    }

    #[test]
    fn file_names_win_over_extensions() {
        assert_eq!(name_of_path("src/CMakeLists.txt"), "CMake");
        assert_eq!(name_of_path("Makefile"), "Makefile");
        assert_eq!(name_of_path("makefile"), "Makefile");
        assert_eq!(name_of_path("Cargo.lock"), "TOML");
        assert_eq!(name_of_path("home/me/.bashrc"), "Shell");
        assert_eq!(name_of_path(".clang-format"), "YAML");
        assert_eq!(name_of_path("docker/Dockerfile"), "Dockerfile");
    }

    #[test]
    fn backup_suffixes_are_skipped() {
        assert_eq!(name_of_path("main.c.orig"), "C");
        assert_eq!(name_of_path("settings.json.bak"), "JSON");
        assert_eq!(name_of_path("a.py.orig.bak"), "Python");
        assert_eq!(name_of_path("main.orig"), PLAIN_TEXT);
        assert_eq!(name_of_path(".orig"), PLAIN_TEXT);
    }

    #[test]
    fn merge_tool_temp_names_keep_the_extension() {
        assert_eq!(name_of_path("main_BASE_1234.rs"), "Rust");
        assert_eq!(name_of_path("main_LOCAL_1234.rs"), "Rust");
        assert_eq!(name_of_path("main.REMOTE.5678.rs"), "Rust");
    }

    #[test]
    fn unknown_paths_fall_back_to_plain_text() {
        assert_eq!(name_of_path("notes.txt"), PLAIN_TEXT);
        assert_eq!(name_of_path("LICENSE"), PLAIN_TEXT);
        assert_eq!(name_of_path("archive.tar.gz"), PLAIN_TEXT);
        assert_eq!(name_of_path(".gitignore"), PLAIN_TEXT);
        assert_eq!(name_of_path(""), PLAIN_TEXT);
        assert_eq!(detect(&[]).name(), PLAIN_TEXT);
    }

    #[test]
    fn shebangs_name_languages() {
        assert_eq!(name_of_line("#!/bin/bash"), "Shell");
        assert_eq!(name_of_line("#!/bin/sh -e"), "Shell");
        assert_eq!(name_of_line("#! /usr/bin/env python3"), "Python");
        assert_eq!(name_of_line("#!/usr/bin/python3.11"), "Python");
        assert_eq!(name_of_line("#!/usr/bin/env -S node --no-warnings"), "JavaScript");
        assert_eq!(name_of_line("#!/usr/bin/env -i LANG=C perl"), "Perl");
        assert_eq!(name_of_line("#!/usr/bin/perl -w"), "Perl");
        assert_eq!(name_of_line("#!/usr/bin/env Rscript"), "R");
        assert_eq!(name_of_line("\u{feff}#!/usr/bin/ruby"), "Ruby");
    }

    #[test]
    fn unknown_shebangs_fall_back_to_plain_text() {
        assert_eq!(name_of_line("#!/usr/bin/env"), PLAIN_TEXT);
        assert_eq!(name_of_line("#!"), PLAIN_TEXT);
        assert_eq!(name_of_line("#!/opt/bin/mystery"), PLAIN_TEXT);
        assert_eq!(name_of_line("# just a comment"), PLAIN_TEXT);
    }

    #[test]
    fn first_line_markers_name_languages() {
        assert_eq!(name_of_line("<?xml version=\"1.0\"?>"), "XML");
        assert_eq!(name_of_line("<?php"), "PHP");
        assert_eq!(name_of_line("<!DOCTYPE html>"), "HTML");
        assert_eq!(name_of_line("  <html lang=\"en\">"), "HTML");
        assert_eq!(name_of_line(""), PLAIN_TEXT);
    }

    #[test]
    fn a_candidates_path_is_tried_before_its_first_line() {
        let candidates = [Candidate {
            path: Some(Path::new("a.rs")),
            first_line: "#!/bin/bash",
        }];
        assert_eq!(detect(&candidates).name(), "Rust");
        let candidates = [Candidate {
            path: Some(Path::new("a.txt")),
            first_line: "#!/bin/bash",
        }];
        assert_eq!(detect(&candidates).name(), "Shell");
    }

    #[test]
    fn an_earlier_candidate_is_decided_by_its_first_line_before_a_later_ones_path() {
        let candidates = [
            Candidate {
                path: Some(Path::new("a.txt")),
                first_line: "#!/bin/bash",
            },
            Candidate {
                path: Some(Path::new("b.rs")),
                first_line: "",
            },
        ];
        assert_eq!(detect(&candidates).name(), "Shell");
    }

    #[test]
    fn the_first_candidate_that_names_a_language_wins() {
        let candidates = [
            Candidate {
                path: Some(Path::new("out.txt")),
                first_line: "",
            },
            Candidate {
                path: Some(Path::new("a.py")),
                first_line: "",
            },
            Candidate {
                path: Some(Path::new("b.rb")),
                first_line: "",
            },
        ];
        assert_eq!(detect(&candidates).name(), "Python");
    }

    #[test]
    fn the_table_has_no_duplicates() {
        let mut names = HashSet::new();
        let mut extensions = HashSet::new();
        let mut file_names = HashSet::new();
        for rule in RULES {
            assert!(names.insert(rule.name.to_ascii_lowercase()), "language {} is listed twice", rule.name);
            for extension in rule.extensions {
                assert_eq!(*extension, extension.to_ascii_lowercase(), "extension {extension} must be lower case");
                assert!(extensions.insert(*extension), "extension {extension} belongs to two languages");
            }
            for file_name in rule.file_names {
                assert!(file_names.insert(file_name.to_ascii_lowercase()), "file name {file_name} belongs to two languages");
            }
        }
        assert!(!names.contains(&PLAIN_TEXT.to_ascii_lowercase()));
    }

    #[test]
    fn shebang_programs_are_lower_case_and_unique() {
        let mut interpreters = HashSet::new();
        for rule in RULES {
            for interpreter in rule.interpreters {
                assert_eq!(*interpreter, interpreter.to_ascii_lowercase());
                assert!(interpreters.insert(*interpreter), "program {interpreter} belongs to two languages");
            }
        }
    }

    #[test]
    fn first_line_markers_name_listed_languages() {
        for (_, name) in FIRST_LINE_MARKERS {
            assert!(RULES.iter().any(|rule| rule.name == *name), "no language {name}");
        }
    }

    #[test]
    fn languages_list_plain_text_first_then_sorted() {
        let names: Vec<&str> = Language::all().iter().map(|language| language.name()).collect();
        assert_eq!(names.first(), Some(&PLAIN_TEXT));
        assert_eq!(names.len(), RULES.len() + 1);
        let mut sorted: Vec<&str> = names[1..].to_vec();
        sorted.sort_by_key(|name| name.to_ascii_lowercase());
        assert_eq!(&names[1..], &sorted[..]);
        for language in ["Rust", "C++", "JSON", "YAML", "TOML", "Python"] {
            assert!(names.contains(&language), "{language} is missing");
        }
    }

    fn source(document: Document, path: &str, text: &str) -> DocumentText {
        let lines = if text.is_empty() { Vec::new() } else { vec![0..text.len()] };
        DocumentText {
            document,
            path: PathBuf::from(path),
            label: None,
            eol: Some(EolStyle::Lf),
            text: text.to_string(),
            lines,
        }
    }

    /// A session over `sources` (path and one line of text, for A, B and C in turn) with D at `output_path` when given.
    fn mirror_of(sources: &[(&str, &str)], output_path: Option<&str>) -> Mirror {
        let documents = sources
            .iter()
            .zip([Document::A, Document::B, Document::C])
            .map(|((path, text), document)| source(document, path, text))
            .collect();
        let output = output_path.map(|path| OutputDocument {
            path: Some(PathBuf::from(path)),
            text: String::new(),
            modified: false,
            eol: EolChoice::default(),
        });
        let snapshot = Snapshot {
            revision: Revision(1),
            documents,
            output,
            comparison: Comparison::default(),
            engine: EngineCapabilities::default(),
            merge: Vec::new(),
            options: ComparisonOptions::default(),
            current_region: None,
            char_diffs: Vec::new(),
            alignments: kdiff4_protocol::Alignments::default(),
        };
        Mirror::from_snapshot(snapshot, false)
    }

    #[test]
    fn a_decides_when_the_documents_are_named_for_different_languages() {
        let mirror = mirror_of(&[("base.h", ""), ("left.cpp", ""), ("right.cpp", "")], Some("merged.cpp"));
        assert_eq!(detect_in(&mirror).name(), "C");
        let mirror = mirror_of(&[("a.js", ""), ("b.ts", "")], None);
        assert_eq!(detect_in(&mirror).name(), "JavaScript");
    }

    #[test]
    fn a_first_line_in_a_beats_a_path_in_b() {
        let mirror = mirror_of(&[("run", "#!/usr/bin/env python3"), ("run.sh", "")], None);
        assert_eq!(detect_in(&mirror).name(), "Python");
    }

    #[test]
    fn b_and_c_count_when_a_says_nothing() {
        let mirror = mirror_of(&[("base.txt", ""), ("left.json", ""), ("right.yaml", "")], Some("merged.rs"));
        assert_eq!(detect_in(&mirror).name(), "JSON");
        let mirror = mirror_of(&[("base.txt", ""), ("left.txt", ""), ("right.yaml", "")], Some("merged.rs"));
        assert_eq!(detect_in(&mirror).name(), "YAML");
    }

    #[test]
    fn the_output_counts_last() {
        let mirror = mirror_of(&[("a.txt", ""), ("b.txt", "")], Some("merged.rs"));
        assert_eq!(detect_in(&mirror).name(), "Rust");
        assert_eq!(detect_in(&mirror_of(&[("a.go", ""), ("b.go", "")], None)).name(), "Go");
    }

    #[test]
    fn a_session_is_detected_from_the_first_line_when_no_path_says_anything() {
        let mirror = mirror_of(&[("a", ""), ("b", "#!/usr/bin/env python3")], None);
        assert_eq!(detect_in(&mirror).name(), "Python");
    }

    #[test]
    fn a_session_with_nothing_to_go_on_is_plain_text() {
        let mirror = mirror_of(&[("a.txt", "hello"), ("b.txt", "world")], Some("out.txt"));
        assert_eq!(detect_in(&mirror), Language::plain_text());
    }

    #[test]
    fn a_picked_language_stays_for_the_same_source_files() {
        let mirror = mirror_of(&[("a.rs", ""), ("b.rs", "")], None);
        let mut state = LanguageState::default();
        state.follow(&mirror);
        assert_eq!(state.current().name(), "Rust");
        state.pick(Language { name: "C" }, &mirror);
        state.follow(&mirror);
        assert_eq!(state.current().name(), "C", "a new snapshot of the same files keeps the pick");
    }

    #[test]
    fn a_picked_language_is_replaced_when_the_source_files_change() {
        let mut state = LanguageState::default();
        let mirror = mirror_of(&[("a.rs", ""), ("b.rs", "")], None);
        state.follow(&mirror);
        state.pick(Language { name: "C" }, &mirror);
        state.follow(&mirror_of(&[("a.rs", ""), ("c.py", "")], None));
        assert_eq!(state.current().name(), "Rust", "reloading B from another path detects again");
        state.follow(&mirror_of(&[("x.txt", ""), ("y.txt", "")], None));
        assert_eq!(state.current(), &Language::plain_text());
    }

    #[test]
    fn picking_plain_text_sticks_too() {
        let mirror = mirror_of(&[("a.rs", ""), ("b.rs", "")], None);
        let mut state = LanguageState::default();
        state.follow(&mirror);
        state.pick(Language::plain_text(), &mirror);
        state.follow(&mirror);
        assert_eq!(state.current(), &Language::plain_text());
    }

    #[test]
    fn the_drop_down_returns_the_picked_language() {
        let mut harness = Harness::builder().with_size(vec2(400.0, 600.0)).build_ui_state(
            |ui, picked: &mut Option<Language>| {
                egui_extras::install_image_loaders(ui.ctx());
                if let Some(language) = show_selector(ui, &Language::plain_text()) {
                    *picked = Some(language);
                }
            },
            None,
        );
        harness.run();
        harness.get_by_label(SELECTOR_HINT).click();
        harness.run();
        harness.get_by_label_contains("Dart").click();
        harness.run();
        assert_eq!(harness.state(), &Some(Language { name: "Dart" }));
    }

    #[test]
    fn every_icon_loads_as_an_image() {
        let mut harness = Harness::builder().with_size(vec2(100.0, 100.0)).build_ui_state(
            |ui, failed: &mut Vec<String>| {
                egui_extras::install_image_loaders(ui.ctx());
                failed.clear();
                for language in Language::all() {
                    let loaded = language.icon().load_for_size(ui.ctx(), vec2(ICON_SIZE, ICON_SIZE));
                    if !matches!(loaded, Ok(egui::load::TexturePoll::Ready { .. })) {
                        failed.push(format!("{}: {:?}", language.name(), loaded.err()));
                    }
                }
            },
            Vec::new(),
        );
        harness.run();
        assert!(harness.state().is_empty(), "icons that do not load: {:?}", harness.state());
    }

    #[test]
    fn every_language_has_an_icon() {
        let names: Vec<&str> = Language::all().iter().map(|language| language.name()).collect();
        let icons: Vec<&str> = ICONS.iter().map(|icon| icon.language).collect();
        let mut sorted = icons.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), icons.len(), "a language has two icons");
        for name in &names {
            assert!(icons.contains(name), "{name} has no icon");
        }
        assert_eq!(icons.len(), names.len(), "an icon belongs to no language");
        for icon in ICONS {
            let start = String::from_utf8_lossy(&icon.svg[..icon.svg.len().min(5)]).to_string();
            assert!(start.starts_with("<svg") || start.starts_with("<?xml"), "the icon of {} is not an SVG", icon.language);
        }
    }
}
