use crate::TermIdError;

const INLINE_FLAG: u64 = 1 << 63;
const TAG_SHIFT: u64 = 60;
const TAG_MASK: u64 = 0x7 << TAG_SHIFT;
const PAYLOAD_MASK: u64 = (1 << TAG_SHIFT) - 1;
const I60_MIN: i64 = -(1 << 59);
const I60_MAX: i64 = (1 << 59) - 1;
const I60_SIGN_BIT: u64 = 1 << 59;
const I60_EXTEND_MASK: i64 = !((1_i64 << 60) - 1);
const SHORT_STRING_LEN_SHIFT: u64 = 56;
const SHORT_STRING_BYTES_MASK: u64 = (1 << SHORT_STRING_LEN_SHIFT) - 1;

const TAG_BOOL: u8 = 0x1;
const TAG_I60: u8 = 0x2;
const TAG_SHORT_STRING: u8 = 0x3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InlineValue {
    Bool(bool),
    I64(i64),
    ShortString(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TermId(u64);

impl TermId {
    pub const MAX_DICTIONARY_ID: u64 = INLINE_FLAG - 1;

    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn into_raw(self) -> u64 {
        self.0
    }

    pub fn dictionary(id: u64) -> Result<Self, TermIdError> {
        if id == 0 || id > Self::MAX_DICTIONARY_ID {
            return Err(TermIdError::InvalidDictionaryId(id));
        }
        Ok(Self(id))
    }

    pub const fn is_inline(self) -> bool {
        self.0 & INLINE_FLAG != 0
    }

    pub const fn as_dictionary(self) -> Option<u64> {
        if self.is_inline() { None } else { Some(self.0) }
    }

    pub const fn inline_bool(value: bool) -> Self {
        let payload = if value { 1 } else { 0 };
        Self(INLINE_FLAG | ((TAG_BOOL as u64) << TAG_SHIFT) | payload)
    }

    pub fn inline_i60(value: i64) -> Result<Self, TermIdError> {
        if !(I60_MIN..=I60_MAX).contains(&value) {
            return Err(TermIdError::IntegerOutOfRange(value));
        }
        let payload = (value as i128 & PAYLOAD_MASK as i128) as u64;
        Ok(Self(INLINE_FLAG | ((TAG_I60 as u64) << TAG_SHIFT) | payload))
    }

    pub fn inline_short_string(value: &str) -> Result<Self, TermIdError> {
        let bytes = value.as_bytes();
        if bytes.len() > 7 {
            return Err(TermIdError::ShortStringTooLong(bytes.len()));
        }

        let packed_bytes = bytes
            .iter()
            .enumerate()
            .fold(0_u64, |acc, (index, byte)| acc | (u64::from(*byte) << (index * 8)));
        let payload = ((bytes.len() as u64) << SHORT_STRING_LEN_SHIFT) | packed_bytes;
        Ok(Self(INLINE_FLAG | ((TAG_SHORT_STRING as u64) << TAG_SHIFT) | payload))
    }

    pub fn decode_inline(self) -> Result<InlineValue, TermIdError> {
        if !self.is_inline() {
            return Err(TermIdError::NotInline(self.0));
        }

        let tag = ((self.0 & TAG_MASK) >> TAG_SHIFT) as u8;
        let payload = self.0 & PAYLOAD_MASK;
        match tag {
            TAG_BOOL => Ok(InlineValue::Bool(payload & 1 == 1)),
            TAG_I60 => {
                let value = if payload & I60_SIGN_BIT != 0 {
                    (payload as i64) | I60_EXTEND_MASK
                } else {
                    payload as i64
                };
                Ok(InlineValue::I64(value))
            }
            TAG_SHORT_STRING => {
                let len = ((payload >> SHORT_STRING_LEN_SHIFT) & 0x7) as usize;
                let bytes_packed = payload & SHORT_STRING_BYTES_MASK;
                let bytes = (0..len)
                    .map(|index| ((bytes_packed >> (index * 8)) & 0xFF) as u8)
                    .collect::<Vec<_>>();
                let text =
                    String::from_utf8(bytes).map_err(|_| TermIdError::InvalidUtf8ShortString)?;
                Ok(InlineValue::ShortString(text))
            }
            _ => Err(TermIdError::UnsupportedInlineTag(tag)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{I60_MAX, I60_MIN, InlineValue, TAG_SHIFT, TermId};
    use crate::TermIdError;

    #[test]
    fn dictionary_ids_are_validated() {
        assert!(TermId::dictionary(0).is_err());
        assert!(TermId::dictionary(TermId::MAX_DICTIONARY_ID + 1).is_err());
        assert_eq!(TermId::dictionary(1).expect("valid id").as_dictionary(), Some(1));
    }

    #[test]
    fn boolean_inline_round_trip() {
        let value = TermId::inline_bool(true).decode_inline().expect("decode bool");
        assert_eq!(value, InlineValue::Bool(true));
    }

    #[test]
    fn i60_inline_round_trip() {
        for number in [I60_MIN, -1, 0, 1, I60_MAX] {
            let decoded = TermId::inline_i60(number)
                .expect("encode i60")
                .decode_inline()
                .expect("decode i60");
            assert_eq!(decoded, InlineValue::I64(number));
        }
    }

    #[test]
    fn i60_rejects_out_of_range_values() {
        assert_eq!(
            TermId::inline_i60(I60_MIN - 1),
            Err(TermIdError::IntegerOutOfRange(I60_MIN - 1))
        );
        assert_eq!(
            TermId::inline_i60(I60_MAX + 1),
            Err(TermIdError::IntegerOutOfRange(I60_MAX + 1))
        );
    }

    #[test]
    fn short_string_inline_round_trip() {
        let decoded = TermId::inline_short_string("rust")
            .expect("encode short string")
            .decode_inline()
            .expect("decode short string");
        assert_eq!(decoded, InlineValue::ShortString(String::from("rust")));
    }

    #[test]
    fn short_string_rejects_long_input() {
        assert_eq!(
            TermId::inline_short_string("12345678"),
            Err(TermIdError::ShortStringTooLong(8))
        );
    }

    #[test]
    fn decode_non_inline_value_fails() {
        let id = TermId::dictionary(42).expect("dictionary id");
        assert_eq!(id.decode_inline(), Err(TermIdError::NotInline(42)));
    }

    #[test]
    fn unknown_inline_tag_fails() {
        let raw = (1 << 63) | (0x7 << TAG_SHIFT);
        let id = TermId::from_raw(raw);
        assert_eq!(id.decode_inline(), Err(TermIdError::UnsupportedInlineTag(0x7)));
    }
}
