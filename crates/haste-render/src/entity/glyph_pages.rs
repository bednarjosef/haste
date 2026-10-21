//! The font pages the entity atlas holds, keyed by a `Copy` value: a glyph's page is found with a
//! map lookup and no `Identifier` is built per glyph per frame (world text asks for its pages
//! every frame). Each page's `Identifier` is built once, the first time the page is needed.

use super::*;

/// A font page the entity atlas holds: a bitmap provider's page, or a unihex glyph by index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum GlyphPage {
    Bitmap(u16),
    Hex(u32),
}

impl GlyphPage {
    /// The page's texture key in the atlas (`haste:font/N` / `haste:hex/N`).
    fn identifier(self) -> Identifier {
        match self {
            GlyphPage::Bitmap(page) => Identifier::new("haste", &format!("font/{page}")),
            GlyphPage::Hex(index) => Identifier::new("haste", &format!("hex/{index}")),
        }
    }
}

/// The texture key of every glyph page the entity atlas has been asked for.
#[derive(Default)]
pub(super) struct GlyphPageKeys(HashMap<GlyphPage, Identifier>);

impl GlyphPageKeys {
    /// The page's texture key. Built on the first request for the page; a plain lookup after.
    pub(super) fn get_or_insert(&mut self, page: GlyphPage) -> &Identifier {
        self.0.entry(page).or_insert_with(|| page.identifier())
    }

    /// The page's texture key if it was requested before.
    pub(super) fn get(&self, page: GlyphPage) -> Option<&Identifier> {
        self.0.get(&page)
    }

    /// How many pages have a key.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.0.len()
    }
}
