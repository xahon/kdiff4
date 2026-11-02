use std::sync::Arc;
use std::thread::JoinHandle;

use egui::{FontData, FontDefinitions, FontFamily};
use fontique::{Collection, CollectionOptions};
use skrifa::instance::{LocationRef, Size};
use skrifa::{FontRef, MetadataProvider};


/// Key of the user's chosen font in the egui font definitions.
const CHOSEN_FONT_KEY: &str = "kdiff4-monospace";


/// An installed monospace font family, already loaded (system fonts are memory-mapped, so this is cheap).
#[derive(Clone)]
pub struct MonospaceFamily {
    pub name: String,
    pub data: FontData,
}


/// The built-in fonts, with `chosen` put first for monospace text. Proportional text keeps the built-in font.
pub fn font_definitions(chosen: Option<FontData>) -> FontDefinitions {
    let mut definitions = FontDefinitions::default();
    if let Some(data) = chosen {
        definitions.font_data.insert(CHOSEN_FONT_KEY.to_string(), Arc::new(data));
        definitions.families.entry(FontFamily::Monospace).or_default().insert(0, CHOSEN_FONT_KEY.to_string());
    }
    definitions
}


/// Trusts the font's fixed-pitch flag, and otherwise compares the advances of narrow and wide characters, since some monospace fonts don't set the flag.
pub fn is_monospace(bytes: &[u8], index: u32) -> bool {
    let font = match FontRef::from_index(bytes, index) {
        Ok(font) => font,
        Err(_) => return false,
    };
    if font.metrics(Size::unscaled(), LocationRef::default()).is_monospace {
        return true;
    }
    let charmap = font.charmap();
    let glyph_metrics = font.glyph_metrics(Size::unscaled(), LocationRef::default());
    let advances: Vec<Option<f32>> = ['i', 'M', 'W', '.', '0']
        .iter()
        .map(|character| charmap.map(*character).and_then(|glyph| glyph_metrics.advance_width(glyph)))
        .collect();
    match advances[0] {
        Some(first) if first > 0.0 => advances.iter().all(|advance| *advance == Some(first)),
        _ => false,
    }
}


/// Every installed monospace family, sorted by name. Slow: reads every installed font, so run it off the UI thread.
pub fn find_monospace_families() -> Vec<MonospaceFamily> {
    let mut collection = Collection::new(CollectionOptions { shared: false, system_fonts: true });
    let names: Vec<String> = collection
        .family_names()
        .filter(|name| !name.starts_with('@'))
        .map(str::to_string)
        .collect();
    let mut families: Vec<MonospaceFamily> = names
        .into_iter()
        .filter_map(|name| load_from(&mut collection, &name).map(|data| MonospaceFamily { name, data }))
        .filter(|family| is_monospace(family.data.bytes(), family.data.index))
        .collect();
    families.sort_by_key(|family| family.name.to_lowercase());
    families.dedup_by(|a, b| a.name.eq_ignore_ascii_case(&b.name));
    families
}


/// The regular face of an installed family, or `None` if it isn't installed.
pub fn load_family(name: &str) -> Option<FontData> {
    let mut collection = Collection::new(CollectionOptions { shared: false, system_fonts: true });
    load_from(&mut collection, name)
}


fn load_from(collection: &mut Collection, name: &str) -> Option<FontData> {
    let family = collection.family_by_name(name)?;
    let font = family.default_font()?;
    let blob = font.load(None)?;
    Some(FontData::from_blob(Arc::new(blob), font.index()))
}


/// Runs `find_monospace_families` on a background thread, once.
#[derive(Default)]
pub struct FontScan {
    running: Option<JoinHandle<Vec<MonospaceFamily>>>,
    families: Option<Vec<MonospaceFamily>>,
}


impl FontScan {
    /// Starts the scan unless it has started already; `ctx` is repainted when it finishes.
    pub fn start(&mut self, ctx: &egui::Context) {
        if self.running.is_some() || self.families.is_some() {
            return;
        }
        let repaint_ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("kdiff4-font-scan".to_string())
            .spawn(move || {
                let families = find_monospace_families();
                repaint_ctx.request_repaint();
                families
            });
        match spawned {
            Ok(handle) => self.running = Some(handle),
            Err(error) => {
                log::error!("cannot start the font scan: {error}");
                self.families = Some(Vec::new());
            }
        }
    }


    /// The families once the scan has finished; `None` while it runs or before it starts.
    pub fn families(&mut self) -> Option<&[MonospaceFamily]> {
        if self.running.as_ref().is_some_and(JoinHandle::is_finished)
            && let Some(handle) = self.running.take()
        {
            let families = handle.join().unwrap_or_else(|_| {
                log::error!("the font scan panicked");
                Vec::new()
            });
            log::info!("found {} monospace font families", families.len());
            self.families = Some(families);
        }
        self.families.as_deref()
    }
}


#[cfg(test)]
mod tests {
    use super::*;


    fn builtin_font(name: &str) -> Arc<FontData> {
        FontDefinitions::default().font_data[name].clone()
    }


    #[test]
    fn builtin_monospace_font_is_detected() {
        let hack = builtin_font("Hack");
        assert!(is_monospace(hack.bytes(), hack.index));
    }


    #[test]
    fn builtin_proportional_font_is_rejected() {
        let ubuntu = builtin_font("Ubuntu-Light");
        assert!(!is_monospace(ubuntu.bytes(), ubuntu.index));
    }


    #[test]
    fn garbage_is_not_monospace() {
        assert!(!is_monospace(b"not a font", 0));
        assert!(!is_monospace(&[], 0));
    }


    #[test]
    fn chosen_font_goes_first_for_monospace_only() {
        let hack = builtin_font("Hack");
        let definitions = font_definitions(Some((*hack).clone()));
        assert_eq!(definitions.families[&FontFamily::Monospace][0], CHOSEN_FONT_KEY);
        assert!(!definitions.families[&FontFamily::Proportional].iter().any(|key| key == CHOSEN_FONT_KEY));
        assert!(definitions.font_data.contains_key(CHOSEN_FONT_KEY));
    }


    #[test]
    fn no_chosen_font_keeps_builtin_definitions() {
        assert_eq!(font_definitions(None), FontDefinitions::default());
    }


    #[test]
    fn installed_families_are_monospace_and_sorted() {
        let families = find_monospace_families();
        let names: Vec<String> = families
            .iter()
            .map(|family| family.name.to_lowercase())
            .collect();
        assert!(names.is_sorted());
        assert!(families.iter().all(|family| is_monospace(family.data.bytes(), family.data.index)));
        assert!(families.iter().all(|family| !family.name.starts_with('@')));
    }


    #[test]
    fn unknown_family_is_not_loaded() {
        assert!(load_family("No Such Font Family 1234").is_none());
    }
}
