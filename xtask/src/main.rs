mod args;
mod completions;

use anyhow::{Context, Result, ensure};
use args::{Cli, Common, Profile, Task};
use clap::Parser;
use flate2::{Compression, write::GzEncoder};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    env,
    ffi::OsString,
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
};
use zip::{ZipWriter, write::SimpleFileOptions};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct VersionInfo {
    version: String,
    commit: String,
}

#[derive(Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    #[serde(flatten)]
    info: VersionInfo,
    target: String,
    profile: String,
    sha256: String,
}

#[derive(Debug)]
struct ChildFailure(ExitStatus);
impl std::fmt::Display for ChildFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "command failed with {}", self.0)
    }
}
impl std::error::Error for ChildFailure {}

fn run(command: &mut Command) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("start {command:?}"))?;
    if !status.success() {
        return Err(ChildFailure(status).into());
    }
    Ok(())
}

fn output(command: &mut Command) -> Result<String> {
    let result = command
        .stderr(Stdio::inherit())
        .output()
        .with_context(|| format!("start {command:?}"))?;
    if !result.status.success() {
        return Err(ChildFailure(result.status).into());
    }
    Ok(String::from_utf8(result.stdout)?.trim().to_owned())
}

fn gitversion_command(root: &Path) -> Command {
    let mut command = Command::new("gitversion");
    // The full checkout already exists. Never fetch or normalize local refs.
    command
        .current_dir(root)
        .arg(root)
        .args(["-output", "json", "-nofetch", "-nonormalize"]);
    command
}

fn parse_version(json: &str, commit: String, expected_tag: Option<&str>) -> Result<VersionInfo> {
    #[derive(Deserialize)]
    struct GitVersion {
        #[serde(rename = "SemVer")]
        semver: String,
    }
    let version = serde_json::from_str::<GitVersion>(json)?.semver;
    let parsed = semver::Version::parse(&version).context("GitVersion returned invalid SemVer")?;
    ensure!(
        parsed.build.is_empty(),
        "Expected SemVer without build metadata from GitVersion"
    );
    if let Some(tag) = expected_tag {
        ensure!(
            tag == format!("v{version}"),
            "Release tag {tag:?} does not match GitVersion v{version}"
        );
    }
    Ok(VersionInfo { version, commit })
}

fn version_info(root: &Path, expected_tag: Option<&str>) -> Result<VersionInfo> {
    let shallow = output(
        Command::new("git")
            .current_dir(root)
            .args(["rev-parse", "--is-shallow-repository"]),
    )?;
    ensure!(
        shallow != "true",
        "GitVersion needs full history and tags; run git fetch --unshallow --tags."
    );
    let json = output(&mut gitversion_command(root))?;
    let commit = output(
        Command::new("git")
            .current_dir(root)
            .args(["rev-parse", "HEAD"]),
    )?;
    parse_version(&json, commit, expected_tag)
}

struct Options {
    target: String,
    profile: Profile,
    target_dir: PathBuf,
    offline: bool,
    jobs: Option<std::num::NonZeroUsize>,
}

