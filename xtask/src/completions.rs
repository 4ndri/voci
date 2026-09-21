//! Install managed completion scripts and source blocks without replacing user configuration.
use anyhow::{Context, Result, ensure};
use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

const BEGIN: &str = "# >>> voci completions >>>";
const END: &str = "# <<< voci completions <<<";

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Shell {
    Bash,
    #[value(alias = "nu")]
    Nushell,
}

struct Locations {
    home: PathBuf,
    config: PathBuf,
}
impl Locations {
    fn from_env() -> Result<Self> {
        Self::resolve(
            env::var_os("HOME")
                .or_else(|| env::var_os("USERPROFILE"))
                .map(PathBuf::from),
            env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
            env::var_os("APPDATA").map(PathBuf::from),
        )
    }
    fn resolve(
        home: Option<PathBuf>,
        xdg: Option<PathBuf>,
        appdata: Option<PathBuf>,
    ) -> Result<Self> {
        let home = home
            .filter(|p| !p.as_os_str().is_empty())
            .context("Cannot find the user home directory for completion setup")?;
        ensure!(
            home.is_absolute(),
            "The user home directory must be absolute"
        );
        let config = if let Some(xdg) = xdg.filter(|p| !p.as_os_str().is_empty()) {
            xdg
        } else if cfg!(windows) {
            appdata
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| home.join("AppData/Roaming"))
        } else if cfg!(target_os = "macos") {
            home.join("Library/Application Support")
        } else {
            home.join(".config")
        };
        ensure!(
            config.is_absolute(),
            "The configuration directory must be absolute"
        );
        Ok(Self { home, config })
    }
}

fn read(path: &Path) -> Result<String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

fn source_block(existing: &str, body: &str) -> Result<String> {
    let newline = if existing.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut result = String::new();
    let mut inside = false;
    for line in existing.split_inclusive('\n') {
        match line.trim_end_matches(['\r', '\n']) {
            BEGIN => {
                ensure!(
                    !inside,
                    "Nested voci completion markers; repair the managed block before retrying"
                );
                inside = true;
            }
            END => {
                ensure!(
                    inside,
                    "Unmatched voci completion end marker; repair the managed block before retrying"
                );
                inside = false;
            }
            _ if !inside => result.push_str(line),
            _ => {}
        }
    }
    ensure!(
        !inside,
        "Unclosed voci completion block; repair the managed block before retrying"
    );
    if !result.is_empty() && !result.ends_with('\n') {
        result.push_str(newline);
    }
    result.push_str(BEGIN);
    result.push_str(newline);
    result.push_str(&body.replace('\n', newline));
    result.push_str(newline);
    result.push_str(END);
    result.push_str(newline);
    Ok(result)
}

fn path_text(path: &Path) -> Result<String> {
    let path = path
        .to_str()
        .context("Completion setup requires UTF-8 configuration paths")?;
    ensure!(
        !path.chars().any(char::is_control),
        "Completion paths cannot contain control characters"
    );
    Ok(if cfg!(windows) {
        path.replace('\\', "/")
    } else {
        path.to_owned()
    })
}

fn plan(locations: &Locations, shell: Option<Shell>) -> Result<Vec<(PathBuf, String)>> {
    let scripts = locations.config.join("voci/completions");
    let mut files = vec![];
    if shell.is_none() || shell == Some(Shell::Bash) {
        let script = scripts.join("voci.bash");
        let quoted = format!("'{}'", path_text(&script)?.replace('\'', "'\\''"));
        let body = format!(
            "if [ -n \"${{BASH_VERSION-}}\" ]; then\n    case $- in\n        *i*) . {quoted} ;;\n    esac\nfi"
        );
        files.push((
            script,
            include_str!("../../assets/completions/voci.bash").into(),
        ));
        let login = [".bash_profile", ".bash_login", ".profile"]
            .into_iter()
            .map(|name| locations.home.join(name))
            .find(|path| path.exists())
            .unwrap_or_else(|| locations.home.join(".bash_profile"));
        for path in [locations.home.join(".bashrc"), login] {
            let updated = source_block(&read(&path)?, &body)
                .with_context(|| format!("update {}", path.display()))?;
            files.push((path, updated));
        }
    }
    if shell.is_none() || shell == Some(Shell::Nushell) {
        let script = scripts.join("voci.nu");
        let body = format!("source {}", serde_json::to_string(&path_text(&script)?)?);
        files.push((
            script,
            include_str!("../../assets/completions/voci.nu").into(),
        ));
        let path = locations.config.join("nushell/config.nu");
        let updated = source_block(&read(&path)?, &body)
            .with_context(|| format!("update {}", path.display()))?;
        files.push((path, updated));
    }
    Ok(files)
}

fn write_if_changed(path: &Path, contents: &str) -> Result<bool> {
    if read(path)? == contents {
        return Ok(false);
    }
    // Follow existing dotfile symlinks, keeping links into dotfile repositories intact.
    let target = if path.is_symlink() {
        fs::canonicalize(path)?
    } else {
        path.to_owned()
    };
    let parent = target.parent().context("Completion path has no parent")?;
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    if let Ok(metadata) = fs::metadata(&target) {
        temporary
            .as_file()
            .set_permissions(metadata.permissions())?;
    }
    temporary.write_all(contents.as_bytes())?;
    temporary.as_file().sync_all()?;
    temporary.persist(&target)?;
    Ok(true)
}

