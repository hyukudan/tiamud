//! Telnet adapter for story sessions (ADR 0029). Story views render as plain
//! text with numbered choices; typing a number chooses. GMCP clients that
//! negotiated `Tiamud.Story 1` also receive a bounded summary, and media
//! opt-ins receive story cues as `Client.Media`. Losing GMCP or media never
//! loses an action.
//!
//! Shared instances (ADR 0033) add text tallies, `votar N`/`vote N`,
//! `asientos`/`seats`, host commands and GMCP `Tiamud.Story.Lobby` and
//! `Tiamud.Story.Tally`; the session follows the instance's shared frames.

use std::sync::Arc;

use super::{
    MAX_GMCP_BYTES, OutputPriority, TelnetInput, TelnetOutputQueue, TelnetSession, append_gmcp,
    bounded_utf8_prefix, escape_negotiated_mxp_text,
};
use crate::story_sessions::{
    InvitationInbox, SHARED_STORY_HELP, STORY_INPUT_HELP, SharedFrame, StoryCommand, StoryFailure,
    StoryInputCommand, StoryLink, StoryManager, StoryProjection, mode_label, parse_story_command,
    parse_story_input, render_continue_offer, render_listing, render_lobby, render_slots,
    render_tally, render_text,
};
use serde_json::json;
use std::io;
use tiamud_core::AccountId;
use tiamud_protocol::{
    self as protocol, MediaChannel, StoryCueAction, StoryDecisionMode, StoryEndingKind, StoryItem,
    StoryLeftReason, StorySeatRole, StorySlotSummary, StorySlots,
};
use tokio::sync::watch;

/// Result of offering one line to the story adapter.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum StoryLineOutcome {
    /// Not a story line: the world handles it.
    World,
    /// Answered by the story adapter.
    Handled,
    /// The session left its story; the caller re-renders the world scene.
    Left,
}

/// Something the story side has to tell this session without a request.
pub(super) enum StoryEvent {
    Frame,
    Closed,
    Invitation(protocol::StoryInvitation),
    Nothing,
}

pub(super) struct TelnetStory {
    manager: Option<StoryManager>,
    account: AccountId,
    name: String,
    link: Option<StoryLink>,
    last_view: Option<StoryProjection>,
    /// True while story music may be playing through `Client.Media`.
    music: bool,
    frames: Option<watch::Receiver<Arc<SharedFrame>>>,
    inbox: Option<InvitationInbox>,
    sent_view_rev: u32,
    sent_lobby_rev: u64,
    sent_tally: (u64, u32),
}

