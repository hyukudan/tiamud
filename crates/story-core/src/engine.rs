//! Turn-based transition: gamebook choices, menu actions and dialogue
//! choices, all addressed by opaque per-view tokens.

use serde::{Deserialize, Serialize};

use crate::rng::SplitMix64;
use crate::state::key;
use crate::world::{Location, ObjectFlag};
use crate::{
    Choice, Condition, Cue, Dialogue, DialogueNext, DialogueNode, DialogueOption, DialoguePosition,
    Effect, Ending, LastResponse, LocalizedText, MAX_CONDITION_DEPTH, MAX_DICE_COUNT,
    MAX_DICE_SIDES, MAX_EFFECT_DEPTH, MAX_EFFECTS_PER_CHOICE, MAX_MENU_USES, MAX_SCORE_ABS,
    MAX_TOKEN_BYTES, MAX_TURNS, MenuEntry, Node, Response, StoryError, StoryPack, StoryState,
    TargetKind, Value, VariableKind, VerbDecl,
};

/// Player input for one turn. Every string is an opaque token taken from the
/// current view; tokens from an earlier view are rejected as stale.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum StoryInput {
    /// Takes the choice named by an opaque token from the current view.
    Choose { token: String },
    /// Applies a menu verb, with a target token when the verb takes one.
    MenuAction {
        verb: String,
        #[serde(default)]
        target: Option<String>,
    },
    /// Answers the active dialogue with an option token.
    DialogueChoice { token: String },
    /// Free text for the parser of a format v3 pack with a world. The
    /// language picks a lexicon (`es`, `en`); absent or unknown, the pack's
    /// first lexicon is used. Meta commands (undo, save, restore, quit,
    /// restart) are only reported by [`crate::step_command`].
    Command {
        text: String,
        #[serde(default)]
        language: Option<String>,
    },
}

/// Ordered, internal record of what one step did. A transport layer projects
/// these; they may name authored ids and must not be forwarded verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoryEvent {
    NodeEntered {
        node: String,
    },
    VariableChanged {
        var: String,
        from: Value,
        to: Value,
        /// True when the requested value was clamped or saturated.
        clamped: bool,
    },
    ItemGained {
        item: String,
    },
    ItemLost {
        item: String,
    },
    DiceRolled {
        count: u8,
        sides: u8,
        rolls: Vec<u8>,
        modifier: i64,
        total: i64,
        passed: Option<bool>,
    },
    Cue(Cue),
    ScoreChanged {
        delta: i64,
        total: i64,
    },
    Ended(Ending),
    /// A menu action was answered. `entry`/`response` are absent when a
    /// default answered it.
    MenuAnswered {
        verb: String,
        target: Option<String>,
        entry: Option<String>,
        response: Option<String>,
    },
    DialogueEntered {
        dialogue: String,
        node: String,
    },
    DialogueExited {
        dialogue: String,
    },
}

/// Authored ids a menu action resolved to. For authoring tools and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuSelection<'a> {
    pub verb: &'a str,
    pub target: Option<&'a str>,
}

/// Resolves a token to the authored choice it names in the current view.
/// Intended for authoring tools and tests; transports use [`step`].
pub fn resolve_choice<'a>(
    pack: &'a StoryPack,
    state: &StoryState,
    token_text: &str,
) -> Result<&'a Choice, StoryError> {
    state.check_against(pack)?;
    if state.ended.is_some() {
        return Err(StoryError::Ended);
    }
    if active_dialogue(pack, state)?.is_some() {
        return Err(StoryError::InDialogue);
    }
    let Token::Choice(ordinal) = current_token(state, token_text)? else {
        return Err(StoryError::MalformedToken);
    };
    let node = current_node(pack, state)?;
    available_choices(pack, state, node)?
        .into_iter()
        .nth(ordinal)
        .ok_or(StoryError::UnknownChoice)
}

