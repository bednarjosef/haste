//! Tests for the glyph page keys: a repeated bitmap or hex page reuses its key and adds no entry.

use super::glyph_pages::{GlyphPage, GlyphPageKeys};
use super::*;

#[test]
fn repeated_bitmap_page_reuses_its_key() {
    let mut keys = GlyphPageKeys::default();
    let first = keys.get_or_insert(GlyphPage::Bitmap(3)).clone();
    assert_eq!(first, Identifier::new("haste", "font/3"));
    assert_eq!(keys.len(), 1);
    let again = keys.get_or_insert(GlyphPage::Bitmap(3)).clone();
    assert_eq!(again, first);
    assert_eq!(keys.len(), 1, "the second lookup must not add an entry");
    assert_eq!(keys.get(GlyphPage::Bitmap(3)), Some(&first));
    assert_eq!(keys.get(GlyphPage::Bitmap(4)), None);
}

#[test]
fn repeated_hex_page_reuses_its_key() {
    let mut keys = GlyphPageKeys::default();
    let first = keys.get_or_insert(GlyphPage::Hex(0x4e2d)).clone();
    assert_eq!(first, Identifier::new("haste", "hex/20013"));
    assert_eq!(keys.len(), 1);
    let again = keys.get_or_insert(GlyphPage::Hex(0x4e2d)).clone();
    assert_eq!(again, first);
    assert_eq!(keys.len(), 1, "the second lookup must not add an entry");
    // A hex index and a bitmap page with the same number are different pages.
    keys.get_or_insert(GlyphPage::Bitmap(20013));
    assert_eq!(keys.len(), 2);
}
