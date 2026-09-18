use super::*;
use flate2::read::GzDecoder;
use tempfile::TempDir;

fn info() -> VersionInfo {
    VersionInfo {
        version: "1.2.3-beta.4".into(),
        commit: "abc123".into(),
    }
}

fn options(root: &Path, target: &str) -> Options {
    Options {
        target: target.into(),
        profile: Profile::Release,
        target_dir: root.join("target"),
        offline: true,
        jobs: None,
    }
}

fn fixture(root: &Path, options: &Options) {
    let binary = options.binary();
    fs::create_dir_all(binary.parent().unwrap()).unwrap();
    fs::write(&binary, b"executable fixture").unwrap();
    fs::write(
        receipt_path(&binary),
        serde_json::to_vec(&options.receipt(&info()).unwrap()).unwrap(),
    )
    .unwrap();
    let attribution = root.join("docs/pitches/lookup/wikdict-attribution.md");
    fs::create_dir_all(attribution.parent().unwrap()).unwrap();
    fs::write(attribution, "Attribution").unwrap();
    fs::write(root.join("README.md"), "Documentation").unwrap();
}

#[test]
fn version_validation_and_tag_matching() {
    let json = r#"{"SemVer":"2.3.4-beta.1"}"#;
    assert_eq!(
        parse_version(json, "abc".into(), Some("v2.3.4-beta.1"))
            .unwrap()
            .version,
        "2.3.4-beta.1"
    );
    assert!(parse_version(json, "abc".into(), Some("v0.1.0")).is_err());
    for version in ["../oops", "1.2.3\nunsafe", "", "01.2.3", "1.2.3-beta.01"] {
        let json = serde_json::json!({"SemVer":version}).to_string();
        assert!(parse_version(&json, "abc".into(), None).is_err());
    }
}

#[test]
fn argument_forwarding_preserves_flags_spaces_and_separators() {
    for arguments in [
        vec![
            "xtask",
            "run",
            "--offline",
            "--",
            "--from",
            "de",
            "ice cream",
        ],
        vec!["xtask", "run", "--offline", "--from", "de", "ice cream"],
    ] {
        let Task::Run { common, args, .. } = Cli::try_parse_from(arguments).unwrap().task else {
            panic!()
        };
        assert!(common.offline);
        assert_eq!(args, ["--from", "de", "ice cream"].map(OsString::from));
    }
    let Task::Run { args, .. } = Cli::try_parse_from(["xtask", "run", "--help"])
        .unwrap()
        .task
    else {
        panic!()
    };
    assert_eq!(args, ["--help"]);
    let Task::Run { args, .. } = Cli::try_parse_from(["xtask", "run", "--", "--", "shell"])
        .unwrap()
        .task
    else {
        panic!()
    };
    assert_eq!(args, ["--", "shell"]);
    let Task::Test {
        test, filter, args, ..
    } = Cli::try_parse_from([
        "xtask",
        "test",
        "--test",
        "cli",
        "--filter",
        "help",
        "--nocapture",
    ])
    .unwrap()
    .task
    else {
        panic!()
    };
    assert_eq!(test.as_deref(), Some("cli"));
    assert_eq!(filter.as_deref(), Some("help"));
    assert_eq!(args, ["--nocapture"]);
}

#[test]
fn task_help_and_invalid_flags_are_handled_before_building() {
    use clap::error::ErrorKind;
    assert_eq!(
        Cli::try_parse_from(["xtask", "run", "--task-help"])
            .unwrap_err()
            .kind(),
        ErrorKind::DisplayHelp
    );
    assert_eq!(
        Cli::try_parse_from(["xtask", "build", "--help"])
            .unwrap_err()
            .kind(),
        ErrorKind::DisplayHelp
    );
    assert!(Cli::try_parse_from(["xtask", "build", "--misspelled"]).is_err());
    assert!(Cli::try_parse_from(["xtask", "build", "--jobs", "0"]).is_err());
}

#[test]
fn cargo_command_is_locked_versioned_and_selects_only_the_application() {
    let temporary = TempDir::new().unwrap();
    let options = options(temporary.path(), "x86_64-unknown-linux-gnu");
    let command = options.cargo(temporary.path(), "test", &info());
    let args: Vec<_> = command.get_args().collect();
    assert!(args.contains(&std::ffi::OsStr::new("--locked")));
    assert!(args.contains(&std::ffi::OsStr::new("--offline")));
    assert!(args.windows(2).any(|pair| pair == ["--package", "voci"]));
    assert!(
        command
            .get_envs()
            .any(|(key, value)| key == "VOCI_BUILD_VERSION"
                && value == Some("1.2.3-beta.4".as_ref()))
    );
    let install = options.cargo(temporary.path(), "install", &info());
    assert!(!install.get_args().any(|arg| arg == "--package"));
}