/// Resolves a verb token (and target token) to authored ids.
pub fn resolve_menu<'a>(
    pack: &'a StoryPack,
    state: &StoryState,
    verb_token: &str,
    target_token: Option<&str>,
) -> Result<MenuSelection<'a>, StoryError> {
    state.check_against(pack)?;
    if state.ended.is_some() {
        return Err(StoryError::Ended);
    }
    if active_dialogue(pack, state)?.is_some() {
        return Err(StoryError::InDialogue);
    }
    let Token::Verb(verb_ordinal) = current_token(state, verb_token)? else {
        return Err(StoryError::MalformedToken);
    };
    let node = current_node(pack, state)?;
    let verb = *menu_verbs(pack, state, node)?
        .get(verb_ordinal)
        .ok_or(StoryError::UnknownChoice)?;
    let target = match (verb.targets, target_token) {
        (TargetKind::None, None) => None,
        (TargetKind::None, Some(_)) | (_, None) => return Err(StoryError::BadTarget),
        (_, Some(text)) => {
            let Token::Target(owner, ordinal) = current_token(state, text)? else {
                return Err(StoryError::MalformedToken);
            };
            if owner != verb_ordinal {
                return Err(StoryError::BadTarget);
            }
            let (id, _) = *verb_targets(pack, state, node, verb)?
                .get(ordinal)
                .ok_or(StoryError::UnknownChoice)?;
            Some(id)
        }
    };
    Ok(MenuSelection {
        verb: verb.id.as_str(),
        target,
    })
}

/// Resolves a dialogue option token to the authored option.
pub fn resolve_dialogue_option<'a>(
    pack: &'a StoryPack,
    state: &StoryState,
    token_text: &str,
) -> Result<&'a DialogueOption, StoryError> {
    state.check_against(pack)?;
    if state.ended.is_some() {
        return Err(StoryError::Ended);
    }
    let Some(active) = active_dialogue(pack, state)? else {
        return Err(StoryError::NotInDialogue);
    };
    let Token::Option(ordinal) = current_token(state, token_text)? else {
        return Err(StoryError::MalformedToken);
    };
    active
        .options
        .get(ordinal)
        .copied()
        .ok_or(StoryError::UnknownChoice)
}

/// Applies one input. Pure: `state` is not modified; on error nothing changes.
pub fn step(
    pack: &StoryPack,
    state: &StoryState,
    input: StoryInput,
) -> Result<(StoryState, Vec<StoryEvent>), StoryError> {
    match input {
        StoryInput::Choose { token } => {
            let choice = resolve_choice(pack, state, &token)?;
            let mut exec = Exec::begin(pack, state)?;
            exec.run(&choice.effects, 0)?;
            exec.finish_move(choice.target.as_deref())?;
            Ok(exec.finish())
        }
        StoryInput::MenuAction { verb, target } => {
            let selection = resolve_menu(pack, state, &verb, target.as_deref())?;
            menu_action(pack, state, selection)
        }
        StoryInput::DialogueChoice { token } => {
            let option = resolve_dialogue_option(pack, state, &token)?;
            dialogue_choice(pack, state, option)
        }
        StoryInput::Command { text, language } => {
            let stepped = crate::command::step_command(pack, state, &text, language.as_deref())?;
            Ok((stepped.state, stepped.events))
        }
    }
}

fn menu_action(
    pack: &StoryPack,
    state: &StoryState,
    selection: MenuSelection<'_>,
) -> Result<(StoryState, Vec<StoryEvent>), StoryError> {
    let node = current_node(pack, state)?;
    let menu = node
        .menu
        .as_ref()
        .ok_or_else(|| StoryError::InvalidPack("node has no menu".to_owned()))?;
    let entry = menu
        .entries
        .iter()
        .find(|entry| entry.verb == selection.verb && entry.target.as_deref() == selection.target);
    let mut exec = Exec::begin(pack, state)?;
    let mut answered = StoryEvent::MenuAnswered {
        verb: selection.verb.to_owned(),
        target: selection.target.map(str::to_owned),
        entry: None,
        response: None,
    };
    let Some(entry) = entry else {
        exec.state.last = Some(default_response(node, selection.verb));
        exec.events.push(answered);
        return Ok(exec.finish());
    };
    // Responses are chosen against the state *before* this use is counted.
    let response = select_response(pack, &exec.state, node, entry)?;
    let counter = key(&[&node.id, &entry.id]);
    let uses = exec.state.menu_uses.get(&counter).copied().unwrap_or(0);
    exec.state
        .menu_uses
        .insert(counter, uses.saturating_add(1).min(MAX_MENU_USES));
    if let StoryEvent::MenuAnswered {
        entry: answered_entry,
        response: answered_response,
        ..
    } = &mut answered
    {
        *answered_entry = Some(entry.id.clone());
        *answered_response = response.map(|response| response.id.clone());
    }
    exec.events.push(answered);
    let Some(response) = response else {
        exec.state.last = Some(if entry.exhausted.is_empty() {
            default_response(node, selection.verb)
        } else {
            LastResponse::Exhausted {
                node: node.id.clone(),
                entry: entry.id.clone(),
            }
        });
        return Ok(exec.finish());
    };
    if response.once {
        exec.state
            .spent_responses
            .insert(key(&[&node.id, &entry.id, &response.id]));
    }
    exec.state.last = Some(LastResponse::Entry {
        node: node.id.clone(),
        entry: entry.id.clone(),
        response: response.id.clone(),
    });
    exec.run(&response.effects, 0)?;
    let moved = exec.finish_move(response.target.as_deref())?;
    if !moved
        && !exec.ended
        && let Some(dialogue_id) = &response.dialogue
    {
        let dialogue = pack
            .dialogue(dialogue_id)
            .ok_or_else(|| StoryError::InvalidPack("response names no dialogue".to_owned()))?;
        exec.enter_dialogue(dialogue, &dialogue.start)?;
    }
    Ok(exec.finish())
}

