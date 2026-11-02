use sha2::{Digest, Sha256};
use std::error::Error;
use std::path::{Path, PathBuf};

fn collect(path: &Path, files: &mut Vec<PathBuf>) -> Result<(), Box<dyn Error>> {
    if path.is_symlink() {
        return Err(format!("build source must not be a symlink: {}", path.display()).into());
    }
    if path.is_file() {
        files.push(path.to_path_buf());
        return Ok(());
    }
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if matches!(name.as_ref(), "target" | ".git" | ".DS_Store" | "__pycache__") || name.ends_with(".pyc") {
            continue;
        }
        collect(&entry.path(), files)?;
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").ok_or("missing manifest directory")?);
    let root = manifest.ancestors().find(|path| path.join("service/src/node_main.rs").is_file()).ok_or("missing canonical Mindmap workspace")?;
    let mut files = Vec::new();
    for relative in
        ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "service/Cargo.toml", "service/src", "skills/mindmap", "server/figcraft/crates", "server/figcraft/apps"]
    {
        let path = root.join(relative);
        println!("cargo:rerun-if-changed={}", path.display());
        collect(&path, &mut files)?;
    }
    files.sort();
    let mut digest = Sha256::new();
    digest.update(b"mindmap-source-v1\0");
    for path in files {
        let relative = path.strip_prefix(root)?.to_str().ok_or("source path is not UTF-8")?.replace('\\', "/");
        let bytes = std::fs::read(&path)?;
        digest.update((relative.len() as u64).to_le_bytes());
        digest.update(relative.as_bytes());
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(&bytes);
    }
    println!("cargo:rustc-env=MINDMAP_BUILD_ID={:x}", digest.finalize());
    Ok(())
}
