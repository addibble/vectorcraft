use super::{VectorcraftApp, io, place};

impl VectorcraftApp {
    /// A file dialog is open off the UI thread (a sheet on macOS, [`picks`]): what asked for it runs
    /// again when it answers.
    pub fn file_dialog_open(&self) -> bool {
        self.picks.is_waiting()
    }

    /// A file dialog is open as a sheet on the window ([`Services::dialogs_are_sheets`]): nothing
    /// closes or quits until it answers.
    pub fn file_sheet_open(&self) -> bool {
        self.services.dialogs_are_sheets && self.file_dialog_open()
    }

    pub(crate) fn drain_inbox(&mut self) {
        let arrived: Vec<OpenArrival> =
            self.services.inbox.as_ref().map(|q| std::mem::take(&mut *q.lock().unwrap_or_else(|e| e.into_inner()))).unwrap_or_default();
        for OpenArrival { name, bytes, title, path } in arrived {
            match io::open_bytes(self, &name, &bytes, path) {
                Ok(_) => {
                    if let Some(title) = title {
                        self.title_active(title);
                    }
                }
                Err(e) => io::report_open_error(self, &name, &e),
            }
        }
        place::drain(self);
    }

    /// Show `title` on the active document's tab in place of its name, while the name stays what
    /// it is now ([`state::HostTitle`]).
    fn title_active(&mut self, title: String) {
        let Some(d) = self.session.active() else { return };
        let (uid, name) = (d.uid, d.title());
        let open: Vec<u64> = self.session.documents().iter().map(|d| d.uid).collect();
        self.ui.host_titles.retain(|t| t.uid != uid && open.contains(&t.uid));
        self.ui.host_titles.push(state::HostTitle { uid, name, title });
    }
}