fn dialogue_choice(
    pack: &StoryPack,
    state: &StoryState,
    option: &DialogueOption,
) -> Result<(StoryState, Vec<StoryEvent>), StoryError> {
    let position = state.dialogue.clone().ok_or(StoryError::NotInDialogue)?;
    let dialogue = pack
        .dialogue(&position.dialogue)
        .ok_or_else(|| StoryError::StateMismatch("dialogue is unknown".to_owned()))?;
    let mut exec = Exec::begin(pack, state)?;
    // `begin` drops an inactive dialogue; this one is active, so restore it.
    exec.state.dialogue = Some(position.clone());
    if option.once {
        exec.state
            .spent_options
            .insert(key(&[&position.dialogue, &position.node, &option.id]));
    }
    exec.run(&option.effects, 0)?;
    let moved = exec.finish_move(None)?;
    if exec.ended || moved {
        return Ok(exec.finish());
    }
    match &option.next {
        DialogueNext::Goto(node) => exec.enter_dialogue(dialogue, node)?,
        DialogueNext::Return => exec.enter_dialogue(dialogue, &dialogue.start)?,
        DialogueNext::Exit => exec.leave_dialogue(),
    }
    Ok(exec.finish())
}

fn default_response(node: &Node, verb: &str) -> LastResponse {
    let scene_default = node
        .menu
        .as_ref()
        .is_some_and(|menu| menu.defaults.iter().any(|default| default.verb == verb));
    if scene_default {
        LastResponse::SceneDefault {
            node: node.id.clone(),
            verb: verb.to_owned(),
        }
    } else {
        LastResponse::VerbDefault {
            verb: verb.to_owned(),
        }
    }
}

fn select_response<'a>(
    pack: &StoryPack,
    state: &StoryState,
    node: &Node,
    entry: &'a MenuEntry,
) -> Result<Option<&'a Response>, StoryError> {
    for response in &entry.responses {
        if response.once
            && state
                .spent_responses
                .contains(&key(&[&node.id, &entry.id, &response.id]))
        {
            continue;
        }
        let open = match &response.when {
            None => true,
            Some(condition) => evaluate(pack, state, condition, 0)?,
        };
        if open {
            return Ok(Some(response));
        }
    }
    Ok(None)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Token {
    Choice(usize),
    Verb(usize),
    Target(usize, usize),
    Option(usize),
}

pub(crate) fn choice_token(turn: u32, ordinal: usize) -> String {
    format!("t{turn}c{ordinal}")
}

pub(crate) fn verb_token(turn: u32, ordinal: usize) -> String {
    format!("t{turn}v{ordinal}")
}

pub(crate) fn target_token(turn: u32, verb: usize, ordinal: usize) -> String {
    format!("t{turn}v{verb}o{ordinal}")
}

pub(crate) fn option_token(turn: u32, ordinal: usize) -> String {
    format!("t{turn}d{ordinal}")
}

fn current_token(state: &StoryState, text: &str) -> Result<Token, StoryError> {
    let (turn, token) = parse_token(text)?;
    if turn != state.turn {
        return Err(StoryError::StaleChoice);
    }
    Ok(token)
}

fn parse_token(text: &str) -> Result<(u32, Token), StoryError> {
    if text.len() > MAX_TOKEN_BYTES {
        return Err(StoryError::MalformedToken);
    }
    let rest = text.strip_prefix('t').ok_or(StoryError::MalformedToken)?;
    let split = rest
        .find(|c: char| !c.is_ascii_digit())
        .ok_or(StoryError::MalformedToken)?;
    let (turn, rest) = rest.split_at(split);
    let turn = parse_digits(turn)?;
    let mut chars = rest.chars();
    let kind = chars.next().ok_or(StoryError::MalformedToken)?;
    let body = chars.as_str();
    let token = match kind {
        'c' => Token::Choice(parse_digits(body)?),
        'd' => Token::Option(parse_digits(body)?),
        'v' => match body.split_once('o') {
            None => Token::Verb(parse_digits(body)?),
            Some((verb, target)) => Token::Target(parse_digits(verb)?, parse_digits(target)?),
        },
        _ => return Err(StoryError::MalformedToken),
    };
    Ok((turn, token))
}

