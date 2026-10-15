//! Static StoryPack validation and authoring lints.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use crate::world::{LocalizedWords, Location, ObjectDecl, ObjectFlag};
use crate::{
    Condition, Cue, Dialogue, DialogueNext, Effect, Ending, MAX_CHARACTERS, MAX_CHOICES_PER_NODE,
    MAX_CONDITION_DEPTH, MAX_CONDITION_NODES, MAX_CUES_PER_NODE, MAX_DIALOGUE_NODES,
    MAX_DIALOGUE_OPTIONS, MAX_DIALOGUES, MAX_DICE_COUNT, MAX_DICE_MODIFIER, MAX_DICE_SIDES,
    MAX_EFFECT_DEPTH, MAX_EFFECTS_PER_CHOICE, MAX_EXPRESSIONS, MAX_HOTSPOTS_PER_NODE, MAX_ITEMS,
    MAX_LABEL_BYTES, MAX_MENU_ENTRIES_PER_NODE, MAX_NODES, MAX_PRESENT_PER_NODE,
    MAX_REPORTED_ISSUES, MAX_RESPONSES_PER_ENTRY, MAX_SCORE_DELTA, MAX_TEXT_BLOCKS, MAX_TEXT_BYTES,
    MAX_TOTAL_CHOICES, MAX_TOTAL_DIALOGUE_NODES, MAX_TOTAL_MENU_ENTRIES, MAX_VAR_ABS,
    MAX_VARIABLES, MAX_VERBS, Node, STORY_PACK_FORMAT_VERSION, STORY_PACK_MIN_FORMAT_VERSION,
    StoryPack, TargetKind, TextBlock, Value, VariableKind, valid_id,
};
use crate::{
    MAX_EXITS, MAX_EXTENSION_VERBS, MAX_FOUND_IN, MAX_LEXICONS, MAX_MESSAGE_BYTES,
    MAX_OBJECT_NAME_BYTES, MAX_OBJECTS, MAX_RULES, MAX_SCORE_ABS, MAX_TURNS,
};

const V3_MESSAGE: &str =
    "format v3 feature in a pack with an older format_version; run `tiamud-story fmt --upgrade`";

/// One located validation finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    pub path: String,
    pub message: String,
}

impl fmt::Display for ValidationIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

/// Successful validation: the pack is safe to run; warnings are authoring lints.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValidationReport {
    pub warnings: Vec<ValidationIssue>,
    /// True when more warnings existed than [`MAX_REPORTED_ISSUES`].
    pub truncated: bool,
}

/// Failed validation. At most [`MAX_REPORTED_ISSUES`] errors are retained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationErrors {
    pub errors: Vec<ValidationIssue>,
    pub truncated: bool,
}

impl fmt::Display for ValidationErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, issue) in self.errors.iter().enumerate() {
            if index > 0 {
                writeln!(f)?;
            }
            write!(f, "{issue}")?;
        }
        if self.truncated {
            write!(f, "\n(more errors omitted)")?;
        }
        Ok(())
    }
}

impl std::error::Error for ValidationErrors {}

#[derive(Default)]
struct Issues {
    list: Vec<ValidationIssue>,
    truncated: bool,
}

impl Issues {
    fn push(&mut self, path: impl Into<String>, message: impl Into<String>) {
        if self.list.len() >= MAX_REPORTED_ISSUES {
            self.truncated = true;
            return;
        }
        self.list.push(ValidationIssue {
            path: path.into(),
            message: message.into(),
        });
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum VarType {
    Int { min: i64, max: i64 },
    Bool,
}

struct Validator<'a> {
    pack: &'a StoryPack,
    errors: Issues,
    warnings: Issues,
    nodes: BTreeMap<&'a str, usize>,
    vars: BTreeMap<&'a str, VarType>,
    items: BTreeSet<&'a str>,
    verbs: BTreeMap<&'a str, TargetKind>,
    characters: BTreeSet<&'a str>,
    dialogues: BTreeSet<&'a str>,
    /// `(node, entry)` pairs that a `uses` condition may name.
    entries: BTreeSet<(&'a str, &'a str)>,
    /// World objects by id (format v3).
    objects: BTreeMap<&'a str, &'a ObjectDecl>,
    /// Nodes with a room (format v3).
    rooms: BTreeSet<&'a str>,
    /// Lexicon languages of the world, in pack order.
    languages: Vec<&'a str>,
    /// Actions any lexicon produces.
    actions: BTreeSet<String>,
    /// Directions every lexicon knows.
    directions: BTreeSet<String>,
}

impl StoryPack {
    /// Validates structure, references, types and every bound, then runs the
    /// reachability and dead-end lints. Errors make the pack unusable;
    /// warnings do not.
    pub fn validate(&self) -> Result<ValidationReport, ValidationErrors> {
        let mut validator = Validator {
            pack: self,
            errors: Issues::default(),
            warnings: Issues::default(),
            nodes: BTreeMap::new(),
            vars: BTreeMap::new(),
            items: BTreeSet::new(),
            verbs: BTreeMap::new(),
            characters: BTreeSet::new(),
            dialogues: BTreeSet::new(),
            entries: BTreeSet::new(),
            objects: BTreeMap::new(),
            rooms: BTreeSet::new(),
            languages: Vec::new(),
            actions: BTreeSet::new(),
            directions: BTreeSet::new(),
        };
        validator.run();
        if validator.errors.list.is_empty() {
            Ok(ValidationReport {
                warnings: validator.warnings.list,
                truncated: validator.warnings.truncated,
            })
        } else {
            Err(ValidationErrors {
                errors: validator.errors.list,
                truncated: validator.errors.truncated,
            })
        }
    }
}

impl<'a> Validator<'a> {
    fn error(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.errors.push(path, message);
    }

