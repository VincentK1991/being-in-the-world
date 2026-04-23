use proptest::prelude::*;
use rdf_model::Literal;

proptest! {
    #[test]
    fn rejects_numeric_primary_subtags(
        primary in "[0-9][A-Za-z0-9]{0,7}",
        suffix in prop::collection::vec("[A-Za-z0-9]{1,8}", 0..3),
    ) {
        let mut tag = primary;
        for part in suffix {
            tag.push('-');
            tag.push_str(&part);
        }

        prop_assert!(Literal::new_language_tagged("hello", &tag).is_err());
    }
}
