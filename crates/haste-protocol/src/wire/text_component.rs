//! Text components in their two wire forms: network NBT (play and
//! configuration) and a JSON string (status, login disconnect). Both decode
//! through wire/component.rs, which goes via `serde_json::Value`.

use haste_core::Component;

use super::component::{from_json_str, from_nbt, to_json_string, to_nbt};
use super::{DecodeResult, Reader, Wire, Writer, MAX_JSON_COMPONENT};

impl Reader<'_> {
    /// Text Component (network NBT form).
    pub fn component(&mut self) -> DecodeResult<Component> {
        match self.nbt_tag()? {
            None => Ok(Component::default()),
            Some(tag) => from_nbt(&tag),
        }
    }

    /// JSON Text Component (a string holding JSON).
    pub fn json_component(&mut self) -> DecodeResult<Component> {
        let s = self.string_max(MAX_JSON_COMPONENT)?;
        from_json_str(&s)
    }
}

impl Writer {
    /// Text Component in network NBT form.
    pub fn component(&mut self, c: &Component) {
        let tag = to_nbt(c);
        self.nbt_tag(Some(&tag));
    }
    /// JSON Text Component.
    pub fn json_component(&mut self, c: &Component) {
        self.string(&to_json_string(c));
    }
}

impl Wire for Component {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        r.component()
    }
    fn write(&self, w: &mut Writer) {
        w.component(self)
    }
}