    fn run(&mut self) {
        let pack = self.pack;
        if !(STORY_PACK_MIN_FORMAT_VERSION..=STORY_PACK_FORMAT_VERSION)
            .contains(&pack.format_version)
        {
            self.error("format_version", "unsupported format version");
        }
        if !valid_id(&pack.id) {
            self.error("id", "invalid identifier");
        }
        if let Err(message) = pack.title.check(MAX_LABEL_BYTES, false) {
            self.error("title", message);
        }
        if pack.nodes.is_empty() || pack.nodes.len() > MAX_NODES {
            self.error("nodes", format!("expected 1..={MAX_NODES} nodes"));
            return;
        }
        if pack.variables.len() > MAX_VARIABLES {
            self.error("variables", format!("at most {MAX_VARIABLES} variables"));
            return;
        }
        if pack.items.len() > MAX_ITEMS {
            self.error("items", format!("at most {MAX_ITEMS} items"));
            return;
        }
        if pack.verbs.len() > MAX_VERBS {
            self.error("verbs", format!("at most {MAX_VERBS} verbs"));
            return;
        }
        if pack.characters.len() > MAX_CHARACTERS {
            self.error("characters", format!("at most {MAX_CHARACTERS} characters"));
            return;
        }
        if pack.dialogues.len() > MAX_DIALOGUES {
            self.error("dialogues", format!("at most {MAX_DIALOGUES} dialogues"));
            return;
        }
        let total_choices = pack
            .nodes
            .iter()
            .fold(0usize, |sum, node| sum.saturating_add(node.choices.len()));
        if total_choices > MAX_TOTAL_CHOICES {
            self.error(
                "nodes",
                format!("at most {MAX_TOTAL_CHOICES} choices in total"),
            );
            return;
        }
        let total_entries = pack.nodes.iter().fold(0usize, |sum, node| {
            sum.saturating_add(node.menu.as_ref().map_or(0, |menu| menu.entries.len()))
        });
        if total_entries > MAX_TOTAL_MENU_ENTRIES {
            self.error(
                "nodes",
                format!("at most {MAX_TOTAL_MENU_ENTRIES} menu entries in total"),
            );
            return;
        }
        let total_dialogue_nodes = pack.dialogues.iter().fold(0usize, |sum, dialogue| {
            sum.saturating_add(dialogue.nodes.len())
        });
        if total_dialogue_nodes > MAX_TOTAL_DIALOGUE_NODES {
            self.error(
                "dialogues",
                format!("at most {MAX_TOTAL_DIALOGUE_NODES} dialogue nodes in total"),
            );
            return;
        }
        if pack.format_version < 2 {
            self.v1_compatibility();
        }
        self.declarations();
        self.world_declarations();
        if !self.nodes.contains_key(pack.start.as_str()) {
            self.error("start", "start node does not exist");
        }
        if pack.initial_inventory.len() > MAX_ITEMS {
            self.error("initial_inventory", format!("at most {MAX_ITEMS} entries"));
        } else {
            let mut seen = BTreeSet::new();
            for (index, item) in pack.initial_inventory.iter().enumerate() {
                let path = format!("initial_inventory[{index}]");
                if !self.items.contains(item.as_str()) {
                    self.error(path, "undeclared item");
                } else if !seen.insert(item.as_str()) {
                    self.error(path, "duplicate item");
                }
            }
        }
        for index in 0..pack.verbs.len() {
            let verb = &pack.verbs[index];
            self.blocks(&format!("verbs[{index}].default"), &verb.default, 1);
        }
        for index in 0..pack.nodes.len() {
            self.node(index);
        }
        for index in 0..pack.dialogues.len() {
            self.dialogue(index);
        }
        self.world_rules();
        if self.errors.list.is_empty() {
            self.lints();
        }
    }

    /// A document that still declares v1 may not use v2 features.
    fn v1_compatibility(&mut self) {
        let pack = self.pack;
        let message =
            "format v2 feature in a format_version 1 pack; run `tiamud-story fmt --upgrade`";
        if !pack.verbs.is_empty() {
            self.error("verbs", message);
        }
        if !pack.characters.is_empty() {
            self.error("characters", message);
        }
        if !pack.dialogues.is_empty() {
            self.error("dialogues", message);
        }
        for (index, node) in pack.nodes.iter().enumerate() {
            if node.uses_v2() {
                self.error(format!("nodes[{index}]"), message);
            }
            for (choice_index, choice) in node.choices.iter().enumerate() {
                let uses = choice.when.as_ref().is_some_and(condition_uses_v2)
                    || effects_use_v2(&choice.effects);
                if uses {
                    self.error(format!("nodes[{index}].choices[{choice_index}]"), message);
                }
            }
        }
    }

    fn declarations(&mut self) {
        let pack = self.pack;
        for (index, node) in pack.nodes.iter().enumerate() {
            let path = format!("nodes[{index}].id");
            if !valid_id(&node.id) {
                self.error(path, "invalid identifier");
            } else if self.nodes.insert(node.id.as_str(), index).is_some() {
                self.error(path, "duplicate node id");
            }
            if let Some(menu) = &node.menu {
                for entry in &menu.entries {
                    self.entries.insert((node.id.as_str(), entry.id.as_str()));
                }
            }
        }
        for (index, decl) in pack.variables.iter().enumerate() {
            let path = format!("variables[{index}]");
            if !valid_id(&decl.id) {
                self.error(format!("{path}.id"), "invalid identifier");
                continue;
            }
            if let Some(label) = &decl.label
                && let Err(message) = label.check(MAX_LABEL_BYTES, false)
            {
                self.error(format!("{path}.label"), message);
            }
            let kind = match decl.kind {
                VariableKind::Int { min, max, initial } => {
                    if min > max {
                        self.error(format!("{path}.kind"), "min exceeds max");
                    }
                    if min < -MAX_VAR_ABS || max > MAX_VAR_ABS {
                        self.error(
                            format!("{path}.kind"),
                            format!("bounds must lie within ±{MAX_VAR_ABS}"),
                        );
                    }
                    if initial < min || initial > max {
                        self.error(format!("{path}.kind"), "initial value is out of bounds");
                    }
                    VarType::Int { min, max }
                }
                VariableKind::Bool { .. } => VarType::Bool,
            };
            if self.vars.insert(decl.id.as_str(), kind).is_some() {
                self.error(format!("{path}.id"), "duplicate variable id");
            }
        }
        for (index, item) in pack.items.iter().enumerate() {
            let path = format!("items[{index}]");
            if !valid_id(&item.id) {
                self.error(format!("{path}.id"), "invalid identifier");
            } else if !self.items.insert(item.id.as_str()) {
                self.error(format!("{path}.id"), "duplicate item id");
            }
            if let Err(message) = item.name.check(MAX_LABEL_BYTES, false) {
                self.error(format!("{path}.name"), message);
            }
        }
        for (index, verb) in pack.verbs.iter().enumerate() {
            let path = format!("verbs[{index}]");
            if !valid_id(&verb.id) {
                self.error(format!("{path}.id"), "invalid identifier");
            } else if self.verbs.insert(verb.id.as_str(), verb.targets).is_some() {
                self.error(format!("{path}.id"), "duplicate verb id");
            }
            if let Err(message) = verb.label.check(MAX_LABEL_BYTES, false) {
                self.error(format!("{path}.label"), message);
            }
        }
        for (index, character) in pack.characters.iter().enumerate() {
            let path = format!("characters[{index}]");
            if !valid_id(&character.id) {
                self.error(format!("{path}.id"), "invalid identifier");
            } else if !self.characters.insert(character.id.as_str()) {
                self.error(format!("{path}.id"), "duplicate character id");
            }
            if let Err(message) = character.name.check(MAX_LABEL_BYTES, false) {
                self.error(format!("{path}.name"), message);
            }
            if character.portraits.len() > MAX_EXPRESSIONS {
                self.error(
                    format!("{path}.portraits"),
                    format!("at most {MAX_EXPRESSIONS} expressions"),
                );
            }
            for (expression, asset) in &character.portraits {
                if !valid_id(expression) {
                    self.error(format!("{path}.portraits"), "invalid expression id");
                }
                if !valid_id(asset) {
                    self.error(
                        format!("{path}.portraits.{expression}"),
                        "portrait must be a reviewed asset id, not a URL",
                    );
                }
            }
            if let Some(expression) = &character.default_expression
                && !valid_id(expression)
            {
                self.error(
                    format!("{path}.default_expression"),
                    "invalid expression id",
                );
            }
        }
        for (index, dialogue) in pack.dialogues.iter().enumerate() {
            let path = format!("dialogues[{index}].id");
            if !valid_id(&dialogue.id) {
                self.error(path, "invalid identifier");
            } else if !self.dialogues.insert(dialogue.id.as_str()) {
                self.error(path, "duplicate dialogue id");
            }
        }
    }