fn parse_digits<T: std::str::FromStr>(text: &str) -> Result<T, StoryError> {
    let canonical = !text.is_empty()
        && text.bytes().all(|b| b.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'));
    if !canonical {
        return Err(StoryError::MalformedToken);
    }
    text.parse().map_err(|_| StoryError::MalformedToken)
}

pub(crate) fn current_node<'a>(
    pack: &'a StoryPack,
    state: &StoryState,
) -> Result<&'a Node, StoryError> {
    pack.node(&state.node)
        .ok_or_else(|| StoryError::StateMismatch("current node is unknown".to_owned()))
}

pub(crate) fn available_choices<'a>(
    pack: &StoryPack,
    state: &StoryState,
    node: &'a Node,
) -> Result<Vec<&'a Choice>, StoryError> {
    let mut out = Vec::new();
    for choice in &node.choices {
        if holds(pack, state, choice.when.as_ref())? {
            out.push(choice);
        }
    }
    Ok(out)
}

/// Verbs the current scene offers, in menu order.
pub(crate) fn menu_verbs<'a>(
    pack: &'a StoryPack,
    state: &StoryState,
    node: &Node,
) -> Result<Vec<&'a VerbDecl>, StoryError> {
    let Some(menu) = &node.menu else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for offered in &menu.verbs {
        if holds(pack, state, offered.when.as_ref())? {
            let verb = pack
                .verb(&offered.verb)
                .ok_or_else(|| StoryError::InvalidPack("menu names an unknown verb".to_owned()))?;
            out.push(verb);
        }
    }
    Ok(out)
}

/// Targets a verb offers in the current scene: `(authored id, label)`.
pub(crate) fn verb_targets<'a>(
    pack: &'a StoryPack,
    state: &StoryState,
    node: &'a Node,
    verb: &VerbDecl,
) -> Result<Vec<(&'a str, &'a LocalizedText)>, StoryError> {
    let mut out = Vec::new();
    match verb.targets {
        TargetKind::None => {}
        TargetKind::Hotspot => {
            for hotspot in &node.hotspots {
                if holds(pack, state, hotspot.when.as_ref())? {
                    out.push((hotspot.id.as_str(), &hotspot.label));
                }
            }
        }
        TargetKind::Character => {
            for presence in &node.present {
                if holds(pack, state, presence.when.as_ref())? {
                    let character = pack.character(&presence.character).ok_or_else(|| {
                        StoryError::InvalidPack("presence names an unknown character".to_owned())
                    })?;
                    out.push((character.id.as_str(), &character.name));
                }
            }
        }
        TargetKind::Item => {
            for item in &pack.items {
                if state.inventory.contains(&item.id) {
                    out.push((item.id.as_str(), &item.name));
                }
            }
        }
    }
    Ok(out)
}

/// The active dialogue's currently available options.
pub(crate) struct ActiveDialogue<'a> {
    pub(crate) options: Vec<&'a DialogueOption>,
}

/// The dialogue the player must answer, if any. A dialogue position whose
/// node offers no available option does not block the scene menu.
pub(crate) fn active_dialogue<'a>(
    pack: &'a StoryPack,
    state: &StoryState,
) -> Result<Option<ActiveDialogue<'a>>, StoryError> {
    let Some((dialogue, node)) = dialogue_node(pack, state)? else {
        return Ok(None);
    };
    let mut options = Vec::new();
    for option in &node.options {
        if option.once
            && state
                .spent_options
                .contains(&key(&[&dialogue.id, &node.id, &option.id]))
        {
            continue;
        }
        if holds(pack, state, option.when.as_ref())? {
            options.push(option);
        }
    }
    Ok((!options.is_empty()).then_some(ActiveDialogue { options }))
}

/// The dialogue node the state points at, whether or not it is active.
pub(crate) fn dialogue_node<'a>(
    pack: &'a StoryPack,
    state: &StoryState,
) -> Result<Option<(&'a Dialogue, &'a DialogueNode)>, StoryError> {
    let Some(position) = &state.dialogue else {
        return Ok(None);
    };
    let dialogue = pack
        .dialogue(&position.dialogue)
        .ok_or_else(|| StoryError::StateMismatch("dialogue is unknown".to_owned()))?;
    let node = dialogue
        .node(&position.node)
        .ok_or_else(|| StoryError::StateMismatch("dialogue node is unknown".to_owned()))?;
    Ok(Some((dialogue, node)))
}