fn setup_at(locations: &Locations, shell: Option<Shell>) -> Result<()> {
    // Validate every managed block before writing any files.
    for (path, contents) in plan(locations, shell)? {
        let changed = write_if_changed(&path, &contents)
            .with_context(|| format!("set up {}", path.display()))?;
        eprintln!(
            "{} {}",
            if changed { "Updated" } else { "Unchanged" },
            path.display()
        );
    }
    eprintln!(
        "Completions are configured. Open a new shell session to load them; voci must be on PATH."
    );
    Ok(())
}

pub fn setup(shell: Option<Shell>) -> Result<()> {
    setup_at(&Locations::from_env()?, shell)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn locations(root: &Path) -> Locations {
        Locations {
            home: root.join("home"),
            config: root.join("configuration with spaces"),
        }
    }

    #[test]
    fn setup_all_is_idempotent_preserves_user_settings_and_updates_scripts() {
        let root = tempfile::tempdir().unwrap();
        let locations = locations(root.path());
        fs::create_dir_all(&locations.home).unwrap();
        fs::create_dir_all(locations.config.join("nushell")).unwrap();
        let bash = locations.home.join(".bashrc");
        let login = locations.home.join(".profile");
        let nu = locations.config.join("nushell/config.nu");
        fs::write(&bash, "alias ll='ls -l'\n").unwrap();
        fs::write(&login, "export EDITOR=vim\n").unwrap();
        fs::write(&nu, "$env.config.show_banner = false\n").unwrap();
        setup_at(&locations, None).unwrap();
        assert!(!locations.home.join(".bash_profile").exists());
        assert!(read(&bash).unwrap().starts_with("alias ll='ls -l'\n"));
        assert!(read(&login).unwrap().starts_with("export EDITOR=vim\n"));
        assert!(
            read(&nu)
                .unwrap()
                .starts_with("$env.config.show_banner = false\n")
        );
        let files = plan(&locations, None).unwrap();
        let modified: Vec<_> = files
            .iter()
            .map(|(path, _)| fs::metadata(path).unwrap().modified().unwrap())
            .collect();
        setup_at(&locations, None).unwrap();
        for ((path, contents), before) in files.iter().zip(modified) {
            assert_eq!(&read(path).unwrap(), contents);
            assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), before);
        }
        for path in [&bash, &login, &nu] {
            assert_eq!(read(path).unwrap().matches(BEGIN).count(), 1);
        }
        let script = locations.config.join("voci/completions/voci.nu");
        fs::write(&script, "old generated script").unwrap();
        setup_at(&locations, None).unwrap();
        assert_eq!(
            read(&script).unwrap(),
            include_str!("../../assets/completions/voci.nu")
        );
    }

    #[test]
    fn selected_shell_does_not_create_the_other_shells_files() {
        for shell in [Shell::Bash, Shell::Nushell] {
            let root = tempfile::tempdir().unwrap();
            let locations = locations(root.path());
            setup_at(&locations, Some(shell)).unwrap();
            assert_eq!(
                locations.home.join(".bashrc").exists(),
                shell == Shell::Bash
            );
            assert_eq!(
                locations.config.join("nushell/config.nu").exists(),
                shell == Shell::Nushell
            );
        }
    }

    #[test]
    fn managed_blocks_collapse_duplicates_and_preserve_crlf() {
        let block = format!("{BEGIN}\r\nold source\r\n{END}\r\n");
        let original = format!("before\r\n{block}middle\r\n{block}after\r\n");
        let updated = source_block(&original, "new\nsource").unwrap();
        assert!(updated.starts_with("before\r\nmiddle\r\nafter\r\n"));
        assert_eq!(updated.matches(BEGIN).count(), 1);
        assert_eq!(source_block(&updated, "new\nsource").unwrap(), updated);
    }

    #[test]
    fn malformed_blocks_fail_before_any_files_are_changed() {
        let root = tempfile::tempdir().unwrap();
        let locations = locations(root.path());
        fs::create_dir_all(locations.config.join("nushell")).unwrap();
        let config = locations.config.join("nushell/config.nu");
        let invalid = format!("my settings\n{BEGIN}\nmissing end");
        fs::write(&config, &invalid).unwrap();
        assert!(setup_at(&locations, None).is_err());
        assert_eq!(read(&config).unwrap(), invalid);
        assert!(!locations.home.exists());
        assert!(!locations.config.join("voci").exists());
    }

    #[test]
    fn config_location_honors_xdg_and_rejects_relative_paths() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let config = root.path().join("custom");
        assert_eq!(
            Locations::resolve(Some(home.clone()), Some(config.clone()), None)
                .unwrap()
                .config,
            config
        );
        assert!(Locations::resolve(Some(home), Some("relative".into()), None).is_err());
        assert!(Locations::resolve(None, None, None).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn setup_preserves_dotfile_symlinks_and_permissions() {
        use std::os::unix::{fs::PermissionsExt, fs::symlink};
        let root = tempfile::tempdir().unwrap();
        let locations = locations(root.path());
        fs::create_dir_all(&locations.home).unwrap();
        let target = root.path().join("dotfile");
        fs::write(&target, "# user settings\n").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        let link = locations.home.join(".bashrc");
        symlink(&target, &link).unwrap();
        setup_at(&locations, Some(Shell::Bash)).unwrap();
        assert!(link.is_symlink());
        assert!(read(&target).unwrap().contains(BEGIN));
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o640
        );
        setup_at(&locations, Some(Shell::Bash)).unwrap();
        assert_eq!(read(&target).unwrap().matches(BEGIN).count(), 1);
    }
}