impl Options {
    fn resolve(common: Common, default_profile: Profile, root: &Path) -> Result<Self> {
        let target = match common.target {
            Some(target) => target,
            None => output(Command::new("rustc").arg("-vV"))?
                .lines()
                .find_map(|line| line.strip_prefix("host: "))
                .context("rustc did not report a host target")?
                .to_owned(),
        };
        ensure!(
            target.contains('-')
                && target.split('-').all(|part| !part.is_empty()
                    && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')),
            "Use a Rust target triple for --target, not a custom target path"
        );
        let directory = common
            .target_dir
            .or_else(|| env::var_os("CARGO_TARGET_DIR").map(PathBuf::from))
            .unwrap_or_else(|| root.join("target"));
        Ok(Self {
            target,
            profile: common.profile.unwrap_or(default_profile),
            target_dir: std::path::absolute(directory)?,
            offline: common.offline,
            jobs: common.jobs,
        })
    }

    fn cargo(&self, root: &Path, task: &str, info: &VersionInfo) -> Command {
        let mut command = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
        command
            .current_dir(root)
            .env("VOCI_BUILD_VERSION", &info.version)
            .args([
                task,
                "--locked",
                "--profile",
                self.profile.name(),
                "--target",
                &self.target,
            ])
            .arg("--target-dir")
            .arg(&self.target_dir);
        if task != "install" {
            command.args(["--package", "voci"]);
        }
        if self.offline {
            command.arg("--offline");
        }
        if let Some(jobs) = self.jobs {
            command.args(["--jobs", &jobs.to_string()]);
        }
        command
    }

    fn binary(&self) -> PathBuf {
        let profile = match self.profile {
            Profile::Dev => "debug",
            Profile::Release => "release",
        };
        self.target_dir
            .join(&self.target)
            .join(profile)
            .join(if self.target.contains("windows") {
                "voci.exe"
            } else {
                "voci"
            })
    }

    fn receipt(&self, info: &VersionInfo) -> Result<Receipt> {
        Ok(Receipt {
            info: info.clone(),
            target: self.target.clone(),
            profile: self.profile.name().into(),
            sha256: digest(&self.binary())?,
        })
    }
}

fn receipt_path(binary: &Path) -> PathBuf {
    let mut name = binary.file_name().expect("binary filename").to_os_string();
    name.push(".build.json");
    binary.with_file_name(name)
}

fn digest(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("read {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn build(root: &Path, options: &Options, info: &VersionInfo) -> Result<()> {
    let receipt = receipt_path(&options.binary());
    match fs::remove_file(&receipt) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    run(options.cargo(root, "build", info).args(["--bin", "voci"]))?;
    fs::write(
        receipt,
        serde_json::to_string_pretty(&options.receipt(info)?)? + "\n",
    )?;
    eprintln!(
        "Built voci {}: {}",
        info.version,
        options.binary().display()
    );
    Ok(())
}

fn package(
    root: &Path,
    options: &Options,
    info: &VersionInfo,
    no_build: bool,
    output_dir: &Path,
) -> Result<PathBuf> {
    if !no_build {
        build(root, options, info)?;
    }
    let binary = options.binary();
    let receipt = fs::read(receipt_path(&binary)).context(
        "No verified build found; run mise run build with matching options or omit --no-build",
    )?;
    ensure!(
        serde_json::from_slice::<Receipt>(&receipt)? == options.receipt(info)?,
        "Build metadata or executable does not match this version/commit/target/profile; rebuild before packaging"
    );
    let mut name = format!("voci-v{}-{}", info.version, options.target);
    if matches!(options.profile, Profile::Dev) {
        name.push_str("-dev");
    }
    let output_dir = std::path::absolute(output_dir)?;
    fs::create_dir_all(&output_dir)?;
    let documents = [
        "README.md",
        "LICENSE",
        "docs/pitches/lookup/wikdict-attribution.md",
    ];
    let mut files = vec![
        (
            binary.clone(),
            binary.file_name().unwrap().to_string_lossy().into_owned(),
            0o755,
        ),
        (receipt_path(&binary), "build-info.json".into(), 0o644),
    ];
    files.extend(documents.map(|document| (root.join(document), document.to_owned(), 0o644)));
    let windows = options.target.contains("windows");
    let archive = output_dir.join(format!("{name}.{}", if windows { "zip" } else { "tar.gz" }));
    // Finish in a temporary file so failed packaging cannot replace a good archive.
    let mut temporary = tempfile::NamedTempFile::new_in(&output_dir)?;
    if windows {
        let mut zip = ZipWriter::new(temporary.as_file_mut());
        for (source, relative, mode) in &files {
            zip.start_file(
                format!("{name}/{relative}"),
                SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated)
                    .unix_permissions(*mode),
            )?;
            io::copy(&mut File::open(source)?, &mut zip)?;
        }
        zip.finish()?;
    } else {
        let gzip = GzEncoder::new(temporary.as_file_mut(), Compression::default());
        let mut tar = tar::Builder::new(gzip);
        for (source, relative, mode) in &files {
            let mut file = File::open(source)?;
            let mut header = tar::Header::new_gnu();
            header.set_size(file.metadata()?.len());
            header.set_mode(*mode);
            header.set_cksum();
            tar.append_data(&mut header, format!("{name}/{relative}"), &mut file)?;
        }
        tar.into_inner()?.finish()?;
    }
    temporary.persist(&archive)?;
    let archive_name = archive.file_name().unwrap().to_string_lossy();
    fs::write(
        output_dir.join(format!("{archive_name}.sha256")),
        format!("{}  {archive_name}\n", digest(&archive)?),
    )?;
    Ok(archive)
}

fn forwarded(command: &mut Command, arguments: &[OsString]) {
    if !arguments.is_empty() {
        command.arg("--").args(arguments);
    }
}

fn execute(cli: Cli, root: &Path) -> Result<()> {
    if let Task::SetupCompletions { shell } = cli.task {
        return completions::setup(shell);
    }
    if let Task::Version { expect_tag } = cli.task {
        println!("{}", version_info(root, expect_tag.as_deref())?.version);
        return Ok(());
    }
    let info = version_info(root, None)?;
    match cli.task {
        Task::Build(common) => build(
            root,
            &Options::resolve(common, Profile::Release, root)?,
            &info,
        )?,
        Task::Package {
            common,
            no_build,
            output_dir,
        } => {
            let archive = package(
                root,
                &Options::resolve(common, Profile::Release, root)?,
                &info,
                no_build,
                &output_dir,
            )?;
            println!("{}", archive.display());
        }
        Task::Run { common, args, .. } => {
            let options = Options::resolve(common, Profile::Dev, root)?;
            let mut command = options.cargo(root, "run", &info);
            command.args(["--bin", "voci"]);
            forwarded(&mut command, &args);
            run(&mut command)?;
        }
        Task::Test {
            common,
            filter,
            test,
            lib,
            args,
            ..
        } => {
            let options = Options::resolve(common, Profile::Dev, root)?;
            let mut command = options.cargo(root, "test", &info);
            if let Some(test) = test {
                command.args(["--test", &test]);
            }
            if lib {
                command.arg("--lib");
            }
            if let Some(filter) = filter {
                command.arg(filter);
            }
            forwarded(&mut command, &args);
            run(&mut command)?;
        }
        Task::Install {
            common,
            root: install_root,
        } => {
            let options = Options::resolve(common, Profile::Release, root)?;
            let mut command = options.cargo(root, "install", &info);
            command
                .arg("--path")
                .arg(root)
                .args(["--bin", "voci", "--force"]);
            if let Some(path) = install_root {
                command.arg("--root").arg(path);
            }
            run(&mut command)?;
            completions::setup(None)?;
        }
        Task::Version { .. } | Task::SetupCompletions { .. } => unreachable!(),
    }
    Ok(())
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    if let Err(error) = execute(Cli::parse(), root) {
        eprintln!("voci task: {error:#}");
        let code = error
            .downcast_ref::<ChildFailure>()
            .map_or(1, |failure| failure.0.code().unwrap_or(130));
        std::process::exit(code);
    }
}

#[cfg(test)]
mod tests;
