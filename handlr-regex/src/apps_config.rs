use mime::Mime;
use serde::Serialize;
use std::{
    collections::{HashMap, VecDeque},
    io::IsTerminal,
    str::FromStr,
};
use tabled::Tabled;

use crate::{
    apps::{DesktopList, MimeApps, SystemApps},
    common::{render_table, DesktopHandler, Handleable, Handler, UserPath},
    config::Config,
    error::{Error, ErrorKind, Result},
};

/// A single struct that holds all apps and config.
/// Used to streamline explicitly passing state.
#[derive(Default)]
pub struct AppsConfig {
    pub mime_apps: MimeApps,
    pub system_apps: SystemApps,
    pub config: Config,
}

impl AppsConfig {
    /// Create a new instance of AppsConfig
    pub fn new() -> Result<Self> {
        Ok(Self {
            mime_apps: MimeApps::read()?,
            system_apps: SystemApps::populate()?,
            config: Config::load()?,
        })
    }

    /// Get the handler associated with a given mime
    pub fn get_handler(
        &self,
        mime: &Mime,
        selector: &str,
        enable_selector: bool,
    ) -> Result<DesktopHandler> {
        match self.mime_apps.get_handler_from_user(
            mime,
            selector,
            enable_selector,
        ) {
            Err(e) if matches!(*e.kind, ErrorKind::Cancelled) => Err(e),
            h => h
                .or_else(|_| {
                    let wildcard =
                        Mime::from_str(&format!("{}/*", mime.type_()))?;
                    self.mime_apps.get_handler_from_user(
                        &wildcard,
                        selector,
                        enable_selector,
                    )
                })
                .or_else(|_| self.get_handler_from_added_associations(mime)),
        }
    }

    /// Get the handler associated with a given mime from mimeapps.list's added associations
    /// If there is none, default to the system apps
    fn get_handler_from_added_associations(
        &self,
        mime: &Mime,
    ) -> Result<DesktopHandler> {
        self.mime_apps
            .added_associations
            .get(mime)
            .map_or_else(
                || self.system_apps.get_handler(mime),
                |h| h.front().cloned(),
            )
            .ok_or_else(|| Error::from(ErrorKind::NotFound(mime.to_string())))
    }

    /// Given a mime and arguments, launch the associated handler with the arguments
    pub fn launch_handler(
        &mut self,
        mime: &Mime,
        args: Vec<UserPath>,
        selector: &str,
        enable_selector: bool,
    ) -> Result<()> {
        self.get_handler(mime, selector, enable_selector)?.launch(
            self,
            args.into_iter().map(|a| a.to_string()).collect(),
            selector,
            enable_selector,
        )
    }

    /// Get the handler associated with a given mime
    pub fn show_handler(
        &mut self,
        mime: &Mime,
        output_json: bool,
        selector: &str,
        enable_selector: bool,
    ) -> Result<()> {
        let handler = self.get_handler(mime, selector, enable_selector)?;

        let output = if output_json {
            let entry = handler.get_entry()?;
            let cmd = entry.get_cmd(self, vec![], selector, enable_selector)?;

            (serde_json::json!( {
                "handler": handler.to_string(),
                "name": entry.name,
                "cmd": cmd.0 + " " + &cmd.1.join(" "),
            }))
            .to_string()
        } else {
            handler.to_string()
        };
        println!("{}", output);
        Ok(())
    }

    /// Open the given paths with their respective handlers
    pub fn open_paths(
        &mut self,
        paths: &[UserPath],
        selector: &str,
        enable_selector: bool,
    ) -> Result<()> {
        let mut handlers: HashMap<Handler, Vec<String>> = HashMap::new();

        for path in paths.iter() {
            handlers
                .entry(self.get_handler_from_path(
                    path,
                    selector,
                    enable_selector,
                )?)
                .or_default()
                .push(path.to_string())
        }

        for (handler, paths) in handlers.into_iter() {
            handler.open(self, paths, selector, enable_selector)?;
        }

        Ok(())
    }

    /// Get the handler associated with a given path
    fn get_handler_from_path(
        &self,
        path: &UserPath,
        selector: &str,
        enable_selector: bool,
    ) -> Result<Handler> {
        Ok(if let Ok(handler) = self.config.get_regex_handler(path) {
            handler.into()
        } else {
            self.get_handler(&path.get_mime()?, selector, enable_selector)?
                .into()
        })
    }

