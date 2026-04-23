use rdf_model::{Literal, ModelError};

#[test]
fn accepts_common_bcp47_language_tags() {
    let en = Literal::new_language_tagged("hello", "en").expect("valid language tag");
    assert_eq!(en.language(), Some("en"));

    let regional = Literal::new_language_tagged("hello", "EN-US").expect("valid language tag");
    assert_eq!(regional.language(), Some("en-us"));

    let script = Literal::new_language_tagged("hello", "zh-Hant").expect("valid language tag");
    assert_eq!(script.language(), Some("zh-hant"));
}

#[test]
fn rejects_numeric_primary_language_subtag() {
    assert_eq!(
        Literal::new_language_tagged("hello", "123"),
        Err(ModelError::InvalidLanguageTag(String::from("123")))
    );
}
