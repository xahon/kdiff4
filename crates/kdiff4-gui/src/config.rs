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
pub const DEFAULT_JUMP_CONTEXT_LINES: u8 = 3;
/// A light blue wash: visible on every shade, and text keeps its contrast.
pub const DEFAULT_CURRENT_REGION_COLOR: [u8; 4] = [40, 110, 255, 28];


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
    /// Rows kept above a region that navigation jumps to.
    pub jump_context_lines: u8,
    /// RGBA wash over the rows of the current region.
    pub current_region_color: [u8; 4],
}


impl Default for ViewConfig {
    fn default() -> Self {
        ViewConfig { font_family: None, font_size: DEFAULT_FONT_SIZE, font_hinting: true, subpixel_positioning: true, corner_radius: DEFAULT_CORNER_RADIUS, show_line_numbers: true, show_gutters: true, align_output: false, jump_context_lines: DEFAULT_JUMP_CONTEXT_LINES, current_region_color: DEFAULT_CURRENT_REGION_COLOR }
    }
}


impl ViewConfig {
    /// Brings out-of-range values from a hand-edited file back into range.
    pub fn sanitized(mut self) -> ViewConfig {
        self.font_size = if self.font_size.is_finite() { self.font_size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE) } else { DEFAULT_FONT_SIZE };
        self.corner_radius = self.corner_radius.min(MAX_CORNER_RADIUS);
        self.jump_context_lines = self.jump_context_lines.min(MAX_JUMP_CONTEXT_LINES);
        if self.font_family.as_deref().is_some_and(|family| family.trim().is_empty()) {
            self.font_family = None;
        }
        self
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
        config.view = ViewConfig { font_family: Some("Consolas".to_string()), font_size: 15.5, font_hinting: false, subpixel_positioning: false, corner_radius: 0, show_line_numbers: false, show_gutters: false, align_output: true, jump_context_lines: 7, current_region_color: [1, 2, 3, 4] };
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
    fn missing_keys_take_defaults_and_unknown_keys_are_ignored() {
        let dir = temp_dir("partial");
        let path = dir.join("gui.toml");
        fs::write(&path, "future_key = 3\n[recent]\n").unwrap();
        assert_eq!(load(&path).unwrap(), GuiConfig::default());
    }


    #[test]
    fn view_values_out_of_range_are_sanitized() {
        let dir = temp_dir("sanitize");
        let path = dir.join("gui.toml");
        fs::write(&path, "[view]\nfont_family = \"  \"\nfont_size = 500.0\ncorner_radius = 200\njump_context_lines = 99\n").unwrap();
        let view = load(&path).unwrap().view;
        assert_eq!(view.font_family, None);
        assert_eq!(view.corner_radius, MAX_CORNER_RADIUS);
        assert_eq!(view.jump_context_lines, MAX_JUMP_CONTEXT_LINES);
        assert_eq!(view.font_size, MAX_FONT_SIZE);
        let tiny = ViewConfig { font_size: 1.0, ..ViewConfig::default() }.sanitized();
        assert_eq!(tiny.font_size, MIN_FONT_SIZE);
        let nan = ViewConfig { font_size: f32::NAN, ..ViewConfig::default() }.sanitized();
        assert_eq!(nan.font_size, DEFAULT_FONT_SIZE);
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
