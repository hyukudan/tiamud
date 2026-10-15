//! Parser commands for format v3 packs (ADR 0031): `story-parser` turns a
//! line into commands against a scope built from [`StoryState`]; rules and
//! the default library run each command in order with a fresh scope.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use tiamud_story_parser::{
    Command, Lexicon, LexiconError, ParseOutcome, Parser, Pending, ScopeObject, SimpleScope, fold,
};

use crate::engine::{Exec, active_dialogue, holds};
use crate::messages::definite;
use crate::runtime::{contents, held, inside, lit, room_objects, visible, visible_objects};
use crate::world::{LexiconRef, LibraryMessage as M, Location, Rule, RulePhase, World};
use crate::{
    MAX_EFFECTS_PER_CHOICE, MAX_MULTI_OBJECTS, MAX_PARSER_REPLY_BYTES, MAX_PENDING_ANSWERS,
    MAX_REPLIES, MAX_TURNS, PendingQuestion, Reply, StoryError, StoryEvent, StoryPack, StoryState,
    bounded_message,
};

/// An out-of-world command the host must handle (undo, save, restore, quit,
/// restart) or may decorate (score and help are also answered in the
/// transcript). `action` is the lexicon's canonical action id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaRequest {
    pub action: String,
}

/// Result of one parser input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandStep {
    pub state: StoryState,
    pub events: Vec<StoryEvent>,
    /// Meta commands in the order typed. A meta command ends the line.
    pub meta: Vec<MetaRequest>,
}

fn base_lexicon(id: &str) -> Result<&'static Lexicon, LexiconError> {
    static SPANISH: OnceLock<Result<Lexicon, LexiconError>> = OnceLock::new();
    static ENGLISH: OnceLock<Result<Lexicon, LexiconError>> = OnceLock::new();
    let cell = match id {
        "es" => &SPANISH,
        "en" => &ENGLISH,
        _ => {
            return Err(LexiconError::Invalid(format!(
                "unknown shipped lexicon {:?}",
                bounded_message(id)
            )));
        }
    };
    cell.get_or_init(|| {
        let json = Parser::builtin_lexicon_json(id).unwrap_or_default();
        Lexicon::from_json_slice(json.as_bytes())
    })
    .as_ref()
    .map_err(Clone::clone)
}

/// Compiles a shipped lexicon with the pack's extension verbs.
pub fn build_parser(lexicon: &LexiconRef) -> Result<Parser, LexiconError> {
    let mut merged = base_lexicon(&lexicon.id)?.clone();
    merged.verbs.extend(lexicon.verbs.iter().cloned());
    Parser::new(&merged)
}

/// Canonical action ids a lexicon can produce, including its go action.
pub(crate) fn lexicon_actions(lexicon: &LexiconRef) -> Result<BTreeSet<String>, LexiconError> {
    let base = base_lexicon(&lexicon.id)?;
    let mut actions = BTreeSet::new();
    actions.insert(base.go_action.clone());
    for verb in base.verbs.iter().chain(&lexicon.verbs) {
        actions.insert(verb.action.clone());
        actions.extend(
            verb.patterns
                .iter()
                .filter_map(|pattern| pattern.action.clone()),
        );
    }
    Ok(actions)
}

/// Canonical direction ids of a lexicon.
pub(crate) fn lexicon_directions(lexicon: &LexiconRef) -> Result<BTreeSet<String>, LexiconError> {
    Ok(base_lexicon(&lexicon.id)?
        .directions
        .iter()
        .map(|direction| direction.id.clone())
        .collect())
}

/// Picks the lexicon for an input: the requested language (exact or by its
/// primary subtag), else the first one.
fn pick_lexicon<'a>(world: &'a World, language: Option<&str>) -> Option<&'a LexiconRef> {
    let wanted = language.map(|language| {
        language
            .split('-')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase()
    });
    wanted
        .and_then(|wanted| world.lexicons.iter().find(|lexicon| lexicon.id == wanted))
        .or_else(|| world.lexicons.first())
}

