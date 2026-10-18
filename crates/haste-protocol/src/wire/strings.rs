//! Strings and identifiers: length-prefixed UTF-8 with a UTF-16 code unit
//! limit, and `ns:path` identifiers whose namespace defaults to `minecraft`.

use haste_core::Identifier;

use super::{DecodeError, DecodeResult, Reader, Wire, Writer, MAX_STRING};

impl Reader<'_> {
    /// String with a limit of `max` UTF-16 code units (vanilla counts Java chars).
    pub fn string_max(&mut self, max: usize) -> DecodeResult<String> {
        let len = self.len_prefix(max.saturating_mul(3))?;
        let bytes = self.take(len)?;
        let s = std::str::from_utf8(bytes)
            .map_err(|_| DecodeError::Invalid("string is not UTF-8".into()))?;
        if s.len() > max && s.encode_utf16().nth(max).is_some() {
            return Err(DecodeError::TooLarge(format!(
                "string longer than {max} chars"
            )));
        }
        Ok(s.to_owned())
    }
    /// String (32767).
    pub fn string(&mut self) -> DecodeResult<String> {
        self.string_max(MAX_STRING)
    }

    /// Identifier (`ns:path`, namespace defaults to `minecraft`).
    pub fn identifier(&mut self) -> DecodeResult<Identifier> {
        let s = self.string()?;
        Identifier::parse(&s).map_err(|e| DecodeError::Invalid(e.to_string()))
    }
}

impl Writer {
    /// String.
    pub fn string(&mut self, s: &str) {
        self.len_prefix(s.len());
        self.bytes(s.as_bytes());
    }
    /// Identifier.
    pub fn identifier(&mut self, id: &Identifier) {
        self.string(&id.to_string());
    }
}

impl Wire for String {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        r.string()
    }
    fn write(&self, w: &mut Writer) {
        w.string(self)
    }
}
impl Wire for Identifier {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        r.identifier()
    }
    fn write(&self, w: &mut Writer) {
        w.identifier(self)
    }
}
