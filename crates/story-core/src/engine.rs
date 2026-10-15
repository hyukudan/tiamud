//! Turn-based transition and player view.

use serde::{Deserialize, Serialize};

use crate::rng::SplitMix64;
use crate::{
    Choice, Condition, Cue, Effect, Ending, LocalizedText, MAX_CONDITION_DEPTH, MAX_DICE_COUNT,
    MAX_DICE_SIDES, MAX_EFFECT_DEPTH, MAX_EFFECTS_PER_CHOICE, MAX_SCORE_ABS, MAX_TOKEN_BYTES,
    MAX_TURNS, Node, StoryError, StoryPack, StoryState, Value, VariableKind,
};

/// Player input for one turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum StoryInput {
    /// Takes the choice named by an opaque token from the current view.
    Choose { token: String },
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
}

/// What a player may see: resolved node text, available choices behind opaque
/// tokens, the visible stat sheet and inventory. Hidden variables, authored
/// choice ids, unavailable choices and RNG state are never included.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryView {
    pub title: LocalizedText,
    pub text: LocalizedText,
    pub turn: u32,
    pub choices: Vec<ViewChoice>,
    pub stats: Vec<ViewStat>,
    pub inventory: Vec<LocalizedText>,
    pub score: i64,
    pub ending: Option<Ending>,
}

/// A selectable choice. `token` is bound to the view's turn and the choice's
/// ordinal among the *available* choices; it never encodes the authored id or
/// label, and reveals nothing about hidden choices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewChoice {
    pub token: String,
    pub label: LocalizedText,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewStat {
    pub label: LocalizedText,
    pub value: Value,
}

/// Builds the player view of `state`.
pub fn view(pack: &StoryPack, state: &StoryState) -> Result<StoryView, StoryError> {
    state.check_against(pack)?;
    let node = current_node(pack, state)?;
    let choices = if state.ended.is_some() {
        Vec::new()
    } else {
        available(state, node)?
            .into_iter()
            .enumerate()
            .map(|(ordinal, choice)| ViewChoice {
                token: token(state.turn, ordinal),
                label: choice.label.clone(),
            })
            .collect()
    };
    let stats = pack
        .variables
        .iter()
        .filter(|decl| decl.visible)
        .filter_map(|decl| {
            let value = *state.vars.get(&decl.id)?;
            let label = decl
                .label
                .clone()
                .unwrap_or_else(|| LocalizedText::single("es", &decl.id));
            Some(ViewStat { label, value })
        })
        .collect();
    let inventory = pack
        .items
        .iter()
        .filter(|item| state.inventory.contains(&item.id))
        .map(|item| item.name.clone())
        .collect();
    Ok(StoryView {
        title: pack.title.clone(),
        text: node.text.clone(),
        turn: state.turn,
        choices,
        stats,
        inventory,
        score: state.score,
        ending: state.ended.clone(),
    })
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
    let (turn, ordinal) = parse_token(token_text)?;
    if turn != state.turn {
        return Err(StoryError::StaleChoice);
    }
    let node = current_node(pack, state)?;
    available(state, node)?
        .into_iter()
        .nth(ordinal)
        .ok_or(StoryError::UnknownChoice)
}

/// Applies one input. Pure: `state` is not modified; on error nothing changes.
pub fn step(
    pack: &StoryPack,
    state: &StoryState,
    input: StoryInput,
) -> Result<(StoryState, Vec<StoryEvent>), StoryError> {
    let StoryInput::Choose { token } = input;
    let choice = resolve_choice(pack, state, &token)?;
    let mut next = state.clone();
    next.turn = state
        .turn
        .checked_add(1)
        .filter(|turn| *turn <= MAX_TURNS)
        .ok_or(StoryError::TurnLimit)?;
    let mut exec = Exec {
        pack,
        rng: SplitMix64::from_state(state.rng),
        state: next,
        events: Vec::new(),
        budget: MAX_EFFECTS_PER_CHOICE,
        destination: None,
        ended: false,
    };
    exec.run(&choice.effects, 0)?;
    if !exec.ended
        && let Some(destination) = exec.destination.take().or_else(|| choice.target.clone())
    {
        exec.enter(&destination)?;
    }
    let Exec {
        mut state,
        rng,
        events,
        ..
    } = exec;
    state.rng = rng.state();
    Ok((state, events))
}