#[test]
fn tar_contains_versioned_executable_attribution_and_checksums() {
    let temporary = TempDir::new().unwrap();
    let root = temporary.path();
    let options = options(root, "x86_64-unknown-linux-gnu");
    fixture(root, &options);
    let path = package(root, &options, &info(), true, &root.join("dist")).unwrap();
    let name = "voci-v1.2.3-beta.4-x86_64-unknown-linux-gnu";
    assert_eq!(path.file_name().unwrap(), format!("{name}.tar.gz").as_str());
    let mut archive = tar::Archive::new(GzDecoder::new(File::open(&path).unwrap()));
    let mut names = Vec::new();
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let path = entry.path().unwrap().to_string_lossy().into_owned();
        if path.ends_with("/voci") {
            assert_eq!(entry.header().mode().unwrap(), 0o755);
        }
        if path.ends_with("/build-info.json") {
            let receipt: Receipt = serde_json::from_reader(&mut entry).unwrap();
            assert_eq!(receipt.info, info());
        }
        names.push(path);
    }
    assert!(names.contains(&format!(
        "{name}/docs/pitches/lookup/wikdict-attribution.md"
    )));
    assert_eq!(names.len(), 4);
    let checksum =
        fs::read_to_string(path.with_file_name(format!("{name}.tar.gz.sha256"))).unwrap();
    assert_eq!(
        checksum,
        format!("{}  {name}.tar.gz\n", digest(&path).unwrap())
    );
    // Close the reader before replacing the archive; Windows can deny replacing
    // an open destination file even after we have finished iterating its entries.
    drop(archive);
    // Repackaging replaces the prior archive, including on Windows.
    package(root, &options, &info(), true, &root.join("dist")).unwrap();
}

#[test]
fn windows_archive_is_zip_and_dev_name_is_distinct() {
    let temporary = TempDir::new().unwrap();
    let root = temporary.path();
    let mut options = options(root, "x86_64-pc-windows-msvc");
    options.profile = Profile::Dev;
    fixture(root, &options);
    let path = package(root, &options, &info(), true, &root.join("dist")).unwrap();
    let name = "voci-v1.2.3-beta.4-x86_64-pc-windows-msvc-dev";
    assert_eq!(path.file_name().unwrap(), format!("{name}.zip").as_str());
    let mut zip = zip::ZipArchive::new(File::open(path).unwrap()).unwrap();
    let mut bytes = Vec::new();
    zip.by_name(&format!("{name}/voci.exe"))
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    assert_eq!(bytes, b"executable fixture");
}

#[test]
fn package_rejects_missing_stale_or_modified_builds() {
    let temporary = TempDir::new().unwrap();
    let root = temporary.path();
    let options = options(root, "x86_64-unknown-linux-gnu");
    assert!(package(root, &options, &info(), true, &root.join("dist")).is_err());
    fixture(root, &options);
    for field in ["version", "commit", "target", "profile"] {
        let mut receipt = serde_json::to_value(options.receipt(&info()).unwrap()).unwrap();
        receipt[field] = "stale".into();
        fs::write(
            receipt_path(&options.binary()),
            serde_json::to_vec(&receipt).unwrap(),
        )
        .unwrap();
        assert!(package(root, &options, &info(), true, &root.join("dist")).is_err());
        fixture(root, &options);
    }
    fs::write(options.binary(), "tampered").unwrap();
    assert!(package(root, &options, &info(), true, &root.join("dist")).is_err());
}