impl TelnetStory {
    pub(super) fn new(manager: Option<StoryManager>, account: AccountId) -> Self {
        Self {
            manager,
            account,
            name: String::new(),
            link: None,
            last_view: None,
            music: false,
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

    fn frame(&self) -> Option<Arc<SharedFrame>> {
        self.frames.as_ref().map(|frames| frames.borrow().clone())
    }

    /// Waits for a shared frame change or an invitation.
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

    /// Writes what a [`StoryEvent`] changed. `Left` asks the caller to
    /// re-render the world; `World` means nothing was written.
    pub(super) fn handle_event(
        &mut self,
        output: &TelnetOutputQueue,
        input: &mut TelnetInput,
        event: StoryEvent,
    ) -> io::Result<StoryLineOutcome> {
        match event {
            StoryEvent::Frame => {
                let Some(frame) = self
                    .frames
                    .as_mut()
                    .map(|frames| frames.borrow_and_update().clone())
                else {
                    return Ok(StoryLineOutcome::World);
                };
                if let Some(reason) = frame.removal(self.account) {
                    return self.removed(output, input, reason);
                }
                self.push_frame(output, input, &frame, false)?;
                Ok(StoryLineOutcome::Handled)
            }
            StoryEvent::Closed if self.frames.is_some() => {
                self.removed(output, input, StoryLeftReason::Ended)
            }
            StoryEvent::Closed | StoryEvent::Nothing => Ok(StoryLineOutcome::World),
            StoryEvent::Invitation(invitation) => {
                write_line(
                    output,
                    input,
                    &format!(
                        "{} te invita a la historia compartida «{}». Escribe «historia unirse {}» (story join {}) para unirte o «historia mirar {}» (story watch) para mirar.",
                        invitation.host,
                        invitation.title,
                        invitation.code,
                        invitation.code,
                        invitation.code
                    ),
                )?;
                let gmcp = input.negotiation.story_invitation_gmcp(&invitation);
                if !gmcp.is_empty() {
                    output.bytes(OutputPriority::Interactive, gmcp)?;
                }
                Ok(StoryLineOutcome::Handled)
            }
        }
    }

    fn removed(
        &mut self,
        output: &TelnetOutputQueue,
        input: &mut TelnetInput,
        reason: StoryLeftReason,
    ) -> io::Result<StoryLineOutcome> {
        let (code, message) = match reason {
            StoryLeftReason::Kicked => (
                "kicked",
                "El anfitrión te ha retirado de la historia compartida. Vuelves al mundo.",
            ),
            StoryLeftReason::Ended => (
                "ended",
                "La historia compartida ha terminado: no queda nadie sentado. Vuelves al mundo.",
            ),
            StoryLeftReason::Left => ("left", "Sales de la historia y vuelves al mundo."),
            StoryLeftReason::Evicted => ("evicted", StoryFailure::Gone.message()),
        };
        self.leave_locally(output, input, code)?;
        write_line(output, input, message)?;
        Ok(StoryLineOutcome::Left)
    }

    /// Writes the lobby, view and tally changes of `frame` (all with `force`).
    fn push_frame(
        &mut self,
        output: &TelnetOutputQueue,
        input: &mut TelnetInput,
        frame: &SharedFrame,
        force: bool,
    ) -> io::Result<()> {
        if force || frame.table.lobby_rev != self.sent_lobby_rev {
            self.sent_lobby_rev = frame.table.lobby_rev;
            let lobby = frame.lobby(self.account);
            write_line(output, input, &render_lobby(&lobby))?;
            let gmcp = input.negotiation.story_lobby_gmcp(&lobby);
            if !gmcp.is_empty() {
                output.bytes(OutputPriority::Interactive, gmcp)?;
            }
        }
        if frame.view.view_rev > self.sent_view_rev {
            self.render(output, input, frame.view.clone())?;
        }
        let tally = (frame.table.tally_rev, frame.view.view_rev);
        if force || tally != self.sent_tally {
            self.sent_tally = tally;
            self.write_tally(output, input, frame)?;
        }
        Ok(())
    }

    fn write_tally(
        &mut self,
        output: &TelnetOutputQueue,
        input: &mut TelnetInput,
        frame: &SharedFrame,
    ) -> io::Result<()> {
        let tally = frame.tally(self.account);
        let view = self.last_view.as_ref().unwrap_or(&frame.view);
        if let Some(line) = render_tally(&tally, view) {
            write_line(output, input, &line)?;
        }
        if frame.table.mode() == StoryDecisionMode::Turns
            && let Some(name) = frame.table.turn_name()
            && frame.view.ending.is_none()
        {
            let mut line = format!("Turno de {name}");
            if let Some(seconds) = tally.remaining_seconds {
                line.push_str(&format!(" · cede el turno en {seconds} s"));
            }
            write_line(output, input, &line)?;
        }
        let gmcp = input.negotiation.story_tally_gmcp(&tally, view);
        if !gmcp.is_empty() {
            output.bytes(OutputPriority::Interactive, gmcp)?;
        }
        Ok(())
    }

    /// Re-attaches a reconnecting session to its account's live instance.
    pub(super) async fn resume(
        &mut self,
        output: &TelnetOutputQueue,
        input: &mut TelnetInput,
    ) -> io::Result<()> {
        let Some(link) = self
            .manager
            .as_ref()
            .and_then(|manager| manager.attach(self.account))
        else {
            return Ok(());
        };
        write_line(
            output,
            input,
            &format!("Continúas la historia «{}».", link.title()),
        )?;
        self.set_link(link);
        self.refresh(output, input).await.map(|_| ())
    }

    /// Renders the full current state: lobby, view and tally when shared.
    async fn refresh(
        &mut self,
        output: &TelnetOutputQueue,
        input: &mut TelnetInput,
    ) -> io::Result<StoryLineOutcome> {
        let Some(link) = self.link.clone() else {
            return Ok(StoryLineOutcome::Handled);
        };
        let view = match link.view().await {
            Ok(view) => view,
            Err(failure) => return self.show(output, input, Err(failure)).await,
        };
        let Some(frame) = self
            .frames
            .as_mut()
            .map(|frames| frames.borrow_and_update().clone())
        else {
            self.render(output, input, view)?;
            return Ok(StoryLineOutcome::Handled);
        };
        if let Some(reason) = frame.removal(self.account) {
            return self.removed(output, input, reason);
        }
        self.sent_lobby_rev = 0;
        self.sent_tally = (0, 0);
        self.sent_view_rev = 0;
        self.render(output, input, view)?;
        self.push_frame(output, input, &frame, true)?;
        Ok(StoryLineOutcome::Handled)
    }

    pub(super) async fn line(
        &mut self,
        output: &TelnetOutputQueue,
        input: &mut TelnetInput,
        line: &str,
    ) -> io::Result<StoryLineOutcome> {
        let Some(link) = self.link.clone() else {
            return self.outside(output, input, line).await;
        };
        let result = match parse_story_input(line) {
            StoryInputCommand::Number(number) => {
                let action = self.last_view.as_ref().and_then(|view| {
                    view.actions()
                        .get(number - 1)
                        .map(|action| (action.request(), view.view_rev))
                });
                match action {
                    Some((request, view_rev)) => link.input(request, view_rev).await,
                    None => Err(StoryFailure::UnknownChoice),
                }
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
                match target {
                    Some((leaf, view_rev)) => link.vote(leaf, view_rev).await,
                    None => {
                        write_line(output, input, SHARED_STORY_HELP)?;
                        return Ok(StoryLineOutcome::Handled);
                    }
                }
            }
            StoryInputCommand::Undo => link.undo().await,
            StoryInputCommand::Look => return self.refresh(output, input).await,
            StoryInputCommand::Leave => {
                let outcome = link.leave().await;
                self.leave_locally(output, input, "left")?;
                let message = if outcome.is_ok_and(|outcome| outcome.autosaved) {
                    "Sales de la historia y vuelves al mundo. Tu progreso quedó en el autoguardado."
                } else {
                    "Sales de la historia y vuelves al mundo."
                };
                write_line(output, input, message)?;
                return Ok(StoryLineOutcome::Left);
            }
            StoryInputCommand::Seats => {
                match self.frame() {
                    Some(frame) => {
                        let lobby = frame.lobby(self.account);
                        write_line(output, input, &render_lobby(&lobby))?;
                        let gmcp = input.negotiation.story_lobby_gmcp(&lobby);
                        if !gmcp.is_empty() {
                            output.bytes(OutputPriority::Interactive, gmcp)?;
                        }
                    }
                    None => write_line(output, input, StoryFailure::NotShared.message())?,
                }
                return Ok(StoryLineOutcome::Handled);
            }
            StoryInputCommand::Share => match link.share(&self.name).await {
                Ok(shared) => {
                    self.set_link(shared);
                    return self.refresh(output, input).await;
                }
                Err(failure) => return self.show(output, input, Err(failure)).await,
            },
            StoryInputCommand::Invite(name) => {
                match link.invite(&name).await {
                    Ok(()) => write_line(
                        output,
                        input,
                        &format!("Invitación enviada a {name}, si está conectado."),
                    )?,
                    Err(failure) => return self.show(output, input, Err(failure)).await,
                }
                return Ok(StoryLineOutcome::Handled);
            }
            StoryInputCommand::Kick(name) => {
                match link.kick(&name).await {
                    Ok(()) => write_line(
                        output,
                        input,
                        &format!("{name} ya no participa en la historia."),
                    )?,
                    Err(failure) => return self.show(output, input, Err(failure)).await,
                }
                return Ok(StoryLineOutcome::Handled);
            }
            StoryInputCommand::Mode(Some((mode, seconds))) => {
                match link.set_mode(mode, seconds).await {
                    Ok(()) => write_line(
                        output,
                        input,
                        &format!("Modo de decisión: {}.", mode_label(mode)),
                    )?,
                    Err(failure) => return self.show(output, input, Err(failure)).await,
                }
                return Ok(StoryLineOutcome::Handled);
            }
            StoryInputCommand::Mode(None) => {
                write_line(output, input, StoryFailure::InvalidSetting.message())?;
                return Ok(StoryLineOutcome::Handled);
            }
            StoryInputCommand::Save(slot) => {
                match link.save(slot.unwrap_or(u8::MAX)).await {
                    Ok(saved) => write_line(
                        output,
                        input,
                        &format!("Partida guardada en la ranura {}.", saved.slot),
                    )?,
                    Err(failure) => return self.show(output, input, Err(failure)).await,
                }
                return Ok(StoryLineOutcome::Handled);
            }
            StoryInputCommand::Restore(slot) => {
                return self
                    .restore(output, input, &link, slot.unwrap_or(u8::MAX))
                    .await;
            }
            StoryInputCommand::Continue => return self.restore(output, input, &link, 0).await,
            StoryInputCommand::Slots => {
                match link.slots().await {
                    Ok(slots) => self.slots(output, input, slots, false)?,
                    Err(failure) => return self.show(output, input, Err(failure)).await,
                }
                return Ok(StoryLineOutcome::Handled);
            }
            StoryInputCommand::Delete(slot) => {
                let slot = slot.unwrap_or(u8::MAX);
                match link.delete_slot(slot).await {
                    Ok(slots) => {
                        write_line(
                            output,
                            input,
                            &format!("Partida de la ranura {slot} borrada."),
                        )?;
                        self.slots(output, input, slots, false)?;
                    }
                    Err(failure) => return self.show(output, input, Err(failure)).await,
                }
                return Ok(StoryLineOutcome::Handled);
            }
            StoryInputCommand::List => {
                let listing = self
                    .manager
                    .as_ref()
                    .map(|manager| render_listing(&manager.listing()))
                    .unwrap_or_default();
                write_line(output, input, &listing)?;
                return Ok(StoryLineOutcome::Handled);
            }
            StoryInputCommand::Other => {
                let help = if link.is_shared() {
                    SHARED_STORY_HELP
                } else {
                    STORY_INPUT_HELP
                };
                write_line(output, input, help)?;
                return Ok(StoryLineOutcome::Handled);
            }
        };
        self.show(output, input, result).await
    }

    async fn outside(
        &mut self,
        output: &TelnetOutputQueue,
        input: &mut TelnetInput,
        line: &str,
    ) -> io::Result<StoryLineOutcome> {
        let Some(command) = parse_story_command(line) else {
            return Ok(StoryLineOutcome::World);
        };
        let Some(manager) = self.manager.clone() else {
            write_line(output, input, StoryFailure::Disabled.message())?;
            return Ok(StoryLineOutcome::Handled);
        };
        match command {
            StoryCommand::List => write_line(output, input, &render_listing(&manager.listing()))?,
            StoryCommand::Leave => write_line(output, input, StoryFailure::NotInStory.message())?,
            StoryCommand::Enter(story_id) => match manager.enter(self.account, &story_id) {
                Ok(link) => {
                    write_line(output, input, &format!("Entras en «{}».", link.title()))?;
                    self.set_link(link.clone());
                    let outcome = self.refresh(output, input).await?;
                    if outcome == StoryLineOutcome::Handled
                        && let Some(slots) = link.entry_slots().await
                    {
                        if let Some(offer) = render_continue_offer(&slots) {
                            write_line(output, input, &offer)?;
                        }
                        self.slots_gmcp(output, input, &slots)?;
                    }
                    return Ok(outcome);
                }
                Err(failure) => write_line(output, input, failure.message())?,
            },
            StoryCommand::Host(story_id) => {
                match manager.host(self.account, &self.name, &story_id).await {
                    Ok(link) => {
                        write_line(output, input, &format!("Compartes «{}».", link.title()))?;
                        self.set_link(link);
                        return self.refresh(output, input).await;
                    }
                    Err(failure) => write_line(output, input, failure.message())?,
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
                        write_line(output, input, &message)?;
                        self.set_link(link);
                        return self.refresh(output, input).await;
                    }
                    Err(failure) => write_line(output, input, failure.message())?,
                }
            }
        }
        Ok(StoryLineOutcome::Handled)
    }

    /// Writes one view: prose, numbered actions, GMCP and media cues.
    fn render(
        &mut self,
        output: &TelnetOutputQueue,
        input: &mut TelnetInput,
        view: StoryProjection,
    ) -> io::Result<()> {
        let view = match self.frame() {
            Some(frame) => frame.view_for(self.account, view),
            None => view,
        };
        self.sent_view_rev = self.sent_view_rev.max(view.view_rev);
        let gmcp = input.negotiation.story_gmcp(&view, &mut self.music);
        let mut text = render_text(&view);
        if let Some(frame) = self.frame()
            && view.ending.is_none()
        {
            let hint = match (frame.table.mode(), frame.role(self.account)) {
                (_, Some(StorySeatRole::Spectator)) => "Miras como espectador.",
                (StoryDecisionMode::Vote, _) => "Historia compartida: tu número cuenta como voto.",
                (StoryDecisionMode::Host, Some(StorySeatRole::Host)) => "Decides tú (anfitrión).",
                (StoryDecisionMode::Host, _) => {
                    "Decide el anfitrión; «votar N» (vote N) deja un voto orientativo."
                }
                (StoryDecisionMode::Turns, _) => "Turnos rotatorios: decide quien tenga el turno.",
            };
            text.push_str(hint);
            text.push_str("\r\n");
        }
        let text = escape_negotiated_mxp_text(input, &text);
        output.bytes(OutputPriority::Interactive, text.into_bytes())?;
        if !gmcp.is_empty() {
            output.bytes(OutputPriority::Interactive, gmcp)?;
        }
        self.last_view = Some(view);
        Ok(())
    }

    async fn show(
        &mut self,
        output: &TelnetOutputQueue,
        input: &mut TelnetInput,
        result: Result<StoryProjection, StoryFailure>,
    ) -> io::Result<StoryLineOutcome> {
        match result {
            Ok(view) => {
                let Some(frame) = self
                    .frames
                    .as_mut()
                    .map(|frames| frames.borrow_and_update().clone())
                else {
                    self.render(output, input, view)?;
                    return Ok(StoryLineOutcome::Handled);
                };
                // Shared: apply the frame published before the reply, never
                // repeat a view, and always answer with the current tally.
                if let Some(reason) = frame.removal(self.account) {
                    return self.removed(output, input, reason);
                }
                let before = self.sent_tally;
                self.push_frame(output, input, &frame, false)?;
                if view.view_rev > self.sent_view_rev {
                    self.render(output, input, view)?;
                }
                if self.sent_tally == before {
                    self.write_tally(output, input, &frame)?;
                }
                Ok(StoryLineOutcome::Handled)
            }
            Err(StoryFailure::Gone) => {
                self.leave_locally(output, input, "evicted")?;
                write_line(output, input, StoryFailure::Gone.message())?;
                Ok(StoryLineOutcome::Left)
            }
            Err(StoryFailure::StaleView) => {
                write_line(output, input, StoryFailure::StaleView.message())?;
                Box::pin(self.refresh(output, input)).await
            }
            Err(failure) => {
                write_line(output, input, failure.message())?;
                Ok(StoryLineOutcome::Handled)
            }
        }
    }

    async fn restore(
        &mut self,
        output: &TelnetOutputQueue,
        input: &mut TelnetInput,
        link: &StoryLink,
        slot: u8,
    ) -> io::Result<StoryLineOutcome> {
        let result = link.restore(slot).await;
        if result.is_ok() {
            let message = if slot == 0 {
                "Continúas la partida autoguardada.".to_owned()
            } else {
                format!("Cargas la partida de la ranura {slot}.")
            };
            write_line(output, input, &message)?;
        }
        self.show(output, input, result).await
    }

    /// Writes the slot listing and, with GMCP, `Tiamud.Story.Slots`.
    fn slots(
        &mut self,
        output: &TelnetOutputQueue,
        input: &mut TelnetInput,
        slots: Vec<StorySlotSummary>,
        offer_continue: bool,
    ) -> io::Result<()> {
        write_line(output, input, &render_slots(&slots))?;
        self.slots_gmcp(
            output,
            input,
            &StorySlots {
                slots,
                offer_continue,
            },
        )
    }

    fn slots_gmcp(
        &mut self,
        output: &TelnetOutputQueue,
        input: &mut TelnetInput,
        slots: &StorySlots,
    ) -> io::Result<()> {
        let gmcp = input.negotiation.story_slots_gmcp(slots);
        if !gmcp.is_empty() {
            output.bytes(OutputPriority::Interactive, gmcp)?;
        }
        Ok(())
    }

    fn leave_locally(
        &mut self,
        output: &TelnetOutputQueue,
        input: &mut TelnetInput,
        reason: &str,
    ) -> io::Result<()> {
        self.link = None;
        self.last_view = None;
        self.frames = None;
        let gmcp = input
            .negotiation
            .story_left_gmcp(reason, std::mem::take(&mut self.music));
        if !gmcp.is_empty() {
            output.bytes(OutputPriority::Interactive, gmcp)?;
        }
        Ok(())
    }
}

fn write_line(output: &TelnetOutputQueue, input: &TelnetInput, text: &str) -> io::Result<()> {
    let mut bytes = escape_negotiated_mxp_text(input, &text.replace('\n', "\r\n")).into_bytes();
    bytes.extend_from_slice(b"\r\n");
    output.bytes(OutputPriority::Interactive, bytes)
}

const fn ending_name(kind: StoryEndingKind) -> &'static str {
    match kind {
        StoryEndingKind::Victory => "victory",
        StoryEndingKind::Defeat => "defeat",
        StoryEndingKind::Neutral => "neutral",
    }
}

