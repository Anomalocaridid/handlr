use crate::{
    common::{DesktopHandler, Handleable},
    error::{Error, ErrorKind, Result},
};
use derive_more::{Deref, DerefMut};
use itertools::Itertools;
use mime::Mime;
use serde::{Deserialize, Serialize};
use serde_with::{
    serde_as, DeserializeFromStr, DisplayFromStr, SerializeDisplay,
};
use std::{
    collections::{HashMap, VecDeque},
    fmt::Display,
    path::PathBuf,
    str::FromStr,
};

/// Helper struct for a list of `DesktopHandler`s
#[serde_as]
#[derive(
    Debug, Default, Clone, Deref, DerefMut, SerializeDisplay, DeserializeFromStr,
)]
pub struct DesktopList(VecDeque<DesktopHandler>);

impl FromStr for DesktopList {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(
            s.split(';')
                .filter(|s| !s.is_empty()) // Account for ending semicolon
                .unique()
                .filter_map(|s| DesktopHandler::from_str(s).ok())
                .collect::<VecDeque<DesktopHandler>>(),
        ))
    }
}

/// Represents user-configured mimeapps.list file
#[serde_as]
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct MimeApps {
    #[serde(rename = "Added Associations")]
    #[serde_as(as = "HashMap<DisplayFromStr, _>")]
    pub(crate) added_associations: HashMap<Mime, DesktopList>,
    #[serde(rename = "Default Applications")]
    #[serde_as(as = "HashMap<DisplayFromStr, _>")]
    pub(crate) default_apps: HashMap<Mime, DesktopList>,
}

impl Display for DesktopList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Ensure final semicolon is added
        write!(f, "{};", self.iter().join(";"))
    }
}

impl MimeApps {
    /// Add a handler to an existing default application association
    pub fn add_handler(&mut self, mime: &Mime, handler: &DesktopHandler) {
        self.default_apps
            .entry(mime.clone())
            .or_default()
            .push_back(handler.clone());
    }

    /// Set a default application association, overwriting any existing association for the same mimetype
    pub fn set_handler(&mut self, mime: &Mime, handler: &DesktopHandler) {
        self.default_apps
            .insert(mime.clone(), DesktopList(vec![handler.clone()].into()));
    }

    /// Entirely remove a given mime's default application association
    pub fn unset_handler(&mut self, mime: &Mime) -> Option<DesktopList> {
        self.default_apps.remove(mime)
    }

    /// Remove a given handler from a given mime's default file associaion
    pub fn remove_handler(
        &mut self,
        mime: &Mime,
        handler: &DesktopHandler,
    ) -> Option<DesktopHandler> {
        let handler_list = self.default_apps.entry(mime.clone()).or_default();

        handler_list
            .iter()
            .position(|x| *x == *handler)
            .and_then(|pos| handler_list.remove(pos))
    }

    /// Get a list of handlers associated with a wildcard mime
    fn get_from_wildcard(&self, mime: &Mime) -> Option<&DesktopList> {
        // Get the handlers that wildcard match the given mime
        let associations = self.default_apps.iter().filter(|(m, _)| {
            wildmatch::WildMatch::new(m.as_ref()).matches(mime.as_ref())
        });

        // Get the length of the longest wildcard that matches
        // Assuming the longest match is the best match
        // Inspired by how globs are handled in xdg spec
        let biggest_wildcard_len = associations
            .clone()
            .map(|(ref m, _)| m.as_ref().len())
            .max()?;

        // Keep only the lists of handlers from associations with the longest wildcards
        // And get the first one, assuming it takes precedence
        // Loosely inspired by how globs are handled in xdg spec
        associations
            .filter(|(ref m, _)| m.as_ref().len() == biggest_wildcard_len)
            .map(|(_, handlers)| handlers)
            .collect_vec()
            .first()
            .cloned()
    }

    /// Get the handler associated with a given mime from mimeapps.list's default apps
    pub(crate) fn get_handler_from_user(
        &self,
        mime: &Mime,
        selector: &str,
        use_selector: bool,
    ) -> Result<DesktopHandler> {
        let error = Error::from(ErrorKind::NotFound(mime.to_string()));
        // Check for an exact match first and then fall back to wildcard
        match self
            .default_apps
            .get(mime)
            .or_else(|| self.get_from_wildcard(mime))
        {
            Some(handlers) if use_selector && handlers.len() > 1 => {
                let handlers = handlers
                    .iter()
                    .map(|h| Ok((h, h.get_entry()?.name)))
                    .collect::<Result<Vec<_>>>()?;

                let handler = {
                    let name =
                        select(selector, handlers.iter().map(|h| h.1.clone()))?;

                    handlers
                        .into_iter()
                        .find(|h| h.1 == name)
                        .ok_or(error)?
                        .0
                        .clone()
                };

                Ok(handler)
            }
            Some(handlers) => Ok(handlers.front().ok_or(error)?.clone()),
            None => Err(error),
        }
    }

    /// Get the path to the user's mimeapps.list file
    fn path() -> Result<PathBuf> {
        let mut config = xdg::BaseDirectories::new()?.get_config_home();
        config.push("mimeapps.list");
        Ok(config)
    }

    /// Read and parse mimeapps.list
    pub fn read() -> Result<Self> {
        let exists = std::path::Path::new(&Self::path()?).exists();

        let file = std::fs::OpenOptions::new()
            .write(!exists)
            .create(!exists)
            .read(true)
            .open(Self::path()?)?;

        let mut mimeapps: Self = serde_ini::de::from_read(file)?;

        // Remove empty default associations
        // Can happen if all handlers set are invalid (e.g. do not exist)
        mimeapps.default_apps.retain(|_, h| !h.is_empty());

        Ok(mimeapps)
    }

    /// Save associations to mimeapps.list
    pub fn save(&self) -> Result<()> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .create(true)
            .write(true)
            .truncate(true)
            .open(Self::path()?)?;

        serde_ini::ser::to_writer(file, self)?;

        Ok(())
    }
}

/// Run given selector command
fn select<O: Iterator<Item = String>>(
    selector: &str,
    mut opts: O,
) -> Result<String> {
    use std::{
        io::prelude::*,
        process::{Command, Stdio},
    };

    let process = {
        let mut split = shlex::split(selector).ok_or_else(|| {
            Error::from(ErrorKind::BadCmd(selector.to_string()))
        })?;
        let (cmd, args) = (split.remove(0), split);
        Command::new(cmd)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?
    };

    let output = {
        process
            .stdin
            .ok_or_else(|| ErrorKind::Selector(selector.to_string()))?
            .write_all(opts.join("\n").as_bytes())?;

        let mut output = String::with_capacity(24);

        process
            .stdout
            .ok_or_else(|| ErrorKind::Selector(selector.to_string()))?
            .read_to_string(&mut output)?;

        output.trim_end().to_owned()
    };

    if output.is_empty() {
        Err(Error::from(ErrorKind::Cancelled))
    } else {
        Ok(output)
    }
}
