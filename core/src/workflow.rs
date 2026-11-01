//! Pure batch-programming workflow state machine.
//!
//! The UI and the NFC worker feed events into this module. It decides which
//! screen to show and when session progress may be persisted. The critical
//! invariant is that a URL advances only after read-back verification plus an
//! explicit confirmation, or after an explicit skip.

use crate::errors::{NdefError, PlaylistError, SessionError, TagError};
use crate::session::{ResumeStatus, Session};
use crate::tag::TagModel;

/// Visible screen or phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Startup,
    PlaylistSelect,
    WaitingForTag,
    Writing,
    Success,
    RemoveTag,
    Error,
    Summary,
    Finished,
}

/// Reason the workflow is showing an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowErrorKind {
    Tag(TagError),
    Ndef(NdefError),
    WriteFailed,
    ReadBackFailed,
    TagChanged,
    Playlist(PlaylistError),
    Session(SessionError),
    Storage,
}

impl WorkflowErrorKind {
    /// True when Retry/Skip/Abort are meaningful for this failure.
    pub fn is_tag_error(self) -> bool {
        match self {
            Self::Tag(_) | Self::WriteFailed | Self::ReadBackFailed | Self::TagChanged => true,
            Self::Ndef(NdefError::Url(_)) => false,
            Self::Ndef(_) => true,
            Self::Playlist(_) | Self::Session(_) | Self::Storage => false,
        }
    }
}

/// Final or partial batch summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    pub total: u32,
    pub completed: u32,
    pub skipped: u32,
    pub failed: u32,
    pub remaining: u32,
    pub complete: bool,
    pub aborted: bool,
}

/// Screen request produced by the state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    ShowStartup { can_resume: bool },
    ShowPlaylistSelect,
    ShowWaiting,
    ShowWriting,
    ShowSuccess,
    ShowRemoveTag { same_tag: bool },
    ShowError(WorkflowErrorKind),
    ShowSummary(Summary),
    Exit,
}

/// Batch workflow.
pub struct Workflow {
    screen: Screen,
    session: Option<Session>,
    tag_model: Option<TagModel>,
    error: Option<WorkflowErrorKind>,
    summary: Option<Summary>,
    same_tag: bool,
    aborted: bool,
    dirty: bool,
}

impl Default for Workflow {
    fn default() -> Self {
        Self::new()
    }
}

impl Workflow {
    pub fn new() -> Self {
        Self {
            screen: Screen::Startup,
            session: None,
            tag_model: None,
            error: None,
            summary: None,
            same_tag: false,
            aborted: false,
            dirty: false,
        }
    }

    pub fn screen(&self) -> Screen {
        self.screen
    }

    pub fn session(&self) -> Option<&Session> {
        self.session.as_ref()
    }

    pub fn tag_model(&self) -> Option<TagModel> {
        self.tag_model
    }

    pub fn error(&self) -> Option<WorkflowErrorKind> {
        self.error
    }

    pub fn summary(&self) -> Option<Summary> {
        self.summary
    }

    /// Returns true when the current screen should be offered a Resume choice.
    pub fn can_resume(&self) -> bool {
        self.session
            .as_ref()
            .map(|session| !session.is_complete())
            .unwrap_or(false)
    }

    /// True when the caller must persist the session before its next screen.
    pub fn take_dirty(&mut self) -> bool {
        let dirty = self.dirty;
        self.dirty = false;
        dirty
    }

    /// Current position, one-based for display.
    pub fn current_index(&self) -> u32 {
        self.session
            .as_ref()
            .map(|session| (session.next + 1).min(session.total))
            .unwrap_or(1)
    }

    pub fn total(&self) -> u32 {
        self.session
            .as_ref()
            .map(|session| session.total)
            .unwrap_or(0)
    }

