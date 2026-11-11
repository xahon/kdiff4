use egui::{IconData, Image};


pub struct Icon {
    uri: &'static str,
    bytes: &'static [u8],
}


impl Icon {
    pub fn image(&self) -> Image<'static> {
        Image::from_bytes(self.uri, self.bytes)
    }
}


macro_rules! svg {
    ($name:literal) => {
        Icon {
            uri: concat!("bytes://icons/", $name, ".svg"),
            bytes: include_bytes!(concat!("../../../res/icons/", $name, ".svg")),
        }
    };
}


pub const SHOW_VOID: Icon = svg!("show-void");
pub const OVERVIEW: Icon = svg!("overview");
pub const MINIMAP: Icon = svg!("minimap");
pub const INDEPENDENT_SCROLL: Icon = svg!("independent-scroll");
pub const IGNORE_WHITESPACE: Icon = svg!("ignore-whitespace");
pub const PREVIOUS_MATCH: Icon = svg!("previous-match");
pub const NEXT_MATCH: Icon = svg!("next-match");
pub const CLOSE: Icon = svg!("close");


pub fn application() -> IconData {
    const SIZE: u32 = 64;
    const RASTER: &[u8; SIZE as usize * SIZE as usize * 4] = include_bytes!(concat!(env!("OUT_DIR"), "/kdiff4.rgba"));
    IconData { rgba: RASTER.to_vec(), width: SIZE, height: SIZE }
}


#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn button_icons_load_as_svg_images() {
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        for icon in [SHOW_VOID, OVERVIEW, MINIMAP, INDEPENDENT_SCROLL, IGNORE_WHITESPACE, PREVIOUS_MATCH, NEXT_MATCH, CLOSE] {
            assert!(matches!(icon.image().load_for_size(&ctx, egui::Vec2::splat(18.0)), Ok(egui::load::TexturePoll::Ready { .. })), "failed to load {}", icon.uri);
        }
    }


    #[test]
    fn application_icon_contains_the_rasterized_svg() {
        let icon = application();
        assert_eq!((icon.width, icon.height), (64, 64));
        assert_eq!(icon.rgba.len(), 64 * 64 * 4);
        assert_eq!(&icon.rgba[..4], &[0, 0, 0, 0]);
        assert!(icon.rgba.chunks_exact(4).any(|pixel| pixel == [255, 255, 255, 255]), "the white K did not render");
        assert!(icon.rgba.chunks_exact(4).any(|pixel| pixel == [204, 51, 51, 255]), "the red 4 did not render");
    }
}
