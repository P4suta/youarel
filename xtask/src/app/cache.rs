use crate::{container, fs, quint};
use anyhow::{Context, Result, bail, ensure};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use youarel_core::verification_rules::cache_field_byte;

const HOSTED_VARIABLE: &str = "GITHUB_ACTIONS";
const DIRECTORY_VARIABLE: &str = "RUNNER_TEMP";
const OUTPUT_VARIABLE: &str = "GITHUB_OUTPUT";
const TARGET: &str = "/work/target";

fn field(path: &Path) -> Result<&str> {
    let text = path.to_str().context("RUNNER_TEMP must be UTF-8")?;
    ensure!(
        text.bytes().all(cache_field_byte),
        "RUNNER_TEMP must fit one output line and a quoted Docker mount"
    );
    Ok(text)
}

#[derive(Debug)]
enum Target {
    Volume(String),
    Hosted { key: String, directory: PathBuf },
}

#[derive(Debug)]
pub(super) struct CompilerCache {
    target: Target,
}

impl CompilerCache {
    pub(super) fn from_facts(root: &Path, image: &str, runner_temp: Option<&Path>) -> Result<Self> {
        let root = std::fs::canonicalize(root).context("resolve the cache's checkout")?;
        let key = container::compiler_volume(&root, image, super::TARGET_VOLUME)?;
        let target = if let Some(parent) = runner_temp {
            ensure!(parent.is_absolute(), "RUNNER_TEMP must be absolute");
            field(parent)?;
            let parent = std::fs::canonicalize(parent).context("resolve RUNNER_TEMP")?;
            ensure!(
                fs::is_directory(&parent)?,
                "RUNNER_TEMP must name a directory"
            );
            let directory = parent.join(&key);
            field(&directory)?;
            Target::Hosted { key, directory }
        } else {
            Target::Volume(key)
        };
        Ok(Self { target })
    }

    pub(super) fn from_environment(root: &Path, image: &str) -> Result<Self> {
        let runner_temp = match std::env::var(HOSTED_VARIABLE) {
            Err(std::env::VarError::NotPresent) => None,
            Ok(value) if value.is_empty() || value == "false" => None,
            Ok(value) if value == "true" => Some(PathBuf::from(
                std::env::var_os(DIRECTORY_VARIABLE)
                    .context("the hosted cache needs RUNNER_TEMP")?,
            )),
            Ok(_) => bail!("GITHUB_ACTIONS must be true or false"),
            Err(error) => return Err(error).context("read GITHUB_ACTIONS"),
        };
        Self::from_facts(root, image, runner_temp.as_deref())
    }

    pub(super) fn prepare(&self) -> Result<()> {
        if let Target::Hosted { directory, .. } = &self.target {
            fs::create_dir_all(directory)?;
            ensure!(
                std::fs::canonicalize(directory).context("resolve the compiler cache")?
                    == *directory,
                "the compiler cache must keep its validated directory"
            );
        }
        Ok(())
    }

    pub(super) fn mount(&self) -> OsString {
        match &self.target {
            Target::Volume(key) => format!("type=volume,source={key},target={TARGET}").into(),
            Target::Hosted { directory, .. } => quint::mount(directory, TARGET, false),
        }
    }

    fn output(&self) -> Result<String> {
        match &self.target {
            Target::Hosted { key, directory } => Ok(format!(
                "key={key}\npath={}\n",
                directory.to_str().context("the cache path must be UTF-8")?
            )),
            Target::Volume(_) => bail!("cache outputs require a GitHub Actions runner"),
        }
    }
}