    /// Startup decision from a loaded (or absent) session.
    pub fn start(&mut self, session: Option<Session>, status: ResumeStatus) -> Action {
        let Some(session) = session else {
            self.session = None;
            self.summary = None;
            self.error = None;
            self.tag_model = None;
            self.aborted = false;
            self.same_tag = false;
            self.dirty = false;
            self.screen = Screen::PlaylistSelect;
            return Action::ShowPlaylistSelect;
        };

        match status {
            ResumeStatus::NoSession => {
                self.screen = Screen::PlaylistSelect;
                Action::ShowPlaylistSelect
            }
            ResumeStatus::Resume => {
                self.session = Some(session);
                self.screen = Screen::WaitingForTag;
                Action::ShowWaiting
            }
            ResumeStatus::Completed => {
                self.summary = Some(Summary {
                    total: session.total,
                    completed: session.completed,
                    skipped: session.skipped,
                    failed: session.failed,
                    remaining: 0,
                    complete: true,
                    aborted: false,
                });
                self.session = Some(session);
                self.screen = Screen::Summary;
                Action::ShowSummary(self.summary.unwrap())
            }
            ResumeStatus::PlaylistChanged => {
                self.fail(WorkflowErrorKind::Session(SessionError::PlaylistChanged));
                Action::ShowError(WorkflowErrorKind::Session(SessionError::PlaylistChanged))
            }
            ResumeStatus::PlaylistMissing => {
                self.fail(WorkflowErrorKind::Session(SessionError::PlaylistMissing));
                Action::ShowError(WorkflowErrorKind::Session(SessionError::PlaylistMissing))
            }
        }
    }

    /// Starts a new session after the playlist was parsed and validated.
    pub fn playlist_loaded(&mut self, session: Session) -> Action {
        self.session = Some(session);
        self.error = None;
        self.summary = None;
        self.aborted = false;
        self.dirty = false;
        self.screen = Screen::WaitingForTag;
        Action::ShowWaiting
    }

    pub fn playlist_failed(&mut self, error: PlaylistError) -> Action {
        self.fail(WorkflowErrorKind::Playlist(error))
    }

    pub fn session_failed(&mut self, error: SessionError) -> Action {
        self.fail(WorkflowErrorKind::Session(error))
    }

    pub fn storage_failed(&mut self) -> Action {
        self.fail(WorkflowErrorKind::Storage)
    }

    /// A valid tag was detected and fully validated.
    pub fn tag_validated(&mut self, model: TagModel) -> Action {
        if self.screen != Screen::WaitingForTag {
            return self.current_action();
        }
        self.tag_model = Some(model);
        self.screen = Screen::Writing;
        Action::ShowWriting
    }

    /// A tag failed model/CC/lock/capacity validation.
    pub fn tag_rejected(&mut self, error: TagError) -> Action {
        self.record_failure();
        self.fail(WorkflowErrorKind::Tag(error))
    }

    /// The write completed; the workflow now waits for read-back verification.
    pub fn write_succeeded(&mut self) -> Action {
        if self.screen != Screen::Writing {
            return self.current_action();
        }
        Action::ShowWriting
    }

    pub fn write_failed(&mut self) -> Action {
        self.record_failure();
        self.fail(WorkflowErrorKind::WriteFailed)
    }

    pub fn readback_failed(&mut self) -> Action {
        self.record_failure();
        self.fail(WorkflowErrorKind::ReadBackFailed)
    }

    /// The tag changed or was removed between validation and writing.
    pub fn tag_changed(&mut self) -> Action {
        self.record_failure();
        self.fail(WorkflowErrorKind::TagChanged)
    }

    /// The URL could not be encoded or does not fit after all.
    pub fn ndef_rejected(&mut self, error: NdefError) -> Action {
        self.record_failure();
        self.fail(WorkflowErrorKind::Ndef(error))
    }

    pub fn verification_failed(&mut self) -> Action {
        self.record_failure();
        self.fail(WorkflowErrorKind::Ndef(NdefError::VerificationMismatch))
    }

    /// Read-back verified the exact URL. The success screen is shown, but the
    /// session does not advance until [`Self::confirm_success`].
    pub fn verification_succeeded(&mut self) -> Action {
        if self.screen != Screen::Writing {
            return self.current_action();
        }
        self.error = None;
        self.screen = Screen::Success;
        Action::ShowSuccess
    }

    /// User pressed OK on the success screen.
    pub fn confirm_success(&mut self) -> Result<Action, SessionError> {
        if self.screen != Screen::Success {
            return Ok(self.current_action());
        }
        let session = self.session.as_mut().ok_or(SessionError::Corrupt)?;
        session.record_verified()?;
        self.dirty = true;
        self.same_tag = false;
        self.screen = Screen::RemoveTag;
        Ok(Action::ShowRemoveTag { same_tag: false })
    }

