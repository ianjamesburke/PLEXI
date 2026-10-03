use crate::{Error, IoContext, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

pub const MANIFEST: &str = "package.json";
pub const WASI_FILE: &str = "wasm-bundles/cpython-3.12.12/python.wasm";
pub const STDLIB_FILE: &str = "wasm-bundles/cpython-3.12.12/Lib/encodings/__init__.py";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Manifest {
    pub schema: u32,
    pub version: String,
    pub tag: String,
    pub build_id: String,
    pub source_commit: String,
    pub channel: String,
    pub platform: String,
    pub executable: String,
    pub resources: String,
    pub files: BTreeMap<String, String>,
}

#[derive(Debug)]
pub struct Package {
    pub root: PathBuf,
    pub manifest: Manifest,
}

pub fn relative(value: &str) -> Result<&Path> {
    let path = Path::new(value);
    if value.is_empty()
        || value.contains(['\\', ':', '\n', '\r'])
        || path
            .components()
            .any(|p| !matches!(p, Component::Normal(_)))
    {
        return Err(Error::Invalid(format!("unsafe package path: {value:?}")));
    }
    Ok(path)
}

pub fn hash_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).context(format!("open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = file
            .read(&mut buffer)
            .context(format!("hash {}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn inventory(root: &Path) -> Result<BTreeMap<String, String>> {
    fn walk(root: &Path, dir: &Path, files: &mut BTreeMap<String, String>) -> Result<()> {
        for entry in fs::read_dir(dir).context(format!("list {}", dir.display()))? {
            let entry = entry.context("read package entry")?;
            let path = entry.path();
            let kind = entry.file_type().context("read package file type")?;
            if kind.is_symlink() || (!kind.is_file() && !kind.is_dir()) {
                return Err(Error::Invalid(format!(
                    "package contains a link or special file: {}",
                    path.display()
                )));
            }
            if kind.is_dir() {
                walk(root, &path, files)?;
            } else {
                let name = path
                    .strip_prefix(root)
                    .map_err(|e| Error::Invalid(e.to_string()))?
                    .to_str()
                    .ok_or_else(|| Error::Invalid("non-UTF8 package path".into()))?
                    .replace('\\', "/");
                if name != MANIFEST {
                    files.insert(name, hash_file(&path)?);
                }
            }
        }
        Ok(())
    }
    let mut files = BTreeMap::new();
    walk(root, root, &mut files)?;
    Ok(files)
}

impl Package {
    pub fn load(root: &Path) -> Result<Self> {
        let root = root
            .canonicalize()
            .context(format!("resolve package {}", root.display()))?;
        let bytes = fs::read(root.join(MANIFEST))
            .context(format!("read {}/package.json", root.display()))?;
        Ok(Self {
            root,
            manifest: serde_json::from_slice(&bytes)?,
        })
    }

    pub fn at_executable(exe: &Path) -> Result<Option<Self>> {
        // A release binary is at the root or inside its Mac bundle; never search
        // arbitrary parent directories beyond the package layout.
        for root in exe.ancestors().skip(1).take(5) {
            if root.join(MANIFEST).is_file() {
                let package = Self::load(root)?;
                if package.executable()? == exe.canonicalize().context("resolve executable")? {
                    return Ok(Some(package));
                }
            }
        }
        Ok(None)
    }

    pub fn executable(&self) -> Result<PathBuf> {
        Ok(self.root.join(relative(&self.manifest.executable)?))
    }
    pub fn resources(&self) -> Result<PathBuf> {
        Ok(self.root.join(relative(&self.manifest.resources)?))
    }
    pub fn validate(&self) -> Result<()> {
        let m = &self.manifest;
        if m.schema != 1
            || m.build_id.is_empty()
            || m.source_commit.is_empty()
            || semver::Version::parse(&m.version).is_err()
        {
            return Err(Error::Invalid(
                "unsupported or incomplete package identity".into(),
            ));
        }
        crate::release::validate_channel(&m.channel)?;
        for path in m.files.keys() {
            relative(path)?;
        }
        let resources = relative(&m.resources)?;
        for required in [
            relative(&m.executable)?.to_path_buf(),
            resources.join(WASI_FILE),
            resources.join(STDLIB_FILE),
            resources.join("sdk/plexi_sdk/_v3_process.py"),
            resources.join("smoke-app/manifest.toml"),
        ] {
            let key = required.to_string_lossy().replace('\\', "/");
            if !m.files.contains_key(&key) {
                return Err(Error::Invalid(format!(
                    "package is missing required file {key}"
                )));
            }
        }
        if inventory(&self.root)? != m.files {
            return Err(Error::Invalid(
                "package file inventory or SHA-256 mismatch".into(),
            ));
        }
        log::info!(
            "distribution: validated package build={} channel={} root={}",
            m.build_id,
            m.channel,
            self.root.display()
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_paths_cannot_escape_the_inventory() {
        for path in [
            "../plexi",
            "/tmp/plexi",
            "C:\\plexi",
            "bin/../../plexi",
            "bin\\plexi",
        ] {
            assert!(relative(path).is_err(), "{path}");
        }
        assert_eq!(
            relative("Plexi Alpha.app/Contents/MacOS/plexi-alpha").unwrap(),
            std::path::Path::new("Plexi Alpha.app/Contents/MacOS/plexi-alpha")
        );
    }

    #[test]
    fn incomplete_runtime_fails_before_activation() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), r#"{"schema":1,"version":"0.3.4","tag":"v0.3.4","build_id":"abc","source_commit":"abc","channel":"stable","platform":"macos-arm64","executable":"plexi","resources":"resources","files":{}}"#).unwrap();
        assert!(Package::load(dir.path()).unwrap().validate().is_err());
    }
}
