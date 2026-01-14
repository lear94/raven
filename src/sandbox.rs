use crate::core::RavenError;
use nix::mount::{mount, MsFlags};
use nix::sched::{unshare, CloneFlags};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

pub struct ScriptSandbox {
    root: std::path::PathBuf,
}

impl ScriptSandbox {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    pub fn run(&self, script: &str, log: std::fs::File) -> Result<(), RavenError> {
        let root = self.root.clone();

        let output = unsafe {
            Command::new("/bin/sh")
                .arg("-c")
                .arg(script)
                .stdout(log.try_clone().unwrap())
                .stderr(log)
                .pre_exec(move || {
                    // 1. Isolate Filesystem & Hostname
                    // NOTE: CLONE_NEWPID removed to avoid "cannot fork" errors in chroot without init
                    unshare(CloneFlags::CLONE_NEWNS | CloneFlags::CLONE_NEWUTS)
                        .map_err(|e| std::io::Error::from_raw_os_error(e as i32))?;

                    // 2. Make mount propagation private
                    mount(
                        None::<&str>,
                        "/",
                        None::<&str>,
                        MsFlags::MS_REC | MsFlags::MS_PRIVATE,
                        None::<&str>,
                    )
                    .map_err(|e| std::io::Error::from_raw_os_error(e as i32))?;

                    // 3. Mount /proc (Crucial for build tools)
                    let proc_path = root.join("proc");
                    if proc_path.exists() {
                        mount(
                            Some("proc"),
                            proc_path.as_path(),
                            Some("proc"),
                            MsFlags::empty(),
                            None::<&str>,
                        )
                        .map_err(|e| std::io::Error::from_raw_os_error(e as i32))?;
                    }

                    // 4. Bind Mounts: Project host tools into sandbox
                    let bind_dirs = ["/bin", "/usr", "/lib", "/lib64", "/dev", "/etc"];

                    for dir in bind_dirs {
                        let host_source = Path::new(dir);
                        let sandbox_target = root.join(dir.trim_start_matches('/'));

                        if host_source.exists() {
                            if !sandbox_target.exists() {
                                let _ = std::fs::create_dir_all(&sandbox_target);
                            }

                            mount(
                                Some(host_source),
                                &sandbox_target,
                                Some("none"),
                                MsFlags::MS_BIND | MsFlags::MS_REC,
                                None::<&str>,
                            )
                            .map_err(|e| std::io::Error::from_raw_os_error(e as i32))?;
                        }
                    }

                    // 5. Enter Jail
                    nix::unistd::chroot(&root)
                        .map_err(|e| std::io::Error::from_raw_os_error(e as i32))?;
                    nix::unistd::chdir("/")
                        .map_err(|e| std::io::Error::from_raw_os_error(e as i32))?;

                    Ok(())
                })
                .spawn()
        }
        .map_err(|e| RavenError::IoError(e))?
        .wait_with_output()
        .map_err(|e| RavenError::IoError(e))?;

        if !output.status.success() {
            return Err(RavenError::DependencyError(
                "Build script failed (check build.log)".into(),
            ));
        }
        Ok(())
    }
}
