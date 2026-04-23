use rdf_io::{parse_nquads, parse_ntriples, parse_trig, parse_turtle};
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Supported test status emitted by manifest runners.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManifestStatus {
    /// The test case has been discovered but no assertion is executed yet.
    Pending,
    /// The test case passed runner assertions.
    Passed,
    /// The test case failed runner assertions.
    Failed,
}

/// A discovered entry in a W3C-style manifest tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestCase {
    /// Stable identifier based on relative path.
    pub id: String,
    /// Filesystem path for the source file.
    pub path: PathBuf,
}

/// Result status for a discovered manifest entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestResult {
    /// Case metadata.
    pub case: ManifestCase,
    /// Status returned by the runner.
    pub status: ManifestStatus,
    /// Optional diagnostic details.
    pub details: Option<String>,
}

/// Enumerates manifest-like files (`.ttl`, `.nt`, `.nq`, `.trig`, `.rq`) under a path.
pub fn enumerate_manifest_entries(path: impl AsRef<Path>) -> io::Result<Vec<ManifestCase>> {
    let base = path.as_ref();
    let root = if base.is_file() { base.parent().unwrap_or_else(|| Path::new(".")) } else { base };

    let mut files = Vec::new();
    collect_manifest_files(base, &mut files)?;
    files.sort();

    Ok(files
        .into_iter()
        .map(|manifest_path| {
            let id =
                manifest_path.strip_prefix(root).unwrap_or(&manifest_path).display().to_string();
            ManifestCase { id, path: manifest_path }
        })
        .collect())
}

/// Stub runner for phase-0: all discovered tests are reported as pending.
pub fn run_manifest_stub(path: impl AsRef<Path>) -> io::Result<Vec<ManifestResult>> {
    Ok(enumerate_manifest_entries(path)?
        .into_iter()
        .map(|case| ManifestResult { case, status: ManifestStatus::Pending, details: None })
        .collect())
}

/// Smoke runner for phase-1: each discovered entry must be a non-empty file.
pub fn run_manifest_smoke(path: impl AsRef<Path>) -> io::Result<Vec<ManifestResult>> {
    enumerate_manifest_entries(path)?
        .into_iter()
        .map(|case| {
            let content = fs::read(&case.path)?;
            let has_non_whitespace = content.iter().any(|byte| !byte.is_ascii_whitespace());
            let status =
                if has_non_whitespace { ManifestStatus::Passed } else { ManifestStatus::Failed };
            let details = if status == ManifestStatus::Failed {
                Some(String::from("fixture file is empty or whitespace-only"))
            } else {
                None
            };
            Ok(ManifestResult { case, status, details })
        })
        .collect()
}

/// N-Triples conformance runner for `.nt` fixtures.
///
/// Naming convention:
/// - `*.pos.nt`: parse must succeed
/// - `*.neg.nt`: parse must fail
pub fn run_manifest_ntriples(path: impl AsRef<Path>) -> io::Result<Vec<ManifestResult>> {
    enumerate_manifest_entries(path)?
        .into_iter()
        .filter(|case| case.path.extension().and_then(OsStr::to_str) == Some("nt"))
        .map(|case| run_case(case, parse_ntriples))
        .collect()
}

/// N-Quads conformance runner for `.nq` fixtures.
pub fn run_manifest_nquads(path: impl AsRef<Path>) -> io::Result<Vec<ManifestResult>> {
    enumerate_manifest_entries(path)?
        .into_iter()
        .filter(|case| case.path.extension().and_then(OsStr::to_str) == Some("nq"))
        .map(|case| run_case(case, parse_nquads))
        .collect()
}

/// Turtle conformance runner for `.ttl` fixtures.
pub fn run_manifest_turtle(path: impl AsRef<Path>) -> io::Result<Vec<ManifestResult>> {
    enumerate_manifest_entries(path)?
        .into_iter()
        .filter(|case| case.path.extension().and_then(OsStr::to_str) == Some("ttl"))
        .map(|case| run_case(case, parse_turtle))
        .collect()
}

/// TriG conformance runner for `.trig` fixtures.
pub fn run_manifest_trig(path: impl AsRef<Path>) -> io::Result<Vec<ManifestResult>> {
    enumerate_manifest_entries(path)?
        .into_iter()
        .filter(|case| case.path.extension().and_then(OsStr::to_str) == Some("trig"))
        .map(|case| run_case(case, parse_trig))
        .collect()
}

