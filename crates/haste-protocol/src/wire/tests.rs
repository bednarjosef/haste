use bytes::Bytes;
use uuid::Uuid;

use haste_core::{BlockPos, DVec3, Identifier};

use super::*;
use crate::nbt::NbtCompound;

fn varint_bytes(v: i32) -> Vec<u8> {
    let mut w = Writer::new();
    w.var_int(v);
    w.into_inner()
}

#[test]
fn varint_wiki_vectors() {
    // minecraft.wiki "VarInt and VarLong" sample table.
    let cases: &[(i32, &[u8])] = &[
        (0, &[0x00]),
        (1, &[0x01]),
        (2, &[0x02]),
        (127, &[0x7f]),
        (128, &[0x80, 0x01]),
        (255, &[0xff, 0x01]),
        (25565, &[0xdd, 0xc7, 0x01]),
        (2097151, &[0xff, 0xff, 0x7f]),
        (2147483647, &[0xff, 0xff, 0xff, 0xff, 0x07]),
        (-1, &[0xff, 0xff, 0xff, 0xff, 0x0f]),
        (-2147483648, &[0x80, 0x80, 0x80, 0x80, 0x08]),
    ];
    for (v, bytes) in cases {
        assert_eq!(varint_bytes(*v), *bytes, "encode {v}");
        assert_eq!(Reader::new(bytes).var_int().unwrap(), *v, "decode {v}");
    }
    assert!(Reader::new(&[0xff, 0xff, 0xff, 0xff, 0xff, 0x01])
        .var_int()
        .is_err());
    assert!(Reader::new(&[0x80]).var_int().is_err());
}

#[test]
fn varlong_wiki_vectors() {
    let cases: &[(i64, &[u8])] = &[
        (0, &[0x00]),
        (127, &[0x7f]),
        (128, &[0x80, 0x01]),
        (2147483647, &[0xff, 0xff, 0xff, 0xff, 0x07]),
        (
            9223372036854775807,
            &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f],
        ),
        (
            -1,
            &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01],
        ),
        (
            -2147483648,
            &[0x80, 0x80, 0x80, 0x80, 0xf8, 0xff, 0xff, 0xff, 0xff, 0x01],
        ),
        (
            -9223372036854775808,
            &[0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x01],
        ),
    ];
    for (v, bytes) in cases {
        let mut w = Writer::new();
        w.var_long(*v);
        assert_eq!(w.buf, *bytes, "encode {v}");
        assert_eq!(Reader::new(bytes).var_long().unwrap(), *v, "decode {v}");
    }
}

#[test]
fn position_wiki_example() {
    // Data types page: x=18357644, z=-20882616, y=831.
    let p = BlockPos::new(18357644, 831, -20882616);
    let mut w = Writer::new();
    w.block_pos(&p);
    let v = i64::from_be_bytes(w.buf.clone().try_into().unwrap());
    assert_eq!(
        v as u64,
        0b0100011000000111011000110010110000010101101101001000001100111111
    );
    assert_eq!(Reader::new(&w.buf).block_pos().unwrap(), p);
}

#[test]
fn lp_vec3_wiki_samples() {
    type Case = ((f64, f64, f64), &'static [u8]);
    let cases: &[Case] = &[
        ((0.0, 0.0, 0.0), &[0]),
        ((1.0, 0.0, -1.0), &[0xF1, 0xFF, 0x00, 0x00, 0xFF, 0xFF]),
        (
            (10.0, 0.2, -5.0),
            &[0xF6, 0xFF, 0x40, 0x01, 0x05, 0x1F, 0x02],
        ),
        (
            (123457.0, 15.071, 0.0),
            &[0xF5, 0xFF, 0x7F, 0xFF, 0x00, 0x07, 0x90, 0xF1, 0x01],
        ),
    ];
    for ((x, y, z), bytes) in cases {
        let mut w = Writer::new();
        w.lp_vec3(DVec3::new(*x, *y, *z));
        assert_eq!(w.buf, *bytes, "encode {x},{y},{z}");
        let v = Reader::new(bytes).lp_vec3().unwrap();
        let tol = (x.abs().max(y.abs()).max(z.abs())).ceil() / 16000.0 + 1e-9;
        assert!(
            (v.x - x).abs() <= tol && (v.y - y).abs() <= tol && (v.z - z).abs() <= tol,
            "{v:?}"
        );
    }
}

#[test]
fn strings_and_limits() {
    let mut w = Writer::new();
    w.string("héllo");
    let mut r = Reader::new(&w.buf);
    assert_eq!(r.string().unwrap(), "héllo");
    r.finish().unwrap();
    let mut w = Writer::new();
    w.string("abcdef");
    assert!(Reader::new(&w.buf).string_max(5).is_err());
    assert!(Reader::new(&[5, b'a']).string().is_err());
}

#[test]
fn options_lists_bitsets_uuid_angle() {
    let mut w = Writer::new();
    w.write(&Some(5i32));
    w.write(&None::<i32>);
    w.write(&vec![1i16, 2, 3]);
    w.bit_set(&[1, u64::MAX]);
    let u = Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);
    w.uuid(&u);
    w.angle(90.0);
    let mut r = Reader::new(&w.buf);
    assert_eq!(r.read::<Option<i32>>().unwrap(), Some(5));
    assert_eq!(r.read::<Option<i32>>().unwrap(), None);
    assert_eq!(r.read::<Vec<i16>>().unwrap(), vec![1, 2, 3]);
    assert_eq!(r.bit_set().unwrap(), vec![1, u64::MAX]);
    assert_eq!(r.uuid().unwrap(), u);
    assert_eq!(r.angle().unwrap(), 90.0);
    r.finish().unwrap();
}