/// Applies one line of free text. Pure: `state` is not modified.
pub fn step_command(
    pack: &StoryPack,
    state: &StoryState,
    text: &str,
    language: Option<&str>,
) -> Result<CommandStep, StoryError> {
    state.check_against(pack)?;
    if state.ended.is_some() {
        return Err(StoryError::Ended);
    }
    let world = pack.world.as_ref().ok_or(StoryError::NoParser)?;
    if active_dialogue(pack, state)?.is_some() {
        return Err(StoryError::InDialogue);
    }
    let pending = state.pending.clone();
    let language = pending
        .as_ref()
        .map(|pending| pending.language.as_str())
        .or(language);
    let lexicon = pick_lexicon(world, language)
        .ok_or_else(|| StoryError::InvalidPack("world has no lexicon".to_owned()))?;
    let parser = build_parser(lexicon)
        .map_err(|error| StoryError::InvalidPack(bounded_message(error.to_string())))?;
    let mut exec = Exec::resume(pack, state.clone());
    exec.state.transcript.clear();
    exec.state.pending = None;
    let mut run = Run {
        pack,
        world,
        language: lexicon.id.as_str(),
        parser: &parser,
        exec,
        meta: Vec::new(),
        full: false,
    };
    let answered = match &pending {
        Some(pending) if pending.language == lexicon.id => run.answer(pending, text)?,
        _ => false,
    };
    if !answered {
        run.line(text)?;
    }
    let Run { exec, meta, .. } = run;
    let (state, events) = exec.finish();
    Ok(CommandStep {
        state,
        events,
        meta,
    })
}

/// Replies that describe the current node: heading, description and the
/// listing of visible things, or the darkness message. Empty outside rooms.
pub(crate) fn look_replies(pack: &StoryPack, state: &StoryState) -> Result<Vec<Reply>, StoryError> {
    let node = state.node.as_str();
    if pack.room(node).is_none() {
        return Ok(Vec::new());
    }
    if !lit(pack, state) {
        return Ok(vec![message(M::Dark, None, None)]);
    }
    let mut replies = vec![
        Reply::RoomName {
            node: node.to_owned(),
        },
        Reply::RoomText {
            node: node.to_owned(),
        },
    ];
    let listed: Vec<&str> = room_objects(pack, state, node)
        .into_iter()
        .filter(|decl| !decl.scenery)
        .map(|decl| decl.id.as_str())
        .collect();
    if !listed.is_empty() {
        replies.push(list_message(M::YouSee, None, &listed));
    }
    for decl in room_objects(pack, state, node) {
        if decl.container
            && state
                .objects
                .get(&decl.id)
                .is_some_and(|object| object.open)
        {
            let inner = contents(pack, state, &decl.id);
            if !inner.is_empty() && !decl.scenery {
                replies.push(list_message(M::InsideYouSee, Some(&decl.id), &inner));
            }
        }
    }
    Ok(replies)
}

fn message(key: M, obj: Option<&str>, obj2: Option<&str>) -> Reply {
    Reply::Message {
        message: key,
        obj: obj.map(str::to_owned),
        obj2: obj2.map(str::to_owned),
        list: Vec::new(),
    }
}

fn list_message(key: M, obj: Option<&str>, list: &[&str]) -> Reply {
    Reply::Message {
        message: key,
        obj: obj.map(str::to_owned),
        obj2: None,
        list: list.iter().map(|id| (*id).to_owned()).collect(),
    }
}

struct Run<'a> {
    pack: &'a StoryPack,
    world: &'a World,
    language: &'a str,
    parser: &'a Parser,
    exec: Exec<'a>,
    meta: Vec<MetaRequest>,
    /// The transcript reached [`MAX_REPLIES`]; the line stops.
    full: bool,
}

impl<'a> Run<'a> {
    fn state(&self) -> &StoryState {
        &self.exec.state
    }

    fn reply(&mut self, reply: Reply) {
        if self.exec.state.transcript.len() < MAX_REPLIES {
            self.exec.state.transcript.push(reply);
        } else {
            self.full = true;
        }
    }

    fn say(&mut self, key: M, obj: Option<&str>, obj2: Option<&str>) {
        self.reply(message(key, obj, obj2));
    }

