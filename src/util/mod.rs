mod cargo;

pub use self::cargo::CargoCommand;
use regex::Regex;
use std::{
    env,
    ffi::OsStr,
    fmt,
    fs::File,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Output, Stdio},
};

pub fn read_str(path: impl AsRef<OsStr>) -> io::Result<String> {
    File::open(path.as_ref()).and_then(|mut file| {
        let mut buf = String::new();
        file.read_to_string(&mut buf).map(|_| buf)
    })
}

pub fn has_match(re: &Regex, body: &str, pattern: &str) -> bool {
    re.captures(body)
        .and_then(|caps| {
            caps.iter()
                .find(|cap| cap.map(|cap| cap.as_str() == pattern).unwrap_or_default())
        })
        .is_some()
}

// yay for bad string ergonomics
// https://github.com/rust-lang/rust/issues/42671
pub trait FriendlyContains<T> {
    fn friendly_contains(&self, value: impl PartialEq<T>) -> bool;
}

impl<T> FriendlyContains<T> for Vec<T> {
    fn friendly_contains(&self, value: impl PartialEq<T>) -> bool {
        self.iter().any(|item| value == *item)
    }
}

pub fn add_to_path(path: impl fmt::Display) -> String {
    format!("{}:{}", path, env::var("PATH").unwrap())
}

#[derive(Debug, derive_more::From)]
pub enum CommandError {
    UnableToSpawn(io::Error),
    NonZeroExitStatus(Option<i32>),
}

pub type CommandResult<T> = Result<T, CommandError>;

pub trait IntoResult<T, E> {
    fn into_result(self) -> Result<T, E>;
}

impl IntoResult<(), ()> for bool {
    fn into_result(self) -> Result<(), ()> {
        if self {
            Ok(())
        } else {
            Err(())
        }
    }
}

impl IntoResult<(), CommandError> for ExitStatus {
    fn into_result(self) -> CommandResult<()> {
        self.success().into_result().map_err(|_| self.code().into())
    }
}

impl IntoResult<(), CommandError> for io::Result<ExitStatus> {
    fn into_result(self) -> CommandResult<()> {
        self.map_err(Into::into).and_then(IntoResult::into_result)
    }
}

impl IntoResult<Output, CommandError> for io::Result<Output> {
    fn into_result(self) -> CommandResult<Output> {
        self.map_err(Into::into)
            .and_then(|output| output.status.into_result().map(|_| output))
    }
}

impl IntoResult<Child, CommandError> for io::Result<Child> {
    fn into_result(self) -> CommandResult<Child> {
        self.map_err(Into::into)
    }
}

pub fn force_symlink(src: impl AsRef<OsStr>, dest: impl AsRef<OsStr>) -> CommandResult<()> {
    Command::new("ln")
        .arg("-sf") // always recreate symlink
        .arg(src)
        .arg(dest)
        .status()
        .into_result()
}

fn common_root(abs_src: &Path, abs_dest: &Path) -> PathBuf {
    let mut dest_root = abs_dest.to_owned();
    loop {
        if abs_src.starts_with(&dest_root) {
            return dest_root;
        } else {
            if !dest_root.pop() {
                unreachable!("`abs_src` and `abs_dest` have no common root");
            }
        }
    }
}

pub fn relativize_path(abs_path: impl AsRef<Path>, abs_relative_to: impl AsRef<Path>) -> PathBuf {
    let (abs_path, abs_relative_to) = (abs_path.as_ref(), abs_relative_to.as_ref());
    assert!(abs_path.is_absolute());
    assert!(abs_relative_to.is_absolute());
    let (path, relative_to) = {
        let common_root = common_root(abs_path, abs_relative_to);
        let path = abs_path.strip_prefix(&common_root).unwrap();
        let relative_to = abs_relative_to.strip_prefix(&common_root).unwrap();
        (path, relative_to)
    };
    let mut rel_path = PathBuf::new();
    for _ in 0..relative_to.iter().count() {
        rel_path.push("..");
    }
    let rel_path = rel_path.join(path);
    log::info!("translated {:?} to {:?}", abs_path, rel_path);
    rel_path
}

pub fn relative_symlink(
    abs_src: impl AsRef<Path>,
    abs_dest: impl AsRef<Path>,
) -> CommandResult<()> {
    let rel_src = relativize_path(abs_src, &abs_dest);
    force_symlink(rel_src, abs_dest.as_ref())
}

pub fn git(dir: &impl AsRef<Path>, args: &[&str]) -> CommandResult<()> {
    Command::new("git")
        .arg("-C")
        .arg(dir.as_ref())
        .args(args)
        .status()
        .into_result()
}

pub fn rustup_add(triple: &str) -> CommandResult<()> {
    Command::new("rustup")
        .args(&["target", "add", triple])
        .status()
        .into_result()
}

#[derive(Debug, derive_more::From)]
pub enum PipeError {
    TxCommandError(CommandError),
    RxCommandError(CommandError),
    PipeError(io::Error),
}

pub fn pipe(mut tx_command: Command, mut rx_command: Command) -> Result<(), PipeError> {
    let tx_output = tx_command
        .output()
        .into_result()
        .map_err(PipeError::TxCommandError)?;
    let rx_command = rx_command
        .stdin(Stdio::piped())
        .spawn()
        .into_result()
        .map_err(PipeError::RxCommandError)?;
    rx_command
        .stdin
        .unwrap()
        .write_all(&tx_output.stdout)
        .map_err(From::from)
}
