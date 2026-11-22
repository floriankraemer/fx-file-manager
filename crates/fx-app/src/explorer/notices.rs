use super::Explorer;
use crate::ops::{Notice, NoticeKind};

/// Notices the UI has not shown yet; older ones are dropped beyond this.
const MAX_NOTICES: usize = 50;

impl Explorer {
    /// The oldest pending notice, if any.
    pub fn take_notice(&mut self) -> Option<Notice> {
        self.notices.pop_front()
    }

    /// Shows `text` as an error notice.
    pub fn notify_error(&mut self, text: impl Into<String>) {
        self.notify(NoticeKind::Error, text);
    }

    pub(super) fn push_notice(&mut self, notice: Notice) {
        if self.notices.len() >= MAX_NOTICES {
            self.notices.pop_front();
        }
        self.notices.push_back(notice);
    }

    pub(super) fn notify(&mut self, kind: NoticeKind, text: impl Into<String>) {
        self.push_notice(Notice {
            kind,
            text: text.into(),
        });
    }
}