    /// The programmed tag is still present (same UID).
    pub fn same_tag_detected(&mut self) -> Action {
        if self.screen != Screen::RemoveTag {
            return self.current_action();
        }
        self.same_tag = true;
        Action::ShowRemoveTag { same_tag: true }
    }

    /// The programmed tag was removed or a different UID was presented.
    pub fn tag_cleared(&mut self) -> Action {
        if self.screen != Screen::RemoveTag {
            return self.current_action();
        }
        self.same_tag = false;
        self.advance_or_summary()
    }

    /// User chose Retry on a tag error.
    pub fn retry(&mut self) -> Action {
        if self.screen == Screen::Error
            && self
                .error
                .map(|error| error.is_tag_error())
                .unwrap_or(false)
        {
            self.error = None;
            self.screen = Screen::WaitingForTag;
            Action::ShowWaiting
        } else {
            self.current_action()
        }
    }

    /// User explicitly skipped the current URL.
    pub fn skip(&mut self) -> Result<Action, SessionError> {
        if self.screen != Screen::Error
            || !self
                .error
                .map(|error| error.is_tag_error())
                .unwrap_or(false)
        {
            return Ok(self.current_action());
        }
        let session = self.session.as_mut().ok_or(SessionError::Corrupt)?;
        session.record_skipped()?;
        self.dirty = true;
        self.error = None;
        Ok(self.advance_or_summary())
    }

    /// User aborted. Progress is preserved for a later resume.
    pub fn abort(&mut self) -> Action {
        if self.screen == Screen::Finished {
            return Action::Exit;
        }
        self.dirty = true;
        self.aborted = true;
        self.error = None;
        self.build_summary();
        self.screen = Screen::Summary;
        Action::ShowSummary(self.summary.unwrap())
    }

    /// User finished reviewing the summary.
    pub fn finish(&mut self) -> Action {
        self.screen = Screen::Finished;
        Action::Exit
    }

    pub fn current_action(&self) -> Action {
        match self.screen {
            Screen::Startup => Action::ShowStartup {
                can_resume: self.can_resume(),
            },
            Screen::PlaylistSelect => Action::ShowPlaylistSelect,
            Screen::WaitingForTag => Action::ShowWaiting,
            Screen::Writing => Action::ShowWriting,
            Screen::Success => Action::ShowSuccess,
            Screen::RemoveTag => Action::ShowRemoveTag {
                same_tag: self.same_tag,
            },
            Screen::Error => Action::ShowError(self.error.unwrap_or(WorkflowErrorKind::Storage)),
            Screen::Summary => Action::ShowSummary(self.summary.unwrap_or(Summary {
                total: self.total(),
                completed: 0,
                skipped: 0,
                failed: 0,
                remaining: 0,
                complete: false,
                aborted: true,
            })),
            Screen::Finished => Action::Exit,
        }
    }

    fn record_failure(&mut self) {
        // The failed-attempt counter is kept in memory and is written with the
        // next verified/skip/abort save. A failure alone never touches the
        // session file or advances the index.
        if let Some(session) = self.session.as_mut() {
            session.record_failed_attempt();
        }
    }

    fn fail(&mut self, error: WorkflowErrorKind) -> Action {
        self.error = Some(error);
        self.screen = Screen::Error;
        Action::ShowError(error)
    }

    fn advance_or_summary(&mut self) -> Action {
        let complete = self
            .session
            .as_ref()
            .map(|session| session.next >= session.total)
            .unwrap_or(true);
        if complete {
            self.build_summary();
            self.screen = Screen::Summary;
            Action::ShowSummary(self.summary.unwrap())
        } else {
            self.tag_model = None;
            self.screen = Screen::WaitingForTag;
            Action::ShowWaiting
        }
    }

