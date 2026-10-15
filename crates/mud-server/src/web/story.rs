//! Browser adapter for story sessions (ADR 0029). The WebSocket session owns
//! one [`WebStorySession`]; while it holds a link every story input goes to the
//! story instance and never to the world actor.
//!
//! In a shared instance (ADR 0033) the session also follows the instance's
//! `watch` of shared frames ([`WebStorySession::next_event`]) and turns each
//! change into `story.lobby`, `story.view` and `story.tally` frames for this
//! participant; recovery re-sends all three.

use std::sync::Arc;

use super::{OutboundHandle, OutboundPriority, queue_action_result, queue_sequenced};
use crate::story_sessions::{
    CommandFollowUp, InvitationInbox, SHARED_STORY_HELP, STORY_INPUT_HELP, SharedFrame,
    StoryCommand, StoryFailure, StoryInputCommand, StoryLink, StoryManager, StoryProjection,
    StoryRequest, parse_parser_story_input, parse_story_command, parse_story_input, render_listing,
    render_lobby, render_slots,
};
use tiamud_core::AccountId;
use tiamud_protocol::{self as protocol, ServerMessageType, StoryDecisionMode, StoryLeftReason};
use tokio::sync::watch;

/// Something the story side has to tell this session without a request.
pub(super) enum StoryEvent {
    /// The shared frame changed.
    Frame,
    /// The shared instance's frames ended.
    Closed,
    Invitation(protocol::StoryInvitation),
    /// Nothing to do (an inbox closed).
    Nothing,
}

pub(super) struct WebStorySession {
    manager: Option<StoryManager>,
    account: AccountId,
    /// Character name used for seats and invitations.
    name: String,
    audio: bool,
    link: Option<StoryLink>,
    /// Last projection sent, used to map typed choice numbers to opaque ids.
    last_view: Option<StoryProjection>,
    frames: Option<watch::Receiver<Arc<SharedFrame>>>,
    inbox: Option<InvitationInbox>,
    /// Highest view revision published to this client.
    sent_view_rev: u32,
    sent_lobby_rev: u64,
    sent_tally: (u64, u32),
}

impl WebStorySession {
    pub(super) fn new(manager: Option<StoryManager>, account: AccountId, audio: bool) -> Self {
        Self {
            manager,
            account,
            name: String::new(),
            audio,
            link: None,
            last_view: None,
            frames: None,
            inbox: None,
            sent_view_rev: 0,
            sent_lobby_rev: 0,
            sent_tally: (0, 0),
        }
    }

    /// Names the session's character and registers its invitation inbox.
    pub(super) fn with_name(mut self, name: &str) -> Self {
        self.name = name.to_owned();
        self.inbox = self
            .manager
            .as_ref()
            .and_then(|manager| manager.register_inbox(name));
        self
    }

    /// True while input is routed to a story instance.
    pub(super) fn active(&self) -> bool {
        self.link.is_some()
    }

    fn set_link(&mut self, link: StoryLink) {
        self.frames = link.frames();
        self.link = Some(link);
        self.sent_view_rev = 0;
        self.sent_lobby_rev = 0;
        self.sent_tally = (0, 0);
    }

    fn clear_link(&mut self) {
        self.link = None;
        self.last_view = None;
        self.frames = None;
    }

    /// Waits for a shared frame change or an invitation. Pending forever
    /// while there is neither a shared instance nor an inbox.
    pub(super) async fn next_event(&mut self) -> StoryEvent {
        let frames = self.frames.as_mut();
        let inbox = self.inbox.as_mut();
        tokio::select! {
            changed = async {
                match frames {
                    Some(frames) => frames.changed().await,
                    None => std::future::pending().await,
                }
            } => match changed {
                Ok(()) => StoryEvent::Frame,
                Err(_) => StoryEvent::Closed,
            },
            invitation = async {
                match inbox {
                    Some(inbox) => inbox.recv().await,
                    None => std::future::pending().await,
                }
            } => match invitation {
                Some(invitation) => StoryEvent::Invitation(invitation),
                None => {
                    self.inbox = None;
                    StoryEvent::Nothing
                }
            },
        }
    }

