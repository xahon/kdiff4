use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;


pub const CONFIG_VERSION: u32 = 1;
pub const MIN_FONT_SIZE: f32 = 6.0;
pub const MAX_FONT_SIZE: f32 = 48.0;
pub const DEFAULT_FONT_SIZE: f32 = 13.0;
pub const MAX_CORNER_RADIUS: u8 = 12;
pub const DEFAULT_CORNER_RADIUS: u8 = 2;
pub const MAX_JUMP_CONTEXT_LINES: u8 = 20;
pub const DEFAULT_JUMP_CONTEXT_LINES: u8 = 12;
pub const MIN_SMOOTH_SCROLL_MS: u16 = 20;
pub const MAX_SMOOTH_SCROLL_MS: u16 = 1000;
pub const DEFAULT_SMOOTH_SCROLL_MS: u16 = 150;
pub const MIN_SCROLL_STEP_LINES: u8 = 1;
pub const MAX_SCROLL_STEP_LINES: u8 = 20;
pub const DEFAULT_SCROLL_STEP_LINES: u8 = 3;
/// A translucent yellow: text keeps its contrast on both themes.
pub const DEFAULT_CURRENT_REGION_COLOR: [u8; 4] = [255, 214, 0, 72];
pub const MAX_REGION_CORNER_RADIUS: u8 = 8;
pub const DEFAULT_REGION_CORNER_RADIUS: u8 = 2;
pub const MAX_OTHER_REGION_BORDER_PERCENT: u8 = 100;
pub const DEFAULT_OTHER_REGION_BORDER_PERCENT: u8 = 45;
/// Cyan, a color none of the documents, the red or the jumped-to region uses.
pub const DEFAULT_ALIGNMENT_COLOR: [u8; 4] = [0, 188, 212, 255];


#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("cannot read {}: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("cannot parse {}: {source}", path.display())]
    Parse { path: PathBuf, source: toml::de::Error },
    #[error("cannot serialize the GUI config: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("cannot write {}: {source}", path.display())]
    Write { path: PathBuf, source: io::Error },
    #[error("cannot back up {}: {source}", path.display())]
    Backup { path: PathBuf, source: io::Error },
}


/// Contents of `gui.toml`. Missing keys take their defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GuiConfig {
    pub version: u32,
    pub view: ViewConfig,
    pub recent: RecentConfig,
}


impl Default for GuiConfig {
    fn default() -> Self {
        GuiConfig { version: CONFIG_VERSION, view: ViewConfig::default(), recent: RecentConfig::default() }
    }
}


#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewConfig {
    /// Monospace font family of the panes; `None` is the built-in font.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    pub font_size: f32,
    pub font_hinting: bool,
    pub subpixel_positioning: bool,
    /// Rounding of widgets, menus, dialogs, panes and the window frame, in pixels.
    pub corner_radius: u8,
    pub show_line_numbers: bool,
    /// The region gutters of the source panes. D's column of line origins is always shown.
    pub show_gutters: bool,
    /// D lines up with the source panes: every region gets the same rows in every pane, padded with void space.
    pub align_output: bool,
    /// The overview strip: the current line of every document, between the source panes and D.
    pub show_overview_strip: bool,
    /// Rows kept above a region that navigation jumps to.
    pub jump_context_lines: u8,
    /// Wheel scrolling and jumps glide to where they go instead of moving at once.
    pub smooth_scrolling: bool,
    /// How long a smooth scroll takes, in milliseconds.
    pub smooth_scroll_ms: u16,
    /// Lines one notch of the mouse wheel scrolls.
    pub scroll_step_lines: u8,
    /// RGBA background of the jumped-to region, void space included.
    pub current_region_color: [u8; 4],
    /// Rounding of the region boxes, in pixels.
    pub region_corner_radius: u8,
    /// Opacity of the borders of the regions other than the jumped-to one, in percent.
    pub other_region_border_percent: u8,
    /// RGBA of the alignment marks in the gutters of the source panes.
    pub alignment_color: [u8; 4],
    /// Previous and Next Difference also stop at the first region of every applied alignment.
    pub alignment_stops: bool,
    pub theme: ThemeChoice,
    pub light_colors: ThemeColors,
    pub dark_colors: ThemeColors,
    pub whitespace: WhitespaceConfig,
}