    fn parser_text(&mut self, text: &str) {
        let mut text = text.chars().filter(|c| !c.is_control()).collect::<String>();
        if text.len() > MAX_PARSER_REPLY_BYTES {
            let mut cut = MAX_PARSER_REPLY_BYTES;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
        }
        let language = self.language.to_owned();
        self.reply(Reply::Parser { language, text });
    }

    /// The parser's view of the world: visible objects with their nouns in
    /// this language, the pronoun referent while it is visible, and every
    /// object word as known vocabulary ("you can't see that").
    fn scope(&self) -> SimpleScope {
        let state = self.state();
        let language = self.language;
        let objects = visible_objects(self.pack, state)
            .into_iter()
            .map(|decl| ScopeObject {
                id: decl.id.clone(),
                name: definite(decl, language),
                nouns: decl.nouns.get(language).cloned().unwrap_or_default(),
                adjectives: decl.adjectives.get(language).cloned().unwrap_or_default(),
            })
            .collect();
        let vocabulary = self
            .world
            .objects
            .iter()
            .flat_map(|decl| {
                decl.nouns
                    .get(language)
                    .into_iter()
                    .chain(decl.adjectives.get(language))
                    .flatten()
            })
            .map(|word| fold(word))
            .collect();
        SimpleScope {
            objects,
            last: state
                .pronoun
                .clone()
                .filter(|id| visible(self.pack, state, id)),
            vocabulary,
        }
    }

    /// Regenerates the pending question by replaying its line and answers.
    fn replay(&self, pending: &PendingQuestion) -> Option<Pending> {
        let segments = self.parser.split_line(&pending.line).ok()?;
        let segment = segments.get(usize::from(pending.segment))?;
        let scope = self.scope();
        let mut outcome = self.parser.parse_segment(segment, &scope);
        for answer in &pending.answers {
            let question = match &outcome {
                ParseOutcome::Ambiguous { pending, .. }
                | ParseOutcome::Incomplete { pending, .. } => pending.clone(),
                _ => return None,
            };
            let mut outcomes = self.parser.answer(&question, answer, &scope);
            if outcomes.len() != 1 {
                return None;
            }
            outcome = outcomes.remove(0);
        }
        match outcome {
            ParseOutcome::Ambiguous { pending, .. } | ParseOutcome::Incomplete { pending, .. } => {
                Some(pending)
            }
            _ => None,
        }
    }

    /// Treats `text` as the reply to a pending question. Returns false when
    /// it must be parsed as a new line instead.
    fn answer(&mut self, pending: &PendingQuestion, text: &str) -> Result<bool, StoryError> {
        if text.len() > tiamud_story_parser::MAX_INPUT_BYTES {
            return Ok(false);
        }
        let Some(question) = self.replay(pending) else {
            return Ok(false);
        };
        let scope = self.scope();
        let mut outcomes = self.parser.answer(&question, text, &scope);
        if outcomes.len() != 1 {
            return Ok(false);
        }
        let mut chain = pending.clone();
        chain.answers.push(text.to_owned());
        self.outcome(outcomes.remove(0), chain)?;
        Ok(true)
    }

    /// Parses and runs a line command by command, each against a fresh
    /// scope, stopping at the first failure, question or meta command.
    fn line(&mut self, text: &str) -> Result<(), StoryError> {
        let segments = match self.parser.split_line(text) {
            Ok(segments) => segments,
            Err(error) => {
                self.parser_text(&error.message);
                return Ok(());
            }
        };
        for (index, segment) in segments.iter().enumerate() {
            if self.exec.ended || self.full {
                break;
            }
            let scope = self.scope();
            let outcome = self.parser.parse_segment(segment, &scope);
            let chain = PendingQuestion {
                language: self.language.to_owned(),
                line: text.to_owned(),
                segment: u8::try_from(index).unwrap_or(u8::MAX),
                answers: Vec::new(),
            };
            if !self.outcome(outcome, chain)? {
                break;
            }
        }
        Ok(())
    }

