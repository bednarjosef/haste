//! Optionals (prefixed bool), prefixed arrays, prefixed byte arrays and
//! BitSets (26.3 byte-count form, see Reader::bit_set).

use bytes::Bytes;

use super::{DecodeResult, Reader, Wire, Writer, MAX_LIST};

impl Reader<'_> {
    /// Prefixed array of bytes, at most `max` long.
    pub fn byte_array(&mut self, max: usize) -> DecodeResult<Bytes> {
        let n = self.len_prefix(max)?;
        let s = self.take(n)?;
        Ok(self.to_bytes(s))
    }

    /// Prefixed Optional.
    pub fn option<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> DecodeResult<T>,
    ) -> DecodeResult<Option<T>> {
        if self.bool()? {
            Ok(Some(f(self)?))
        } else {
            Ok(None)
        }
    }

    /// Prefixed Array with at most `max` elements.
    pub fn list<T>(
        &mut self,
        max: usize,
        mut f: impl FnMut(&mut Self) -> DecodeResult<T>,
    ) -> DecodeResult<Vec<T>> {
        let n = self.len_prefix(max)?;
        // Never trust the count for the allocation.
        let mut v = Vec::with_capacity(n.min(self.remaining()));
        for _ in 0..n {
            v.push(f(self)?);
        }
        Ok(v)
    }

    /// BitSet. Since 26.3 this is a VarInt byte count followed by the set's
    /// longs as little-endian bytes with trailing zero bytes dropped (before
    /// 26.3, and on minecraft.wiki: a count of big-endian longs). Returns
    /// `ceil(bytes / 8)` longs.
    pub fn bit_set(&mut self) -> DecodeResult<Vec<u64>> {
        let n = self.len_prefix(MAX_LIST * 8)?;
        let bytes = self.take(n)?;
        Ok(bytes
            .chunks(8)
            .map(|c| {
                let mut b = [0u8; 8];
                b[..c.len()].copy_from_slice(c);
                u64::from_le_bytes(b)
            })
            .collect())
    }

    /// Fixed BitSet of `bits` bits (`ceil(bits/8)` bytes).
    pub fn fixed_bit_set(&mut self, bits: usize) -> DecodeResult<Vec<u8>> {
        Ok(self.take(bits.div_ceil(8))?.to_vec())
    }
}

impl Writer {
    /// Prefixed byte array.
    pub fn byte_array(&mut self, b: &[u8]) {
        self.len_prefix(b.len());
        self.bytes(b);
    }
    /// Prefixed Optional.
    pub fn option<T>(&mut self, v: &Option<T>, f: impl FnOnce(&mut Self, &T)) {
        match v {
            Some(x) => {
                self.bool(true);
                f(self, x);
            }
            None => self.bool(false),
        }
    }
    /// Prefixed Array.
    pub fn list<T>(&mut self, v: &[T], mut f: impl FnMut(&mut Self, &T)) {
        self.len_prefix(v.len());
        for x in v {
            f(self, x);
        }
    }
    /// BitSet in the 26.3 form (see [`Reader::bit_set`]): little-endian
    /// bytes of the longs, trailing zero bytes dropped, VarInt byte count.
    pub fn bit_set(&mut self, longs: &[u64]) {
        let len = longs
            .iter()
            .rposition(|&l| l != 0)
            .map_or(0, |i| i * 8 + 8 - (longs[i].leading_zeros() / 8) as usize);
        self.len_prefix(len);
        for (i, l) in longs.iter().enumerate() {
            let take = len.saturating_sub(i * 8).min(8);
            if take == 0 {
                break;
            }
            self.buf.extend_from_slice(&l.to_le_bytes()[..take]);
        }
    }
}

/// Prefixed Optional.
impl<T: Wire> Wire for Option<T> {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        r.option(T::read)
    }
    fn write(&self, w: &mut Writer) {
        w.option(self, |w, v| v.write(w))
    }
}
/// Prefixed Array.
impl<T: Wire> Wire for Vec<T> {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        r.list(MAX_LIST, T::read)
    }
    fn write(&self, w: &mut Writer) {
        w.list(self, |w, v| v.write(w))
    }
}
