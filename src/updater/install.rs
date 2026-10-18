//! Finding how this copy was installed, and applying a verified update over
//! it. Each `apply` leaves the new version running, or about to run, and
//! expects the caller to quit.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context as _, Result, anyhow, ensure};

use super::protocol::Bundle;

/// The package name the Linux packages install under.
const PACKAGE: &str = "quadrant-native";

/// How this copy was installed, or `None` when it shouldn't update itself:
/// a build run from `target/`, or a package manager that owns its updates.
pub fn detect() -> Option<Bundle> {
    let exe = env::current_exe().ok()?;
    if cfg!(windows) {
        // The NSIS setup puts its uninstaller beside the exe.
        exe.with_file_name("uninstall.exe")
            .exists()
            .then_some(Bundle::Nsis)
    } else if cfg!(target_os = "macos") {
        app_bundle(&exe).map(|_| Bundle::App)
    } else if env::var_os("APPIMAGE").is_some() {
        Some(Bundle::AppImage)
    } else if Path::new(&format!("/var/lib/dpkg/info/{PACKAGE}.list")).exists() {
        Some(Bundle::Deb)
    } else {
        let rpm = Command::new("rpm")
            .args(["-q", "--quiet", PACKAGE])
            .stderr(Stdio::null())
            .status();
        rpm.is_ok_and(|status| status.success())
            .then_some(Bundle::Rpm)
    }
}

/// Installs `bytes`, the verified artifact for `bundle`.
pub fn apply(bundle: Bundle, bytes: &[u8]) -> Result<()> {
    match bundle {
        Bundle::Nsis => run_nsis(bytes),
        Bundle::AppImage => replace_appimage(bytes),
        Bundle::Deb => install_package(bytes, "deb", &["dpkg", "-i"]),
        Bundle::Rpm => install_package(bytes, "rpm", &["rpm", "-U"]),
        Bundle::App => replace_app(bytes),
    }
}

/// Runs the setup passively (`/P`); `/R` starts the app again when it is
/// done. The setup closes this copy if it is still running.
fn run_nsis(bytes: &[u8]) -> Result<()> {
    let setup = env::temp_dir().join(format!(
        "quadrant-native-update-{}-setup.exe",
        std::process::id()
    ));
    fs::write(&setup, bytes).with_context(|| format!("couldn't write {}", setup.display()))?;
    // Start-Process goes through ShellExecute, so a setup that needs
    // elevation asks for it instead of failing.
    let quoted = setup.display().to_string().replace('\'', "''");
    let mut command = Command::new("powershell.exe");
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-WindowStyle",
        "Hidden",
        "-Command",
        &format!("Start-Process -FilePath '{quoted}' -ArgumentList '/P','/R'"),
    ]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let status = command
        .status()
        .context("couldn't start the update's setup")?;
    ensure!(
        status.success(),
        "couldn't start the update's setup ({status})"
    );
    Ok(())
}

/// Swaps the AppImage file in place and starts it.
fn replace_appimage(bytes: &[u8]) -> Result<()> {
    let target = PathBuf::from(env::var_os("APPIMAGE").context("APPIMAGE isn't set")?);
    let staged = target.with_extension("update");
    fs::write(&staged, bytes).with_context(|| format!("couldn't write {}", staged.display()))?;
    make_executable(&staged)?;
    fs::rename(&staged, &target)
        .with_context(|| format!("couldn't replace {}", target.display()))?;
    relaunch(&target)
}

/// Upgrades the system package through polkit, then starts the new binary.
fn install_package(bytes: &[u8], extension: &str, tool: &[&str]) -> Result<()> {
    let exe = env::current_exe()?;
    let dir = private_temp_dir()?;
    let package = dir.join(format!("quadrant-native.{extension}"));
    fs::write(&package, bytes)?;
    let status = Command::new("pkexec").args(tool).arg(&package).status();
    let _ = fs::remove_dir_all(&dir);
    let status = status.context("couldn't run pkexec")?;
    ensure!(status.success(), "the package manager failed ({status})");
    relaunch(&exe)
}

/// Unpacks the `.app.tar.gz` beside the running bundle and swaps the two.
fn replace_app(bytes: &[u8]) -> Result<()> {
    let exe = env::current_exe()?;
    let bundle = app_bundle(&exe).context("not running from an .app bundle")?;
    let parent = bundle.parent().context("the .app has no parent folder")?;
    let staging = parent.join(format!(".quadrant-native-update-{}", std::process::id()));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;
    let result = (|| {
        let archive = staging.join("update.app.tar.gz");
        fs::write(&archive, bytes)?;
        let status = Command::new("tar")
            .arg("-xzf")
            .arg(&archive)
            .arg("-C")
            .arg(&staging)
            .status()?;
        ensure!(status.success(), "couldn't unpack the update ({status})");
        let unpacked = fs::read_dir(&staging)?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .find(|path| path.extension().is_some_and(|ext| ext == "app"))
            .context("the update has no .app inside")?;
        let old = staging.join("previous.app");
        fs::rename(&bundle, &old)?;
        if let Err(error) = fs::rename(&unpacked, &bundle) {
            fs::rename(&old, &bundle)?;
            return Err(error.into());
        }
        Ok(())
    })();
    let _ = fs::remove_dir_all(&staging);
    result?;
    let status = Command::new("open").arg("-n").arg(&bundle).status()?;
    ensure!(
        status.success(),
        "couldn't start the updated app ({status})"
    );
    Ok(())
}

/// The `.app` folder holding `exe`, as in `Foo.app/Contents/MacOS/foo`.
fn app_bundle(exe: &Path) -> Option<PathBuf> {
    exe.ancestors()
        .find(|path| path.extension().is_some_and(|ext| ext == "app"))
        .map(Path::to_path_buf)
}

/// Starts `program` with this run's arguments.
fn relaunch(program: &Path) -> Result<()> {
    Command::new(program)
        .args(env::args_os().skip(1))
        .spawn()
        .map(drop)
        .map_err(|error| anyhow!("couldn't start {}: {error}", program.display()))
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_: &Path) -> Result<()> {
    Ok(())
}

/// A temp folder only this user can read, so nobody swaps the package
/// between writing it and installing it as root.
#[cfg(unix)]
fn private_temp_dir() -> Result<PathBuf> {
    use std::os::unix::fs::DirBuilderExt as _;
    let dir = env::temp_dir().join(format!("quadrant-native-update-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::DirBuilder::new().mode(0o700).create(&dir)?;
    Ok(dir)
}

#[cfg(not(unix))]
fn private_temp_dir() -> Result<PathBuf> {
    Ok(env::temp_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_app_bundle_of_its_binary() {
        let exe = Path::new("/Applications/Quadrant-Native.app/Contents/MacOS/quadrant-native");
        assert_eq!(
            app_bundle(exe),
            Some(PathBuf::from("/Applications/Quadrant-Native.app"))
        );
        assert_eq!(app_bundle(Path::new("/usr/bin/quadrant-native")), None);
    }

    #[test]
    fn a_build_in_target_doesnt_update_itself() {
        assert_eq!(detect(), None);
    }
}