    /// Get the command for the x-scheme-handler/terminal handler if one is set.
    /// Otherwise, finds a terminal emulator program, sets it as the handler, and makes a notification.
    pub fn terminal(
        &mut self,
        selector: &str,
        enable_selector: bool,
    ) -> Result<String> {
        let terminal_entry = self
            .get_handler(
                &Mime::from_str("x-scheme-handler/terminal")?,
                selector,
                enable_selector,
            )
            .ok()
            .and_then(|h| h.get_entry().ok());

        terminal_entry
            .or_else(|| {
                let entry = SystemApps::get_entries()
                    .ok()?
                    .find(|(_handler, entry)| {
                        entry.is_terminal_emulator()
                    })?;

                crate::utils::notify(
                    "handlr",
                    &format!(
                        "Guessed terminal emulator: {}.\n\nIf this is wrong, use `handlr set x-scheme-handler/terminal` to update it.",
                        entry.0.to_string_lossy()
                    )
                ).ok()?;

                self.mime_apps.set_handler(
                    &Mime::from_str("x-scheme-handler/terminal").ok()?,
                    &DesktopHandler::assume_valid(entry.0),
                );
                self.mime_apps.save().ok()?;

                Some(entry.1)
            })
            .map(|e| {
                let mut exec = e.exec.to_owned();

                if let Some(opts) = &self.config.term_exec_args {
                    exec.push(' ');
                    exec.push_str(opts)
                }

                exec
            })
            .ok_or(Error::from(ErrorKind::NoTerminal))
    }

    /// Print the set associations and system-level associations in a table
    pub fn print(&self, detailed: bool, output_json: bool) -> Result<()> {
        let mimeapps_table =
            MimeAppsTable::new(&self.mime_apps, &self.system_apps);

        if detailed {
            if output_json {
                println!("{}", serde_json::to_string(&mimeapps_table)?)
            } else {
                println!("Default Apps");
                println!("{}", render_table(&mimeapps_table.default_apps));
                if !self.mime_apps.added_associations.is_empty() {
                    println!("Added associations");
                    println!(
                        "{}",
                        render_table(&mimeapps_table.added_associations)
                    );
                }
                println!("System Apps");
                println!("{}", render_table(&mimeapps_table.system_apps))
            }
        } else if output_json {
            println!("{}", serde_json::to_string(&mimeapps_table.default_apps)?)
        } else {
            println!("{}", render_table(&mimeapps_table.default_apps))
        }

        Ok(())
    }
}

/// Internal helper struct for turning MimeApps into tabular data
#[derive(PartialEq, Eq, PartialOrd, Ord, Tabled, Serialize)]
struct MimeAppsEntry {
    mime: String,
    #[tabled(display_with("Self::display_handlers", self))]
    handlers: Vec<String>,
}

impl MimeAppsEntry {
    /// Create a new `MimeAppsEntry`
    fn new(mime: &Mime, handlers: &VecDeque<DesktopHandler>) -> Self {
        Self {
            mime: mime.to_string(),
            handlers: handlers
                .iter()
                .map(|x| x.to_string())
                .collect::<Vec<String>>(),
        }
    }

    /// Display list of handlers as a string
    fn display_handlers(&self) -> String {
        // If output is a terminal, optimize for readability
        // Otherwise, if piped, optimize for parseability
        let separator = if std::io::stdout().is_terminal() {
            ",\n"
        } else {
            ", "
        };

        self.handlers.join(separator)
    }
}

/// Internal helper struct for turning MimeApps into tabular data
#[derive(Serialize)]
struct MimeAppsTable {
    added_associations: Vec<MimeAppsEntry>,
    default_apps: Vec<MimeAppsEntry>,
    system_apps: Vec<MimeAppsEntry>,
}

impl MimeAppsTable {
    /// Create a new `MimeAppsTable`
    fn new(mimeapps: &MimeApps, system_apps: &SystemApps) -> Self {
        fn to_entries(map: &HashMap<Mime, DesktopList>) -> Vec<MimeAppsEntry> {
            let mut rows = map
                .iter()
                .map(|(mime, handlers)| MimeAppsEntry::new(mime, handlers))
                .collect::<Vec<_>>();
            rows.sort_unstable();
            rows
        }
        Self {
            added_associations: to_entries(&mimeapps.added_associations),
            default_apps: to_entries(&mimeapps.default_apps),
            system_apps: to_entries(system_apps),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_mimes() -> Result<()> {
        let mut apps_config = AppsConfig::default();
        apps_config.mime_apps.add_handler(
            &Mime::from_str("video/*")?,
            &DesktopHandler::assume_valid("mpv.desktop".into()),
        );
        apps_config.mime_apps.add_handler(
            &Mime::from_str("video/webm")?,
            &DesktopHandler::assume_valid("brave.desktop".into()),
        );

        assert_eq!(
            apps_config
                .get_handler(&Mime::from_str("video/mp4")?, "", false)?
                .to_string(),
            "mpv.desktop"
        );
        assert_eq!(
            apps_config
                .get_handler(&Mime::from_str("video/asdf")?, "", false)?
                .to_string(),
            "mpv.desktop"
        );

        assert_eq!(
            apps_config
                .get_handler(&Mime::from_str("video/webm")?, "", false)?
                .to_string(),
            "brave.desktop"
        );

        Ok(())
    }
}