    fn node(&mut self, index: usize) {
        let node = &self.pack.nodes[index];
        let path = format!("nodes[{index}]");
        if let Err(message) = node.text.check(MAX_TEXT_BYTES, true) {
            self.error(format!("{path}.text"), message);
        }
        if let Some(ending) = &node.ending {
            self.ending(&format!("{path}.ending"), ending);
            if !node.choices.is_empty() {
                self.error(
                    format!("{path}.choices"),
                    "an ending node cannot offer choices",
                );
            }
            if node.menu.is_some() {
                self.error(format!("{path}.menu"), "an ending node cannot offer a menu");
            }
        }
        if node.choices.len() > MAX_CHOICES_PER_NODE {
            self.error(
                format!("{path}.choices"),
                format!("at most {MAX_CHOICES_PER_NODE} choices"),
            );
            return;
        }
        let mut choice_ids = BTreeSet::new();
        for (choice_index, choice) in node.choices.iter().enumerate() {
            let path = format!("{path}.choices[{choice_index}]");
            if !valid_id(&choice.id) {
                self.error(format!("{path}.id"), "invalid identifier");
            } else if !choice_ids.insert(choice.id.as_str()) {
                self.error(format!("{path}.id"), "duplicate choice id within node");
            }
            if let Err(message) = choice.label.check(MAX_LABEL_BYTES, false) {
                self.error(format!("{path}.label"), message);
            }
            self.optional_condition(&format!("{path}.when"), choice.when.as_ref());
            if let Some(target) = &choice.target {
                self.node_ref(&format!("{path}.target"), target);
            }
            self.effect_list(&format!("{path}.effects"), &choice.effects);
        }
        self.scene(&path, node);
        self.room(&path, node);
    }

    /// Format v3 room data of a node: name and exits.
    fn room(&mut self, path: &str, node: &'a Node) {
        let Some(room) = &node.room else {
            return;
        };
        let path = format!("{path}.room");
        if self.pack.format_version < 3 {
            self.error(path.clone(), V3_MESSAGE);
        }
        if self.pack.world.is_none() {
            self.error(path.clone(), "a room needs a pack `world`");
        }
        if node.ending.is_some() {
            self.error(path.clone(), "an ending node cannot be a room");
        }
        if let Err(message) = room.name.check(MAX_LABEL_BYTES, false) {
            self.error(format!("{path}.name"), message);
        }
        if room.exits.len() > MAX_EXITS {
            self.error(
                format!("{path}.exits"),
                format!("at most {MAX_EXITS} exits"),
            );
            return;
        }
        let mut seen = BTreeSet::new();
        for (index, exit) in room.exits.iter().enumerate() {
            let path = format!("{path}.exits[{index}]");
            if !self.directions.contains(&exit.direction) {
                self.error(
                    format!("{path}.direction"),
                    "not a direction every lexicon knows",
                );
            } else if !seen.insert(exit.direction.as_str()) {
                self.error(format!("{path}.direction"), "duplicate exit direction");
            }
            self.node_ref(&format!("{path}.to"), &exit.to);
            self.optional_condition(&format!("{path}.when"), exit.when.as_ref());
            if let Some(door) = &exit.door {
                match self.objects.get(door.as_str()).copied() {
                    None => self.error(format!("{path}.door"), "unknown object"),
                    Some(decl) => {
                        let here = decl.location == Location::Room(node.id.clone())
                            || decl.found_in.contains(&node.id);
                        if !decl.openable {
                            self.error(format!("{path}.door"), "a door must be openable");
                        }
                        if !here {
                            self.error(
                                format!("{path}.door"),
                                "a door must stand in the room it leads from (location or found_in)",
                            );
                        }
                    }
                }
            }
            if let Some(blocked) = &exit.blocked
                && let Err(message) = blocked.check(MAX_TEXT_BYTES, true)
            {
                self.error(format!("{path}.blocked"), message);
            }
        }
    }

