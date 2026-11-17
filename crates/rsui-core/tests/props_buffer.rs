use proptest::prelude::*;
use rsui_core::{PropValue, Props, Symbol};

#[test]
fn drained_buffer_preserves_old_sets_and_float_bits() {
    let key = Symbol::new("value");
    let mut entries = Vec::with_capacity(32);
    let mut props = Props::new();
    for value in [0.0, -0.0, f64::from_bits(0x7ff8000000000042), 3.0] {
        entries.push((key, PropValue::Float(value)));
        props.replace_from(&mut entries);
        let retained = props.clone();
        let identity = props.identity();
        entries.push((key, PropValue::Float(value)));
        props.replace_from(&mut entries);
        assert_eq!(props.identity(), identity);
        assert!(entries.is_empty());
        assert_eq!(entries.capacity(), 32);
        entries.push((key, PropValue::Float(value + 1.0)));
        props.replace_from(&mut entries);
        assert_eq!(retained.get("value"), Some(&PropValue::Float(value)));
    }
    props.replace_from(&mut entries);
    assert!(props.is_empty());
}
proptest! {
    #[test]
    fn replacement_agrees_with_collect_and_last_duplicate_wins(values in prop::collection::vec((0u8..8, any::<i64>()), 0..128)) {
        let mut entries: Vec<_> = values.iter().map(|&(key,value)| (Symbol::new(&format!("p{key}")), PropValue::Int(value))).collect();
        let expected: Props = entries.clone().into_iter().collect();
        let capacity=entries.capacity();
        let mut props: Props=[("stale", "must disappear")].into();
        props.replace_from(&mut entries);
        prop_assert_eq!(&props, &expected);
        prop_assert!(entries.is_empty());
        prop_assert_eq!(entries.capacity(),capacity);

    }
}
