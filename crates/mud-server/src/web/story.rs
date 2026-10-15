//! Browser adapter for story sessions (ADR 0029). The WebSocket session owns
//! one [`WebStorySession`]; while it holds a link every story input goes to the
//! story instance and never to the world actor.

use super::{OutboundHandle, OutboundPriority, queue_action_result, queue_sequenced};
use crate::story_sessions::{
    STORY_INPUT_HELP, StoryCommand, StoryFailure, StoryInputCommand, StoryLink, StoryManager,
    StoryProjection, StoryRequest, parse_story_command, parse_story_input, render_listing,
};
use tiamud_core::AccountId;
use tiamud_protocol::{self as protocol, ServerMessageType, StoryLeftReason};

pub(super) struct WebStorySession {
    manager: Option<StoryManager>,
    account: AccountId,
    audio: bool,
    link: Option<StoryLink>,
    /// Last projection sent, used to map typed choice numbers to opaque ids.
    last_view: Option<StoryProjection>,
}

impl WebStorySession {
    pub(super) fn new(manager: Option<StoryManager>, account: AccountId, audio: bool) -> Self {
        Self {
            manager,
            account,
            audio,
            link: None,
            last_view: None,
        }
    }

    /// True while input is routed to a story instance.
    pub(super) fn active(&self) -> bool {
        self.link.is_some()
    }

    /// Re-attaches a reconnecting session to its account's live instance.
    pub(super) async fn resume(&mut self, outbound: &OutboundHandle, sequence: &mut u64) -> bool {
        let Some(link) = self
            .manager
            .as_ref()
            .and_then(|manager| manager.attach(self.account))
        else {
            return true;
        };
        self.link = Some(link.clone());
        queue_entered(outbound, sequence, &link) && self.resend(outbound, sequence).await
    }

    /// Re-sends the authoritative current view (entry, reconnect, sync).
    pub(super) async fn resend(&mut self, outbound: &OutboundHandle, sequence: &mut u64) -> bool {
        let Some(link) = self.link.clone() else {
            return true;
        };
        match link.view().await {
            Ok(view) => self.publish(outbound, sequence, view),
            Err(failure) => self.fail(outbound, sequence, failure).await,
        }
    }

    /// `story.choose`, `story.dialogue` or `story.menu`.
    pub(super) async fn input(
        &mut self,
        outbound: &OutboundHandle,
        sequence: &mut u64,
        request: StoryRequest,
        view_rev: u32,
    ) -> bool {
        let Some(link) = self.link.clone() else {
            return queue_error(outbound, sequence, StoryFailure::NotInStory);
        };
        match link.input(request, view_rev).await {
            Ok(view) => self.publish(outbound, sequence, view),
            Err(failure) => self.fail(outbound, sequence, failure).await,
        }
    }

    pub(super) async fn undo(&mut self, outbound: &OutboundHandle, sequence: &mut u64) -> bool {
        let Some(link) = self.link.clone() else {
            return queue_error(outbound, sequence, StoryFailure::NotInStory);
        };
        match link.undo().await {
            Ok(view) => self.publish(outbound, sequence, view),
            Err(failure) => self.fail(outbound, sequence, failure).await,
        }
    }

    pub(super) async fn leave(&mut self, outbound: &OutboundHandle, sequence: &mut u64) -> bool {
        let Some(link) = self.link.take() else {
            return queue_error(outbound, sequence, StoryFailure::NotInStory);
        };
        self.last_view = None;
        // A vanished instance is already gone; either way the session leaves.
        let reason = match link.leave().await {
            Ok(()) => StoryLeftReason::Left,
            Err(_) => StoryLeftReason::Evicted,
        };
        queue_left(outbound, sequence, reason, self.audio)
    }

    /// Handles a `command.submit` line. Returns `None` when the line is an
    /// ordinary world command (the session is not in a story and the line is
    /// not a story command); otherwise the line was answered here.
    pub(super) async fn text_command(
        &mut self,
        outbound: &OutboundHandle,
        sequence: &mut u64,
        request_id: &str,
        text: &str,
    ) -> Option<bool> {
        if self.link.is_some() {
            return Some(
                self.in_story_text(outbound, sequence, request_id, text)
                    .await,
            );
        }
        let command = parse_story_command(text)?;
        let Some(manager) = self.manager.clone() else {
            return Some(queue_action_result(
                outbound,
                sequence,
                request_id,
                false,
                Some(StoryFailure::Disabled.message().to_owned()),
            ));
        };
        Some(match command {
            StoryCommand::List => queue_action_result(
                outbound,
                sequence,
                request_id,
                true,
                Some(render_listing(&manager.listing())),
            ),
            StoryCommand::Leave => queue_action_result(
                outbound,
                sequence,
                request_id,
                false,
                Some(StoryFailure::NotInStory.message().to_owned()),
            ),
            StoryCommand::Enter(story_id) => match manager.enter(self.account, &story_id) {
                Ok(link) => {
                    let message = format!("Entras en «{}».", link.title());
                    self.link = Some(link.clone());
                    queue_action_result(outbound, sequence, request_id, true, Some(message))
                        && queue_entered(outbound, sequence, &link)
                        && self.resend(outbound, sequence).await
                }
                Err(failure) => queue_action_result(
                    outbound,
                    sequence,
                    request_id,
                    false,
                    Some(failure.message().to_owned()),
                ),
            },
        })
    }

