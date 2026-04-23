//! SPARQL execution engine.

#![forbid(unsafe_code)]

/// Placeholder crate marker for workspace smoke checks.
pub const CRATE_MARKER: &str = "sparql-eval";

#[cfg(test)]
mod tests {
    use super::CRATE_MARKER;

    #[test]
    fn marker_is_stable() {
        assert_eq!(CRATE_MARKER, "sparql-eval");
    }
}
