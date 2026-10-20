use super::*;

#[test]
fn range_alloc_first_fit_and_coalesce() {
    let mut r = RangeAlloc::new(100);
    let a = r.alloc(10).unwrap();
    let b = r.alloc(20).unwrap();
    let c = r.alloc(30).unwrap();
    assert_eq!((a, b, c), (0, 10, 30));
    r.free(b, 20);
    assert_eq!(r.alloc(5), Some(10), "first fit reuses the hole");
    r.free(10, 5);
    r.free(a, 10);
    r.free(c, 30);
    assert_eq!(r.free, vec![(0, 100)], "all ranges merge back");
    assert_eq!(r.alloc(101), None);
    assert_eq!(r.alloc(100), Some(0));
}
