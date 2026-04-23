//! HTTP server for SPARQL and Graph Store protocols.

#![forbid(unsafe_code)]

/// Placeholder crate marker for workspace smoke checks.
pub const CRATE_MARKER: &str = "rdf-server";

#[cfg(test)]
mod tests {
    use super::CRATE_MARKER;

    #[test]
    fn marker_is_stable() {
        assert_eq!(CRATE_MARKER, "rdf-server");
    }
}
