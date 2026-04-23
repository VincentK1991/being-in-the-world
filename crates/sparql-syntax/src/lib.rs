//! SPARQL parser and AST.

#![forbid(unsafe_code)]

/// Placeholder crate marker for workspace smoke checks.
pub const CRATE_MARKER: &str = "sparql-syntax";

#[cfg(test)]
mod tests {
    use super::CRATE_MARKER;

    #[test]
    fn marker_is_stable() {
        assert_eq!(CRATE_MARKER, "sparql-syntax");
    }
}