pub(crate) fn holds(
    pack: &StoryPack,
    state: &StoryState,
    condition: Option<&Condition>,
) -> Result<bool, StoryError> {
    match condition {
        None => Ok(true),
        Some(condition) => evaluate(pack, state, condition, 0),
    }
}

pub(crate) fn evaluate(
    pack: &StoryPack,
    state: &StoryState,
    condition: &Condition,
    depth: usize,
) -> Result<bool, StoryError> {
    if depth >= MAX_CONDITION_DEPTH {
        return Err(StoryError::InvalidPack("condition is too deep".to_owned()));
    }
    Ok(match condition {
        Condition::All(children) => {
            for child in children {
                if !evaluate(pack, state, child, depth + 1)? {
                    return Ok(false);
                }
            }
            true
        }
        Condition::Any(children) => {
            for child in children {
                if evaluate(pack, state, child, depth + 1)? {
                    return Ok(true);
                }
            }
            false
        }
        Condition::Not(child) => !evaluate(pack, state, child, depth + 1)?,
        Condition::VarCmp { var, op, value } => match state.vars.get(var) {
            Some(Value::Int(current)) => op.apply(*current, *value),
            _ => {
                return Err(StoryError::InvalidPack(
                    "comparison names a non-integer variable".to_owned(),
                ));
            }
        },
        Condition::HasItem(item) => state.inventory.contains(item),
        Condition::Visited(node) => state.visited.contains(node),
        Condition::Flag(var) => match state.vars.get(var) {
            Some(Value::Bool(flag)) => *flag,
            _ => {
                return Err(StoryError::InvalidPack(
                    "flag names a non-boolean variable".to_owned(),
                ));
            }
        },
        Condition::Uses {
            node,
            entry,
            op,
            value,
        } => {
            let uses = state
                .menu_uses
                .get(&key(&[node, entry]))
                .copied()
                .unwrap_or(0);
            op.apply(i64::from(uses), *value)
        }
        Condition::Carried(obj) => {
            object_state(state, obj)?;
            crate::runtime::held(state, obj)
        }
        Condition::ObjectAt { obj, at } => object_state(state, obj)?.at == *at,
        Condition::ObjectIs { obj, flag } => object_state(state, obj)?.flag(*flag),
        Condition::Visible(obj) => {
            object_state(state, obj)?;
            crate::runtime::visible(pack, state, obj)
        }
        Condition::PlayerIn(node) => state.node == *node,
        Condition::TurnCmp { op, value } => op.apply(i64::from(state.turn), *value),
    })
}

fn object_state<'a>(
    state: &'a StoryState,
    obj: &str,
) -> Result<&'a crate::world::ObjectState, StoryError> {
    state
        .objects
        .get(obj)
        .ok_or_else(|| StoryError::InvalidPack("condition names an unknown object".to_owned()))
}

pub(crate) struct Exec<'a> {
    pub(crate) pack: &'a StoryPack,
    rng: SplitMix64,
    pub(crate) state: StoryState,
    pub(crate) events: Vec<StoryEvent>,
    pub(crate) budget: usize,
    destination: Option<String>,
    pub(crate) ended: bool,
}

impl<'a> Exec<'a> {
    /// Starts a turn: advances the bounded turn counter, clears the last menu
    /// response and drops a dialogue position that no longer blocks.
    fn begin(pack: &'a StoryPack, state: &StoryState) -> Result<Self, StoryError> {
        let mut next = state.clone();
        next.turn = state
            .turn
            .checked_add(1)
            .filter(|turn| *turn <= MAX_TURNS)
            .ok_or(StoryError::TurnLimit)?;
        next.last = None;
        next.dialogue = None;
        Ok(Self {
            pack,
            rng: SplitMix64::from_state(state.rng),
            state: next,
            events: Vec::new(),
            budget: MAX_EFFECTS_PER_CHOICE,
            destination: None,
            ended: false,
        })
    }

    /// Continues from `state` without advancing the turn, for parser
    /// commands that count their own turns. Clears the last menu response
    /// and drops a dialogue position that no longer blocks.
    pub(crate) fn resume(pack: &'a StoryPack, mut state: StoryState) -> Self {
        state.last = None;
        state.dialogue = None;
        Self {
            pack,
            rng: SplitMix64::from_state(state.rng),
            state,
            events: Vec::new(),
            budget: MAX_EFFECTS_PER_CHOICE,
            destination: None,
            ended: false,
        }
    }

