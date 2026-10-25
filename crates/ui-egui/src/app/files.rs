use super::{VectorcraftApp, io, place};
use crate::services::{Inbox, OpenArrival};

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
        fn take(q: Option<&Inbox>) -> Vec<OpenArrival> {
            q.map(|q| std::mem::take(&mut *q.lock().unwrap_or_else(|e| e.into_inner()))).unwrap_or_default()
        }
        let answers = self.services.picked_inbox.as_ref().map(|q| std::mem::take(&mut *q.lock().unwrap_or_else(|e| e.into_inner())));
        // The web's file picker answers for automation when automation opened it; the page's script
        // is automation.
        let mut arrived: Vec<_> = take(self.services.inbox.as_ref()).into_iter().map(|f| (f, false)).collect();
        for answer in answers.unwrap_or_default() {
            let automated = self.picks.answer_web_open();
            arrived.extend(answer.map(|(name, bytes)| (OpenArrival::file(name, bytes), automated)));
        }
        arrived.extend(take(self.services.page_inbox.as_ref()).into_iter().map(|f| (f, true)));
        for (OpenArrival { name, bytes, title }, automated) in arrived {
            crate::picks::as_automated(self, automated, |app| match io::open_bytes(app, &name, &bytes, None) {
                Ok(_) => {
                    if let Some(title) = title {
                        app.title_active(title);
                    }
                }
                Err(e) => io::report_open_error(app, &name, &e),
            });
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
