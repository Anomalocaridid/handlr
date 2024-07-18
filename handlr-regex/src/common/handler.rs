use crate::{
    common::{DesktopEntry, ExecMode, UserPath},
    config::Config,
    error::{Error, ErrorKind, Result},
};
use derive_more::Deref;
use enum_dispatch::enum_dispatch;
use regex::RegexSet;
use serde::{Deserialize, Serialize};
use serde_regex;
use std::{
    convert::TryFrom,
    ffi::OsString,
    fmt::Display,
    hash::{Hash, Hasher},
    path::PathBuf,
    str::FromStr,
};

/// Represents a program or command that is used to open a file
#[derive(PartialEq, Eq, Hash)]
#[enum_dispatch(Handleable)]
pub enum Handler {
    DesktopHandler,
    RegexHandler,
}

/// Trait providing common functionality for handlers
#[enum_dispatch]
pub trait Handleable {
    /// Get the desktop entry associated with the handler
    fn get_entry(&self) -> Result<DesktopEntry>;
    /// Open the given paths with the handler
    fn open(
        &self,
        config: &mut Config,
        args: Vec<String>,
        selector: &str,
        enable_selector: bool,
    ) -> Result<()> {
        self.get_entry()?.exec(
            config,
            ExecMode::Open,
            args,
            selector,
            enable_selector,
        )
    }
}

/// Represents a handler defined in a desktop file
#[derive(
    Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize,
)]
pub struct DesktopHandler(OsString);

impl Display for DesktopHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0.to_string_lossy())
    }
}

impl FromStr for DesktopHandler {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        DesktopHandler::resolve(s.into())
    }
}

impl Handleable for DesktopHandler {
    fn get_entry(&self) -> Result<DesktopEntry> {
        DesktopEntry::try_from(Self::get_path(&self.0)?)
    }
}

impl DesktopHandler {
    /// Create a DesktopHandler, skipping validity checks
    pub fn assume_valid(name: OsString) -> Self {
        Self(name)
    }

    /// Get the path of a given desktop entry file
    pub fn get_path(name: &std::ffi::OsStr) -> Result<PathBuf> {
        let mut path = PathBuf::from("applications");
        path.push(name);
        Ok(xdg::BaseDirectories::new()?
            .find_data_file(path)
            .ok_or_else(|| {
                ErrorKind::NotFound(name.to_string_lossy().into())
            })?)
    }

    /// Check a a desktop entry exists and if so, return a Desktop Handler
    pub fn resolve(name: OsString) -> Result<Self> {
        let path = Self::get_path(&name)?;
        DesktopEntry::try_from(path)?;
        Ok(Self(name))
    }

    /// Launch a DesktopHandler's desktop entry
    pub fn launch(
        &self,
        config: &mut Config,
        args: Vec<String>,
        selector: &str,
        enable_selector: bool,
    ) -> Result<()> {
        self.get_entry()?.exec(
            config,
            ExecMode::Launch,
            args,
            selector,
            enable_selector,
        )
    }
}

/// Represents a regex handler from the config
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
pub struct RegexHandler {
    exec: String,
    #[serde(default)]
    terminal: bool,
    regexes: HandlerRegexSet,
}

impl RegexHandler {
    /// Test if a given path matches the handler's regex
    fn is_match(&self, path: &str) -> bool {
        self.regexes.is_match(path)
    }
}

impl Handleable for RegexHandler {
    fn get_entry(&self) -> Result<DesktopEntry> {
        Ok(DesktopEntry::fake_entry(&self.exec, self.terminal))
    }
}

// Wrapping RegexSet in a struct and implementing Eq and Hash for it
// saves us from having to implement them for RegexHandler as a whole.
#[derive(Debug, Clone, Deserialize, Deref)]
struct HandlerRegexSet(#[serde(with = "serde_regex")] RegexSet);

impl PartialEq for HandlerRegexSet {
    fn eq(&self, other: &Self) -> bool {
        self.patterns() == other.patterns()
    }
}

impl Eq for HandlerRegexSet {}

impl Hash for HandlerRegexSet {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.patterns().hash(state);
    }
}

/// A collection of all of the defined RegexHandlers
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RegexApps(Vec<RegexHandler>);

impl RegexApps {
    /// Get a handler matching a given path
    pub fn get_handler(&self, path: &UserPath) -> Result<RegexHandler> {
        Ok(self
            .0
            .iter()
            .find(|app| app.is_match(&path.to_string()))
            .ok_or_else(|| ErrorKind::NotFound(path.to_string()))?
            .clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::DesktopEntry;
    use url::Url;

    #[test]
    fn regex_handlers() -> Result<()> {
        let exec: &str = "freetube %u";
        let regexes: &[String] =
            &[String::from(r"(https://)?(www\.)?youtu(be\.com|\.be)/*")];

        let regex_handler = RegexHandler {
            exec: String::from(exec),
            terminal: false,
            regexes: HandlerRegexSet(RegexSet::new(regexes)?),
        };

        let regex_apps = RegexApps(vec![regex_handler.clone()]);

        assert_eq!(
            regex_apps
                .get_handler(&UserPath::Url(Url::parse(
                    "https://youtu.be/dQw4w9WgXcQ"
                )?))?
                .get_entry()?,
            DesktopEntry::fake_entry(exec, false)
        );

        assert!(regex_apps
            .get_handler(&UserPath::Url(Url::parse(
                "https://en.wikipedia.org",
            )?))
            .is_err());

        Ok(())
    }
}