const fn channel_name(channel: MediaChannel) -> &'static str {
    match channel {
        MediaChannel::Ambience => "ambience",
        MediaChannel::Effects => "effects",
        MediaChannel::Music => "music",
    }
}

/// Maximum bytes of one choice label in the GMCP overflow fallback.
const FALLBACK_LABEL_BYTES: usize = 64;

impl TelnetSession {
    /// `Tiamud.Story.View` summary plus `Client.Media` cues. The summary
    /// carries numbered choices, stats and inventory; prose stays in-band.
    pub(super) fn story_gmcp(&mut self, view: &StoryProjection, music: &mut bool) -> Vec<u8> {
        if !self.q.local_enabled(super::OPT_GMCP) {
            return Vec::new();
        }
        let mut output = Vec::new();
        if self.gmcp.supports("tiamud.story") {
            let actions = view.actions();
            let mut stats = Vec::new();
            let mut inventory = Vec::new();
            let mut present = Vec::new();
            let mut dialogue = false;
            for item in &view.items {
                match item {
                    StoryItem::Stat { label, value } => {
                        stats.push(json!({"label": label, "value": value}));
                    }
                    StoryItem::Item { name } => inventory.push(name.as_str()),
                    StoryItem::Portrait { name } => present.push(name.as_str()),
                    StoryItem::DialogueOption { .. } => dialogue = true,
                    _ => {}
                }
            }
            let choices = |limit: Option<usize>| {
                actions
                    .iter()
                    .enumerate()
                    .map(|(index, action)| {
                        let label = match limit {
                            Some(limit) => bounded_utf8_prefix(action.label(), limit).0,
                            None => action.label().to_owned(),
                        };
                        json!({"number": index + 1, "label": label})
                    })
                    .collect::<Vec<_>>()
            };
            let summary = json!({
                "viewRev": view.view_rev,
                "title": view.title,
                "turn": view.turn,
                "score": view.score,
                "ending": view.ending.map(ending_name),
                "canUndo": view.can_undo,
                "dialogue": dialogue,
                "choices": choices(None),
                "present": present,
                "stats": stats,
                "inventory": inventory,
            });
            if !append_gmcp(&mut output, "Tiamud.Story.View", &summary) {
                let fallback = json!({
                    "viewRev": view.view_rev,
                    "title": bounded_utf8_prefix(&view.title, FALLBACK_LABEL_BYTES).0,
                    "turn": view.turn,
                    "score": view.score,
                    "ending": view.ending.map(ending_name),
                    "canUndo": view.can_undo,
                    "choices": choices(Some(FALLBACK_LABEL_BYTES)),
                    "truncated": true,
                    "overflow": {"package": "Tiamud.Story.View", "maximumBytes": MAX_GMCP_BYTES},
                });
                if !append_gmcp(&mut output, "Tiamud.Story.View", &fallback) {
                    append_gmcp(
                        &mut output,
                        "Tiamud.Story.View",
                        &json!({
                            "viewRev": view.view_rev,
                            "choiceCount": actions.len(),
                            "truncated": true,
                            "overflow": {"package": "Tiamud.Story.View", "maximumBytes": MAX_GMCP_BYTES},
                        }),
                    );
                }
            }
        }
        if self.gmcp.media_opt_in && !self.rich_output_blocked() {
            for item in &view.items {
                let StoryItem::Cue {
                    action,
                    channel: cue_channel,
                    url,
                    looped,
                } = item
                else {
                    continue;
                };
                let channel = channel_name(*cue_channel);
                match (*action, url.as_deref()) {
                    (StoryCueAction::Play, Some(url)) => {
                        if *cue_channel == MediaChannel::Music {
                            // Story music replaces the scene track; the scene
                            // re-emits its own after the story ends.
                            if self
                                .gmcp
                                .active_media
                                .as_ref()
                                .is_some_and(|active| active.channel == channel)
                            {
                                self.gmcp.active_media = None;
                            }
                            *music = true;
                        }
                        append_gmcp(
                            &mut output,
                            "Client.Media.Play",
                            &json!({"channel": channel, "url": url, "loop": looped}),
                        );
                    }
                    (StoryCueAction::Stop, _) => {
                        if *cue_channel == MediaChannel::Music {
                            *music = false;
                        }
                        append_gmcp(
                            &mut output,
                            "Client.Media.Stop",
                            &json!({"channel": channel}),
                        );
                    }
                    (StoryCueAction::Play, None) => {}
                }
            }
        }
        output
    }