    /// Format v3 world: lexicons, objects, rule ids, messages and score.
    fn world_declarations(&mut self) {
        let pack = self.pack;
        for node in &pack.nodes {
            if node.room.is_some() {
                self.rooms.insert(node.id.as_str());
            }
        }
        let Some(world) = &pack.world else {
            return;
        };
        if pack.format_version < 3 {
            self.error("world", V3_MESSAGE);
        }
        if world.lexicons.is_empty() || world.lexicons.len() > MAX_LEXICONS {
            self.error(
                "world.lexicons",
                format!("expected 1..={MAX_LEXICONS} lexicons"),
            );
            return;
        }
        let mut directions: Option<BTreeSet<String>> = None;
        for (index, lexicon) in world.lexicons.iter().enumerate() {
            let path = format!("world.lexicons[{index}]");
            if self.languages.contains(&lexicon.id.as_str()) {
                self.error(format!("{path}.id"), "duplicate lexicon");
                continue;
            }
            if lexicon.verbs.len() > MAX_EXTENSION_VERBS {
                self.error(
                    format!("{path}.verbs"),
                    format!("at most {MAX_EXTENSION_VERBS} pack verbs"),
                );
                continue;
            }
            if let Err(error) = crate::command::build_parser(lexicon) {
                self.error(path, error.to_string());
                continue;
            }
            self.languages.push(lexicon.id.as_str());
            if let Ok(actions) = crate::command::lexicon_actions(lexicon) {
                self.actions.extend(actions);
            }
            if let Ok(known) = crate::command::lexicon_directions(lexicon) {
                directions = Some(match directions {
                    None => known,
                    Some(previous) => previous.intersection(&known).cloned().collect(),
                });
            }
        }
        self.directions = directions.unwrap_or_default();
        if world.objects.len() > MAX_OBJECTS {
            self.error("world.objects", format!("at most {MAX_OBJECTS} objects"));
            return;
        }
        if world.rules.len() > MAX_RULES {
            self.error("world.rules", format!("at most {MAX_RULES} rules"));
            return;
        }
        for (index, decl) in world.objects.iter().enumerate() {
            let path = format!("world.objects[{index}].id");
            if !valid_id(&decl.id) {
                self.error(path, "invalid identifier");
            } else if self.objects.insert(decl.id.as_str(), decl).is_some() {
                self.error(path, "duplicate object id");
            }
        }
        for (index, decl) in world.objects.iter().enumerate() {
            self.object(&format!("world.objects[{index}]"), decl);
        }
        // Containment must be a forest.
        for (index, decl) in world.objects.iter().enumerate() {
            let mut current = &decl.location;
            let mut steps = 0usize;
            while let Location::In(container) = current {
                steps += 1;
                if container == &decl.id || steps > world.objects.len() {
                    self.error(
                        format!("world.objects[{index}].location"),
                        "containment cycle",
                    );
                    break;
                }
                match self.objects.get(container.as_str()) {
                    Some(holder) => current = &holder.location,
                    None => break,
                }
            }
        }
        for (key, text) in &world.messages {
            if let Err(message) = text.check(MAX_MESSAGE_BYTES, false) {
                self.error(format!("world.messages.{key:?}"), message);
            }
        }
        if let Some(intro) = &world.intro
            && let Err(message) = intro.check(MAX_TEXT_BYTES, true)
        {
            self.error("world.intro", message);
        }
        if let Some(max) = world.max_score
            && !(1..=MAX_SCORE_ABS).contains(&max)
        {
            self.error("world.max_score", format!("expected 1..={MAX_SCORE_ABS}"));
        }
    }

    fn object(&mut self, path: &str, decl: &'a ObjectDecl) {
        if let Err(message) = decl.name.check(MAX_OBJECT_NAME_BYTES, false) {
            self.error(format!("{path}.name"), message);
        }
        if let Err(message) = decl.description.check(MAX_TEXT_BYTES, true) {
            self.error(format!("{path}.description"), message);
        }
        if let Some(text) = &decl.text
            && let Err(message) = text.check(MAX_TEXT_BYTES, true)
        {
            self.error(format!("{path}.text"), message);
        }
        self.words(&format!("{path}.nouns"), &decl.nouns, true);
        self.words(&format!("{path}.adjectives"), &decl.adjectives, false);
        self.location(&format!("{path}.location"), &decl.location, Some(&decl.id));
        if decl.found_in.len() > MAX_FOUND_IN {
            self.error(
                format!("{path}.found_in"),
                format!("at most {MAX_FOUND_IN} rooms"),
            );
        }
        if !decl.found_in.is_empty()
            && (decl.portable || !matches!(decl.location, Location::Room(_)))
        {
            self.error(
                format!("{path}.found_in"),
                "only fixed objects standing in a room may be found in other rooms",
            );
        }
        for room in &decl.found_in {
            if !self.rooms.contains(room.as_str()) {
                self.error(format!("{path}.found_in"), "unknown room");
            }
        }
        let mut contradiction = |message: &str| self.errors.push(path.to_owned(), message);
        if decl.portable && decl.scenery {
            contradiction("an object cannot be both portable and scenery");
        }
        if decl.open && !(decl.openable || decl.container) {
            contradiction("only openable objects or containers can be open");
        }
        if decl.locked && (!decl.openable || decl.open) {
            contradiction("a locked object must be openable and closed");
        }
        if decl.worn && !(decl.wearable && decl.location == Location::Carried) {
            contradiction("a worn object must be wearable and carried");
        }
        if let Some(key) = &decl.key {
            if !decl.openable {
                self.error(format!("{path}.key"), "only openable objects take a key");
            }
            match self.objects.get(key.as_str()) {
                None => self.error(format!("{path}.key"), "unknown object"),
                Some(found) if found.id == decl.id => {
                    self.error(format!("{path}.key"), "an object cannot be its own key")
                }
                Some(found) if !found.portable => {
                    self.error(format!("{path}.key"), "a key must be portable")
                }
                Some(_) => {}
            }
        }
    }

    /// Nouns or adjectives per lexicon language: every key is a lexicon
    /// language, each word folds to one alphanumeric word the parser accepts.
    fn words(&mut self, path: &str, words: &LocalizedWords, required: bool) {
        use tiamud_story_parser::{MAX_SCOPE_WORDS_PER_OBJECT, MAX_WORD_BYTES, fold};
        for language in words.keys() {
            if !self.languages.contains(&language.as_str()) {
                self.error(
                    format!("{path}.{language}"),
                    "not a lexicon language of this world",
                );
            }
        }
        for language in self.languages.clone() {
            let list = words.get(language).map(Vec::as_slice).unwrap_or_default();
            if required && list.is_empty() {
                self.error(
                    format!("{path}.{language}"),
                    "at least one noun is required for every lexicon language",
                );
            }
            if list.len() > MAX_SCOPE_WORDS_PER_OBJECT {
                self.error(
                    format!("{path}.{language}"),
                    format!("at most {MAX_SCOPE_WORDS_PER_OBJECT} words"),
                );
                continue;
            }
            for word in list {
                let folded = fold(word);
                if word.len() > MAX_WORD_BYTES * 4
                    || folded.is_empty()
                    || folded.len() > MAX_WORD_BYTES
                    || !folded.chars().all(char::is_alphanumeric)
                {
                    self.error(
                        format!("{path}.{language}"),
                        format!(
                            "'{word}' is not one alphanumeric word of 1..={MAX_WORD_BYTES} bytes"
                        ),
                    );
                }
            }
        }
    }

    /// An object location: rooms must be room nodes and containers declared
    /// container objects other than `owner`.
    fn location(&mut self, path: &str, location: &Location, owner: Option<&str>) {
        match location {
            Location::Room(node) => {
                if !self.rooms.contains(node.as_str()) {
                    self.error(path, "not a room node");
                }
            }
            Location::In(container) => match self.objects.get(container.as_str()) {
                None => self.error(path, "unknown object"),
                Some(decl) if !decl.container => self.error(path, "not a container"),
                Some(decl) if Some(decl.id.as_str()) == owner => {
                    self.error(path, "an object cannot contain itself")
                }
                Some(_) => {}
            },
            Location::Carried | Location::Nowhere => {}
        }
    }

    fn object_ref(&mut self, path: &str, obj: &str) {
        if !self.objects.contains_key(obj) {
            self.error(path, "unknown object");
        }
    }