#[test]
fn packaging_builds_and_failure_invalidates_receipt_without_editing_manifest() {
    let temporary = TempDir::new().unwrap();
    let root = temporary.path();
    let Task::Build(common) = Cli::try_parse_from(["xtask", "build", "--offline"])
        .unwrap()
        .task
    else {
        panic!()
    };
    let options = Options::resolve(common, Profile::Release, root).unwrap();
    fixture(root, &options);
    let manifest = "[package]\nname = \"voci\"\nversion = \"0.1.0\"\nedition = \"2024\"\n";
    let lock = "version = 4\n[[package]]\nname = \"voci\"\nversion = \"0.1.0\"\n";
    fs::write(root.join("Cargo.toml"), manifest).unwrap();
    fs::write(root.join("Cargo.lock"), lock).unwrap();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(
        root.join("src/main.rs"),
        "fn main() { println!(\"{}\", env!(\"VOCI_BUILD_VERSION\")); }",
    )
    .unwrap();
    package(root, &options, &info(), false, &root.join("dist")).unwrap();
    assert_eq!(
        output(&mut Command::new(options.binary())).unwrap(),
        info().version
    );
    assert_eq!(
        fs::read_to_string(root.join("Cargo.toml")).unwrap(),
        manifest
    );
    assert_eq!(fs::read_to_string(root.join("Cargo.lock")).unwrap(), lock);
    fs::write(
        root.join("src/main.rs"),
        "compile_error!(\"intentional fixture failure\");",
    )
    .unwrap();
    let error = build(root, &options, &info()).unwrap_err();
    assert_eq!(
        error.downcast_ref::<ChildFailure>().unwrap().0.code(),
        Some(101)
    );
    assert!(!receipt_path(&options.binary()).exists());
}

fn git(root: &Path, args: &[&str]) -> String {
    output(
        Command::new("git")
            .current_dir(root)
            .args([
                "-c",
                "user.name=Task tests",
                "-c",
                "user.email=tests@example.invalid",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args),
    )
    .unwrap()
}

fn git_fixture() -> TempDir {
    let temporary = TempDir::new().unwrap();
    let root = temporary.path();
    git(root, &["init", "-b", "main"]);
    fs::write(
        root.join("GitVersion.yml"),
        include_str!("../../GitVersion.yml"),
    )
    .unwrap();
    git(root, &["add", "GitVersion.yml"]);
    git(root, &["commit", "-m", "chore: initialize"]);
    temporary
}

#[test]
fn shallow_checkout_fails_before_invoking_gitversion() {
    let temporary = git_fixture();
    let root = temporary.path();
    fs::write(
        root.join(".git/shallow"),
        git(root, &["rev-parse", "HEAD"]) + "\n",
    )
    .unwrap();
    assert!(
        version_info(root, None)
            .unwrap_err()
            .to_string()
            .contains("full history")
    );
}

#[test]
fn gitversion_conventional_commits_and_detached_tags() {
    if Command::new("gitversion").arg("-version").output().is_err() {
        assert!(
            env::var_os("CI").is_none(),
            "GitVersion must be installed in CI"
        );
        eprintln!("Skipping GitVersion integration; run mise exec -- cargo test --package xtask");
        return;
    }
    let temporary = git_fixture();
    let root = temporary.path();
    let calculate = |expected: Option<&str>, github: bool| {
        let mut command = gitversion_command(root);
        for (key, _) in env::vars_os() {
            let name = key.to_string_lossy();
            if name.starts_with("GITHUB_") || name.starts_with("GIT_") || name == "CI" {
                command.env_remove(key);
            }
        }
        if github {
            command
                .env("GITHUB_ACTIONS", "true")
                .env("GITHUB_REF", "refs/tags/v2.0.0-beta.1")
                .env("GITHUB_SHA", git(root, &["rev-parse", "HEAD"]))
                .env("GITHUB_REPOSITORY", "test/voci")
                .env("GITHUB_WORKSPACE", root)
                .env("GITHUB_RUN_NUMBER", "1");
        }
        parse_version(&output(&mut command).unwrap(), "fixture".into(), expected)
            .unwrap()
            .version
    };
    git(root, &["tag", "v1.2.3"]);
    assert_eq!(calculate(Some("v1.2.3"), false), "1.2.3");
    git(
        root,
        &["commit", "--allow-empty", "-m", "fix: repair lookup"],
    );
    assert!(calculate(None, false).starts_with("1.2.4"));
    git(root, &["commit", "--allow-empty", "-m", "feat: add lookup"]);
    assert!(calculate(None, false).starts_with("1.3.0"));
    git(root, &["tag", "v1.3.0"]);
    git(root, &["checkout", "--detach", "v1.3.0"]);
    assert_eq!(calculate(Some("v1.3.0"), false), "1.3.0");
    git(root, &["checkout", "main"]);
    git(
        root,
        &[
            "commit",
            "--allow-empty",
            "-m",
            "feat!: incompatible lookup",
        ],
    );
    assert!(calculate(None, false).starts_with("2.0.0"));
    git(root, &["tag", "v2.0.0-beta.1"]);
    git(root, &["checkout", "--detach", "v2.0.0-beta.1"]);
    assert_eq!(calculate(Some("v2.0.0-beta.1"), false), "2.0.0-beta.1");
    assert_eq!(calculate(Some("v2.0.0-beta.1"), true), "2.0.0-beta.1");
}