pub(super) fn publish(root: &Path, image: &str) -> Result<()> {
    let cache = CompilerCache::from_environment(root, image)?;
    let output = cache.output()?;
    let destination = std::env::var_os(OUTPUT_VARIABLE).context("the cache needs GITHUB_OUTPUT")?;
    cache.prepare()?;
    fs::append(Path::new(&destination), output.as_bytes())?;
    print!("{output}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::CompilerCache;
    use crate::fs;
    use std::path::{Path, PathBuf};

    fn output_path(cache: &CompilerCache) -> PathBuf {
        let output = cache.output().unwrap();
        PathBuf::from(
            output
                .lines()
                .find_map(|line| line.strip_prefix("path="))
                .unwrap(),
        )
    }

    #[test]
    fn a_hosted_run_mounts_its_announced_compiler_directory() {
        let fixture = tempfile::tempdir().unwrap();
        let cache =
            CompilerCache::from_facts(fixture.path(), "image:a", Some(fixture.path())).unwrap();
        let command = super::super::command(fixture.path(), "image:a", &cache);
        let mount = command
            .get_args()
            .find(|argument| argument.to_string_lossy().ends_with(",target=/work/target"))
            .unwrap()
            .to_string_lossy();
        assert!(mount.starts_with("type=bind,"), "{mount}");
        assert_eq!(
            mount,
            format!(
                "type=bind,\"source={}\",target=/work/target",
                output_path(&cache).display()
            )
        );
        let arguments: Vec<_> = command
            .get_args()
            .map(|argument| argument.to_string_lossy())
            .collect();
        for pair in [["--cpus", "4"], ["--memory", "8g"], ["--memory-swap", "8g"]] {
            assert!(arguments.windows(2).any(|values| values == pair));
        }
    }

    #[test]
    fn cache_identity_binds_the_canonical_checkout_and_image() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let describe = |root: &Path, image: &str| {
            CompilerCache::from_facts(root, image, Some(first.path()))
                .unwrap()
                .output()
                .unwrap()
        };
        let original = describe(first.path(), "image:a");
        assert_eq!(original, describe(&first.path().join("."), "image:a"));
        assert_ne!(original, describe(second.path(), "image:a"));
        assert_ne!(original, describe(first.path(), "image:b"));
        let owner = CompilerCache::from_facts(first.path(), "image:a", None).unwrap();
        assert!(owner.mount().to_string_lossy().starts_with("type=volume,"));
        owner.output().unwrap_err();
    }

    #[test]
    fn unsafe_runner_directories_refuse_before_creating_a_cache() {
        let root = tempfile::tempdir().unwrap();
        CompilerCache::from_facts(root.path(), "image:a", Some(Path::new("relative"))).unwrap_err();
        let file = root.path().join("file");
        fs::write(&file, b"kept").unwrap();
        CompilerCache::from_facts(root.path(), "image:a", Some(&file)).unwrap_err();
        for name in ["line\nbreak", "line\rbreak", "quoted\"path"] {
            let parent = root.path().join(name);
            let error =
                CompilerCache::from_facts(root.path(), "image:a", Some(&parent)).unwrap_err();
            assert!(error.to_string().contains("one output line"));
        }
        let parent = root.path().join("spaces, and an apostrophe's");
        fs::create_dir_all(&parent).unwrap();
        let cache = CompilerCache::from_facts(root.path(), "image:a", Some(&parent)).unwrap();
        assert!(
            cache
                .mount()
                .to_string_lossy()
                .contains("spaces, and an apostrophe's")
        );
    }

    #[test]
    fn preparing_a_directory_preserves_its_contents_and_refuses_a_file() {
        let root = tempfile::tempdir().unwrap();
        let cache = CompilerCache::from_facts(root.path(), "image:a", Some(root.path())).unwrap();
        let path = output_path(&cache);
        assert_eq!(fs::kind(&path).unwrap(), None);
        fs::write(&path, b"kept").unwrap();
        cache.prepare().unwrap_err();
        assert_eq!(fs::read(&path).unwrap(), b"kept");
        let other = CompilerCache::from_facts(root.path(), "image:b", Some(root.path())).unwrap();
        other.prepare().unwrap();
        let marker = output_path(&other).join("marker");
        fs::write(&marker, b"compiler output").unwrap();
        other.prepare().unwrap();
        assert_eq!(fs::read(&marker).unwrap(), b"compiler output");
    }
}
