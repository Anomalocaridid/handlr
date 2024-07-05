use crate::{
    common::{DesktopEntry, ExecMode},
    Config, Error, ErrorKind, MimeApps, Result, SystemApps, UserPath,
};
use derive_more::Deref;
use enum_dispatch::enum_dispatch;
use regex::RegexSet;
use serde::{Deserialize, Serialize};
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
        config: &Config,
        mime_apps: &mut MimeApps,
        system_apps: &SystemApps,
        args: Vec<String>,
    ) -> Result<()> {
        self.get_entry()?.exec(
            config,
            mime_apps,
            system_apps,
            ExecMode::Open,
            args,
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
        DesktopEntry::try_from(Self::get_path(&self.0).unwrap())
    }
}

impl DesktopHandler {
    pub fn assume_valid(name: OsString) -> Self {
        Self(name)
    }
    pub fn get_path(name: &std::ffi::OsStr) -> Option<PathBuf> {
        let mut path = PathBuf::from("applications");
        path.push(name);
        xdg::BaseDirectories::new().ok()?.find_data_file(path)
    }
    pub fn resolve(name: OsString) -> Result<Self> {
        let path = Self::get_path(&name).ok_or_else(|| {
            ErrorKind::NotFound(name.to_string_lossy().into())
        })?;
        DesktopEntry::try_from(path)?;
        Ok(Self(name))
    }
    pub fn launch(
        &self,
        config: &Config,
        mime_apps: &mut MimeApps,
        system_apps: &SystemApps,
        args: Vec<String>,
    ) -> Result<()> {
        self.get_entry()?.exec(
            config,
            mime_apps,
            system_apps,
            ExecMode::Launch,
            args,
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
    use url::Url;

    #[test]
    fn regex_handlers() -> Result<()> {
        let exec: &str = "freetube %u";
        let regexes: &[String] =
            &[String::from(r"(https://)?(www\.)?youtu(be\.com|\.be)/*")];

        let regex_handler = RegexHandler {
            exec: String::from(exec),
            terminal: false,
            regexes: HandlerRegexSet(
                RegexSet::new(regexes).expect("Test regex is invalid"),
            ),
        };

        let regex_apps = RegexApps(vec![regex_handler.clone()]);

        assert_eq!(
            regex_apps
                .get_handler(&UserPath::Url(
                    Url::parse("https://youtu.be/dQw4w9WgXcQ").unwrap()
                ))
                .expect("RegexApps::get_handler() returned Err"),
            regex_handler
        );

        assert!(regex_apps
            .get_handler(&UserPath::Url(
                Url::parse("https://en.wikipedia.org").unwrap()
            ))
            .is_err());

        Ok(())
    }
}