    /// Format v3 rules: action known to a lexicon, keys and DSL.
    fn world_rules(&mut self) {
        let Some(world) = &self.pack.world else {
            return;
        };
        if world.rules.len() > MAX_RULES || world.objects.len() > MAX_OBJECTS {
            return;
        }
        let mut ids = BTreeSet::new();
        for (index, rule) in world.rules.iter().enumerate() {
            let path = format!("world.rules[{index}]");
            if !valid_id(&rule.id) {
                self.error(format!("{path}.id"), "invalid identifier");
            } else if !ids.insert(rule.id.as_str()) {
                self.error(format!("{path}.id"), "duplicate rule id");
            }
            if !self.actions.contains(&rule.action) {
                self.error(
                    format!("{path}.action"),
                    "no lexicon of this world produces this action",
                );
            }
            if let Some(obj) = &rule.obj {
                self.object_ref(&format!("{path}.obj"), obj);
            }
            if let Some(obj) = &rule.obj2 {
                self.object_ref(&format!("{path}.obj2"), obj);
            }
            if let Some(direction) = &rule.direction
                && !self.directions.contains(direction)
            {
                self.error(
                    format!("{path}.direction"),
                    "not a direction every lexicon knows",
                );
            }
            self.optional_condition(&format!("{path}.when"), rule.when.as_ref());
            self.effect_list(&format!("{path}.effects"), &rule.effects);
            if let Some(text) = &rule.text
                && let Err(message) = text.check(MAX_TEXT_BYTES, true)
            {
                self.error(format!("{path}.text"), message);
            }
            if rule.text.is_none() && rule.effects.is_empty() {
                self.warnings
                    .push(path, "rule has neither text nor effects");
            }
        }
    }

    fn v3(&mut self, path: &str) {
        if self.pack.format_version < 3 {
            self.error(path, V3_MESSAGE);
        }
    }

    /// Format v2 scene fields: background, cues, hotspots, presence, menu.
    fn scene(&mut self, path: &str, node: &'a Node) {
        if let Some(background) = &node.background
            && !valid_id(background)
        {
            self.error(
                format!("{path}.background"),
                "background must be a reviewed asset id, not a URL",
            );
        }
        if node.cues.len() > MAX_CUES_PER_NODE {
            self.error(
                format!("{path}.cues"),
                format!("at most {MAX_CUES_PER_NODE} cues"),
            );
        }
        for (index, cue) in node.cues.iter().enumerate() {
            self.cue(&format!("{path}.cues[{index}]"), cue);
        }
        if node.hotspots.len() > MAX_HOTSPOTS_PER_NODE {
            self.error(
                format!("{path}.hotspots"),
                format!("at most {MAX_HOTSPOTS_PER_NODE} hotspots"),
            );
            return;
        }
        let mut hotspots = BTreeSet::new();
        for (index, hotspot) in node.hotspots.iter().enumerate() {
            let path = format!("{path}.hotspots[{index}]");
            if !valid_id(&hotspot.id) {
                self.error(format!("{path}.id"), "invalid identifier");
            } else if !hotspots.insert(hotspot.id.as_str()) {
                self.error(format!("{path}.id"), "duplicate hotspot id within node");
            }
            if let Err(message) = hotspot.label.check(MAX_LABEL_BYTES, false) {
                self.error(format!("{path}.label"), message);
            }
            self.optional_condition(&format!("{path}.when"), hotspot.when.as_ref());
        }
        if node.present.len() > MAX_PRESENT_PER_NODE {
            self.error(
                format!("{path}.present"),
                format!("at most {MAX_PRESENT_PER_NODE} characters"),
            );
            return;
        }
        let mut present = BTreeSet::new();
        for (index, presence) in node.present.iter().enumerate() {
            let path = format!("{path}.present[{index}]");
            if !self.characters.contains(presence.character.as_str()) {
                self.error(format!("{path}.character"), "undeclared character");
            } else if !present.insert(presence.character.as_str()) {
                self.error(format!("{path}.character"), "character is already present");
            }
            if let Some(expression) = &presence.expression
                && !valid_id(expression)
            {
                self.error(format!("{path}.expression"), "invalid expression id");
            }
            self.optional_condition(&format!("{path}.when"), presence.when.as_ref());
        }
        let Some(menu) = &node.menu else {
            return;
        };
        let path = format!("{path}.menu");
        if menu.verbs.is_empty() || menu.verbs.len() > MAX_VERBS {
            self.error(
                format!("{path}.verbs"),
                format!("expected 1..={MAX_VERBS} verbs"),
            );
            return;
        }
        if menu.entries.len() > MAX_MENU_ENTRIES_PER_NODE {
            self.error(
                format!("{path}.entries"),
                format!("at most {MAX_MENU_ENTRIES_PER_NODE} entries"),
            );
            return;
        }
        let mut offered = BTreeSet::new();
        for (index, menu_verb) in menu.verbs.iter().enumerate() {
            let path = format!("{path}.verbs[{index}]");
            if !self.verbs.contains_key(menu_verb.verb.as_str()) {
                self.error(format!("{path}.verb"), "undeclared verb");
            } else if !offered.insert(menu_verb.verb.as_str()) {
                self.error(format!("{path}.verb"), "verb is offered twice");
            }
            self.optional_condition(&format!("{path}.when"), menu_verb.when.as_ref());
        }
        let mut entry_ids = BTreeSet::new();
        let mut pairs = BTreeSet::new();
        for (index, entry) in menu.entries.iter().enumerate() {
            let path = format!("{path}.entries[{index}]");
            if !valid_id(&entry.id) {
                self.error(format!("{path}.id"), "invalid identifier");
            } else if !entry_ids.insert(entry.id.as_str()) {
                self.error(format!("{path}.id"), "duplicate entry id within node");
            }
            if !offered.contains(entry.verb.as_str()) {
                self.error(format!("{path}.verb"), "verb is not offered by this menu");
            }
            if !pairs.insert((entry.verb.as_str(), entry.target.as_deref())) {
                self.error(
                    path.clone(),
                    "another entry already answers this verb and target",
                );
            }
            let kind = self.verbs.get(entry.verb.as_str()).copied();
            match (kind, entry.target.as_deref()) {
                (None, _) | (Some(TargetKind::None), None) => {}
                (Some(TargetKind::None), Some(_)) => {
                    self.error(format!("{path}.target"), "this verb takes no target");
                }
                (Some(_), None) => self.error(format!("{path}.target"), "this verb needs a target"),
                (Some(TargetKind::Hotspot), Some(target)) => {
                    if node.hotspot(target).is_none() {
                        self.error(
                            format!("{path}.target"),
                            "target is not a hotspot of this node",
                        );
                    }
                }
                (Some(TargetKind::Character), Some(target)) => {
                    if node.presence(target).is_none() {
                        self.error(
                            format!("{path}.target"),
                            "target is not a character present on this node",
                        );
                    }
                }
                (Some(TargetKind::Item), Some(target)) => {
                    self.item_ref(&format!("{path}.target"), target)
                }
            }
            if entry.responses.is_empty() || entry.responses.len() > MAX_RESPONSES_PER_ENTRY {
                self.error(
                    format!("{path}.responses"),
                    format!("expected 1..={MAX_RESPONSES_PER_ENTRY} responses"),
                );
                continue;
            }
            self.blocks(&format!("{path}.exhausted"), &entry.exhausted, 0);
            let mut response_ids = BTreeSet::new();
            for (response_index, response) in entry.responses.iter().enumerate() {
                let path = format!("{path}.responses[{response_index}]");
                if !valid_id(&response.id) {
                    self.error(format!("{path}.id"), "invalid identifier");
                } else if !response_ids.insert(response.id.as_str()) {
                    self.error(format!("{path}.id"), "duplicate response id within entry");
                }
                self.optional_condition(&format!("{path}.when"), response.when.as_ref());
                self.blocks(&format!("{path}.blocks"), &response.blocks, 1);
                self.effect_list(&format!("{path}.effects"), &response.effects);
                if let Some(target) = &response.target {
                    self.node_ref(&format!("{path}.target"), target);
                }
                if let Some(dialogue) = &response.dialogue
                    && !self.dialogues.contains(dialogue.as_str())
                {
                    self.error(format!("{path}.dialogue"), "unknown dialogue");
                }
            }
        }
        let mut defaults = BTreeSet::new();
        for (index, default) in menu.defaults.iter().enumerate() {
            let path = format!("{path}.defaults[{index}]");
            if !offered.contains(default.verb.as_str()) {
                self.error(format!("{path}.verb"), "verb is not offered by this menu");
            } else if !defaults.insert(default.verb.as_str()) {
                self.error(format!("{path}.verb"), "duplicate default for this verb");
            }
            self.blocks(&format!("{path}.blocks"), &default.blocks, 1);
        }
    }

