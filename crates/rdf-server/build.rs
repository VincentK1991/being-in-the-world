//! Embed the optional Astro static output. Engine/API builds do not require Node.
use std::{env, fs, path::Path};
fn collect(root: &Path, dir: &Path, output: &mut String) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect(root, &path, output)?;
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .expect("asset within root")
            .to_string_lossy()
            .replace('\\', "/");
        let route = if relative == "index.html" { "/".to_owned() } else { format!("/{relative}") };
        let mime = match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
            "html" => "text/html; charset=utf-8",
            "js" => "text/javascript; charset=utf-8",
            "css" => "text/css; charset=utf-8",
            "ttl" => "text/turtle; charset=utf-8",
            "svg" => "image/svg+xml",
            "json" => "application/json",
            _ => "application/octet-stream",
        };
        output.push_str(&format!(
            "({route:?}, {mime:?}, include_bytes!({:?})),\n",
            path.canonicalize()?
        ));
        // Astro directory pages also have a clean URL in embedded deployments.
        if let Some(page) = relative.strip_suffix("/index.html") {
            let route = format!("/{page}");
            output.push_str(&format!(
                "({route:?}, {mime:?}, include_bytes!({:?})),\n",
                path.canonicalize()?
            ));
        }
    }
    Ok(())
}
fn main() {
    let manifest = env::var("CARGO_MANIFEST_DIR").expect("manifest directory");
    let root = Path::new(&manifest).join("../../apps/studio/dist");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut output = String::from("pub const ASSETS: &[(&str, &str, &[u8])] = &[\n");
    if root.is_dir() {
        collect(&root, &root, &mut output).expect("read Studio build");
    }
    output.push_str("];\n");
    fs::write(
        Path::new(&env::var("OUT_DIR").expect("output directory")).join("studio_assets.rs"),
        output,
    )
    .expect("write embedded assets");
}