    async fn in_story_text(
        &mut self,
        outbound: &OutboundHandle,
        sequence: &mut u64,
        request_id: &str,
        text: &str,
    ) -> bool {
        let result = |ok: bool, message: &str, outbound: &OutboundHandle, sequence: &mut u64| {
            queue_action_result(outbound, sequence, request_id, ok, Some(message.to_owned()))
        };
        match parse_story_input(text) {
            StoryInputCommand::Number(number) => {
                let action = self.last_view.as_ref().and_then(|view| {
                    view.actions()
                        .get(number - 1)
                        .map(|action| (action.request(), action.label().to_owned(), view.view_rev))
                });
                let Some((request, label, view_rev)) = action else {
                    return result(
                        false,
                        StoryFailure::UnknownChoice.message(),
                        outbound,
                        sequence,
                    );
                };
                result(true, &format!("Eliges: {label}"), outbound, sequence)
                    && self.input(outbound, sequence, request, view_rev).await
            }
            StoryInputCommand::Undo => {
                result(true, "Deshaces la última elección.", outbound, sequence)
                    && self.undo(outbound, sequence).await
            }
            StoryInputCommand::Leave => {
                result(true, "Sales de la historia.", outbound, sequence)
                    && self.leave(outbound, sequence).await
            }
            StoryInputCommand::Look => {
                result(true, "Vuelves a mirar.", outbound, sequence)
                    && self.resend(outbound, sequence).await
            }
            StoryInputCommand::List => {
                let listing = self
                    .manager
                    .as_ref()
                    .map(|manager| render_listing(&manager.listing()))
                    .unwrap_or_default();
                result(true, &listing, outbound, sequence)
            }
            StoryInputCommand::Other => result(false, STORY_INPUT_HELP, outbound, sequence),
        }
    }

    fn publish(
        &mut self,
        outbound: &OutboundHandle,
        sequence: &mut u64,
        view: StoryProjection,
    ) -> bool {
        let sent = queue_view(outbound, sequence, &view, self.audio);
        self.last_view = Some(view);
        sent
    }

    async fn fail(
        &mut self,
        outbound: &OutboundHandle,
        sequence: &mut u64,
        failure: StoryFailure,
    ) -> bool {
        match failure {
            StoryFailure::Gone => {
                self.link = None;
                self.last_view = None;
                queue_error(outbound, sequence, failure)
                    && queue_left(outbound, sequence, StoryLeftReason::Evicted, self.audio)
            }
            // A stale view is answered with the authoritative current one.
            StoryFailure::StaleView => {
                queue_error(outbound, sequence, failure)
                    && Box::pin(self.resend(outbound, sequence)).await
            }
            _ => queue_error(outbound, sequence, failure),
        }
    }
}

fn queue_entered(outbound: &OutboundHandle, sequence: &mut u64, link: &StoryLink) -> bool {
    queue_sequenced(
        outbound,
        sequence,
        OutboundPriority::Critical,
        None,
        ServerMessageType::StoryEntered,
        protocol::StoryEntered {
            story_id: link.story_id().to_owned(),
            title: link.title().to_owned(),
        },
    )
}

/// Queues every part of a view. Each part is below the 16 KiB frame budget.
pub(super) fn queue_view(
    outbound: &OutboundHandle,
    sequence: &mut u64,
    view: &StoryProjection,
    audio: bool,
) -> bool {
    view.web_parts(audio).into_iter().all(|part| {
        queue_sequenced(
            outbound,
            sequence,
            OutboundPriority::Critical,
            None,
            ServerMessageType::StoryView,
            part,
        )
    })
}

fn queue_left(
    outbound: &OutboundHandle,
    sequence: &mut u64,
    reason: StoryLeftReason,
    audio: bool,
) -> bool {
    // Story music never outlives the story.
    (!audio
        || queue_sequenced(
            outbound,
            sequence,
            OutboundPriority::Critical,
            None,
            ServerMessageType::MediaStop,
            protocol::MediaStop {
                channel: protocol::MediaChannel::Music,
            },
        ))
        && queue_sequenced(
            outbound,
            sequence,
            OutboundPriority::Critical,
            None,
            ServerMessageType::StoryLeft,
            protocol::StoryLeft { reason },
        )
}

pub(super) fn queue_error(
    outbound: &OutboundHandle,
    sequence: &mut u64,
    failure: StoryFailure,
) -> bool {
    queue_sequenced(
        outbound,
        sequence,
        OutboundPriority::Critical,
        None,
        ServerMessageType::StoryError,
        failure.protocol_error(),
    )
}

#[cfg(test)]
mod tests;