    /// Enters the effect destination, or `fallback`. Returns true on a move.
    pub(crate) fn finish_move(&mut self, fallback: Option<&str>) -> Result<bool, StoryError> {
        if self.ended {
            return Ok(false);
        }
        match self
            .destination
            .take()
            .or_else(|| fallback.map(str::to_owned))
        {
            Some(destination) => {
                self.enter(&destination)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    pub(crate) fn finish(self) -> (StoryState, Vec<StoryEvent>) {
        let Exec {
            mut state,
            rng,
            events,
            ..
        } = self;
        state.rng = rng.state();
        (state, events)
    }

    pub(crate) fn run(&mut self, effects: &[Effect], depth: usize) -> Result<(), StoryError> {
        if depth > MAX_EFFECT_DEPTH {
            return Err(StoryError::EffectBudget);
        }
        for effect in effects {
            if self.ended {
                break;
            }
            self.budget = self.budget.checked_sub(1).ok_or(StoryError::EffectBudget)?;
            self.apply(effect, depth)?;
        }
        Ok(())
    }

    fn apply(&mut self, effect: &Effect, depth: usize) -> Result<(), StoryError> {
        match effect {
            Effect::Set { var, value } => match (*value, self.int_bounds(var)?) {
                (Value::Int(requested), Some(_)) => self.write_int(var, i128::from(requested))?,
                (Value::Bool(flag), None) => self.write(var, Value::Bool(flag), false),
                _ => {
                    return Err(StoryError::InvalidPack(
                        "set value does not match the variable type".to_owned(),
                    ));
                }
            },
            Effect::Add { var, delta } => {
                let current = self.read_int(var)?;
                self.write_int(var, i128::from(current) + i128::from(*delta))?;
            }
            Effect::Give(item) => {
                self.require_item(item)?;
                if self.state.inventory.insert(item.clone()) {
                    self.events
                        .push(StoryEvent::ItemGained { item: item.clone() });
                }
            }
            Effect::Take(item) => {
                self.require_item(item)?;
                if self.state.inventory.remove(item) {
                    self.events
                        .push(StoryEvent::ItemLost { item: item.clone() });
                }
            }
            Effect::Goto(node) => {
                self.require_node(node)?;
                self.destination = Some(node.clone());
            }
            Effect::Roll {
                dice,
                modifier_var,
                store,
                test,
            } => {
                if dice.count == 0
                    || dice.count > MAX_DICE_COUNT
                    || dice.sides < 2
                    || dice.sides > MAX_DICE_SIDES
                {
                    return Err(StoryError::InvalidPack("dice are out of bounds".to_owned()));
                }
                let mut rolls = Vec::with_capacity(usize::from(dice.count));
                let mut total = i128::from(dice.modifier);
                for _ in 0..dice.count {
                    // `below(sides)` < sides <= 100, so the face fits in u8.
                    let face = u8::try_from(self.rng.below(u64::from(dice.sides)))
                        .unwrap_or(0)
                        .saturating_add(1);
                    rolls.push(face);
                    total += i128::from(face);
                }
                if let Some(var) = modifier_var {
                    total += i128::from(self.read_int(var)?);
                }
                let total = clamp_i128(total, i64::MIN, i64::MAX);
                let passed = test.as_ref().map(|test| total >= test.at_least);
                self.events.push(StoryEvent::DiceRolled {
                    count: dice.count,
                    sides: dice.sides,
                    rolls,
                    modifier: dice.modifier,
                    total,
                    passed,
                });
                if let Some(var) = store {
                    self.write_int(var, i128::from(total))?;
                }
                if let (Some(test), Some(passed)) = (test, passed) {
                    let branch = if passed { &test.success } else { &test.failure };
                    if let Some(node) = branch {
                        self.require_node(node)?;
                        self.destination = Some(node.clone());
                    }
                }
            }
            Effect::If {
                when,
                then,
                otherwise,
            } => {
                let branch = if evaluate(self.pack, &self.state, when, 0)? {
                    then
                } else {
                    otherwise
                };
                self.run(branch, depth + 1)?;
            }
            Effect::Cue(cue) => self.events.push(StoryEvent::Cue(cue.clone())),
            Effect::Score(delta) => {
                let total = clamp_i128(
                    i128::from(self.state.score) + i128::from(*delta),
                    -MAX_SCORE_ABS,
                    MAX_SCORE_ABS,
                );
                let applied = total - self.state.score;
                self.state.score = total;
                self.events.push(StoryEvent::ScoreChanged {
                    delta: applied,
                    total,
                });
            }
            Effect::End(ending) => self.end(ending.clone()),
            Effect::MoveObject { obj, to } => self.move_object(obj, to)?,
            Effect::SetObject { obj, flag, value } => self.set_object(obj, *flag, *value)?,
        }
        Ok(())
    }

    /// Moves an object; a move into itself or its own contents is ignored
    /// and an object leaving the player's hands is no longer worn.
    pub(crate) fn move_object(&mut self, obj: &str, to: &Location) -> Result<(), StoryError> {
        let invalid = |message: &str| StoryError::InvalidPack(message.to_owned());
        if !self.state.objects.contains_key(obj) {
            return Err(invalid("effect names an unknown object"));
        }
        match to {
            Location::Room(node) => {
                if self.pack.room(node).is_none() {
                    return Err(invalid("effect names an unknown room"));
                }
            }
            Location::In(container) => {
                if !self
                    .pack
                    .object(container)
                    .is_some_and(|decl| decl.container)
                {
                    return Err(invalid("effect names an unknown container"));
                }
                if container == obj || crate::runtime::inside(&self.state, container, obj) {
                    return Ok(());
                }
            }
            Location::Carried | Location::Nowhere => {}
        }
        if let Some(object) = self.state.objects.get_mut(obj) {
            object.at = to.clone();
            if *to != Location::Carried {
                object.worn = false;
            }
        }
        Ok(())
    }

    fn set_object(&mut self, obj: &str, flag: ObjectFlag, value: bool) -> Result<(), StoryError> {
        let decl = self
            .pack
            .object(obj)
            .ok_or_else(|| StoryError::InvalidPack("effect names an unknown object".to_owned()))?;
        let allowed = match flag {
            ObjectFlag::Open | ObjectFlag::Locked => decl.openable,
            ObjectFlag::Lit => decl.lightable || decl.lit,
            ObjectFlag::Worn => decl.wearable,
        };
        if !allowed {
            return Err(StoryError::InvalidPack(
                "effect sets a flag the object does not have".to_owned(),
            ));
        }
        let Some(object) = self.state.objects.get_mut(obj) else {
            return Err(StoryError::InvalidPack(
                "effect names an unknown object".to_owned(),
            ));
        };
        match flag {
            ObjectFlag::Open => {
                object.open = value;
                if value {
                    object.locked = false;
                }
            }
            ObjectFlag::Locked => {
                object.locked = value;
                if value {
                    object.open = false;
                }
            }
            ObjectFlag::Lit => object.lit = value,
            ObjectFlag::Worn => {
                if !value || object.at == Location::Carried {
                    object.worn = value;
                }
            }
        }
        Ok(())
    }

    /// Moves to a scene: leaves any dialogue, records the visit, emits the
    /// scene's cues and ends the story on an ending node.
    pub(crate) fn enter(&mut self, node_id: &str) -> Result<(), StoryError> {
        let node = self
            .pack
            .node(node_id)
            .ok_or_else(|| StoryError::InvalidPack("destination node is missing".to_owned()))?;
        self.leave_dialogue();
        self.state.node = node.id.clone();
        self.state.visited.insert(node.id.clone());
        self.events.push(StoryEvent::NodeEntered {
            node: node.id.clone(),
        });
        for cue in &node.cues {
            self.events.push(StoryEvent::Cue(cue.clone()));
        }
        if let Some(ending) = &node.ending {
            self.end(ending.clone());
        }
        Ok(())
    }

    fn enter_dialogue(&mut self, dialogue: &Dialogue, node_id: &str) -> Result<(), StoryError> {
        let node = dialogue
            .node(node_id)
            .ok_or_else(|| StoryError::InvalidPack("dialogue node is missing".to_owned()))?;
        self.state.dialogue = Some(DialoguePosition {
            dialogue: dialogue.id.clone(),
            node: node.id.clone(),
        });
        self.events.push(StoryEvent::DialogueEntered {
            dialogue: dialogue.id.clone(),
            node: node.id.clone(),
        });
        Ok(())
    }

    fn leave_dialogue(&mut self) {
        if let Some(position) = self.state.dialogue.take() {
            self.events.push(StoryEvent::DialogueExited {
                dialogue: position.dialogue,
            });
        }
    }

    fn end(&mut self, ending: Ending) {
        self.ended = true;
        self.leave_dialogue();
        self.state.ended = Some(ending.clone());
        self.events.push(StoryEvent::Ended(ending));
    }

    fn int_bounds(&self, var: &str) -> Result<Option<(i64, i64)>, StoryError> {
        match self.pack.variable(var).map(|decl| &decl.kind) {
            Some(VariableKind::Int { min, max, .. }) if min <= max => Ok(Some((*min, *max))),
            Some(VariableKind::Bool { .. }) => Ok(None),
            _ => Err(StoryError::InvalidPack(
                "effect names an unknown or invalid variable".to_owned(),
            )),
        }
    }

    fn read_int(&self, var: &str) -> Result<i64, StoryError> {
        match self.state.vars.get(var) {
            Some(Value::Int(value)) => Ok(*value),
            _ => Err(StoryError::InvalidPack(
                "effect names a non-integer variable".to_owned(),
            )),
        }
    }

    /// Clamps a wide requested value to the declared bounds and records it.
    fn write_int(&mut self, var: &str, requested: i128) -> Result<(), StoryError> {
        let (min, max) = self.int_bounds(var)?.ok_or_else(|| {
            StoryError::InvalidPack("integer effect names a boolean variable".to_owned())
        })?;
        let applied = clamp_i128(requested, min, max);
        self.write(var, Value::Int(applied), i128::from(applied) != requested);
        Ok(())
    }

    fn write(&mut self, var: &str, value: Value, clamped: bool) {
        let previous = self.state.vars.insert(var.to_owned(), value);
        if let Some(from) = previous
            && (from != value || clamped)
        {
            self.events.push(StoryEvent::VariableChanged {
                var: var.to_owned(),
                from,
                to: value,
                clamped,
            });
        }
    }

    fn require_item(&self, item: &str) -> Result<(), StoryError> {
        self.pack
            .item(item)
            .map(|_| ())
            .ok_or_else(|| StoryError::InvalidPack("effect names an undeclared item".to_owned()))
    }

    fn require_node(&self, node: &str) -> Result<(), StoryError> {
        self.pack
            .node(node)
            .map(|_| ())
            .ok_or_else(|| StoryError::InvalidPack("effect names an unknown node".to_owned()))
    }
}

/// Clamps a wide intermediate into `min..=max` (with `min <= max`).
fn clamp_i128(value: i128, min: i64, max: i64) -> i64 {
    // The clamp guarantees the conversion succeeds; `max` is a safe fallback.
    i64::try_from(value.clamp(i128::from(min), i128::from(max))).unwrap_or(max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_parse_only_canonical_forms() {
        assert_eq!(parse_token("t0c0"), Ok((0, Token::Choice(0))));
        assert_eq!(parse_token("t12c3"), Ok((12, Token::Choice(3))));
        assert_eq!(parse_token("t4v2"), Ok((4, Token::Verb(2))));
        assert_eq!(parse_token("t4v2o7"), Ok((4, Token::Target(2, 7))));
        assert_eq!(parse_token("t9d1"), Ok((9, Token::Option(1))));
        for bad in [
            "",
            "t",
            "tc",
            "t1",
            "t1c",
            "tc1",
            "t01c1",
            "t1c01",
            "t+1c1",
            "t-1c1",
            "x1c1",
            "t1c1c1",
            "t99999999999c1",
            "t1 c1",
            "T1c1",
            "t1v",
            "t1vo1",
            "t1v1o",
            "t1v1o1o1",
            "t1d",
            "t1x1",
            "t1v01",
            "t1é1",
        ] {
            assert_eq!(parse_token(bad), Err(StoryError::MalformedToken), "{bad:?}");
        }
        assert_eq!(
            parse_token(&format!("t1c{}", "1".repeat(40))),
            Err(StoryError::MalformedToken)
        );
    }

    #[test]
    fn token_builders_round_trip() {
        assert_eq!(parse_token(&choice_token(3, 4)), Ok((3, Token::Choice(4))));
        assert_eq!(parse_token(&verb_token(3, 4)), Ok((3, Token::Verb(4))));
        assert_eq!(
            parse_token(&target_token(MAX_TURNS, 15, 255)),
            Ok((MAX_TURNS, Token::Target(15, 255)))
        );
        assert_eq!(
            parse_token(&option_token(0, 15)),
            Ok((0, Token::Option(15)))
        );
    }

    #[test]
    fn clamp_never_panics() {
        assert_eq!(clamp_i128(i128::MAX, -5, 5), 5);
        assert_eq!(clamp_i128(i128::MIN, -5, 5), -5);
        assert_eq!(clamp_i128(3, -5, 5), 3);
    }
}