/// Dispatches to the best available runner for the given manifest root.
pub fn run_manifest(path: impl AsRef<Path>) -> io::Result<Vec<ManifestResult>> {
    let root = path.as_ref();
    let entries = enumerate_manifest_entries(root)?;
    let has_marked_cases = entries.iter().any(|entry| {
        entry
            .path
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.contains(".pos.") || name.contains(".neg."))
    });
    if !has_marked_cases {
        return run_manifest_smoke(root);
    }

    let mut results = Vec::new();
    results.extend(run_manifest_ntriples(root)?);
    results.extend(run_manifest_nquads(root)?);
    results.extend(run_manifest_turtle(root)?);
    results.extend(run_manifest_trig(root)?);
    if !results.is_empty() {
        return Ok(results);
    }
    run_manifest_smoke(root)
}

fn run_case<T, E>(
    case: ManifestCase,
    parse_fn: fn(&str) -> Result<T, E>,
) -> io::Result<ManifestResult>
where
    E: std::fmt::Display,
{
    let expected_failure =
        case.path.file_name().and_then(OsStr::to_str).is_some_and(|name| name.contains(".neg."));
    let source = fs::read_to_string(&case.path)?;
    let parse_result = parse_fn(&source);
    let (status, details) = match (expected_failure, parse_result) {
        (false, Ok(_)) => (ManifestStatus::Passed, None),
        (true, Err(_)) => (ManifestStatus::Passed, None),
        (false, Err(error)) => {
            (ManifestStatus::Failed, Some(format!("expected parse success but failed: {error}")))
        }
        (true, Ok(_)) => (
            ManifestStatus::Failed,
            Some(String::from("expected parse failure but parser succeeded")),
        ),
    };
    Ok(ManifestResult { case, status, details })
}

fn collect_manifest_files(path: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    if path.is_file() {
        if supported_manifest_extension(path.extension()) {
            files.push(path.to_path_buf());
        }
        return Ok(());
    }

    if !path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("manifest path does not exist: {}", path.display()),
        ));
    }

    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let child = entry.path();
        if file_type.is_dir() {
            collect_manifest_files(&child, files)?;
        } else if file_type.is_file() && supported_manifest_extension(child.extension()) {
            files.push(child);
        }
    }

    Ok(())
}

fn supported_manifest_extension(ext: Option<&OsStr>) -> bool {
    matches!(ext.and_then(OsStr::to_str), Some("ttl" | "nt" | "nq" | "trig" | "rq"))
}

#[cfg(test)]
mod tests {
    use super::{
        ManifestStatus, enumerate_manifest_entries, run_manifest_smoke, run_manifest_stub,
    };
    use std::fs;

    #[test]
    fn enumerates_supported_manifest_files() {
        let temp = std::env::temp_dir().join("rdf-testkit-manifest-smoke");
        if temp.exists() {
            fs::remove_dir_all(&temp).expect("remove old temp tree");
        }
        fs::create_dir_all(temp.join("nested")).expect("create temp tree");
        fs::write(temp.join("manifest.ttl"), "@prefix : <urn:test:> .\n").expect("write ttl");
        fs::write(temp.join("nested").join("query.rq"), "SELECT * WHERE { ?s ?p ?o }\n")
            .expect("write rq");
        fs::write(temp.join("README.md"), "ignored\n").expect("write markdown");

        let cases = enumerate_manifest_entries(&temp).expect("enumerate entries");
        assert_eq!(cases.len(), 2);

        let results = run_manifest_stub(&temp).expect("run stub");
        assert!(results.iter().all(|result| result.status == ManifestStatus::Pending));

        fs::remove_dir_all(temp).expect("cleanup temp tree");
    }

    #[test]
    fn smoke_runner_reports_passed_and_failed_cases() {
        let temp = std::env::temp_dir().join("rdf-testkit-manifest-smoke-status");
        if temp.exists() {
            fs::remove_dir_all(&temp).expect("remove old temp tree");
        }
        fs::create_dir_all(temp.join("nested")).expect("create temp tree");
        fs::write(temp.join("manifest.ttl"), "@prefix : <urn:test:> .\n").expect("write ttl");
        fs::write(temp.join("nested").join("empty.rq"), "").expect("write empty rq");

        let results = run_manifest_smoke(&temp).expect("run smoke");
        assert_eq!(results.len(), 2);

        let passed =
            results.iter().filter(|result| result.status == ManifestStatus::Passed).count();
        let failed =
            results.iter().filter(|result| result.status == ManifestStatus::Failed).count();
        assert_eq!(passed, 1);
        assert_eq!(failed, 1);
        assert!(results.iter().any(|result| result.details.is_some()));

        fs::remove_dir_all(temp).expect("cleanup temp tree");
    }
}