#[test]
fn bit_set_is_trimmed_little_endian_bytes() {
    // Hand-built 26.3 bytes: a sky mask with bits 1..=24 set
    // (0x01_FF_FF_FE) is 4 bytes, LSB first.
    let wire = [4u8, 0xFE, 0xFF, 0xFF, 0x01];
    let mut r = Reader::new(&wire);
    assert_eq!(r.bit_set().unwrap(), vec![0x01FF_FFFE]);
    r.finish().unwrap();
    let mut w = Writer::new();
    w.bit_set(&[0x01FF_FFFE]);
    assert_eq!(w.buf, wire);
    // Empty set: a zero count, also for all-zero longs.
    let mut w = Writer::new();
    w.bit_set(&[0, 0]);
    assert_eq!(w.buf, [0]);
    assert_eq!(Reader::new(&[0]).bit_set().unwrap(), Vec::<u64>::new());
    // Two longs: 8 full bytes then the trimmed second long.
    let mut w = Writer::new();
    w.bit_set(&[u64::MAX, 0x80]);
    assert_eq!(w.buf, [9, 255, 255, 255, 255, 255, 255, 255, 255, 0x80]);
    assert_eq!(Reader::new(&w.buf).bit_set().unwrap(), vec![u64::MAX, 0x80]);
}

#[test]
fn nbt_raw_and_parsed() {
    let mut c = NbtCompound::new();
    c.put_int("a", 7);
    c.put_string("s", "x".into());
    let mut w = Writer::new();
    w.nbt_compound(Some(&c));
    w.nbt_compound(None);
    w.u8(42);
    let b = Bytes::from(w.into_inner());
    let mut r = Reader::from_bytes(&b);
    let raw = r.nbt_raw().unwrap();
    assert_eq!(raw[0], crate::nbt::COMPOUND_ID);
    assert_eq!(r.nbt_compound().unwrap(), None);
    assert_eq!(r.u8().unwrap(), 42);
    let parsed = Reader::new(&raw).nbt_compound().unwrap().unwrap();
    assert_eq!(parsed, c);
    // Truncated NBT is an error, not a silent over-read.
    assert!(Reader::new(&raw[..raw.len() - 1]).nbt_raw().is_err());
}

#[test]
fn id_or_and_id_set() {
    let mut w = Writer::new();
    w.write(&IdOr::<SoundEvent>::Id(4));
    w.write(&IdOr::Inline(SoundEvent {
        name: Identifier::minecraft("a"),
        fixed_range: Some(2.0),
    }));
    w.write(&IdSet::Tag(Identifier::minecraft("logs")));
    w.write(&IdSet::Ids(vec![1, 2]));
    let mut r = Reader::new(&w.buf);
    assert_eq!(r.read::<IdOr<SoundEvent>>().unwrap(), IdOr::Id(4));
    assert!(matches!(
        r.read::<IdOr<SoundEvent>>().unwrap(),
        IdOr::Inline(_)
    ));
    assert_eq!(
        r.read::<IdSet>().unwrap(),
        IdSet::Tag(Identifier::minecraft("logs"))
    );
    assert_eq!(r.read::<IdSet>().unwrap(), IdSet::Ids(vec![1, 2]));
    r.finish().unwrap();
}
