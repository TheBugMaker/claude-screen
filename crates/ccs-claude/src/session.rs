//! What a session is doing, from the reports of the bridge mod in its Claude. No I/O.

use crate::bridge::Bridge;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum State {
    /// Claude started, the mod has not reported yet.
    #[default]
    Starting,
    /// Waiting for a prompt.
    Idle,
    /// A turn is running.
    Working,
    /// A tool is waiting for the user to allow or deny it.
    WaitingPermission,
}

#[derive(Debug, Default)]
pub struct Session {
    pub session_id: Option<String>,
    /// The name given with `/rename`, Claude Code's custom title.
    pub name: Option<String>,
    /// The title Claude generates for the conversation, read from its transcript.
    pub ai_title: Option<String>,
    /// Never `WaitingPermission`: that comes from `dialog`.
    phase: State,
    /// Claude shows a permission dialog.
    dialog: bool,
}

impl Session {
    #[must_use]
    pub fn state(&self) -> State {
        if self.phase == State::Working && self.dialog {
            State::WaitingPermission
        } else {
            self.phase
        }
    }

    /// Applies a report of the bridge mod.
    pub fn bridge(&mut self, event: Bridge) {
        match event {
            Bridge::Ready { session_id, .. } => {
                self.session_id = Some(session_id);
                self.started();
            }
            Bridge::Submitted | Bridge::Working => self.phase = State::Working,
            Bridge::Permission => {
                self.dialog = true;
                self.phase = State::Working;
            }
            Bridge::Approved | Bridge::Denied => self.dialog = false,
            Bridge::Idle => {
                self.dialog = false;
                self.phase = State::Idle;
            }
            Bridge::Title { title } => self.name = Some(title),
            // Where the session works is the app's.
            Bridge::Root { .. } => {}
        }
    }

    /// The session is ready. A prompt typed before that has already made it Working.
    fn started(&mut self) {
        if self.phase == State::Starting {
            self.phase = State::Idle;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_reports_drive_the_state() {
        let mut session = Session::default();
        let mut states = Vec::new();
        for event in [
            Bridge::Ready {
                session_id: "5c1c".into(),
                root: "/work".into(),
            },
            Bridge::Submitted,
            Bridge::Working,
            Bridge::Permission,
            Bridge::Approved,
            Bridge::Permission,
            Bridge::Denied,
            Bridge::Idle,
            Bridge::Title {
                title: "probe".into(),
            },
        ] {
            session.bridge(event);
            states.push(session.state());
        }
        assert_eq!(
            states,
            [
                State::Idle,
                State::Working,
                State::Working,
                State::WaitingPermission,
                State::Working,
                State::WaitingPermission,
                State::Working,
                State::Idle,
                State::Idle,
            ]
        );
        assert_eq!(session.session_id.as_deref(), Some("5c1c"));
        assert_eq!(session.name.as_deref(), Some("probe"));
        // An interrupt during a dialog ends the turn: no dialog is left open.
        session.bridge(Bridge::Permission);
        session.bridge(Bridge::Idle);
        assert_eq!(session.state(), State::Idle);
    }
}