impl Default for ViewConfig {
    fn default() -> Self {
        ViewConfig { font_family: None, font_size: DEFAULT_FONT_SIZE, font_hinting: true, subpixel_positioning: true, corner_radius: DEFAULT_CORNER_RADIUS, show_line_numbers: true, show_gutters: true, align_output: false, show_overview_strip: true, jump_context_lines: DEFAULT_JUMP_CONTEXT_LINES, smooth_scrolling: true, smooth_scroll_ms: DEFAULT_SMOOTH_SCROLL_MS, scroll_step_lines: DEFAULT_SCROLL_STEP_LINES, current_region_color: DEFAULT_CURRENT_REGION_COLOR, region_corner_radius: DEFAULT_REGION_CORNER_RADIUS, other_region_border_percent: DEFAULT_OTHER_REGION_BORDER_PERCENT, alignment_color: DEFAULT_ALIGNMENT_COLOR, alignment_stops: true, theme: ThemeChoice::Light, light_colors: ThemeColors::default(), dark_colors: ThemeColors::default(), whitespace: WhitespaceConfig::default() }
    }
}


impl ViewConfig {
    /// Brings out-of-range values from a hand-edited file back into range.
    pub fn sanitized(mut self) -> ViewConfig {
        self.font_size = if self.font_size.is_finite() { self.font_size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE) } else { DEFAULT_FONT_SIZE };
        self.corner_radius = self.corner_radius.min(MAX_CORNER_RADIUS);
        self.jump_context_lines = self.jump_context_lines.min(MAX_JUMP_CONTEXT_LINES);
        self.smooth_scroll_ms = self.smooth_scroll_ms.clamp(MIN_SMOOTH_SCROLL_MS, MAX_SMOOTH_SCROLL_MS);
        self.scroll_step_lines = self.scroll_step_lines.clamp(MIN_SCROLL_STEP_LINES, MAX_SCROLL_STEP_LINES);
        self.region_corner_radius = self.region_corner_radius.min(MAX_REGION_CORNER_RADIUS);
        self.other_region_border_percent = self.other_region_border_percent.min(MAX_OTHER_REGION_BORDER_PERCENT);
        if self.font_family.as_deref().is_some_and(|family| family.trim().is_empty()) {
            self.font_family = None;
        }
        self
    }


    /// The color overrides of the light or the dark theme.
    pub fn colors(&self, dark: bool) -> &ThemeColors {
        if dark { &self.dark_colors } else { &self.light_colors }
    }


    pub fn colors_mut(&mut self, dark: bool) -> &mut ThemeColors {
        if dark { &mut self.dark_colors } else { &mut self.light_colors }
    }
}


/// The theme the GUI uses; `System` follows the operating system.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeChoice {
    #[default]
    Light,
    Dark,
    System,
}


/// Colors the user picked for one theme, as sRGB; `None` keeps the theme's own color.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeColors {
    /// Text of menus, dialogs, buttons and labels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ui_text: Option<[u8; 3]>,
    /// Text in the panes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pane_text: Option<[u8; 3]>,
    /// The color of document A: its pane header, gutter blocks and char-diff fill in the other panes. The fills are derived from it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_a: Option<[u8; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_b: Option<[u8; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_c: Option<[u8; 3]>,
    /// The red of text and gutter blocks that differ from every other document, and of the `<Merge Conflict>` marker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub red: Option<[u8; 3]>,
}


/// Which whitespace the panes mark. Each kind shows only while `render` is on too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WhitespaceConfig {
    pub render: bool,
    pub spaces: bool,
    pub tabs: bool,
    /// The line feed that ends a line, alone or after a carriage return.
    pub line_feeds: bool,
    /// A carriage return, ending a line or inside one.
    pub carriage_returns: bool,
    /// Non-breaking and other Unicode spaces.
    pub other: bool,
}


impl Default for WhitespaceConfig {
    fn default() -> Self {
        WhitespaceConfig { render: true, spaces: true, tabs: true, line_feeds: true, carriage_returns: true, other: true }
    }
}


#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecentConfig {
    /// Directory of the last file confirmed in a file dialog.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_dir: Option<PathBuf>,
}


pub fn default_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("kdiff4").join("gui.toml"))
}


/// A missing file gives the defaults.
pub fn load(path: &Path) -> Result<GuiConfig, ConfigError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(GuiConfig::default()),
        Err(source) => return Err(ConfigError::Read { path: path.to_path_buf(), source }),
    };
    let mut config: GuiConfig = toml::from_str(&text).map_err(|source| ConfigError::Parse { path: path.to_path_buf(), source })?;
    config.view = config.view.sanitized();
    Ok(config)
}


/// Renames a file that failed to parse to `<name>.bak`, so saving settings doesn't destroy it.
pub fn back_up_broken(path: &Path) -> Result<(), ConfigError> {
    if !path.exists() {
        return Ok(());
    }
    let mut backup_name = path.file_name().unwrap_or_default().to_os_string();
    backup_name.push(".bak");
    std::fs::rename(path, path.with_file_name(backup_name)).map_err(|source| ConfigError::Backup { path: path.to_path_buf(), source })
}


