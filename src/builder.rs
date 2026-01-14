use crate::core::{RavenError, Recipe};
use crate::sandbox::ScriptSandbox;
use crate::ui::{create_download_bar, create_spinner, log_success};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

pub struct Builder {
    work_dir: PathBuf,
}

impl Builder {
    pub fn new(work_dir: PathBuf) -> Self {
        Self { work_dir }
    }

    pub async fn build(&self, recipe: &Recipe) -> Result<PathBuf, RavenError> {
        let pkg_dir = self.work_dir.join(format!("{}-build", recipe.name.0));
        let src_dir = pkg_dir.join("src");
        let out_dir = pkg_dir.join("out");

        let spinner = create_spinner(&format!(
            "Preparing build environment for {}...",
            recipe.name.0
        ));

        // Cleanup previous runs
        if pkg_dir.exists() {
            tokio::fs::remove_dir_all(&pkg_dir).await?;
        }
        tokio::fs::create_dir_all(&src_dir).await?;
        tokio::fs::create_dir_all(&out_dir).await?;

        // Create system directories required for 'configure' and 'make'
        let sys_dirs = ["proc", "dev", "bin", "usr", "lib", "lib64", "etc", "tmp"];
        for dir in sys_dirs {
            tokio::fs::create_dir_all(pkg_dir.join(dir)).await?;
        }

        // QEMU (Cross-Compilation support)
        let current_arch = std::env::consts::ARCH;
        let target_arch = recipe.target_arch.as_deref().unwrap_or(current_arch);
        let needs_qemu = target_arch != current_arch;

        if needs_qemu {
            let qemu_bin = "/usr/bin/qemu-aarch64-static";
            let dest = pkg_dir.join("usr/bin/qemu-aarch64-static");
            if Path::new(qemu_bin).exists() {
                tokio::fs::create_dir_all(dest.parent().unwrap()).await?;
                tokio::fs::copy(qemu_bin, dest).await?;
            }
        }
        spinner.finish_and_clear();

        // Download with Retry & Progress Bar
        let tarball = pkg_dir.join("source.tar");
        self.download_with_retry(
            &recipe.source_url,
            &tarball,
            &recipe.sha256_sum.0,
            &recipe.name.0,
        )
        .await?;

        let spinner_build = create_spinner(&format!(
            "Compiling {} (this may take a while)...",
            recipe.name.0
        ));

        // Unpack
        let tar_clone = tarball.clone();
        let src_clone = src_dir.clone();
        tokio::task::spawn_blocking(move || {
            let f = File::open(tar_clone).unwrap();
            let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(f));
            ar.unpack(src_clone).unwrap();
        })
        .await
        .map_err(|e| RavenError::IoError(std::io::Error::new(std::io::ErrorKind::Other, e)))?;

        // Prepare Sandbox Script
        let mut cross_env = String::new();
        if needs_qemu {
            cross_env = format!(
                "export CC=aarch64-linux-gnu-gcc\nexport CXX=aarch64-linux-gnu-g++\nexport CROSS_COMPILE=aarch64-linux-gnu-\n"
            );
        }

        let script = format!(
            "{}\nexport DESTDIR=/out\ncd /src\nDIR=$(ls -d */ | head -n 1)\nif [ -n \"$DIR\" ]; then cd \"$DIR\"; fi\n{}\n{}", 
            cross_env,
            recipe.build_commands.join("\n"),
            recipe.install_commands.join("\n")
        );

        let sandbox = ScriptSandbox::new(&pkg_dir);
        let log = File::create(pkg_dir.join("build.log"))?;

        // EXECUTE SANDBOX
        sandbox.run(&script, log)?;

        spinner_build.finish_and_clear();
        log_success(&format!("Build complete: {}", recipe.name.0));

        Ok(out_dir)
    }

    // Robust download logic
    async fn download_with_retry(
        &self,
        url: &str,
        path: &Path,
        hash: &str,
        pkg_name: &str,
    ) -> Result<(), RavenError> {
        let max_retries = 3;
        let mut attempt = 0;

        loop {
            attempt += 1;
            match self.download_inner(url, path, hash, pkg_name).await {
                Ok(_) => return Ok(()),
                Err(e) => {
                    if attempt >= max_retries {
                        return Err(e);
                    }
                    tokio::time::sleep(Duration::from_secs(attempt)).await;
                }
            }
        }
    }

    async fn download_inner(
        &self,
        url: &str,
        path: &Path,
        hash: &str,
        pkg_name: &str,
    ) -> Result<(), RavenError> {
        let client = reqwest::Client::builder()
            .user_agent("RavenPackageManager/1.0 (MissionCritical)")
            .build()
            .map_err(RavenError::NetworkError)?;

        let mut resp = client.get(url).send().await?;

        if !resp.status().is_success() {
            return Err(RavenError::NetworkError(
                resp.error_for_status().unwrap_err(),
            ));
        }

        let total_size = resp.content_length().unwrap_or(0);
        let pb = create_download_bar(total_size, pkg_name);

        let mut file = tokio::fs::File::create(path).await?;
        let mut hasher = Sha256::new();
        let mut downloaded: u64 = 0;

        while let Some(chunk) = resp.chunk().await? {
            file.write_all(&chunk).await?;
            hasher.update(&chunk);
            downloaded += chunk.len() as u64;
            pb.set_position(downloaded);
        }

        pb.finish_with_message("Download complete");

        let calculated_hash = hex::encode(hasher.finalize());
        if calculated_hash != hash {
            return Err(RavenError::HashMismatch);
        }
        Ok(())
    }
}
