use super::Explorer;
use crate::ops::{Notice, NoticeKind};

impl Explorer {
    /// The oldest pending notice, if any.
    pub fn take_notice(&mut self) -> Option<Notice> {
        self.notices.pop_front()
    }

    /// Shows `text` as an error notice.
    pub fn notify_error(&mut self, text: impl Into<String>) {
        self.notify(NoticeKind::Error, text);
    }

    pub(super) fn notify(&mut self, kind: NoticeKind, text: impl Into<String>) {
        self.notices.push_back(Notice {
            kind,
            text: text.into(),
        });
    }
}
