//! Telnet adapter for story sessions (ADR 0029). Story views render as plain
//! text with numbered choices; typing a number chooses. GMCP clients that
//! negotiated `Tiamud.Story 1` also receive a bounded summary, and media
//! opt-ins receive story cues as `Client.Media`. Losing GMCP or media never
//! loses an action.

use super::{
    MAX_GMCP_BYTES, OutputPriority, TelnetInput, TelnetOutputQueue, TelnetSession, append_gmcp,
    bounded_utf8_prefix, escape_negotiated_mxp_text,
};
use crate::story_sessions::{
    STORY_INPUT_HELP, StoryCommand, StoryFailure, StoryInputCommand, StoryLink, StoryManager,
    StoryProjection, parse_story_command, parse_story_input, render_continue_offer, render_listing,
    render_slots, render_text,
};
use serde_json::json;
use std::io;
use tiamud_core::AccountId;
use tiamud_protocol::{
    MediaChannel, StoryCueAction, StoryEndingKind, StoryItem, StorySlotSummary, StorySlots,
};

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

pub(super) struct TelnetStory {
    manager: Option<StoryManager>,
    account: AccountId,
    link: Option<StoryLink>,
    last_view: Option<StoryProjection>,
    /// True while story music may be playing through `Client.Media`.
    music: bool,
}

impl TelnetStory {
    pub(super) fn new(manager: Option<StoryManager>, account: AccountId) -> Self {
        Self {
            manager,
            account,
            link: None,
            last_view: None,
            music: false,
        }
    }

    pub(super) fn active(&self) -> bool {
        self.link.is_some()
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
        self.link = Some(link.clone());
        let result = link.view().await;
        self.show(output, input, result).await.map(|_| ())
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
            StoryInputCommand::Undo => link.undo().await,
            StoryInputCommand::Look => link.view().await,
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
                write_line(output, input, STORY_INPUT_HELP)?;
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
                    self.link = Some(link.clone());
                    let result = link.view().await;
                    let outcome = self.show(output, input, result).await?;
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
        }
        Ok(StoryLineOutcome::Handled)
    }

    async fn show(
        &mut self,
        output: &TelnetOutputQueue,
        input: &mut TelnetInput,
        result: Result<StoryProjection, StoryFailure>,
    ) -> io::Result<StoryLineOutcome> {
        match result {
            Ok(view) => {
                let gmcp = input.negotiation.story_gmcp(&view, &mut self.music);
                let text = escape_negotiated_mxp_text(input, &render_text(&view));
                output.bytes(OutputPriority::Interactive, text.into_bytes())?;
                if !gmcp.is_empty() {
                    output.bytes(OutputPriority::Interactive, gmcp)?;
                }
                self.last_view = Some(view);
                Ok(StoryLineOutcome::Handled)
            }
            Err(StoryFailure::Gone) => {
                self.leave_locally(output, input, "evicted")?;
                write_line(output, input, StoryFailure::Gone.message())?;
                Ok(StoryLineOutcome::Left)
            }
            Err(StoryFailure::StaleView) => {
                write_line(output, input, StoryFailure::StaleView.message())?;
                let Some(link) = self.link.clone() else {
                    return Ok(StoryLineOutcome::Handled);
                };
                let current = link.view().await;
                Box::pin(self.show(output, input, current)).await
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
