# Workflow state machine

The core workflow is a pure state machine. The UI and NFC worker send events;
the workflow returns the next screen and records when the session must be
persisted.

## States

| State | Meaning |
|---|---|
| `Startup` | Initial state. |
| `PlaylistSelect` | Waiting for a valid playlist. |
| `WaitingForTag` | NFC worker is waiting for a tag. |
| `Writing` | A validated tag is being written and verified. |
| `Success` | Read-back verified; waiting for explicit OK. |
| `RemoveTag` | Waiting for the programmed tag to be removed or replaced. |
| `Error` | Tag or setup error; actions depend on the error kind. |
| `Summary` | Final or partial summary. |
| `Finished` | Exit requested. |

## Transitions

| Event | From | To | Session effect |
|---|---|---|---|
| `start(None)` / `start(_, NoSession)` | `Startup` | `PlaylistSelect` | none |
| `start(session, Resume)` | `Startup` | `WaitingForTag` | adopts saved session |
| `start(session, Completed)` | `Startup` | `Summary` | shows completed summary |
| `start(_, PlaylistChanged/Missing)` | `Startup` | `Error` | no resume; setup error |
| `playlist_loaded(session)` | `PlaylistSelect` | `WaitingForTag` | new session |
| `playlist_failed(error)` | any setup | `Error` | none |
| `tag_validated(model)` | `WaitingForTag` | `Writing` | none |
| `tag_rejected(error)` | `WaitingForTag` | `Error` | failed attempts +1 |
| `write_succeeded()` | `Writing` | `Writing` | none (verification still pending) |
| `write_failed()` | `Writing` | `Error` | failed attempts +1 |
| `readback_failed()` | `Writing` | `Error` | failed attempts +1 |
| `verification_failed()` | `Writing` | `Error` | failed attempts +1 |
| `verification_succeeded()` | `Writing` | `Success` | **none** |
| `confirm_success()` | `Success` | `RemoveTag` | completed +1, next +1, dirty |
| `same_tag_detected()` | `RemoveTag` | `RemoveTag` | none |
| `tag_cleared()` | `RemoveTag` | `WaitingForTag` or `Summary` | none |
| `retry()` | `Error` (tag) | `WaitingForTag` | none |
| `skip()` | `Error` (tag) | `WaitingForTag` or `Summary` | skipped +1, next +1, dirty |
| `abort()` | any active | `Summary` | dirty; not complete |
| `finish()` | `Summary` | `Finished` | none |

## Invariants

- One URL is current until it is verified and confirmed, or explicitly skipped.
- A write that is not read back and verified never advances.
- The success screen appears only after verification.
- OK on the success screen is the only input that advances a successful entry.
  Back does not advance.
- The programmed tag must be removed or replaced (different UID) before the
  next tag is accepted.
- Retry does not change the index or the completed/skipped counters. It does
  leave the failed-attempt count incremented in memory; that counter is written
  with the next verified/skip/abort save.
- Skip requires an explicit choice and is persisted.
- Abort persists current progress and leaves the session resumable.
- Setup errors (playlist, session, storage) never offer tag Retry.
- The final summary reports total, completed, skipped, failed attempts, and
  remaining. An aborted session reports a partial summary and is not complete.

The application never restarts the playlist automatically after the summary. A
completed summary offers `New`; an aborted summary offers only `Done` so its
progress stays resumable.
