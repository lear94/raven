use crate::core::{RavenError, Recipe};
use crate::sandbox::ScriptSandbox;
use crate::ui::{create_download_bar, create_spinner, log_success};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

const MAX_DOWNLOAD_ATTEMPTS: u32 = 4;

pub struct Builder {
    work_dir: PathBuf,
    cache_dir: PathBuf,
    client: reqwest::Client,
}

pub struct BuildOutput {
    pub pkg_dir: PathBuf,
    pub out_dir: PathBuf,
}

/// Toolchain settings for building for a different architecture.
struct CrossToolchain {
    triple: &'static str,
}

fn normalize_arch(arch: &str) -> String {
    match arch.trim().to_ascii_lowercase().as_str() {
        "amd64" | "x64" => "x86_64".into(),
        "arm64" => "aarch64".into(),
        other => other.into(),
    }
}

fn resolve_target(target_arch: Option<&str>) -> Result<Option<CrossToolchain>, RavenError> {
    let host = std::env::consts::ARCH;
    let target = match target_arch.map(normalize_arch) {
        None => return Ok(None),
        Some(t) if t.is_empty() || t == "any" || t == "noarch" || t == host => return Ok(None),
        Some(t) => t,
    };
    match target.as_str() {
        "aarch64" => Ok(Some(CrossToolchain {
            triple: "aarch64-linux-gnu",
        })),
        "x86_64" => Ok(Some(CrossToolchain {
            triple: "x86_64-linux-gnu",
        })),
        _ => Err(RavenError::Unsupported(format!(
            "cannot build for target architecture '{}' on host '{}'",
            target, host
        ))),
    }
}

/// Quotes a string for safe inclusion in a POSIX shell script.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn hash_file(path: &Path) -> Result<String, RavenError> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Extracts a tarball (gzip, xz, bzip2 or uncompressed), detected by magic bytes.
pub fn extract_archive(archive: &Path, dest: &Path) -> Result<(), RavenError> {
    let mut file = File::open(archive)?;
    let mut magic = [0u8; 6];
    let n = file.read(&mut magic)?;
    file.seek(SeekFrom::Start(0))?;
    let magic = &magic[..n];
    let reader: Box<dyn Read> = if magic.starts_with(&[0x1f, 0x8b]) {
        Box::new(flate2::read::MultiGzDecoder::new(BufReader::new(file)))
    } else if magic.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0x00]) {
        Box::new(xz2::read::XzDecoder::new_multi_decoder(BufReader::new(
            file,
        )))
    } else if magic.starts_with(b"BZh") {
        Box::new(bzip2::read::MultiBzDecoder::new(BufReader::new(file)))
    } else if magic.starts_with(&[0x50, 0x4b]) || magic.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        return Err(RavenError::Unsupported(
            "source archive format (zip/zstd) is not supported; use .tar, .tar.gz, .tar.xz or .tar.bz2"
                .into(),
        ));
    } else {
        Box::new(BufReader::new(file))
    };
    let mut ar = tar::Archive::new(reader);
    ar.set_preserve_permissions(true);
    ar.set_preserve_mtime(true);
    ar.set_overwrite(true);
    ar.unpack(dest).map_err(|e| {
        RavenError::IoError(std::io::Error::new(
            e.kind(),
            format!("failed to extract source archive: {}", e),
        ))
    })
}

/// If the extracted source contains exactly one top-level directory, returns it.
fn single_top_dir(src_dir: &Path) -> Result<Option<String>, RavenError> {
    let entries: Vec<_> = fs::read_dir(src_dir)?.collect::<Result<_, _>>()?;
    if entries.len() == 1 && entries[0].file_type()?.is_dir() {
        return Ok(Some(entries[0].file_name().to_string_lossy().into_owned()));
    }
    Ok(None)
}

