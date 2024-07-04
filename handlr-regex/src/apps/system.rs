use crate::{
    common::{DesktopEntry, DesktopHandler},
    Result,
};
use mime::Mime;
use std::{
    collections::{HashMap, VecDeque},
    convert::TryFrom,
    ffi::OsString,
    ops::Deref,
};

#[derive(Debug, Default, Clone)]
pub struct SystemApps(HashMap<Mime, VecDeque<DesktopHandler>>);

impl SystemApps {
    pub fn get_handlers(
        &self,
        mime: &Mime,
    ) -> Option<VecDeque<DesktopHandler>> {
        Some(self.0.get(mime)?.clone())
    }
    pub fn get_handler(&self, mime: &Mime) -> Option<DesktopHandler> {
        Some(self.get_handlers(mime)?.front().unwrap().clone())
    }

    pub fn get_entries(
    ) -> Result<impl Iterator<Item = (OsString, DesktopEntry)>> {
        Ok(xdg::BaseDirectories::new()?
            .list_data_files_once("applications")
            .into_iter()
            .filter(|p| {
                p.extension().and_then(|x| x.to_str()) == Some("desktop")
            })
            .filter_map(|p| {
                Some((
                    p.file_name().unwrap().to_owned(),
                    DesktopEntry::try_from(p.clone()).ok()?,
                ))
            }))
    }

    pub fn populate() -> Result<Self> {
        let mut map =
            HashMap::<Mime, VecDeque<DesktopHandler>>::with_capacity(50);

        Self::get_entries()?.for_each(|(_, entry)| {
            let (file_name, mimes) = (entry.file_name, entry.mime_type);
            mimes.into_iter().for_each(|mime| {
                map.entry(mime).or_default().push_back(
                    DesktopHandler::assume_valid(file_name.to_owned()),
                );
            });
        });

        Ok(Self(map))
    }

    /// List the available handlers
    pub fn list_handlers() -> Result<()> {
        use std::{io::Write, os::unix::ffi::OsStrExt};

        let stdout = std::io::stdout();
        let mut stdout = stdout.lock();

        Self::get_entries()?.for_each(|(_, e)| {
            stdout.write_all(e.file_name.as_bytes()).unwrap();
            stdout.write_all(b"\t").unwrap();
            stdout.write_all(e.name.as_bytes()).unwrap();
            stdout.write_all(b"\n").unwrap();
        });

        Ok(())
    }
}

impl Deref for SystemApps {
    type Target = HashMap<Mime, VecDeque<DesktopHandler>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