    fn dialogue(&mut self, index: usize) {
        let dialogue: &'a Dialogue = &self.pack.dialogues[index];
        let path = format!("dialogues[{index}]");
        if dialogue.nodes.is_empty() || dialogue.nodes.len() > MAX_DIALOGUE_NODES {
            self.error(
                format!("{path}.nodes"),
                format!("expected 1..={MAX_DIALOGUE_NODES} nodes"),
            );
            return;
        }
        let mut ids = BTreeSet::new();
        for (node_index, node) in dialogue.nodes.iter().enumerate() {
            let path = format!("{path}.nodes[{node_index}].id");
            if !valid_id(&node.id) {
                self.error(path, "invalid identifier");
            } else if !ids.insert(node.id.as_str()) {
                self.error(path, "duplicate dialogue node id");
            }
        }
        if !ids.contains(dialogue.start.as_str()) {
            self.error(format!("{path}.start"), "unknown dialogue node");
        }
        for (node_index, node) in dialogue.nodes.iter().enumerate() {
            let path = format!("{path}.nodes[{node_index}]");
            self.blocks(&format!("{path}.lines"), &node.lines, 1);
            if node.options.is_empty() || node.options.len() > MAX_DIALOGUE_OPTIONS {
                self.error(
                    format!("{path}.options"),
                    format!("expected 1..={MAX_DIALOGUE_OPTIONS} options"),
                );
                continue;
            }
            let mut option_ids = BTreeSet::new();
            for (option_index, option) in node.options.iter().enumerate() {
                let path = format!("{path}.options[{option_index}]");
                if !valid_id(&option.id) {
                    self.error(format!("{path}.id"), "invalid identifier");
                } else if !option_ids.insert(option.id.as_str()) {
                    self.error(format!("{path}.id"), "duplicate option id within node");
                }
                if let Err(message) = option.label.check(MAX_LABEL_BYTES, false) {
                    self.error(format!("{path}.label"), message);
                }
                self.optional_condition(&format!("{path}.when"), option.when.as_ref());
                self.effect_list(&format!("{path}.effects"), &option.effects);
                if let DialogueNext::Goto(next) = &option.next
                    && !ids.contains(next.as_str())
                {
                    self.error(format!("{path}.next"), "unknown dialogue node");
                }
            }
        }
    }

    /// `min..=MAX_TEXT_BLOCKS` blocks with declared speakers.
    fn blocks(&mut self, path: &str, blocks: &[TextBlock], min: usize) {
        if blocks.len() < min || blocks.len() > MAX_TEXT_BLOCKS {
            self.error(
                path,
                format!("expected {min}..={MAX_TEXT_BLOCKS} text blocks"),
            );
            return;
        }
        for (index, block) in blocks.iter().enumerate() {
            let path = format!("{path}[{index}]");
            if let Err(message) = block.text.check(MAX_TEXT_BYTES, true) {
                self.error(format!("{path}.text"), message);
            }
            if let Some(speaker) = &block.speaker
                && !self.characters.contains(speaker.as_str())
            {
                self.error(format!("{path}.speaker"), "undeclared character");
            }
            if let Some(expression) = &block.expression {
                if !valid_id(expression) {
                    self.error(format!("{path}.expression"), "invalid expression id");
                }
                if block.speaker.is_none() {
                    self.error(
                        format!("{path}.expression"),
                        "an expression needs a speaker",
                    );
                }
            }
        }
    }

    fn optional_condition(&mut self, path: &str, condition: Option<&Condition>) {
        if let Some(condition) = condition {
            let mut budget = MAX_CONDITION_NODES;
            self.condition(path, condition, 0, &mut budget);
        }
    }

    fn effect_list(&mut self, path: &str, effects: &[Effect]) {
        let total = count_effects(effects, 0);
        if total > MAX_EFFECTS_PER_CHOICE {
            self.error(
                path,
                format!("at most {MAX_EFFECTS_PER_CHOICE} effects including nested ones"),
            );
        } else {
            self.effects(path, effects, 0);
        }
    }

    fn ending(&mut self, path: &str, ending: &Ending) {
        if !valid_id(&ending.id) {
            self.error(format!("{path}.id"), "invalid identifier");
        }
    }

    fn node_ref(&mut self, path: &str, node: &str) {
        if !self.nodes.contains_key(node) {
            self.error(path, "unknown node");
        }
    }

    fn int_var(&mut self, path: &str, var: &str) -> Option<(i64, i64)> {
        match self.vars.get(var).copied() {
            Some(VarType::Int { min, max }) => Some((min, max)),
            Some(VarType::Bool) => {
                self.error(path, "expected an integer variable");
                None
            }
            None => {
                self.error(path, "unknown variable");
                None
            }
        }
    }

    fn item_ref(&mut self, path: &str, item: &str) {
        if !self.items.contains(item) {
            self.error(path, "undeclared item");
        }
    }