/// Command that opens `path` with the system's default handler for its file type.
pub fn open_command(path: &Path) -> std::process::Command {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let mut command = std::process::Command::new(program);
    command.arg(path);
    command
}


/// Writes atomically: a temp file next to `path`, then a rename over it.
pub fn save(path: &Path, config: &GuiConfig) -> Result<(), ConfigError> {
    let text = toml::to_string(config)?;
    let write_error = |source| ConfigError::Write { path: path.to_path_buf(), source };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(write_error)?;
    }
    let mut temp_name = path.file_name().unwrap_or_default().to_os_string();
    temp_name.push(".tmp");
    let temp_path = path.with_file_name(temp_name);
    std::fs::write(&temp_path, text).map_err(write_error)?;
    if let Err(source) = std::fs::rename(&temp_path, path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(write_error(source));
    }
    Ok(())
}


#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;


    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kdiff4-gui-config-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }


    #[test]
    fn open_command_passes_path_as_single_argument() {
        let path = PathBuf::from("C:/some dir/gui.toml");
        let command = open_command(&path);
        let expected = if cfg!(windows) { "explorer" } else if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        assert_eq!(command.get_program(), expected);
        assert_eq!(command.get_args().collect::<Vec<_>>(), vec![path.as_os_str()]);
    }


    #[test]
    fn missing_file_gives_defaults() {
        let dir = temp_dir("missing");
        assert_eq!(load(&dir.join("gui.toml")).unwrap(), GuiConfig::default());
    }


    #[test]
    fn save_then_load_round_trips() {
        let dir = temp_dir("round-trip");
        let path = dir.join("nested").join("gui.toml");
        let mut config = GuiConfig::default();
        config.recent.last_dir = Some(PathBuf::from("C:/some/dir"));
        config.view = ViewConfig { font_family: Some("Consolas".to_string()), font_size: 15.5, font_hinting: false, subpixel_positioning: false, corner_radius: 0, show_line_numbers: false, show_gutters: false, align_output: true, show_overview_strip: false, jump_context_lines: 7, smooth_scrolling: false, smooth_scroll_ms: 400, scroll_step_lines: 5, current_region_color: [1, 2, 3, 4], region_corner_radius: 4, other_region_border_percent: 30, alignment_color: [9, 8, 7, 6], alignment_stops: false, theme: ThemeChoice::System, light_colors: ThemeColors { ui_text: Some([1, 2, 3]), pane_text: None, document_a: Some([4, 5, 6]), document_b: None, document_c: Some([7, 8, 9]), red: Some([10, 11, 12]) }, dark_colors: ThemeColors { ui_text: None, pane_text: Some([250, 251, 252]), document_a: None, document_b: Some([13, 14, 15]), document_c: None, red: None }, whitespace: WhitespaceConfig { render: true, spaces: false, tabs: true, line_feeds: false, carriage_returns: true, other: false } };
        save(&path, &config).unwrap();
        assert_eq!(load(&path).unwrap(), config);
        assert!(!dir.join("nested").join("gui.toml.tmp").exists());
    }


    #[test]
    fn save_replaces_existing_file() {
        let dir = temp_dir("replace");
        let path = dir.join("gui.toml");
        fs::write(&path, "version = 1\n[recent]\nlast_dir = \"old\"\n").unwrap();
        let mut config = GuiConfig::default();
        config.recent.last_dir = Some(PathBuf::from("new"));
        save(&path, &config).unwrap();
        assert_eq!(load(&path).unwrap().recent.last_dir, Some(PathBuf::from("new")));
    }


    #[test]
    fn overview_strip_is_shown_unless_turned_off() {
        let dir = temp_dir("overview-strip");
        let path = dir.join("gui.toml");
        fs::write(&path, "[view]
font_size = 14.0
").unwrap();
        assert!(load(&path).unwrap().view.show_overview_strip);
        fs::write(&path, "[view]
show_overview_strip = false
").unwrap();
        assert!(!load(&path).unwrap().view.show_overview_strip);
    }


    #[test]
    fn missing_keys_take_defaults_and_unknown_keys_are_ignored() {
        let dir = temp_dir("partial");
        let path = dir.join("gui.toml");
        fs::write(&path, "future_key = 3\n[recent]\n").unwrap();
        assert_eq!(load(&path).unwrap(), GuiConfig::default());
    }


    #[test]
    fn alignment_keys_default_to_cyan_with_stops_and_load_from_toml() {
        let defaults = ViewConfig::default();
        assert_eq!(defaults.alignment_color, DEFAULT_ALIGNMENT_COLOR);
        assert!(defaults.alignment_stops);
        let dir = temp_dir("alignment");
        let path = dir.join("gui.toml");
        fs::write(&path, "[view]\nalignment_color = [1, 2, 3, 4]\nalignment_stops = false\n").unwrap();
        let view = load(&path).unwrap().view;
        assert_eq!((view.alignment_color, view.alignment_stops), ([1, 2, 3, 4], false));
    }


    #[test]
    fn view_values_out_of_range_are_sanitized() {
        let dir = temp_dir("sanitize");
        let path = dir.join("gui.toml");
        fs::write(&path, "[view]\nfont_family = \"  \"\nfont_size = 500.0\ncorner_radius = 200\njump_context_lines = 99\nsmooth_scroll_ms = 9999\nscroll_step_lines = 0\nregion_corner_radius = 50\nother_region_border_percent = 250\n").unwrap();
        let view = load(&path).unwrap().view;
        assert_eq!(view.font_family, None);
        assert_eq!(view.region_corner_radius, MAX_REGION_CORNER_RADIUS);
        assert_eq!(view.other_region_border_percent, MAX_OTHER_REGION_BORDER_PERCENT);
        assert_eq!(view.corner_radius, MAX_CORNER_RADIUS);
        assert_eq!(view.jump_context_lines, MAX_JUMP_CONTEXT_LINES);
        assert_eq!((view.smooth_scroll_ms, view.scroll_step_lines), (MAX_SMOOTH_SCROLL_MS, MIN_SCROLL_STEP_LINES));

        assert_eq!(view.font_size, MAX_FONT_SIZE);
        let tiny = ViewConfig { font_size: 1.0, ..ViewConfig::default() }.sanitized();
        assert_eq!(tiny.font_size, MIN_FONT_SIZE);
        let nan = ViewConfig { font_size: f32::NAN, ..ViewConfig::default() }.sanitized();
        assert_eq!(nan.font_size, DEFAULT_FONT_SIZE);
    }


    #[test]
    fn appearance_and_whitespace_keys_load_from_toml() {
        let dir = temp_dir("appearance");
        let path = dir.join("gui.toml");
        fs::write(&path, "[view]\ntheme = \"dark\"\n[view.dark_colors]\npane_text = [10, 20, 30]\n[view.whitespace]\ntabs = false\n").unwrap();
        let view = load(&path).unwrap().view;
        assert_eq!(view.theme, ThemeChoice::Dark);
        assert_eq!(view.dark_colors, ThemeColors { pane_text: Some([10, 20, 30]), ..ThemeColors::default() });
        assert_eq!(view.light_colors, ThemeColors::default());
        assert_eq!(view.whitespace, WhitespaceConfig { tabs: false, ..WhitespaceConfig::default() });
    }


    #[test]
    fn whitespace_is_rendered_by_default() {
        let whitespace = ViewConfig::default().whitespace;
        assert!(whitespace.render && whitespace.spaces && whitespace.tabs && whitespace.line_feeds && whitespace.carriage_returns && whitespace.other);
    }


    #[test]
    fn colors_pick_the_theme() {
        let mut view = ViewConfig::default();
        view.colors_mut(true).ui_text = Some([9, 9, 9]);
        assert_eq!(view.colors(true).ui_text, Some([9, 9, 9]));
        assert_eq!(view.colors(false).ui_text, None);
    }


    #[test]
    fn document_and_red_colors_are_read_per_theme_and_the_old_char_diff_red_is_ignored() {
        let dir = temp_dir("document-colors");
        let path = dir.join("gui.toml");
        fs::write(&path, "[view]\nchar_diff_red = [1, 2, 3]\n[view.light_colors]\ndocument_a = [10, 20, 30]\nred = [200, 0, 0]\n[view.dark_colors]\ndocument_c = [40, 50, 60]\n").unwrap();
        let view = load(&path).unwrap().view;
        assert_eq!(view.light_colors, ThemeColors { document_a: Some([10, 20, 30]), red: Some([200, 0, 0]), ..ThemeColors::default() });
        assert_eq!(view.dark_colors, ThemeColors { document_c: Some([40, 50, 60]), ..ThemeColors::default() });
    }


    #[test]
    fn back_up_broken_renames_file() {
        let dir = temp_dir("backup");
        let path = dir.join("gui.toml");
        back_up_broken(&path).unwrap();
        fs::write(&path, "[recent\n").unwrap();
        back_up_broken(&path).unwrap();
        assert!(!path.exists());
        assert_eq!(fs::read_to_string(dir.join("gui.toml.bak")).unwrap(), "[recent\n");
    }


    #[test]
    fn broken_file_is_a_parse_error() {
        let dir = temp_dir("broken");
        let path = dir.join("gui.toml");
        fs::write(&path, "[recent\n").unwrap();
        assert!(matches!(load(&path), Err(ConfigError::Parse { .. })));
    }
}
