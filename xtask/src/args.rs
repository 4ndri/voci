use clap::{Args, Parser, Subcommand, ValueEnum};
use std::{ffi::OsString, num::NonZeroUsize, path::PathBuf};

#[derive(Debug, Parser)]
#[command(
    about = "Build and package voci using GitVersion",
    name = "cargo xtask"
)]
pub struct Cli {
    #[command(subcommand)]
    pub task: Task,
}

#[derive(Debug, Subcommand)]
pub enum Task {
    /// Set up Tab completion for all supported shells, or one selected shell
    #[command(name = "setup:completions")]
    SetupCompletions {
        #[arg(long, value_enum)]
        shell: Option<crate::completions::Shell>,
    },
    /// Print GitVersion SemVer
    Version {
        /// Require this tag to equal v plus GitVersion SemVer
        #[arg(long)]
        expect_tag: Option<String>,
    },
    /// Build a versioned executable (release by default)
    Build(Common),
    /// Build and package an executable with a SHA-256 checksum
    Package {
        #[command(flatten)]
        common: Common,
        /// Reuse a verified build with matching version, commit, target and profile
        #[arg(long)]
        no_build: bool,
        #[arg(long, default_value = "dist")]
        output_dir: PathBuf,
    },
    /// Run voci; options after the task options are forwarded to the app
    #[command(disable_help_flag = true)]
    Run {
        #[command(flatten)]
        common: Common,
        #[arg(long, action = clap::ArgAction::Help)]
        task_help: Option<bool>,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<OsString>,
    },
    /// Run Rust tests; trailing options are forwarded to the test harness
    #[command(disable_help_flag = true)]
    Test {
        #[command(flatten)]
        common: Common,
        #[arg(long, action = clap::ArgAction::Help)]
        task_help: Option<bool>,
        #[arg(long)]
        filter: Option<String>,
        /// Run a single integration-test target
        #[arg(long)]
        test: Option<String>,
        #[arg(long)]
        lib: bool,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<OsString>,
    },
    /// Install or update voci and shell completions (release by default)
    Install {
        #[command(flatten)]
        common: Common,
        /// Cargo installation root (default: Cargo configuration)
        #[arg(long)]
        root: Option<PathBuf>,
    },
}

#[derive(Debug, Args)]
pub struct Common {
    /// Rust target triple (default: rustc host)
    #[arg(long)]
    pub target: Option<String>,
    /// Default: dev for run/test; release otherwise
    #[arg(long, value_enum)]
    pub profile: Option<Profile>,
    /// Cargo output directory (default: CARGO_TARGET_DIR or target/)
    #[arg(long)]
    pub target_dir: Option<PathBuf>,
    /// Disable Cargo network access
    #[arg(long)]
    pub offline: bool,
    #[arg(short, long)]
    pub jobs: Option<NonZeroUsize>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Profile {
    Dev,
    Release,
}

impl Profile {
    pub fn name(self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Release => "release",
        }
    }
}