    /// Handles one [`StoryEvent`]. Returns `false` when the client is gone.
    pub(super) fn handle_event(
        &mut self,
        outbound: &OutboundHandle,
        sequence: &mut u64,
        event: StoryEvent,
    ) -> bool {
        match event {
            StoryEvent::Frame => {
                let Some(frame) = self
                    .frames
                    .as_mut()
                    .map(|frames| frames.borrow_and_update().clone())
                else {
                    return true;
                };
                if let Some(reason) = frame.removal(self.account) {
                    self.clear_link();
                    return queue_left(outbound, sequence, reason, self.audio);
                }
                self.push_frame(outbound, sequence, &frame, false)
            }
            StoryEvent::Closed => {
                if self.frames.is_none() {
                    return true;
                }
                self.clear_link();
                queue_left(outbound, sequence, StoryLeftReason::Ended, self.audio)
            }
            StoryEvent::Invitation(invitation) => queue_sequenced(
                outbound,
                sequence,
                OutboundPriority::Narrative,
                None,
                ServerMessageType::StoryInvitation,
                invitation,
            ),
            StoryEvent::Nothing => true,
        }
    }

    /// Sends what changed in `frame` since the last one (or all of it).
    fn push_frame(
        &mut self,
        outbound: &OutboundHandle,
        sequence: &mut u64,
        frame: &SharedFrame,
        force: bool,
    ) -> bool {
        let mut sent = true;
        if force || frame.table.lobby_rev != self.sent_lobby_rev {
            self.sent_lobby_rev = frame.table.lobby_rev;
            sent &= queue_lobby(outbound, sequence, frame.lobby(self.account));
        }
        if frame.view.view_rev > self.sent_view_rev {
            sent &= self.publish(outbound, sequence, frame.view.clone());
        }
        let tally = (frame.table.tally_rev, frame.view.view_rev);
        if force || tally != self.sent_tally {
            self.sent_tally = tally;
            sent &= queue_tally(outbound, sequence, frame.tally(self.account));
        }
        sent
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
        self.set_link(link.clone());
        queue_entered(outbound, sequence, &link) && self.resend(outbound, sequence).await
    }

    /// Re-sends the authoritative current state (entry, reconnect, sync): the
    /// lobby, the view and the tally of a shared instance, or the view.
    pub(super) async fn resend(&mut self, outbound: &OutboundHandle, sequence: &mut u64) -> bool {
        let Some(link) = self.link.clone() else {
            return true;
        };
        let view = match link.view().await {
            Ok(view) => view,
            Err(failure) => return self.fail(outbound, sequence, failure).await,
        };
        let Some(frame) = self
            .frames
            .as_mut()
            .map(|frames| frames.borrow_and_update().clone())
        else {
            return self.publish(outbound, sequence, view);
        };
        if let Some(reason) = frame.removal(self.account) {
            self.clear_link();
            return queue_left(outbound, sequence, reason, self.audio);
        }
        self.sent_lobby_rev = frame.table.lobby_rev;
        self.sent_tally = (frame.table.tally_rev, frame.view.view_rev);
        queue_lobby(outbound, sequence, frame.lobby(self.account))
            && self.publish(outbound, sequence, view)
            && queue_tally(outbound, sequence, frame.tally(self.account))
    }