    fn build_summary(&mut self) {
        let summary = match self.session.as_ref() {
            Some(session) => Summary {
                total: session.total,
                completed: session.completed,
                skipped: session.skipped,
                failed: session.failed,
                remaining: session.remaining(),
                complete: session.is_complete(),
                aborted: self.aborted,
            },
            None => Summary {
                total: 0,
                completed: 0,
                skipped: 0,
                failed: 0,
                remaining: 0,
                complete: false,
                aborted: true,
            },
        };
        self.summary = Some(summary);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_workflow() -> Workflow {
        let mut workflow = Workflow::new();
        let session = Session::new(0x1234, 3, b"/ext/playlist.txt").unwrap();
        assert_eq!(
            workflow.start(None, ResumeStatus::NoSession),
            Action::ShowPlaylistSelect
        );
        assert_eq!(workflow.playlist_loaded(session), Action::ShowWaiting);
        workflow
    }

    fn reach_success(workflow: &mut Workflow) {
        assert_eq!(
            workflow.tag_validated(TagModel::Ntag213),
            Action::ShowWriting
        );
        assert_eq!(workflow.write_succeeded(), Action::ShowWriting);
        assert_eq!(workflow.verification_succeeded(), Action::ShowSuccess);
    }

    #[test]
    fn starts_new_session_with_playlist_selection() {
        let mut workflow = Workflow::new();
        assert_eq!(
            workflow.start(None, ResumeStatus::NoSession),
            Action::ShowPlaylistSelect
        );
        assert_eq!(workflow.screen(), Screen::PlaylistSelect);
        assert_eq!(
            workflow.playlist_failed(PlaylistError::Empty),
            Action::ShowError(WorkflowErrorKind::Playlist(PlaylistError::Empty))
        );
        assert!(!workflow.error().unwrap().is_tag_error());
    }

    #[test]
    fn resumes_at_saved_index() {
        let mut session = Session::new(0x1234, 5, b"/ext/playlist.txt").unwrap();
        session.record_verified().unwrap();
        session.record_skipped().unwrap();
        let mut workflow = Workflow::new();
        assert_eq!(
            workflow.start(Some(session), ResumeStatus::Resume),
            Action::ShowWaiting
        );
        assert_eq!(workflow.current_index(), 3);
        assert_eq!(workflow.total(), 5);
        assert!(workflow.can_resume());
    }

    #[test]
    fn completed_session_shows_summary() {
        let mut session = Session::new(0x1234, 1, b"/ext/playlist.txt").unwrap();
        session.record_verified().unwrap();
        let mut workflow = Workflow::new();
        let action = workflow.start(Some(session), ResumeStatus::Completed);
        assert!(matches!(action, Action::ShowSummary(summary) if summary.complete));
        assert_eq!(workflow.screen(), Screen::Summary);
    }

    #[test]
    fn changed_and_missing_playlists_are_setup_errors() {
        for status in [ResumeStatus::PlaylistChanged, ResumeStatus::PlaylistMissing] {
            let session = Session::new(0x1234, 3, b"/ext/playlist.txt").unwrap();
            let mut workflow = Workflow::new();
            let action = workflow.start(Some(session), status);
            assert!(matches!(action, Action::ShowError(error) if !error.is_tag_error()));
            assert_eq!(workflow.retry(), action);
        }
    }

    #[test]
    fn write_success_without_verification_does_not_advance() {
        let mut workflow = new_workflow();
        assert_eq!(
            workflow.tag_validated(TagModel::Ntag215),
            Action::ShowWriting
        );
        assert_eq!(workflow.write_succeeded(), Action::ShowWriting);
        assert_eq!(workflow.session().unwrap().next, 0);
        assert_eq!(workflow.session().unwrap().completed, 0);
        assert!(!workflow.take_dirty());
    }

    #[test]
    fn confirmation_is_required_to_advance() {
        let mut workflow = new_workflow();
        reach_success(&mut workflow);
        assert_eq!(workflow.session().unwrap().next, 0);
        assert!(!workflow.take_dirty());

        assert_eq!(
            workflow.confirm_success().unwrap(),
            Action::ShowRemoveTag { same_tag: false }
        );
        assert_eq!(workflow.session().unwrap().next, 1);
        assert_eq!(workflow.session().unwrap().completed, 1);
        assert!(workflow.take_dirty());
    }

    #[test]
    fn same_tag_blocks_advancement() {
        let mut workflow = new_workflow();
        reach_success(&mut workflow);
        workflow.confirm_success().unwrap();
        assert_eq!(
            workflow.same_tag_detected(),
            Action::ShowRemoveTag { same_tag: true }
        );
        assert_eq!(workflow.screen(), Screen::RemoveTag);
        assert_eq!(workflow.session().unwrap().next, 1);
        assert_eq!(workflow.tag_cleared(), Action::ShowWaiting);
        assert_eq!(workflow.current_index(), 2);
    }

    #[test]
    fn retry_keeps_the_index_and_skip_advances() {
        let mut workflow = new_workflow();
        workflow.tag_validated(TagModel::Ntag213);
        assert_eq!(
            workflow.write_failed(),
            Action::ShowError(WorkflowErrorKind::WriteFailed)
        );
        let failed_before = workflow.session().unwrap().failed;
        assert_eq!(workflow.session().unwrap().next, 0);
        assert_eq!(workflow.retry(), Action::ShowWaiting);
        assert_eq!(workflow.session().unwrap().next, 0);
        assert_eq!(workflow.session().unwrap().failed, failed_before);

        assert_eq!(
            workflow.tag_validated(TagModel::Ntag213),
            Action::ShowWriting
        );
        assert_eq!(
            workflow.verification_failed(),
            Action::ShowError(WorkflowErrorKind::Ndef(NdefError::VerificationMismatch))
        );
        assert_eq!(workflow.skip().unwrap(), Action::ShowWaiting);
        assert_eq!(workflow.session().unwrap().next, 1);
        assert_eq!(workflow.session().unwrap().skipped, 1);
        assert_eq!(workflow.session().unwrap().completed, 0);
        assert!(workflow.take_dirty());
    }

    #[test]
    fn setup_error_does_not_offer_tag_retry() {
        let mut workflow = new_workflow();
        workflow.playlist_failed(PlaylistError::Empty);
        let error = workflow.error().unwrap();
        assert!(!error.is_tag_error());
        assert_eq!(workflow.retry(), Action::ShowError(error));
        assert_eq!(workflow.skip().unwrap(), Action::ShowError(error));
        assert_eq!(workflow.session().unwrap().next, 0);
    }

    #[test]
    fn abort_preserves_progress_and_reports_partial_summary() {
        let mut workflow = new_workflow();
        reach_success(&mut workflow);
        workflow.confirm_success().unwrap();
        workflow.tag_cleared();

        let action = workflow.abort();
        match action {
            Action::ShowSummary(summary) => {
                assert_eq!(summary.total, 3);
                assert_eq!(summary.completed, 1);
                assert_eq!(summary.remaining, 2);
                assert!(!summary.complete);
                assert!(summary.aborted);
            }
            other => panic!("unexpected action: {other:?}"),
        }
        assert!(workflow.take_dirty());
    }

    #[test]
    fn last_entry_completes_after_removal() {
        let mut workflow = Workflow::new();
        let session = Session::new(0x1234, 1, b"/ext/playlist.txt").unwrap();
        workflow.playlist_loaded(session);
        reach_success(&mut workflow);
        workflow.confirm_success().unwrap();
        let action = workflow.tag_cleared();
        match action {
            Action::ShowSummary(summary) => {
                assert!(summary.complete);
                assert_eq!(summary.completed, 1);
                assert_eq!(summary.remaining, 0);
            }
            other => panic!("unexpected action: {other:?}"),
        }
    }

    #[test]
    fn skip_on_last_entry_completes() {
        let mut workflow = Workflow::new();
        let session = Session::new(0x1234, 1, b"/ext/playlist.txt").unwrap();
        workflow.playlist_loaded(session);
        workflow.tag_rejected(TagError::Locked);
        let action = workflow.skip().unwrap();
        assert!(
            matches!(action, Action::ShowSummary(summary) if summary.complete && summary.skipped == 1)
        );
    }

    #[test]
    fn failed_attempts_are_counted_separately() {
        let mut workflow = new_workflow();
        workflow.tag_rejected(TagError::UnsupportedTag);
        workflow.retry();
        workflow.tag_rejected(TagError::Locked);
        workflow.retry();
        reach_success(&mut workflow);
        workflow.confirm_success().unwrap();
        workflow.tag_cleared();
        let summary = workflow.session().unwrap();
        assert_eq!(summary.failed, 2);
        assert_eq!(summary.completed, 1);
        assert_eq!(summary.next, 1);
    }

    #[test]
    fn finish_exits() {
        let mut workflow = new_workflow();
        workflow.abort();
        assert_eq!(workflow.finish(), Action::Exit);
        assert_eq!(workflow.screen(), Screen::Finished);
    }
}