fn token(turn: u32, ordinal: usize) -> String {
    format!("t{turn}c{ordinal}")
}

fn parse_token(text: &str) -> Result<(u32, usize), StoryError> {
    if text.len() > MAX_TOKEN_BYTES {
        return Err(StoryError::MalformedToken);
    }
    let rest = text.strip_prefix('t').ok_or(StoryError::MalformedToken)?;
    let (turn, ordinal) = rest.split_once('c').ok_or(StoryError::MalformedToken)?;
    Ok((parse_digits(turn)?, parse_digits(ordinal)?))
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

fn current_node<'a>(pack: &'a StoryPack, state: &StoryState) -> Result<&'a Node, StoryError> {
    pack.node(&state.node)
        .ok_or_else(|| StoryError::StateMismatch("current node is unknown".to_owned()))
}

fn available<'a>(state: &StoryState, node: &'a Node) -> Result<Vec<&'a Choice>, StoryError> {
    let mut out = Vec::new();
    for choice in &node.choices {
        let open = match &choice.when {
            None => true,
            Some(condition) => evaluate(state, condition, 0)?,
        };
        if open {
            out.push(choice);
        }
    }
    Ok(out)
}

fn evaluate(state: &StoryState, condition: &Condition, depth: usize) -> Result<bool, StoryError> {
    if depth >= MAX_CONDITION_DEPTH {
        return Err(StoryError::InvalidPack("condition is too deep".to_owned()));
    }
    Ok(match condition {
        Condition::All(children) => {
            for child in children {
                if !evaluate(state, child, depth + 1)? {
                    return Ok(false);
                }
            }
            true
        }
        Condition::Any(children) => {
            for child in children {
                if evaluate(state, child, depth + 1)? {
                    return Ok(true);
                }
            }
            false
        }
        Condition::Not(child) => !evaluate(state, child, depth + 1)?,
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
    })
}

struct Exec<'a> {
    pack: &'a StoryPack,
    rng: SplitMix64,
    state: StoryState,
    events: Vec<StoryEvent>,
    budget: usize,
    destination: Option<String>,
    ended: bool,
}

impl Exec<'_> {
    fn run(&mut self, effects: &[Effect], depth: usize) -> Result<(), StoryError> {
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
                let branch = if evaluate(&self.state, when, 0)? {
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
        }
        Ok(())
    }

    fn enter(&mut self, node_id: &str) -> Result<(), StoryError> {
        let node = self
            .pack
            .node(node_id)
            .ok_or_else(|| StoryError::InvalidPack("destination node is missing".to_owned()))?;
        self.state.node = node.id.clone();
        self.state.visited.insert(node.id.clone());
        self.events.push(StoryEvent::NodeEntered {
            node: node.id.clone(),
        });
        if let Some(ending) = &node.ending {
            self.end(ending.clone());
        }
        Ok(())
    }

    fn end(&mut self, ending: Ending) {
        self.ended = true;
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
        assert_eq!(parse_token("t0c0"), Ok((0, 0)));
        assert_eq!(parse_token("t12c3"), Ok((12, 3)));
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
        ] {
            assert_eq!(parse_token(bad), Err(StoryError::MalformedToken), "{bad:?}");
        }
        assert_eq!(
            parse_token(&format!("t1c{}", "1".repeat(40))),
            Err(StoryError::MalformedToken)
        );
    }

    #[test]
    fn clamp_never_panics() {
        assert_eq!(clamp_i128(i128::MAX, -5, 5), 5);
        assert_eq!(clamp_i128(i128::MIN, -5, 5), -5);
        assert_eq!(clamp_i128(3, -5, 5), 3);
    }
}