fn log_tail(path: &Path, lines: usize) -> String {
    let content = fs::read(path).unwrap_or_default();
    let text = String::from_utf8_lossy(&content);
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

fn local_source_path(url: &str) -> Option<PathBuf> {
    if let Some(p) = url.strip_prefix("file://") {
        Some(PathBuf::from(p))
    } else if url.starts_with('/') {
        Some(PathBuf::from(url))
    } else {
        None
    }
}

impl Builder {
    pub fn new(work_dir: PathBuf, cache_dir: PathBuf) -> Result<Self, RavenError> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("raven/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(30))
            .read_timeout(Duration::from_secs(120))
            .build()?;
        Ok(Self {
            work_dir,
            cache_dir,
            client,
        })
    }

    pub async fn build(&self, recipe: &Recipe) -> Result<BuildOutput, RavenError> {
        let cross = resolve_target(recipe.target_arch.as_deref())?;
        let pkg_dir = self
            .work_dir
            .join(format!("{}-{}", recipe.name, recipe.version));
        let src_dir = pkg_dir.join("src");
        let out_dir = pkg_dir.join("out");

        // Fresh build root.
        if pkg_dir.exists() {
            fs::remove_dir_all(&pkg_dir)?;
        }
        fs::create_dir_all(&src_dir)?;
        fs::create_dir_all(&out_dir)?;

        // Fetch (or reuse cached) verified source.
        let source = self.fetch_source(recipe).await?;

        let spinner = create_spinner(&format!("Building {} {}...", recipe.name, recipe.version));

        let (archive, dest) = (source.clone(), src_dir.clone());
        tokio::task::spawn_blocking(move || extract_archive(&archive, &dest))
            .await
            .map_err(|e| RavenError::IoError(std::io::Error::other(e)))??;

        let workdir = match single_top_dir(&src_dir)? {
            Some(d) => format!("/src/{}", d),
            None => "/src".to_string(),
        };

        let mut script = String::from("set -e\n");
        if let Some(tc) = &cross {
            script.push_str(&format!(
                "export CC={t}-gcc CXX={t}-g++ AR={t}-ar CROSS_COMPILE={t}- HOST_TRIPLE={t}\n",
                t = tc.triple
            ));
        }
        script.push_str(&format!("cd {}\nset -x\n", shell_quote(&workdir)));
        for cmd in recipe.build_commands.iter().chain(&recipe.install_commands) {
            script.push_str(cmd);
            script.push('\n');
        }

        let jobs = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        let env: Vec<(String, String)> = vec![
            (
                "PATH".into(),
                "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".into(),
            ),
            ("HOME".into(), "/tmp".into()),
            ("TMPDIR".into(), "/tmp".into()),
            ("SHELL".into(), "/bin/sh".into()),
            ("LC_ALL".into(), "C".into()),
            ("TERM".into(), "dumb".into()),
            ("DESTDIR".into(), "/out".into()),
            ("RAVEN_PKG_NAME".into(), recipe.name.0.clone()),
            ("RAVEN_PKG_VERSION".into(), recipe.version.clone()),
            ("RAVEN_JOBS".into(), jobs.to_string()),
        ];

        let log_path = pkg_dir.join("build.log");
        let log = File::create(&log_path)?;
        let sandbox = ScriptSandbox::new(&pkg_dir);
        let success = tokio::task::spawn_blocking(move || sandbox.run(&script, &env, log))
            .await
            .map_err(|e| RavenError::IoError(std::io::Error::other(e)))??;
        spinner.finish_and_clear();

        if !success {
            return Err(RavenError::BuildFailed {
                package: recipe.name.0.clone(),
                tail: log_tail(&log_path, 25),
                log: log_path,
            });
        }

        log_success(&format!("Build complete: {}", recipe.name));
        Ok(BuildOutput { pkg_dir, out_dir })
    }

    /// Removes the build directory after a successful installation.
    pub fn cleanup(&self, output: &BuildOutput) {
        let _ = fs::remove_dir_all(&output.pkg_dir);
    }

    /// Returns the path of a verified copy of the recipe's source archive,
    /// stored in the content-addressed cache.
    pub async fn fetch_source(&self, recipe: &Recipe) -> Result<PathBuf, RavenError> {
        let expected = recipe.sha256_sum.0.trim().to_lowercase();
        fs::create_dir_all(&self.cache_dir)?;
        let cached = self.cache_dir.join(&expected);

        if cached.is_file() {
            if hash_file(&cached)? == expected {
                return Ok(cached);
            }
            fs::remove_file(&cached)?;
        }

        let part = self.cache_dir.join(format!("{}.part", expected));
        if let Some(local) = local_source_path(&recipe.source_url) {
            fs::copy(&local, &part).map_err(|e| {
                RavenError::Download(format!("cannot read {}: {}", local.display(), e))
            })?;
        } else {
            self.download_with_retry(&recipe.source_url, &part, &recipe.name.0)
                .await?;
        }

        let actual = hash_file(&part)?;
        if actual != expected {
            let _ = fs::remove_file(&part);
            return Err(RavenError::HashMismatch {
                file: recipe.source_url.clone(),
                expected,
                actual,
            });
        }
        fs::rename(&part, &cached)?;
        Ok(cached)
    }

    async fn download_with_retry(
        &self,
        url: &str,
        path: &Path,
        pkg: &str,
    ) -> Result<(), RavenError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            match self.download_once(url, path, pkg).await {
                Ok(()) => return Ok(()),
                Err((e, retryable)) => {
                    let _ = fs::remove_file(path);
                    if !retryable || attempt >= MAX_DOWNLOAD_ATTEMPTS {
                        return Err(e);
                    }
                    let wait = Duration::from_secs(1 << attempt);
                    crate::ui::log_warn(&format!(
                        "Download attempt {}/{} failed ({}); retrying in {}s",
                        attempt,
                        MAX_DOWNLOAD_ATTEMPTS,
                        e,
                        wait.as_secs()
                    ));
                    tokio::time::sleep(wait).await;
                }
            }
        }
    }

    async fn download_once(
        &self,
        url: &str,
        path: &Path,
        pkg: &str,
    ) -> Result<(), (RavenError, bool)> {
        let net = |e: reqwest::Error| (RavenError::NetworkError(e), true);

        let mut resp = self.client.get(url).send().await.map_err(net)?;
        let status = resp.status();
        if !status.is_success() {
            let retryable =
                status.is_server_error() || status.as_u16() == 408 || status.as_u16() == 429;
            return Err((
                RavenError::Download(format!("{} returned HTTP {}", url, status)),
                retryable,
            ));
        }

        let pb = create_download_bar(resp.content_length().unwrap_or(0), pkg);
        let mut file = tokio::fs::File::create(path)
            .await
            .map_err(|e| (e.into(), false))?;
        while let Some(chunk) = resp.chunk().await.map_err(net)? {
            file.write_all(&chunk)
                .await
                .map_err(|e| (e.into(), false))?;
            pb.inc(chunk.len() as u64);
        }
        file.sync_all().await.map_err(|e| (e.into(), false))?;
        pb.finish_and_clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::HashSum;
    use std::io::Write;

    fn make_tar() -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        let data = b"hello world";
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, "pkg-1.0/file.txt", &data[..])
            .unwrap();
        builder.into_inner().unwrap()
    }

    fn check_extract(name: &str, bytes: Vec<u8>) {
        let tmp = tempfile::tempdir().unwrap();
        let archive = tmp.path().join(name);
        fs::write(&archive, bytes).unwrap();
        let out = tmp.path().join("out");
        fs::create_dir_all(&out).unwrap();
        extract_archive(&archive, &out).unwrap();
        assert_eq!(
            fs::read_to_string(out.join("pkg-1.0/file.txt")).unwrap(),
            "hello world"
        );
        assert_eq!(single_top_dir(&out).unwrap().as_deref(), Some("pkg-1.0"));
    }

    #[test]
    fn extracts_all_supported_formats() {
        let tar_bytes = make_tar();

        check_extract("plain.tar", tar_bytes.clone());

        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&tar_bytes).unwrap();
        check_extract("a.tar.gz", gz.finish().unwrap());

        let mut xz = xz2::write::XzEncoder::new(Vec::new(), 6);
        xz.write_all(&tar_bytes).unwrap();
        check_extract("a.tar.xz", xz.finish().unwrap());

        let mut bz = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
        bz.write_all(&tar_bytes).unwrap();
        check_extract("a.tar.bz2", bz.finish().unwrap());
    }

    #[test]
    fn rejects_unsupported_archives() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = tmp.path().join("a.zip");
        fs::write(&archive, b"PK\x03\x04junk").unwrap();
        assert!(matches!(
            extract_archive(&archive, tmp.path()),
            Err(RavenError::Unsupported(_))
        ));
    }

    #[test]
    fn shell_quoting() {
        assert_eq!(shell_quote("/src/a b"), "'/src/a b'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn target_resolution() {
        assert!(resolve_target(None).unwrap().is_none());
        assert!(resolve_target(Some("any")).unwrap().is_none());
        assert!(resolve_target(Some(std::env::consts::ARCH))
            .unwrap()
            .is_none());
        assert!(resolve_target(Some("sparc")).is_err());
    }

    #[tokio::test]
    async fn fetches_local_sources_with_verification_and_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src.tar");
        fs::write(&src, b"source bytes").unwrap();
        let sum = hex::encode(Sha256::digest(b"source bytes"));

        let builder = Builder::new(tmp.path().join("build"), tmp.path().join("cache")).unwrap();
        let mut recipe = Recipe {
            name: "x".into(),
            version: "1.0".into(),
            description: String::new(),
            target_arch: None,
            dependencies: vec![],
            source_url: format!("file://{}", src.display()),
            sha256_sum: HashSum(sum.to_uppercase()),
            build_commands: vec![],
            install_commands: vec![],
        };

        let cached = builder.fetch_source(&recipe).await.unwrap();
        assert_eq!(fs::read(&cached).unwrap(), b"source bytes");

        // Served from cache even if the original disappears.
        fs::remove_file(&src).unwrap();
        assert_eq!(builder.fetch_source(&recipe).await.unwrap(), cached);

        // Wrong checksum is rejected and nothing is cached.
        fs::write(&src, b"tampered").unwrap();
        recipe.sha256_sum = HashSum("0".repeat(64));
        let err = builder.fetch_source(&recipe).await.unwrap_err();
        assert!(matches!(err, RavenError::HashMismatch { .. }));
        assert!(!tmp.path().join("cache").join("0".repeat(64)).exists());
    }
}