    fn cue(&mut self, path: &str, cue: &Cue) {
        if let Cue::Music { asset } | Cue::Sfx { asset } = cue
            && !valid_id(asset)
        {
            self.error(path, "cue asset must be a reviewed asset id, not a URL");
        }
    }

    fn condition(&mut self, path: &str, condition: &Condition, depth: usize, budget: &mut usize) {
        if depth >= MAX_CONDITION_DEPTH {
            self.error(
                path,
                format!("conditions nest at most {MAX_CONDITION_DEPTH} levels"),
            );
            return;
        }
        let Some(left) = budget.checked_sub(1) else {
            self.error(
                path,
                format!("at most {MAX_CONDITION_NODES} condition nodes"),
            );
            return;
        };
        *budget = left;
        match condition {
            Condition::All(children) | Condition::Any(children) => {
                if children.is_empty() {
                    self.error(path, "all/any needs at least one condition");
                }
                for (index, child) in children.iter().enumerate() {
                    self.condition(&format!("{path}[{index}]"), child, depth + 1, budget);
                }
            }
            Condition::Not(child) => {
                self.condition(&format!("{path}.not"), child, depth + 1, budget)
            }
            Condition::VarCmp { var, value, .. } => {
                self.int_var(&format!("{path}.var"), var);
                if value.unsigned_abs() > MAX_VAR_ABS.unsigned_abs() {
                    self.error(
                        format!("{path}.value"),
                        "comparison constant is out of range",
                    );
                }
            }
            Condition::HasItem(item) => self.item_ref(path, item),
            Condition::Visited(node) => self.node_ref(path, node),
            Condition::Flag(var) => match self.vars.get(var.as_str()) {
                Some(VarType::Bool) => {}
                Some(VarType::Int { .. }) => self.error(path, "flag needs a boolean variable"),
                None => self.error(path, "unknown variable"),
            },
            Condition::Uses {
                node, entry, value, ..
            } => {
                if !self.entries.contains(&(node.as_str(), entry.as_str())) {
                    self.error(path, "unknown menu entry");
                }
                if value.unsigned_abs() > MAX_VAR_ABS.unsigned_abs() {
                    self.error(
                        format!("{path}.value"),
                        "comparison constant is out of range",
                    );
                }
            }
            Condition::Carried(obj) | Condition::Visible(obj) => {
                self.v3(path);
                self.object_ref(path, obj);
            }
            Condition::ObjectAt { obj, at } => {
                self.v3(path);
                self.object_ref(&format!("{path}.obj"), obj);
                self.location(&format!("{path}.at"), at, Some(obj));
            }
            Condition::ObjectIs { obj, .. } => {
                self.v3(path);
                self.object_ref(&format!("{path}.obj"), obj);
            }
            Condition::PlayerIn(node) => {
                self.v3(path);
                self.node_ref(path, node);
            }
            Condition::TurnCmp { value, .. } => {
                self.v3(path);
                if value.unsigned_abs() > u64::from(MAX_TURNS) {
                    self.error(
                        format!("{path}.value"),
                        "comparison constant is out of range",
                    );
                }
            }
        }
    }

    fn effects(&mut self, path: &str, effects: &[Effect], depth: usize) {
        for (index, effect) in effects.iter().enumerate() {
            self.effect(&format!("{path}[{index}]"), effect, depth);
        }
    }

    fn effect(&mut self, path: &str, effect: &Effect, depth: usize) {
        match effect {
            Effect::Set { var, value } => match (self.vars.get(var.as_str()).copied(), value) {
                (Some(VarType::Int { min, max }), Value::Int(value)) => {
                    if *value < min || *value > max {
                        self.error(path, "set value is outside the variable bounds");
                    }
                }
                (Some(VarType::Bool), Value::Bool(_)) => {}
                (None, _) => self.error(path, "unknown variable"),
                _ => self.error(path, "set value does not match the variable type"),
            },
            Effect::Add { var, delta } => {
                self.int_var(&format!("{path}.var"), var);
                if delta.unsigned_abs() > MAX_VAR_ABS.unsigned_abs() {
                    self.error(format!("{path}.delta"), "delta is out of range");
                }
            }
            Effect::Give(item) | Effect::Take(item) => self.item_ref(path, item),
            Effect::Goto(node) => self.node_ref(path, node),
            Effect::Roll {
                dice,
                modifier_var,
                store,
                test,
            } => {
                if dice.count == 0 || dice.count > MAX_DICE_COUNT {
                    self.error(
                        format!("{path}.dice"),
                        format!("count must be 1..={MAX_DICE_COUNT}"),
                    );
                }
                if dice.sides < 2 || dice.sides > MAX_DICE_SIDES {
                    self.error(
                        format!("{path}.dice"),
                        format!("sides must be 2..={MAX_DICE_SIDES}"),
                    );
                }
                if dice.modifier.unsigned_abs() > MAX_DICE_MODIFIER.unsigned_abs() {
                    self.error(format!("{path}.dice"), "modifier is out of range");
                }
                if let Some(var) = modifier_var {
                    self.int_var(&format!("{path}.modifier_var"), var);
                }
                if let Some(var) = store {
                    self.int_var(&format!("{path}.store"), var);
                }
                if let Some(test) = test {
                    if test.at_least.unsigned_abs() > MAX_VAR_ABS.unsigned_abs() {
                        self.error(format!("{path}.test"), "threshold is out of range");
                    }
                    if test.success.is_none() && test.failure.is_none() {
                        self.warnings.push(
                            format!("{path}.test"),
                            "test has no success or failure destination",
                        );
                    }
                    for node in [&test.success, &test.failure].into_iter().flatten() {
                        self.node_ref(&format!("{path}.test"), node);
                    }
                }
            }
            Effect::If {
                when,
                then,
                otherwise,
            } => {
                if depth >= MAX_EFFECT_DEPTH {
                    self.error(
                        path,
                        format!("if effects nest at most {MAX_EFFECT_DEPTH} levels"),
                    );
                    return;
                }
                let mut budget = MAX_CONDITION_NODES;
                self.condition(&format!("{path}.when"), when, 0, &mut budget);
                self.effects(&format!("{path}.then"), then, depth + 1);
                self.effects(&format!("{path}.otherwise"), otherwise, depth + 1);
            }
            Effect::Cue(cue) => self.cue(path, cue),
            Effect::Score(delta) => {
                if delta.unsigned_abs() > MAX_SCORE_DELTA.unsigned_abs() {
                    self.error(path, "score delta is out of range");
                }
            }
            Effect::End(ending) => self.ending(path, ending),
            Effect::MoveObject { obj, to } => {
                self.v3(path);
                self.object_ref(&format!("{path}.obj"), obj);
                self.location(&format!("{path}.to"), to, Some(obj));
            }
            Effect::SetObject { obj, flag, .. } => {
                self.v3(path);
                match self.objects.get(obj.as_str()).copied() {
                    None => self.error(format!("{path}.obj"), "unknown object"),
                    Some(decl) => {
                        let allowed = match flag {
                            ObjectFlag::Open | ObjectFlag::Locked => decl.openable,
                            ObjectFlag::Lit => decl.lightable || decl.lit,
                            ObjectFlag::Worn => decl.wearable,
                        };
                        if !allowed {
                            self.error(
                                format!("{path}.flag"),
                                "the object does not have this flag",
                            );
                        }
                    }
                }
            }
        }
    }

