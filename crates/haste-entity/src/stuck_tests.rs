use haste_core::entity::{MetadataEntry, MetadataValue};

use crate::metadata::Metadata;
use crate::stuck::{optional_unsigned_int, var_int, var_int_bytes};

fn entry(index: u8, value: MetadataValue) -> MetadataEntry {
    MetadataEntry { index, value }
}

fn other(raw: &'static [u8]) -> MetadataValue {
    MetadataValue::Other {
        type_id: 19,
        // `Bytes` is not a dependency of this crate; `From<&'static [u8]>` builds it.
        raw: raw.into(),
    }
}

#[test]
fn var_int_bytes_reads_the_protocol_encoding() {
    assert_eq!(var_int_bytes(&[0x00]), Some(0));
    assert_eq!(var_int_bytes(&[0x7f]), Some(127));
    assert_eq!(var_int_bytes(&[0x80, 0x01]), Some(128));
    assert_eq!(
        var_int_bytes(&[0xff, 0xff, 0xff, 0xff, 0x0f]),
        Some(u32::MAX)
    );
    assert_eq!(
        var_int_bytes(&[0xff, 0xff, 0xff, 0xff, 0x07]),
        Some(i32::MAX as u32)
    );
    // Truncated (continuation bit set on the last byte) or longer than five bytes.
    assert_eq!(var_int_bytes(&[0x80]), None);
    assert_eq!(var_int_bytes(&[0x80; 6]), None);
}

#[test]
fn optional_unsigned_int_is_empty_at_zero_and_one_less_than_the_wire_value() {
    let mut m = Metadata::default();
    m.apply(&[entry(19, other(&[0x00])), entry(20, other(&[0x03]))]);
    assert_eq!(optional_unsigned_int(&m, 19), None);
    // Vanilla writes `value + 1`: the wire 3 is parrot variant 2.
    assert_eq!(optional_unsigned_int(&m, 20), Some(2));
}

#[test]
fn counts_are_read_as_var_ints_and_absent_entries_are_zero() {
    let mut m = Metadata::default();
    m.apply(&[entry(12, MetadataValue::VarInt(4))]);
    assert_eq!(var_int(&m, 12), 4);
    assert_eq!(var_int(&m, 13), 0);
}