    /// Handles one parse outcome. Returns true when the line may continue.
    fn outcome(
        &mut self,
        outcome: ParseOutcome,
        chain: PendingQuestion,
    ) -> Result<bool, StoryError> {
        match outcome {
            ParseOutcome::Command(command) => self.command(&command),
            ParseOutcome::Ambiguous { question, .. }
            | ParseOutcome::Incomplete { question, .. } => {
                self.parser_text(&question);
                if chain.answers.len() <= MAX_PENDING_ANSWERS
                    && chain.line.len() <= tiamud_story_parser::MAX_INPUT_BYTES
                {
                    self.exec.state.pending = Some(chain);
                }
                Ok(false)
            }
            ParseOutcome::Error(error) => {
                self.parser_text(&error.message);
                Ok(false)
            }
        }
    }

    fn command(&mut self, command: &Command) -> Result<bool, StoryError> {
        if command.meta {
            let action = command.action.as_str();
            match action {
                "score" => {
                    let reply = Reply::Score {
                        score: self.state().score,
                        turns: self.state().turn,
                    };
                    self.reply(reply);
                }
                "help" => self.say(M::Help, None, None),
                _ => {}
            }
            self.meta.push(MetaRequest {
                action: action.to_owned(),
            });
            return Ok(false);
        }
        self.exec.state.turn = self
            .exec
            .state
            .turn
            .checked_add(1)
            .filter(|turn| *turn <= MAX_TURNS)
            .ok_or(StoryError::TurnLimit)?;
        let action = command.action.as_str();
        let obj2 = command.obj2.as_ref().map(|obj| obj.as_str());
        let ok = if command.all {
            let except: Vec<&str> = command.except.iter().map(|obj| obj.as_str()).collect();
            let targets = self.all_targets(action, obj2, &except);
            if targets.is_empty() {
                let key = match action {
                    "take" => M::NothingToTake,
                    "drop" | "put_in" => M::NothingToDrop,
                    _ => M::NothingHappens,
                };
                self.say(key, None, None);
                false
            } else {
                let mut ok = true;
                for target in targets {
                    if self.exec.ended || self.full {
                        break;
                    }
                    ok &= self.action(action, Some(&target), obj2, None)?;
                }
                ok
            }
        } else {
            let obj = command.obj.as_ref().map(|obj| obj.as_str());
            let direction = command
                .direction
                .as_ref()
                .map(|direction| direction.as_str());
            self.action(action, obj, obj2, direction)?
        };
        if let Some(obj) = &command.obj
            && self.pack.object(obj.as_str()).is_some()
        {
            self.exec.state.pronoun = Some(obj.as_str().to_owned());
        }
        Ok(ok && !self.exec.ended && !self.full)
    }

    /// Objects `todo`/`all` covers for an action, in pack order and at most
    /// [`MAX_MULTI_OBJECTS`].
    fn all_targets(&self, action: &str, obj2: Option<&str>, except: &[&str]) -> Vec<String> {
        let state = self.state();
        let node = state.node.as_str();
        self.world
            .objects
            .iter()
            .filter(|decl| !except.contains(&decl.id.as_str()))
            .filter(|decl| {
                let Some(object) = state.objects.get(&decl.id) else {
                    return false;
                };
                match action {
                    "take" => {
                        matches!(&object.at, Location::Room(room) if room == node)
                            && decl.portable
                            && !decl.scenery
                            && !decl.animate
                            && visible(self.pack, state, &decl.id)
                    }
                    "drop" => object.at == Location::Carried && !object.worn,
                    "put_in" => {
                        object.at == Location::Carried
                            && !object.worn
                            && Some(decl.id.as_str()) != obj2
                    }
                    _ => false,
                }
            })
            .take(MAX_MULTI_OBJECTS)
            .map(|decl| decl.id.clone())
            .collect()
    }