    /// Reachability (ignoring conditions), dead ends and missing endings.
    /// Menu responses, and the dialogues they start, count as scene exits.
    fn lints(&mut self) {
        let pack = self.pack;
        let mut edges: Vec<Vec<usize>> = vec![Vec::new(); pack.nodes.len()];
        let mut ends = vec![false; pack.nodes.len()];
        // Scene exits of each dialogue, computed once.
        let mut dialogue_exits: BTreeMap<&str, (Vec<&str>, bool)> = BTreeMap::new();
        for dialogue in &pack.dialogues {
            let mut targets = Vec::new();
            let mut can_end = false;
            for option in dialogue.nodes.iter().flat_map(|node| &node.options) {
                collect_targets(&option.effects, &mut targets);
                can_end |= any_end(&option.effects);
            }
            dialogue_exits.insert(dialogue.id.as_str(), (targets, can_end));
        }
        // Rule destinations and endings apply from any room (format v3).
        let mut rule_targets = Vec::new();
        let mut rules_end = false;
        if let Some(world) = &pack.world {
            for rule in &world.rules {
                collect_targets(&rule.effects, &mut rule_targets);
                rules_end |= any_end(&rule.effects);
            }
        }
        for (index, node) in pack.nodes.iter().enumerate() {
            let mut targets = Vec::new();
            if let Some(room) = &node.room {
                targets.extend(room.exits.iter().map(|exit| exit.to.as_str()));
                targets.extend(rule_targets.iter().copied());
                ends[index] |= rules_end;
            }
            for choice in &node.choices {
                targets.extend(choice.target.as_deref());
                collect_targets(&choice.effects, &mut targets);
                ends[index] |= any_end(&choice.effects);
            }
            if let Some(menu) = &node.menu {
                for response in menu.entries.iter().flat_map(|entry| &entry.responses) {
                    targets.extend(response.target.as_deref());
                    collect_targets(&response.effects, &mut targets);
                    ends[index] |= any_end(&response.effects);
                    if let Some((exits, can_end)) = response
                        .dialogue
                        .as_deref()
                        .and_then(|id| dialogue_exits.get(id))
                    {
                        targets.extend(exits.iter().copied());
                        ends[index] |= *can_end;
                    }
                }
            }
            for target in targets {
                if let Some(next) = self.nodes.get(target) {
                    edges[index].push(*next);
                }
            }
        }
        let mut reachable = vec![false; pack.nodes.len()];
        let mut queue = VecDeque::new();
        if let Some(start) = self.nodes.get(pack.start.as_str()) {
            reachable[*start] = true;
            queue.push_back(*start);
        }
        while let Some(current) = queue.pop_front() {
            for next in &edges[current] {
                if !reachable[*next] {
                    reachable[*next] = true;
                    queue.push_back(*next);
                }
            }
        }
        let mut ending_reachable = false;
        for (index, node) in pack.nodes.iter().enumerate() {
            let path = format!("nodes[{index}]");
            if !reachable[index] {
                self.warnings
                    .push(path.clone(), "node is unreachable from the start node");
            }
            if node.ending.is_some() {
                ending_reachable |= reachable[index];
                continue;
            }
            if reachable[index] && ends[index] {
                ending_reachable = true;
            }
            if node.menu.is_some() || node.room.is_some() {
                continue;
            }
            if node.choices.is_empty() {
                self.warnings
                    .push(path, "dead end: node has no choices and no ending");
            } else if node.choices.iter().all(|choice| choice.when.is_some()) {
                self.warnings
                    .push(path, "possible dead end: every choice is conditional");
            }
        }
        if !ending_reachable {
            self.warnings
                .push("nodes", "no ending is reachable from the start node");
        }
    }
}

fn count_effects(effects: &[Effect], depth: usize) -> usize {
    // Depth beyond the limit is reported by `effect`; stop counting there so
    // hostile nesting cannot recurse further than the bound.
    if depth > MAX_EFFECT_DEPTH {
        return 0;
    }
    effects.iter().fold(0usize, |sum, effect| {
        let nested = match effect {
            Effect::If {
                then, otherwise, ..
            } => count_effects(then, depth + 1).saturating_add(count_effects(otherwise, depth + 1)),
            _ => 0,
        };
        sum.saturating_add(1).saturating_add(nested)
    })
}

fn collect_targets<'a>(effects: &'a [Effect], out: &mut Vec<&'a str>) {
    for effect in effects {
        match effect {
            Effect::Goto(node) => out.push(node),
            Effect::Roll {
                test: Some(test), ..
            } => {
                out.extend(test.success.as_deref());
                out.extend(test.failure.as_deref());
            }
            Effect::If {
                then, otherwise, ..
            } => {
                collect_targets(then, out);
                collect_targets(otherwise, out);
            }
            _ => {}
        }
    }
}

fn any_end(effects: &[Effect]) -> bool {
    effects.iter().any(|effect| match effect {
        Effect::End(_) => true,
        Effect::If {
            then, otherwise, ..
        } => any_end(then) || any_end(otherwise),
        _ => false,
    })
}

/// Bounded by the parser's nesting; called before depth validation only on
/// v1 documents, so it stops at the documented limits.
fn condition_uses_v2(condition: &Condition) -> bool {
    fn walk(condition: &Condition, depth: usize) -> bool {
        if depth > MAX_CONDITION_DEPTH {
            return false;
        }
        match condition {
            Condition::Uses { .. } => true,
            Condition::All(children) | Condition::Any(children) => {
                children.iter().any(|child| walk(child, depth + 1))
            }
            Condition::Not(child) => walk(child, depth + 1),
            _ => false,
        }
    }
    walk(condition, 0)
}

fn effects_use_v2(effects: &[Effect]) -> bool {
    fn walk(effects: &[Effect], depth: usize) -> bool {
        if depth > MAX_EFFECT_DEPTH {
            return false;
        }
        effects.iter().any(|effect| match effect {
            Effect::If {
                when,
                then,
                otherwise,
            } => condition_uses_v2(when) || walk(then, depth + 1) || walk(otherwise, depth + 1),
            _ => false,
        })
    }
    walk(effects, 0)
}