    /// `Tiamud.Story.Slots`: slot metadata only (at most ten small entries,
    /// far below the GMCP budget). Never the saved state or a pack digest.
    pub(super) fn story_slots_gmcp(&mut self, slots: &StorySlots) -> Vec<u8> {
        if !self.q.local_enabled(super::OPT_GMCP) || !self.gmcp.supports("tiamud.story") {
            return Vec::new();
        }
        let mut output = Vec::new();
        let entries = slots
            .slots
            .iter()
            .map(|slot| {
                json!({
                    "slot": slot.slot,
                    "updatedAt": slot.updated_at,
                    "compatible": slot.compatible,
                })
            })
            .collect::<Vec<_>>();
        append_gmcp(
            &mut output,
            "Tiamud.Story.Slots",
            &json!({"slots": entries, "offerContinue": slots.offer_continue}),
        );
        output
    }

    /// `Tiamud.Story.Lobby`: the shared instance's seats and rules (names of
    /// at most eight seats; spectators are counted). The join code is present
    /// for the host only, as in `story.lobby`.
    pub(super) fn story_lobby_gmcp(&mut self, lobby: &protocol::StoryLobby) -> Vec<u8> {
        if !self.q.local_enabled(super::OPT_GMCP) || !self.gmcp.supports("tiamud.story") {
            return Vec::new();
        }
        let mut output = Vec::new();
        let value = serde_json::to_value(lobby).unwrap_or_default();
        if !append_gmcp(&mut output, "Tiamud.Story.Lobby", &value) {
            append_gmcp(
                &mut output,
                "Tiamud.Story.Lobby",
                &json!({
                    "mode": lobby.mode,
                    "seatCount": lobby.seats.len(),
                    "spectators": lobby.spectators,
                    "truncated": true,
                    "overflow": {"package": "Tiamud.Story.Lobby", "maximumBytes": MAX_GMCP_BYTES},
                }),
            );
        }
        output
    }

