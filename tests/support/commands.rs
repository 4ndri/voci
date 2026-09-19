use assert_cmd::{Command, cargo::cargo_bin_cmd};
use std::ops::{Deref, DerefMut};
pub struct IsolatedCommand {
    command: Command,
    _home: tempfile::TempDir,
}
impl Deref for IsolatedCommand {
    type Target = Command;
    fn deref(&self) -> &Command {
        &self.command
    }
}
impl DerefMut for IsolatedCommand {
    fn deref_mut(&mut self) -> &mut Command {
        &mut self.command
    }
}
pub fn command() -> IsolatedCommand {
    let home = tempfile::tempdir().unwrap();
    let mut command = cargo_bin_cmd!("voci");
    isolate(&mut command, home.path());
    IsolatedCommand {
        command,
        _home: home,
    }
}
pub fn isolate(command: &mut Command, home: &std::path::Path) {
    for key in [
        "HOME",
        "USERPROFILE",
        "XDG_DATA_HOME",
        "XDG_CONFIG_HOME",
        "LOCALAPPDATA",
        "APPDATA",
    ] {
        command.env(key, home);
    }
    command
        .env_remove("VOCI_MICROSOFT_KEY")
        .env_remove("VOCI_MICROSOFT_REGION");
}
