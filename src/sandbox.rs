//! Build sandbox based on Linux namespaces and `chroot`.
//!
//! The build root contains `/src` (sources) and `/out` (DESTDIR). Host tool
//! directories are bind-mounted **read-only**, so a build script that ignores
//! `DESTDIR` fails instead of silently writing into the host system.

use crate::core::RavenError;
use std::fs::File;
use std::path::{Path, PathBuf};

pub struct ScriptSandbox {
    root: PathBuf,
}

impl ScriptSandbox {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    /// Runs `script` with `/bin/sh` inside the sandbox, with a clean
    /// environment. stdout/stderr go to `log`. Returns whether it succeeded.
    #[cfg(target_os = "linux")]
    pub fn run(
        &self,
        script: &str,
        env: &[(String, String)],
        log: File,
    ) -> Result<bool, RavenError> {
        use nix::mount::{mount, MsFlags};
        use nix::sched::{unshare, CloneFlags};
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};

        fn errno(e: nix::errno::Errno) -> std::io::Error {
            std::io::Error::from_raw_os_error(e as i32)
        }

        // (host source, target inside root, read-only)
        const BINDS: &[(&str, bool)] = &[
            ("/bin", true),
            ("/sbin", true),
            ("/usr", true),
            ("/lib", true),
            ("/lib32", true),
            ("/lib64", true),
            ("/libx32", true),
            ("/etc", true),
            ("/opt", true),
            ("/sys", true),
            ("/dev", false),
        ];

        // Everything that allocates happens here, before fork().
        let mut binds: Vec<(PathBuf, PathBuf, bool)> = Vec::new();
        for (dir, ro) in BINDS {
            let host = Path::new(dir);
            if host.exists() {
                let target = self.root.join(dir.trim_start_matches('/'));
                std::fs::create_dir_all(&target)?;
                binds.push((host.to_path_buf(), target, *ro));
            }
        }
        let proc_dir = self.root.join("proc");
        std::fs::create_dir_all(&proc_dir)?;
        let tmp_dir = self.root.join("tmp");
        std::fs::create_dir_all(&tmp_dir)?;
        std::fs::set_permissions(&tmp_dir, std::fs::Permissions::from_mode(0o1777))?;
        let root = self.root.clone();

        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c")
            .arg(script)
            .env_clear()
            .envs(env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);

        // SAFETY: the closure only performs async-signal-safe syscalls on
        // pre-computed paths (nix uses stack buffers for path conversion).
        unsafe {
            cmd.pre_exec(move || {
                unshare(
                    CloneFlags::CLONE_NEWNS | CloneFlags::CLONE_NEWUTS | CloneFlags::CLONE_NEWIPC,
                )
                .map_err(errno)?;
                mount(
                    None::<&str>,
                    "/",
                    None::<&str>,
                    MsFlags::MS_REC | MsFlags::MS_PRIVATE,
                    None::<&str>,
                )
                .map_err(errno)?;
                let _ = nix::unistd::sethostname("raven-build");

                for (host, target, ro) in &binds {
                    mount(
                        Some(host.as_path()),
                        target.as_path(),
                        None::<&str>,
                        MsFlags::MS_BIND | MsFlags::MS_REC,
                        None::<&str>,
                    )
                    .map_err(errno)?;
                    if *ro {
                        // Best effort: some container runtimes forbid the remount.
                        let _ = mount(
                            None::<&str>,
                            target.as_path(),
                            None::<&str>,
                            MsFlags::MS_BIND | MsFlags::MS_REMOUNT | MsFlags::MS_RDONLY,
                            None::<&str>,
                        );
                    }
                }
                mount(
                    Some("proc"),
                    proc_dir.as_path(),
                    Some("proc"),
                    MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC,
                    None::<&str>,
                )
                .map_err(errno)?;

                nix::unistd::chroot(root.as_path()).map_err(errno)?;
                nix::unistd::chdir("/").map_err(errno)?;
                Ok(())
            });
        }

        let status = cmd
            .spawn()
            .map_err(|e| {
                if e.raw_os_error() == Some(libc::EPERM) {
                    RavenError::Unsupported(
                        "cannot create the build sandbox (operation not permitted). Run as root; \
                         inside Docker use --privileged"
                            .into(),
                    )
                } else {
                    RavenError::IoError(std::io::Error::new(
                        e.kind(),
                        format!("failed to start build sandbox: {}", e),
                    ))
                }
            })?
            .wait()?;
        Ok(status.success())
    }

    #[cfg(not(target_os = "linux"))]
    pub fn run(
        &self,
        _script: &str,
        _env: &[(String, String)],
        _log: File,
    ) -> Result<bool, RavenError> {
        let _ = &self.root;
        Err(RavenError::Unsupported(
            "sandboxed builds require Linux (namespaces + chroot)".into(),
        ))
    }
}
