//! Auto-install of LSP server binaries. Each `LspLanguage` maps to a
//! `ServerSpec` that describes where to find and how to install its server.
//! All blocking work (subprocess spawning, HTTP downloads) runs on a
//! background OS thread via `iced_runtime::task::blocking` — never the UI
//! thread.
//!
//! Resolution order in `resolve_binary`:
//! 1. System PATH — user's own install always wins.
//! 2. DevScribe-managed dir (under `~/.local/share/devscribe/`), or rustup's
//!    own `~/.cargo/bin` for rust-analyzer specifically.
//!
//! Managed install locations vary by method to avoid root-permission issues:
//! - Pip    → `venvs/<package>/bin/<binary>` (dedicated venv, avoids PEP 668)
//! - Npm    → `servers/npm/bin/<binary>`     (`--prefix`, avoids /usr/lib)
//! - Rustup → `~/.cargo/bin/<binary>` (rustup's own, not DevScribe-managed)
//! - Download → `servers/<binary>`

use std::io::Read;
use std::path::PathBuf;
use std::process::Command;

use devscribe_core::lsp::LspLanguage;

/// How a server gets installed when it isn't already available.
pub enum InstallMethod {
    /// Create a dedicated Python venv and `pip install <package>` into it.
    Pip { package: &'static str },
    /// Run `npm install --prefix <managed_dir>` (avoids needing root for `-g`).
    Npm { packages: &'static [&'static str] },
    /// Download a pre-built binary from a GitHub release.
    GithubRelease {
        /// URL with `{version}` and `{triple}` placeholders.
        url_template: &'static str,
        /// Path inside the archive where the binary lives.
        binary_in_archive: &'static str,
        /// Pinned version string to use.
        version: &'static str,
    },
    /// Installs via `rustup component add rust-analyzer`. Rustup places a
    /// proxy binary for the active toolchain at `~/.cargo/bin/rust-analyzer`,
    /// which is typically already on PATH — so `resolve_binary`'s PATH check
    /// finds it right after install without needing a managed copy, though
    /// `managed_binary_path` also points there directly as a fallback for a
    /// `~/.cargo/bin`-less PATH.
    Rustup,
    /// Download a `.tar.gz` and unpack the entire archive into a managed
    /// subdirectory. `binary_relative` is the path to the executable within
    /// the unpacked tree (e.g. `"bin/jdtls"`).
    TarGzDirectory {
        url: &'static str,
        binary_relative: &'static str,
    },
    /// Cannot be auto-installed; surface `hint` as the error so the user knows
    /// what to run manually.
    Manual { hint: &'static str },
}

pub struct ServerSpec {
    pub language: LspLanguage,
    /// Bare binary name used for PATH lookup (e.g. `"clangd"`).
    pub binary_name: &'static str,
    pub method: InstallMethod,
}

pub fn spec_for(language: LspLanguage) -> ServerSpec {
    match language {
        LspLanguage::Rust => ServerSpec {
            language,
            binary_name: "rust-analyzer",
            method: InstallMethod::Rustup,
        },
        LspLanguage::Java => ServerSpec {
            language,
            binary_name: "jdtls",
            // Eclipse's "latest" snapshot URL always resolves to the newest build.
            // The archive extracts flat (no top-level wrapper dir) so bin/jdtls
            // is directly inside the unpack destination.
            method: InstallMethod::TarGzDirectory {
                url: "https://download.eclipse.org/jdtls/snapshots/jdt-language-server-latest.tar.gz",
                binary_relative: "bin/jdtls",
            },
        },
        LspLanguage::Python => ServerSpec {
            language,
            binary_name: "pyright-langserver",
            method: InstallMethod::Pip { package: "pyright" },
        },
        LspLanguage::JavaScript | LspLanguage::TypeScript => ServerSpec {
            language,
            binary_name: "typescript-language-server",
            method: InstallMethod::Npm {
                // typescript@7+ (Go rewrite) has no tsserver.js — pin to 5.x
                // which is what typescript-language-server requires.
                packages: &["typescript-language-server", "typescript@5"],
            },
        },
        LspLanguage::Cpp => ServerSpec {
            language,
            binary_name: "clangd",
            method: InstallMethod::GithubRelease {
                url_template: "https://github.com/clangd/clangd/releases/download/{version}/clangd-linux-{version}.zip",
                binary_in_archive: "clangd_18.1.3/bin/clangd",
                version: "18.1.3",
            },
        },
    }
}

/// Returns the managed install path for `spec`, or `None` on platforms
/// without a data directory. Paths differ per install method:
/// - Pip    → `~/.local/share/devscribe/venvs/<package>/bin/<binary>`
/// - Npm    → `~/.local/share/devscribe/servers/npm/bin/<binary>`
/// - Rustup → `~/.cargo/bin/<binary>` (rustup's own proxy location, not a
///   DevScribe-managed dir — there is nothing for us to manage, this is just
///   where to look for what rustup already installed)
/// - Download/Manual → `~/.local/share/devscribe/servers/<binary>`
pub fn managed_binary_path(spec: &ServerSpec) -> Option<PathBuf> {
    match &spec.method {
        InstallMethod::Pip { package } => Some(
            dirs::data_dir()?
                .join("devscribe")
                .join("venvs")
                .join(package)
                .join("bin")
                .join(spec.binary_name),
        ),
        InstallMethod::Npm { .. } => Some(
            dirs::data_dir()?
                .join("devscribe")
                .join("servers")
                .join("npm")
                .join("bin")
                .join(spec.binary_name),
        ),
        InstallMethod::Rustup => Some(dirs::home_dir()?.join(".cargo").join("bin").join(spec.binary_name)),
        // The entire archive is extracted into this directory; lsp.rs receives
        // the directory path and builds the java -jar command from it.
        InstallMethod::TarGzDirectory { .. } => {
            Some(dirs::data_dir()?.join("devscribe").join("servers").join(spec.binary_name))
        }
        InstallMethod::GithubRelease { .. } | InstallMethod::Manual { .. } => {
            Some(dirs::data_dir()?.join("devscribe").join("servers").join(spec.binary_name))
        }
    }
}

/// Resolves the binary for `spec`: PATH first, then the managed dir.
/// Returns `None` when neither location has the binary.
pub fn resolve_binary(spec: &ServerSpec) -> Option<PathBuf> {
    // 1. System PATH — user's own install always wins.
    if which_binary(spec.binary_name) {
        return Some(PathBuf::from(spec.binary_name));
    }
    // 2. DevScribe-managed install location.
    let managed = managed_binary_path(spec)?;
    if managed.exists() {
        return Some(managed);
    }
    None
}

pub fn which_binary(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Installs the server described by `spec` into the managed dir. Blocking —
/// must be called from `iced_runtime::task::blocking`.
pub fn install(spec: &ServerSpec) -> Result<(), String> {
    match &spec.method {
        InstallMethod::Rustup => install_via_rustup(),
        InstallMethod::TarGzDirectory { url, binary_relative } => {
            install_tar_gz_directory(spec, url, binary_relative)
        }
        InstallMethod::Manual { hint } => Err((*hint).to_string()),
        InstallMethod::Pip { package } => install_via_pip(package),
        InstallMethod::Npm { packages } => install_via_npm(packages),
        InstallMethod::GithubRelease { url_template, binary_in_archive, version } => {
            install_binary_release(spec, url_template, binary_in_archive, version)
        }
    }
}

/// Creates a dedicated Python venv and `pip install`s the package into it.
/// Avoids the PEP 668 "externally-managed-environment" error that blocks
/// `pip install --user` on modern distros (openSUSE, Ubuntu 23.04+, etc.).
fn install_via_pip(package: &str) -> Result<(), String> {
    let python = find_on_path(&["python3", "python"]).ok_or_else(|| {
        "python3 not found — install Python first: https://python.org".to_string()
    })?;

    let venv_dir = dirs::data_dir()
        .ok_or_else(|| "no data directory available".to_string())?
        .join("devscribe")
        .join("venvs")
        .join(package);

    // Create the venv (idempotent — safe to call if it already exists).
    let out = Command::new(&python)
        .arg("-m")
        .arg("venv")
        .arg(&venv_dir)
        .output()
        .map_err(|e| format!("failed to run python -m venv: {e}"))?;

    if !out.status.success() {
        return Err(format!(
            "venv creation failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }

    // Use the venv's own pip — no system-wide permissions needed.
    let venv_pip = venv_dir.join("bin").join("pip");
    let out = Command::new(&venv_pip)
        .args(["install", package])
        .output()
        .map_err(|e| format!("failed to run venv pip: {e}"))?;

    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

/// Installs rust-analyzer as a rustup component for the active toolchain.
/// Unlike the other methods, this doesn't unpack anything into a
/// DevScribe-managed directory — rustup owns the install and drops its own
/// proxy binary at `~/.cargo/bin/rust-analyzer` (see `managed_binary_path`),
/// which is why `resolve_binary`'s plain PATH check picks it up right after
/// this returns on any machine where `~/.cargo/bin` is on PATH, same as
/// `cargo`/`rustc` themselves.
fn install_via_rustup() -> Result<(), String> {
    let rustup = find_on_path(&["rustup"]).ok_or_else(|| {
        "rustup not found — install Rust via https://rustup.rs first".to_string()
    })?;

    let out = Command::new(&rustup)
        .args(["component", "add", "rust-analyzer"])
        .output()
        .map_err(|e| format!("failed to run rustup: {e}"))?;

    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

/// Installs via `npm install --prefix <managed_dir>` to avoid the EACCES
/// error that `npm install -g` causes when `/usr/lib/node_modules` is
/// root-owned.
fn install_via_npm(packages: &[&str]) -> Result<(), String> {
    let npm = find_on_path(&["npm"]).ok_or_else(|| {
        "npm not found — install Node.js first: https://nodejs.org".to_string()
    })?;

    let prefix = dirs::data_dir()
        .ok_or_else(|| "no data directory available".to_string())?
        .join("devscribe")
        .join("servers")
        .join("npm");

    std::fs::create_dir_all(&prefix)
        .map_err(|e| format!("cannot create npm prefix dir: {e}"))?;

    let mut cmd = Command::new(&npm);
    // -g installs binaries into <prefix>/bin/ (not node_modules/.bin/).
    // --prefix redirects the global root to our user-owned dir, no root needed.
    cmd.arg("install").arg("-g").arg("--prefix").arg(&prefix);
    for pkg in packages {
        cmd.arg(pkg);
    }

    let out = cmd
        .output()
        .map_err(|e| format!("failed to run npm: {e}"))?;

    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

fn install_binary_release(
    spec: &ServerSpec,
    url_template: &str,
    binary_in_archive: &str,
    version: &str,
) -> Result<(), String> {
    let triple = current_platform_triple()?;
    let url = url_template
        .replace("{version}", version)
        .replace("{triple}", &triple);

    let dest = managed_binary_path(spec)
        .ok_or_else(|| "no data directory available".to_string())?;

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create install dir: {e}"))?;
    }

    // Download to memory.
    let mut body = ureq::get(&url)
        .call()
        .map_err(|e| format!("download failed: {e}"))?
        .into_body();

    let mut bytes = Vec::new();
    body.as_reader()
        .read_to_end(&mut bytes)
        .map_err(|e| format!("download read error: {e}"))?;

    if url.ends_with(".zip") {
        extract_from_zip(&bytes, binary_in_archive, &dest)?;
    } else if url.ends_with(".tar.gz") || url.ends_with(".tgz") {
        extract_from_tar_gz(&bytes, binary_in_archive, &dest)?;
    } else if url.ends_with(".gz") {
        extract_gz(&bytes, &dest)?;
    } else {
        std::fs::write(&dest, &bytes)
            .map_err(|e| format!("write failed: {e}"))?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&dest)
            .map_err(|e| format!("stat failed: {e}"))?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&dest, perms)
            .map_err(|e| format!("chmod failed: {e}"))?;
    }

    Ok(())
}

/// Downloads a `.tar.gz` archive and unpacks its entire contents into
/// `<devscribe_data>/servers/<binary_name>/`, preserving the directory tree.
/// Used for servers like jdtls whose binary script needs sibling directories
/// (plugins, config) to be present at known relative paths.
fn install_tar_gz_directory(
    spec: &ServerSpec,
    url: &str,
    binary_relative: &str,
) -> Result<(), String> {
    let dest_dir = dirs::data_dir()
        .ok_or_else(|| "no data directory available".to_string())?
        .join("devscribe")
        .join("servers")
        .join(spec.binary_name);

    std::fs::create_dir_all(&dest_dir)
        .map_err(|e| format!("cannot create install dir: {e}"))?;

    let mut body = ureq::get(url)
        .call()
        .map_err(|e| format!("download failed: {e}"))?
        .into_body();

    let mut bytes = Vec::new();
    body.as_reader()
        .read_to_end(&mut bytes)
        .map_err(|e| format!("download read error: {e}"))?;

    let gz = flate2::read::GzDecoder::new(&bytes[..]);
    let mut archive = tar::Archive::new(gz);
    archive
        .unpack(&dest_dir)
        .map_err(|e| format!("extract failed: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let binary = dest_dir.join(binary_relative);
        if binary.exists() {
            let mut perms = std::fs::metadata(&binary)
                .map_err(|e| format!("stat failed: {e}"))?
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&binary, perms)
                .map_err(|e| format!("chmod failed: {e}"))?;
        }
    }

    Ok(())
}

fn extract_from_zip(bytes: &[u8], path_in_archive: &str, dest: &PathBuf) -> Result<(), String> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| format!("zip open failed: {e}"))?;

    let mut file = archive
        .by_name(path_in_archive)
        .map_err(|_| format!("'{path_in_archive}' not found in archive"))?;

    let mut out = std::fs::File::create(dest)
        .map_err(|e| format!("create failed: {e}"))?;
    std::io::copy(&mut file, &mut out)
        .map_err(|e| format!("extract failed: {e}"))?;

    Ok(())
}

fn extract_from_tar_gz(bytes: &[u8], path_in_archive: &str, dest: &PathBuf) -> Result<(), String> {
    let gz = flate2::read::GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(gz);

    for entry in archive.entries().map_err(|e| format!("tar error: {e}"))? {
        let mut entry = entry.map_err(|e| format!("tar entry error: {e}"))?;
        let entry_path = entry
            .path()
            .map_err(|e| format!("tar path error: {e}"))?
            .to_string_lossy()
            .into_owned();

        if entry_path == path_in_archive
            || entry_path.ends_with(&format!("/{}", path_in_archive))
        {
            let mut out = std::fs::File::create(dest)
                .map_err(|e| format!("create failed: {e}"))?;
            std::io::copy(&mut entry, &mut out)
                .map_err(|e| format!("extract failed: {e}"))?;
            return Ok(());
        }
    }

    Err(format!("'{path_in_archive}' not found in tar archive"))
}

fn extract_gz(bytes: &[u8], dest: &PathBuf) -> Result<(), String> {
    let mut decoder = flate2::read::GzDecoder::new(bytes);
    let mut decompressed = Vec::new();
    decoder
        .read_to_end(&mut decompressed)
        .map_err(|e| format!("gzip decode failed: {e}"))?;
    std::fs::write(dest, &decompressed).map_err(|e| format!("write failed: {e}"))?;
    Ok(())
}

fn find_on_path(names: &[&str]) -> Option<String> {
    for name in names {
        if which_binary(name) {
            return Some(name.to_string());
        }
    }
    None
}

fn current_platform_triple() -> Result<String, String> {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return Ok("linux-x64".into());
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return Ok("linux-arm64".into());
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return Ok("mac-x64".into());
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return Ok("mac-arm64".into());
    #[cfg(target_os = "windows")]
    return Ok("windows".into());
    #[allow(unreachable_code)]
    Err("unsupported platform for automatic install".into())
}