    fn find_rule(
        &self,
        phase: RulePhase,
        action: &str,
        obj: Option<&str>,
        obj2: Option<&str>,
        direction: Option<&str>,
    ) -> Result<Option<&'a Rule>, StoryError> {
        for rule in &self.world.rules {
            let keyed = rule.phase == phase
                && rule.action == action
                && rule.obj.as_deref().is_none_or(|wanted| Some(wanted) == obj)
                && rule
                    .obj2
                    .as_deref()
                    .is_none_or(|wanted| Some(wanted) == obj2)
                && rule
                    .direction
                    .as_deref()
                    .is_none_or(|wanted| Some(wanted) == direction);
            if keyed && holds(self.pack, self.state(), rule.when.as_ref())? {
                return Ok(Some(rule));
            }
        }
        Ok(None)
    }

    fn apply_rule(&mut self, rule: &Rule) -> Result<(), StoryError> {
        if rule.text.is_some() {
            self.reply(Reply::Rule {
                rule: rule.id.clone(),
            });
        }
        self.exec.budget = MAX_EFFECTS_PER_CHOICE;
        self.exec.run(&rule.effects, 0)?;
        self.exec.finish_move(None)?;
        Ok(())
    }

    /// Describes the new node after a move a rule caused.
    fn after_move(&mut self, from: &str) -> Result<(), StoryError> {
        if !self.exec.ended && self.exec.state.node != from {
            let replies = look_replies(self.pack, self.state())?;
            for reply in replies {
                self.reply(reply);
            }
        }
        Ok(())
    }

    /// Runs one action: `before`, then `instead` or the library, then
    /// `after` on success. Returns whether the action succeeded.
    fn action(
        &mut self,
        action: &str,
        obj: Option<&str>,
        obj2: Option<&str>,
        direction: Option<&str>,
    ) -> Result<bool, StoryError> {
        let from = self.exec.state.node.clone();
        if let Some(rule) = self.find_rule(RulePhase::Before, action, obj, obj2, direction)? {
            self.apply_rule(rule)?;
            if self.exec.ended || self.exec.state.node != from {
                self.after_move(&from)?;
                return Ok(true);
            }
        }
        if let Some(rule) = self.find_rule(RulePhase::Instead, action, obj, obj2, direction)? {
            self.apply_rule(rule)?;
            self.after_move(&from)?;
            return Ok(true);
        }
        let ok = self.library(action, obj, obj2, direction)?;
        // The library describes its own moves.
        let from = self.exec.state.node.clone();
        if ok
            && !self.exec.ended
            && let Some(rule) = self.find_rule(RulePhase::After, action, obj, obj2, direction)?
        {
            self.apply_rule(rule)?;
        }
        self.after_move(&from)?;
        Ok(ok)
    }

    fn decl(&self, id: &str) -> Result<&'a crate::world::ObjectDecl, StoryError> {
        self.pack
            .object(id)
            .ok_or_else(|| StoryError::StateMismatch("command names an unknown object".to_owned()))
    }

    fn object_mut(&mut self, id: &str) -> Result<&mut crate::world::ObjectState, StoryError> {
        self.exec
            .state
            .objects
            .get_mut(id)
            .ok_or_else(|| StoryError::StateMismatch("command names an unknown object".to_owned()))
    }

    fn at(&self, id: &str) -> Option<&Location> {
        self.state().objects.get(id).map(|object| &object.at)
    }

    fn flags(&self, id: &str) -> crate::world::ObjectState {
        self.state()
            .objects
            .get(id)
            .cloned()
            .unwrap_or(crate::world::ObjectState {
                at: Location::Nowhere,
                open: false,
                locked: false,
                lit: false,
                worn: false,
            })
    }

    /// The default library. Returns whether the action succeeded.
    fn library(
        &mut self,
        action: &str,
        obj: Option<&str>,
        obj2: Option<&str>,
        direction: Option<&str>,
    ) -> Result<bool, StoryError> {
        match (action, obj, obj2) {
            ("look", _, _) => {
                let replies = look_replies(self.pack, self.state())?;
                for reply in replies {
                    self.reply(reply);
                }
                Ok(true)
            }
            ("inventory", _, _) => {
                self.inventory();
                Ok(true)
            }
            ("wait", _, _) => {
                self.say(M::Wait, None, None);
                Ok(true)
            }
            ("go", _, _) => self.go(direction),
            ("exit", _, _) => self.go(Some("out")),
            ("enter", None, _) => self.go(Some("in")),
            ("enter", Some(obj), _) => self.enter(obj),
            ("examine", Some(obj), _) => {
                self.reply(Reply::Describe {
                    obj: obj.to_owned(),
                });
                self.show_contents(obj);
                Ok(true)
            }
            ("search", Some(obj), _) => self.search(obj),
            ("look_under", Some(_), _) => {
                self.say(M::SearchNothing, None, None);
                Ok(true)
            }
            ("take", Some(obj), _) => self.take(obj),
            ("drop", Some(obj), _) => self.drop(obj),
            ("open", Some(obj), _) => self.open(obj),
            ("close", Some(obj), _) => self.close(obj),
            ("unlock", Some(obj), Some(key)) => self.unlock(obj, key, false),
            ("lock", Some(obj), Some(key)) => self.unlock(obj, key, true),
            ("put_in", Some(obj), Some(container)) => self.put_in(obj, container),
            ("put_on", Some(_), Some(target)) => {
                self.say(M::PutOn, None, Some(target));
                Ok(false)
            }
            ("give", Some(obj), Some(person)) => self.offer(obj, person, M::GiveDeclined),
            ("show", Some(obj), Some(person)) => self.offer(obj, person, M::ShowDeclined),
            ("talk", Some(person), _) => {
                if self.decl(person)?.animate {
                    self.say(M::TalkNothing, Some(person), None);
                    Ok(true)
                } else {
                    self.say(M::TalkNotAnimate, Some(person), None);
                    Ok(false)
                }
            }
            ("read", Some(obj), _) => {
                if self.decl(obj)?.text.is_some() {
                    self.reply(Reply::Read {
                        obj: obj.to_owned(),
                    });
                    Ok(true)
                } else {
                    self.say(M::NothingWritten, Some(obj), None);
                    Ok(false)
                }
            }
            ("wear", Some(obj), _) => self.wear(obj),
            ("remove", Some(obj), _) => self.remove(obj),
            ("switch_on", Some(obj), _) => self.switch(obj, true),
            ("switch_off", Some(obj), _) => self.switch(obj, false),
            _ => {
                self.say(M::NothingHappens, None, None);
                Ok(false)
            }
        }
    }

    fn inventory(&mut self) {
        let state = self.state();
        let mut carried = Vec::new();
        let mut worn = Vec::new();
        for decl in &self.world.objects {
            if let Some(object) = state.objects.get(&decl.id)
                && object.at == Location::Carried
            {
                if object.worn {
                    worn.push(decl.id.as_str());
                } else {
                    carried.push(decl.id.as_str());
                }
            }
        }
        if carried.is_empty() && worn.is_empty() {
            self.say(M::InventoryEmpty, None, None);
            return;
        }
        if !carried.is_empty() {
            self.reply(list_message(M::Inventory, None, &carried));
        }
        if !worn.is_empty() {
            self.reply(list_message(M::Wearing, None, &worn));
        }
    }

    fn show_contents(&mut self, obj: &str) {
        let Ok(decl) = self.decl(obj) else {
            return;
        };
        if decl.container && self.flags(obj).open {
            let inner: Vec<&str> = contents(self.pack, self.state(), obj);
            if !inner.is_empty() {
                self.reply(list_message(M::InsideYouSee, Some(obj), &inner));
            }
        }
    }

    fn go(&mut self, direction: Option<&str>) -> Result<bool, StoryError> {
        let node = self.state().node.clone();
        let exit = direction.and_then(|direction| {
            self.pack
                .room(&node)
                .and_then(|room| room.exits.iter().find(|exit| exit.direction == direction))
        });
        let Some(exit) = exit else {
            self.say(M::NoExit, None, None);
            return Ok(false);
        };
        if !holds(self.pack, self.state(), exit.when.as_ref())? {
            if exit.blocked.is_some() {
                self.reply(Reply::Blocked {
                    node,
                    direction: exit.direction.clone(),
                });
            } else {
                self.say(M::NoExit, None, None);
            }
            return Ok(false);
        }
        if let Some(door) = &exit.door
            && !self.flags(door).open
        {
            self.say(M::DoorClosed, Some(door), None);
            return Ok(false);
        }
        self.exec.enter(&exit.to)?;
        if !self.exec.ended {
            let replies = look_replies(self.pack, self.state())?;
            for reply in replies {
                self.reply(reply);
            }
        }
        Ok(true)
    }

    fn enter(&mut self, obj: &str) -> Result<bool, StoryError> {
        let direction = self
            .pack
            .room(&self.state().node)
            .and_then(|room| {
                room.exits
                    .iter()
                    .find(|exit| exit.door.as_deref() == Some(obj))
            })
            .map(|exit| exit.direction.clone());
        match direction {
            Some(direction) => self.go(Some(&direction)),
            None => {
                self.say(M::CantEnter, Some(obj), None);
                Ok(false)
            }
        }
    }

    fn search(&mut self, obj: &str) -> Result<bool, StoryError> {
        let decl = self.decl(obj)?;
        if !decl.container {
            self.say(M::SearchNothing, None, None);
            return Ok(true);
        }
        if !self.flags(obj).open {
            self.say(M::ContainerClosed, None, Some(obj));
            return Ok(false);
        }
        let inner: Vec<&str> = contents(self.pack, self.state(), obj);
        if inner.is_empty() {
            self.say(M::Empty, Some(obj), None);
        } else {
            self.reply(list_message(M::InsideYouSee, Some(obj), &inner));
        }
        Ok(true)
    }

    fn take(&mut self, obj: &str) -> Result<bool, StoryError> {
        let decl = self.decl(obj)?;
        if self.at(obj) == Some(&Location::Carried) {
            self.say(M::AlreadyCarried, Some(obj), None);
            return Ok(false);
        }
        if decl.animate {
            self.say(M::TakeAnimate, Some(obj), None);
            return Ok(false);
        }
        if decl.scenery || !decl.portable {
            self.say(M::NotPortable, Some(obj), None);
            return Ok(false);
        }
        self.object_mut(obj)?.at = Location::Carried;
        self.say(M::Taken, Some(obj), None);
        Ok(true)
    }

    fn drop(&mut self, obj: &str) -> Result<bool, StoryError> {
        let flags = self.flags(obj);
        if flags.at != Location::Carried {
            self.say(M::NotCarried, Some(obj), None);
            return Ok(false);
        }
        if flags.worn {
            self.say(M::TakeOffFirst, Some(obj), None);
            return Ok(false);
        }
        let node = self.state().node.clone();
        if self.pack.room(&node).is_none() {
            self.say(M::NothingHappens, None, None);
            return Ok(false);
        }
        self.object_mut(obj)?.at = Location::Room(node);
        self.say(M::Dropped, Some(obj), None);
        Ok(true)
    }

    fn open(&mut self, obj: &str) -> Result<bool, StoryError> {
        let flags = self.flags(obj);
        if !self.decl(obj)?.openable {
            self.say(M::NotOpenable, Some(obj), None);
            return Ok(false);
        }
        if flags.open {
            self.say(M::AlreadyOpen, Some(obj), None);
            return Ok(false);
        }
        if flags.locked {
            self.say(M::Locked, Some(obj), None);
            return Ok(false);
        }
        self.object_mut(obj)?.open = true;
        self.say(M::Opened, Some(obj), None);
        self.show_contents(obj);
        Ok(true)
    }

    fn close(&mut self, obj: &str) -> Result<bool, StoryError> {
        if !self.decl(obj)?.openable {
            self.say(M::NotClosable, Some(obj), None);
            return Ok(false);
        }
        if !self.flags(obj).open {
            self.say(M::AlreadyClosed, Some(obj), None);
            return Ok(false);
        }
        self.object_mut(obj)?.open = false;
        self.say(M::Closed, Some(obj), None);
        Ok(true)
    }

    /// Unlocks (or, with `lock`, locks) `obj` with `key`.
    fn unlock(&mut self, obj: &str, key: &str, lock: bool) -> Result<bool, StoryError> {
        let decl = self.decl(obj)?;
        let flags = self.flags(obj);
        let Some(right_key) = decl.key.as_deref() else {
            self.say(M::NoLock, Some(obj), None);
            return Ok(false);
        };
        if lock && flags.locked {
            self.say(M::AlreadyLocked, Some(obj), None);
            return Ok(false);
        }
        if !lock && !flags.locked {
            self.say(M::NotLocked, Some(obj), None);
            return Ok(false);
        }
        if lock && flags.open {
            self.say(M::CloseFirst, Some(obj), None);
            return Ok(false);
        }
        if !held(self.state(), key) {
            self.say(M::NotCarried, Some(key), None);
            return Ok(false);
        }
        if key != right_key {
            self.say(M::WrongKey, Some(obj), Some(key));
            return Ok(false);
        }
        self.object_mut(obj)?.locked = lock;
        let key_message = if lock { M::LockedWith } else { M::Unlocked };
        self.say(key_message, Some(obj), Some(key));
        Ok(true)
    }

    fn put_in(&mut self, obj: &str, container: &str) -> Result<bool, StoryError> {
        let flags = self.flags(obj);
        if obj == container || inside(self.state(), container, obj) {
            self.say(M::PutInSelf, Some(obj), None);
            return Ok(false);
        }
        if flags.at != Location::Carried {
            self.say(M::NotCarried, Some(obj), None);
            return Ok(false);
        }
        if flags.worn {
            self.say(M::TakeOffFirst, Some(obj), None);
            return Ok(false);
        }
        if !self.decl(container)?.container {
            self.say(M::NotContainer, None, Some(container));
            return Ok(false);
        }
        if !self.flags(container).open {
            self.say(M::ContainerClosed, None, Some(container));
            return Ok(false);
        }
        self.object_mut(obj)?.at = Location::In(container.to_owned());
        self.say(M::PutIn, Some(obj), Some(container));
        Ok(true)
    }

    fn offer(&mut self, obj: &str, person: &str, declined: M) -> Result<bool, StoryError> {
        if !held(self.state(), obj) {
            self.say(M::NotCarried, Some(obj), None);
            return Ok(false);
        }
        if !self.decl(person)?.animate {
            self.say(M::GiveNotAnimate, None, Some(person));
            return Ok(false);
        }
        self.say(declined, Some(obj), Some(person));
        Ok(false)
    }

    fn wear(&mut self, obj: &str) -> Result<bool, StoryError> {
        let flags = self.flags(obj);
        if !self.decl(obj)?.wearable {
            self.say(M::NotWearable, Some(obj), None);
            return Ok(false);
        }
        if flags.worn {
            self.say(M::AlreadyWorn, Some(obj), None);
            return Ok(false);
        }
        if flags.at != Location::Carried {
            self.say(M::NotCarried, Some(obj), None);
            return Ok(false);
        }
        self.object_mut(obj)?.worn = true;
        self.say(M::Worn, Some(obj), None);
        Ok(true)
    }

    fn remove(&mut self, obj: &str) -> Result<bool, StoryError> {
        if !self.flags(obj).worn {
            self.say(M::NotWorn, Some(obj), None);
            return Ok(false);
        }
        self.object_mut(obj)?.worn = false;
        self.say(M::Removed, Some(obj), None);
        Ok(true)
    }

    fn switch(&mut self, obj: &str, on: bool) -> Result<bool, StoryError> {
        let flags = self.flags(obj);
        if !self.decl(obj)?.lightable {
            self.say(M::NotLightable, Some(obj), None);
            return Ok(false);
        }
        if on && flags.lit {
            self.say(M::AlreadyOn, Some(obj), None);
            return Ok(false);
        }
        if !on && !flags.lit {
            self.say(M::AlreadyOff, Some(obj), None);
            return Ok(false);
        }
        let was_lit = lit(self.pack, self.state());
        self.object_mut(obj)?.lit = on;
        self.say(
            if on { M::SwitchedOn } else { M::SwitchedOff },
            Some(obj),
            None,
        );
        let now_lit = lit(self.pack, self.state());
        if !was_lit && now_lit {
            let replies = look_replies(self.pack, self.state())?;
            for reply in replies {
                self.reply(reply);
            }
        } else if was_lit && !now_lit {
            self.say(M::GoesDark, None, None);
        }
        Ok(true)
    }
}