    /// Answers an accepted input. A solo instance publishes the reply's
    /// view. A shared one applies the latest frame now (the instance
    /// publishes it before replying) and always answers with at least the
    /// current tally, so an unchanged vote still acknowledges the request.
    fn publish_reply(
        &mut self,
        outbound: &OutboundHandle,
        sequence: &mut u64,
        view: StoryProjection,
    ) -> bool {
        let Some(frame) = self
            .frames
            .as_mut()
            .map(|frames| frames.borrow_and_update().clone())
        else {
            return self.publish(outbound, sequence, view);
        };
        if let Some(reason) = frame.removal(self.account) {
            self.clear_link();
            return queue_left(outbound, sequence, reason, self.audio);
        }
        let before = self.sent_tally;
        let mut sent = self.push_frame(outbound, sequence, &frame, false);
        if view.view_rev > self.sent_view_rev {
            sent &= self.publish(outbound, sequence, view);
        }
        if self.sent_tally == before {
            sent &= queue_tally(outbound, sequence, frame.tally(self.account));
        }
        sent
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
            Ok(view) => self.publish_reply(outbound, sequence, view),
            Err(failure) => self.fail(outbound, sequence, failure).await,
        }
    }

    /// `story.command {text, viewRev}`: one parser line. The view answers it
    /// (echo and replies as blocks); `guardar`/`cargar` without a slot also
    /// list the slots and the parser's quit command leaves the story.
    pub(super) async fn command(
        &mut self,
        outbound: &OutboundHandle,
        sequence: &mut u64,
        text: &str,
        view_rev: u32,
    ) -> bool {
        let Some(link) = self.link.clone() else {
            return queue_error(outbound, sequence, StoryFailure::NotInStory);
        };
        match link.command(text, view_rev).await {
            Ok(reply) => {
                self.publish_reply(outbound, sequence, reply.view)
                    && match reply.follow_up {
                        None => true,
                        Some(CommandFollowUp::Slots) => self.slots(outbound, sequence).await,
                        Some(CommandFollowUp::Leave) => self.leave(outbound, sequence).await,
                    }
            }
            Err(failure) => self.fail(outbound, sequence, failure).await,
        }
    }

    /// `story.vote {choiceId?, viewRev}`.
    pub(super) async fn vote(
        &mut self,
        outbound: &OutboundHandle,
        sequence: &mut u64,
        leaf: Option<String>,
        view_rev: u32,
    ) -> bool {
        let Some(link) = self.link.clone() else {
            return queue_error(outbound, sequence, StoryFailure::NotInStory);
        };
        match link.vote(leaf, view_rev).await {
            Ok(view) => self.publish_reply(outbound, sequence, view),
            Err(failure) => self.fail(outbound, sequence, failure).await,
        }
    }

    pub(super) async fn undo(&mut self, outbound: &OutboundHandle, sequence: &mut u64) -> bool {
        let Some(link) = self.link.clone() else {
            return queue_error(outbound, sequence, StoryFailure::NotInStory);
        };
        match link.undo().await {
            Ok(view) => self.publish_reply(outbound, sequence, view),
            Err(failure) => self.fail(outbound, sequence, failure).await,
        }
    }

    pub(super) async fn leave(&mut self, outbound: &OutboundHandle, sequence: &mut u64) -> bool {
        let Some(link) = self.link.take() else {
            return queue_error(outbound, sequence, StoryFailure::NotInStory);
        };
        self.clear_link();
        // A vanished instance is already gone; either way the session leaves.
        // Leaving writes the autosave first when the story had progress.
        let reason = match link.leave().await {
            Ok(_) => StoryLeftReason::Left,
            Err(_) => StoryLeftReason::Evicted,
        };
        queue_left(outbound, sequence, reason, self.audio)
    }

    /// `story.share {}`: shares the current instance.
    pub(super) async fn share(&mut self, outbound: &OutboundHandle, sequence: &mut u64) -> bool {
        let Some(link) = self.link.clone() else {
            return queue_error(outbound, sequence, StoryFailure::NotInStory);
        };
        match link.share(&self.name).await {
            Ok(shared) => {
                let last = self.sent_view_rev;
                self.set_link(shared);
                self.sent_view_rev = last;
                self.resend(outbound, sequence).await
            }
            Err(failure) => self.fail(outbound, sequence, failure).await,
        }
    }

    /// `story.join {code, spectate}`.
    pub(super) async fn join(
        &mut self,
        outbound: &OutboundHandle,
        sequence: &mut u64,
        code: &str,
        spectate: bool,
    ) -> bool {
        if self.link.is_some() {
            return queue_error(outbound, sequence, StoryFailure::AlreadyInStory);
        }
        let Some(manager) = self.manager.clone() else {
            return queue_error(outbound, sequence, StoryFailure::Disabled);
        };
        match manager.join(self.account, &self.name, code, spectate).await {
            Ok(link) => {
                self.set_link(link.clone());
                queue_entered(outbound, sequence, &link) && self.resend(outbound, sequence).await
            }
            Err(failure) => queue_error(outbound, sequence, failure),
        }
    }

    /// `story.invite {name}` (host).
    pub(super) async fn invite(
        &mut self,
        outbound: &OutboundHandle,
        sequence: &mut u64,
        name: &str,
    ) -> bool {
        let Some(link) = self.link.clone() else {
            return queue_error(outbound, sequence, StoryFailure::NotInStory);
        };
        match link.invite(name).await {
            Ok(()) => true,
            Err(failure) => self.fail(outbound, sequence, failure).await,
        }
    }

    /// `story.kick {name}` (host).
    pub(super) async fn kick(
        &mut self,
        outbound: &OutboundHandle,
        sequence: &mut u64,
        name: &str,
    ) -> bool {
        let Some(link) = self.link.clone() else {
            return queue_error(outbound, sequence, StoryFailure::NotInStory);
        };
        match link.kick(name).await {
            Ok(()) => true,
            Err(failure) => self.fail(outbound, sequence, failure).await,
        }
    }

    /// `story.mode {mode, voteSeconds?}` (host).
    pub(super) async fn mode(
        &mut self,
        outbound: &OutboundHandle,
        sequence: &mut u64,
        mode: StoryDecisionMode,
        vote_seconds: Option<u16>,
    ) -> bool {
        let Some(link) = self.link.clone() else {
            return queue_error(outbound, sequence, StoryFailure::NotInStory);
        };
        match link.set_mode(mode, vote_seconds).await {
            Ok(()) => true,
            Err(failure) => self.fail(outbound, sequence, failure).await,
        }
    }

    /// `story.save {slot}`: answered by `story.saved` or `story.error`.
    pub(super) async fn save(
        &mut self,
        outbound: &OutboundHandle,
        sequence: &mut u64,
        slot: u8,
    ) -> bool {
        let Some(link) = self.link.clone() else {
            return queue_error(outbound, sequence, StoryFailure::NotInStory);
        };
        match link.save(slot).await {
            Ok(saved) => queue_saved(outbound, sequence, saved),
            Err(failure) => self.fail(outbound, sequence, failure).await,
        }
    }

    /// `story.restore {slot}` and `story.continue` (slot 0): answered by
    /// `story.restored` followed by the restored `story.view`.
    pub(super) async fn restore(
        &mut self,
        outbound: &OutboundHandle,
        sequence: &mut u64,
        slot: u8,
    ) -> bool {
        let Some(link) = self.link.clone() else {
            return queue_error(outbound, sequence, StoryFailure::NotInStory);
        };
        match link.restore(slot).await {
            Ok(view) => {
                queue_restored(outbound, sequence, slot)
                    && self.publish_reply(outbound, sequence, view)
            }
            Err(failure) => self.fail(outbound, sequence, failure).await,
        }
    }

    /// `story.slots {}`: the occupied slots of the current story.
    pub(super) async fn slots(&mut self, outbound: &OutboundHandle, sequence: &mut u64) -> bool {
        let Some(link) = self.link.clone() else {
            return queue_error(outbound, sequence, StoryFailure::NotInStory);
        };
        match link.slots().await {
            Ok(slots) => queue_slots(outbound, sequence, slots, false),
            Err(failure) => self.fail(outbound, sequence, failure).await,
        }
    }

    /// `story.deleteSlot {slot}`: answered by the remaining `story.slots`.
    pub(super) async fn delete_slot(
        &mut self,
        outbound: &OutboundHandle,
        sequence: &mut u64,
        slot: u8,
    ) -> bool {
        let Some(link) = self.link.clone() else {
            return queue_error(outbound, sequence, StoryFailure::NotInStory);
        };
        match link.delete_slot(slot).await {
            Ok(slots) => queue_slots(outbound, sequence, slots, false),
            Err(failure) => self.fail(outbound, sequence, failure).await,
        }
    }

    /// After entering a fresh instance: the slot listing with the continue
    /// offer. Nothing is sent without a store or on a listing failure.
    async fn offer(&mut self, outbound: &OutboundHandle, sequence: &mut u64) -> bool {
        let Some(link) = self.link.clone() else {
            return true;
        };
        match link.entry_slots().await {
            Some(slots) => queue_slots(outbound, sequence, slots.slots, slots.offer_continue),
            None => true,
        }
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
        let failed = |failure: StoryFailure, outbound: &OutboundHandle, sequence: &mut u64| {
            queue_action_result(
                outbound,
                sequence,
                request_id,
                false,
                Some(failure.message().to_owned()),
            )
        };
        Some(match command {
            StoryCommand::List => queue_action_result(
                outbound,
                sequence,
                request_id,
                true,
                Some(render_listing(&manager.listing())),
            ),
            StoryCommand::Leave => failed(StoryFailure::NotInStory, outbound, sequence),
            StoryCommand::Enter(story_id) => match manager.enter(self.account, &story_id) {
                Ok(link) => {
                    let message = format!("Entras en «{}».", link.title());
                    self.set_link(link.clone());
                    queue_action_result(outbound, sequence, request_id, true, Some(message))
                        && queue_entered(outbound, sequence, &link)
                        && self.resend(outbound, sequence).await
                        && self.offer(outbound, sequence).await
                }
                Err(failure) => failed(failure, outbound, sequence),
            },
            StoryCommand::Host(story_id) => {
                match manager.host(self.account, &self.name, &story_id).await {
                    Ok(link) => {
                        let message = format!(
                            "Compartes «{}». {}",
                            link.title(),
                            link.frames()
                                .map(|frames| render_lobby(&frames.borrow().lobby(self.account)))
                                .unwrap_or_default()
                        );
                        self.set_link(link.clone());
                        queue_action_result(outbound, sequence, request_id, true, Some(message))
                            && queue_entered(outbound, sequence, &link)
                            && self.resend(outbound, sequence).await
                    }
                    Err(failure) => failed(failure, outbound, sequence),
                }
            }
            StoryCommand::Join { code, spectate } => {
                match manager
                    .join(self.account, &self.name, &code, spectate)
                    .await
                {
                    Ok(link) => {
                        let message = if spectate {
                            format!("Miras «{}» como espectador.", link.title())
                        } else {
                            format!("Te unes a «{}».", link.title())
                        };
                        self.set_link(link.clone());
                        queue_action_result(outbound, sequence, request_id, true, Some(message))
                            && queue_entered(outbound, sequence, &link)
                            && self.resend(outbound, sequence).await
                    }
                    Err(failure) => failed(failure, outbound, sequence),
                }
            }
        })
    }

    fn frame(&self) -> Option<Arc<SharedFrame>> {
        self.frames.as_ref().map(|frames| frames.borrow().clone())
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
        let Some(link) = self.link.clone() else {
            return result(
                false,
                StoryFailure::NotInStory.message(),
                outbound,
                sequence,
            );
        };
        let parsed = if link.is_parser() {
            parse_parser_story_input(text)
        } else {
            parse_story_input(text)
        };
        match parsed {
            StoryInputCommand::Other if link.is_parser() => {
                // Free text of a parser story: the view echoes and answers it.
                let view_rev = self.last_view.as_ref().map_or(0, |view| view.view_rev);
                if text.len() > crate::story_sessions::MAX_COMMAND_BYTES {
                    return self
                        .text_failure(outbound, sequence, request_id, StoryFailure::InvalidCommand)
                        .await;
                }
                queue_action_result(outbound, sequence, request_id, true, None)
                    && self.command(outbound, sequence, text, view_rev).await
            }
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
                let voting = self
                    .frame()
                    .is_some_and(|frame| frame.table.mode() == StoryDecisionMode::Vote);
                let message = if voting {
                    format!("Votas por: {label}")
                } else {
                    format!("Eliges: {label}")
                };
                result(true, &message, outbound, sequence)
                    && self.input(outbound, sequence, request, view_rev).await
            }
            StoryInputCommand::Vote(number) => {
                let target = self.last_view.as_ref().and_then(|view| match number {
                    Some(0) => Some((None, view.view_rev)),
                    Some(number) => view
                        .actions()
                        .get(number - 1)
                        .map(|action| (Some(action.leaf_id().to_owned()), view.view_rev)),
                    None => None,
                });
                let Some((leaf, view_rev)) = target else {
                    return result(false, SHARED_STORY_HELP, outbound, sequence);
                };
                let message = if leaf.is_some() {
                    "Votas."
                } else {
                    "Retiras tu voto."
                };
                result(true, message, outbound, sequence)
                    && self.vote(outbound, sequence, leaf, view_rev).await
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
            StoryInputCommand::Seats => match self.frame() {
                Some(frame) => {
                    let lobby = frame.lobby(self.account);
                    result(true, &render_lobby(&lobby), outbound, sequence)
                        && queue_lobby(outbound, sequence, lobby)
                }
                None => result(false, StoryFailure::NotShared.message(), outbound, sequence),
            },
            StoryInputCommand::Share => match link.share(&self.name).await {
                Ok(shared) => {
                    let lobby = shared
                        .frames()
                        .map(|frames| render_lobby(&frames.borrow().lobby(self.account)))
                        .unwrap_or_default();
                    let last = self.sent_view_rev;
                    self.set_link(shared);
                    self.sent_view_rev = last;
                    result(true, &lobby, outbound, sequence)
                        && self.resend(outbound, sequence).await
                }
                Err(failure) => {
                    self.text_failure(outbound, sequence, request_id, failure)
                        .await
                }
            },
            StoryInputCommand::Invite(name) => match link.invite(&name).await {
                Ok(()) => result(
                    true,
                    &format!("Invitación enviada a {name}, si está conectado."),
                    outbound,
                    sequence,
                ),
                Err(failure) => {
                    self.text_failure(outbound, sequence, request_id, failure)
                        .await
                }
            },
            StoryInputCommand::Kick(name) => match link.kick(&name).await {
                Ok(()) => result(
                    true,
                    &format!("{name} ya no participa en la historia."),
                    outbound,
                    sequence,
                ),
                Err(failure) => {
                    self.text_failure(outbound, sequence, request_id, failure)
                        .await
                }
            },
            StoryInputCommand::Mode(Some((mode, seconds))) => {
                match link.set_mode(mode, seconds).await {
                    Ok(()) => result(
                        true,
                        &format!(
                            "Modo de decisión: {}.",
                            crate::story_sessions::mode_label(mode)
                        ),
                        outbound,
                        sequence,
                    ),
                    Err(failure) => {
                        self.text_failure(outbound, sequence, request_id, failure)
                            .await
                    }
                }
            }
            StoryInputCommand::Mode(None) => result(
                false,
                StoryFailure::InvalidSetting.message(),
                outbound,
                sequence,
            ),
            StoryInputCommand::Save(slot) => match link.save(slot.unwrap_or(u8::MAX)).await {
                Ok(saved) => {
                    result(
                        true,
                        &format!("Partida guardada en la ranura {}.", saved.slot),
                        outbound,
                        sequence,
                    ) && queue_saved(outbound, sequence, saved)
                }
                Err(failure) => {
                    self.text_failure(outbound, sequence, request_id, failure)
                        .await
                }
            },
            StoryInputCommand::Restore(slot) => {
                self.text_restore(outbound, sequence, request_id, slot.unwrap_or(u8::MAX))
                    .await
            }
            StoryInputCommand::Continue => {
                self.text_restore(outbound, sequence, request_id, 0).await
            }
            StoryInputCommand::Slots => match link.slots().await {
                Ok(slots) => {
                    result(true, &render_slots(&slots), outbound, sequence)
                        && queue_slots(outbound, sequence, slots, false)
                }
                Err(failure) => {
                    self.text_failure(outbound, sequence, request_id, failure)
                        .await
                }
            },
            StoryInputCommand::Delete(slot) => {
                let slot = slot.unwrap_or(u8::MAX);
                match link.delete_slot(slot).await {
                    Ok(slots) => {
                        result(
                            true,
                            &format!("Partida de la ranura {slot} borrada."),
                            outbound,
                            sequence,
                        ) && queue_slots(outbound, sequence, slots, false)
                    }
                    Err(failure) => {
                        self.text_failure(outbound, sequence, request_id, failure)
                            .await
                    }
                }
            }
            StoryInputCommand::Other => {
                let help = if link.is_shared() {
                    SHARED_STORY_HELP
                } else {
                    STORY_INPUT_HELP
                };
                result(false, help, outbound, sequence)
            }
        }
    }

    async fn text_restore(
        &mut self,
        outbound: &OutboundHandle,
        sequence: &mut u64,
        request_id: &str,
        slot: u8,
    ) -> bool {
        let Some(link) = self.link.clone() else {
            return queue_action_result(
                outbound,
                sequence,
                request_id,
                false,
                Some(StoryFailure::NotInStory.message().to_owned()),
            );
        };
        match link.restore(slot).await {
            Ok(view) => {
                let message = if slot == 0 {
                    "Continúas la partida autoguardada.".to_owned()
                } else {
                    format!("Cargas la partida de la ranura {slot}.")
                };
                queue_action_result(outbound, sequence, request_id, true, Some(message))
                    && queue_restored(outbound, sequence, slot)
                    && self.publish_reply(outbound, sequence, view)
            }
            Err(failure) => {
                self.text_failure(outbound, sequence, request_id, failure)
                    .await
            }
        }
    }

    /// A failed text slot command: the action result carries the message and
    /// the typed `story.error` follows (with its usual recovery).
    async fn text_failure(
        &mut self,
        outbound: &OutboundHandle,
        sequence: &mut u64,
        request_id: &str,
        failure: StoryFailure,
    ) -> bool {
        queue_action_result(
            outbound,
            sequence,
            request_id,
            false,
            Some(failure.message().to_owned()),
        ) && self.fail(outbound, sequence, failure).await
    }

    fn publish(
        &mut self,
        outbound: &OutboundHandle,
        sequence: &mut u64,
        view: StoryProjection,
    ) -> bool {
        // Only the host of a shared instance may undo.
        let view = match self.frame() {
            Some(frame) => frame.view_for(self.account, view),
            None => view,
        };
        self.sent_view_rev = self.sent_view_rev.max(view.view_rev);
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
                self.clear_link();
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

fn queue_lobby(outbound: &OutboundHandle, sequence: &mut u64, lobby: protocol::StoryLobby) -> bool {
    queue_sequenced(
        outbound,
        sequence,
        OutboundPriority::Critical,
        None,
        ServerMessageType::StoryLobby,
        lobby,
    )
}

fn queue_tally(outbound: &OutboundHandle, sequence: &mut u64, tally: protocol::StoryTally) -> bool {
    queue_sequenced(
        outbound,
        sequence,
        OutboundPriority::Critical,
        None,
        ServerMessageType::StoryTally,
        tally,
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

fn queue_saved(outbound: &OutboundHandle, sequence: &mut u64, saved: protocol::StorySaved) -> bool {
    queue_sequenced(
        outbound,
        sequence,
        OutboundPriority::Critical,
        None,
        ServerMessageType::StorySaved,
        saved,
    )
}

fn queue_restored(outbound: &OutboundHandle, sequence: &mut u64, slot: u8) -> bool {
    queue_sequenced(
        outbound,
        sequence,
        OutboundPriority::Critical,
        None,
        ServerMessageType::StoryRestored,
        protocol::StoryRestored { slot },
    )
}

fn queue_slots(
    outbound: &OutboundHandle,
    sequence: &mut u64,
    slots: Vec<protocol::StorySlotSummary>,
    offer_continue: bool,
) -> bool {
    queue_sequenced(
        outbound,
        sequence,
        OutboundPriority::Critical,
        None,
        ServerMessageType::StorySlots,
        protocol::StorySlots {
            slots,
            offer_continue,
        },
    )
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

#[cfg(test)]
mod shared_tests;