    /// `Tiamud.Story.Tally`: vote counts by action number (no opaque ids and
    /// no voter identities), the recipient's own vote and the window left.
    pub(super) fn story_tally_gmcp(
        &mut self,
        tally: &protocol::StoryTally,
        view: &StoryProjection,
    ) -> Vec<u8> {
        if !self.q.local_enabled(super::OPT_GMCP) || !self.gmcp.supports("tiamud.story") {
            return Vec::new();
        }
        let actions = view.actions();
        let number = |id: &str| {
            actions
                .iter()
                .position(|action| action.leaf_id() == id)
                .map(|index| index + 1)
        };
        let votes = tally
            .choices
            .iter()
            .filter_map(|entry| {
                number(&entry.choice_id)
                    .map(|number| json!({"number": number, "votes": entry.votes}))
            })
            .collect::<Vec<_>>();
        let mut output = Vec::new();
        append_gmcp(
            &mut output,
            "Tiamud.Story.Tally",
            &json!({
                "viewRev": tally.view_rev,
                "mode": tally.mode,
                "votes": votes,
                "voted": tally.voted,
                "eligible": tally.eligible,
                "remainingSeconds": tally.remaining_seconds,
                "yourVote": tally.your_vote.as_deref().and_then(number),
            }),
        );
        output
    }

    /// `Tiamud.Story.Invitation`: host name, story and join code.
    pub(super) fn story_invitation_gmcp(
        &mut self,
        invitation: &protocol::StoryInvitation,
    ) -> Vec<u8> {
        if !self.q.local_enabled(super::OPT_GMCP) || !self.gmcp.supports("tiamud.story") {
            return Vec::new();
        }
        let mut output = Vec::new();
        append_gmcp(
            &mut output,
            "Tiamud.Story.Invitation",
            &serde_json::to_value(invitation).unwrap_or_default(),
        );
        output
    }

    /// `Tiamud.Story.Left` and, when story music may be playing, a stop.
    pub(super) fn story_left_gmcp(&mut self, reason: &str, music: bool) -> Vec<u8> {
        if !self.q.local_enabled(super::OPT_GMCP) {
            return Vec::new();
        }
        let mut output = Vec::new();
        if self.gmcp.supports("tiamud.story") {
            append_gmcp(&mut output, "Tiamud.Story.Left", &json!({"reason": reason}));
        }
        if music && self.gmcp.media_opt_in {
            append_gmcp(
                &mut output,
                "Client.Media.Stop",
                &json!({"channel": "music"}),
            );
        }
        output
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod shared_tests;
