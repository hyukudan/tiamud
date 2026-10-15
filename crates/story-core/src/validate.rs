//! Static StoryPack validation and authoring lints.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use crate::{
    Condition, Cue, Effect, Ending, MAX_CHOICES_PER_NODE, MAX_CONDITION_DEPTH, MAX_CONDITION_NODES,
    MAX_DICE_COUNT, MAX_DICE_MODIFIER, MAX_DICE_SIDES, MAX_EFFECT_DEPTH, MAX_EFFECTS_PER_CHOICE,
    MAX_ITEMS, MAX_LABEL_BYTES, MAX_NODES, MAX_REPORTED_ISSUES, MAX_SCORE_DELTA, MAX_TEXT_BYTES,
    MAX_TOTAL_CHOICES, MAX_VAR_ABS, MAX_VARIABLES, STORY_PACK_FORMAT_VERSION, StoryPack, Value,
    VariableKind, valid_id,
};

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
        if pack.format_version != STORY_PACK_FORMAT_VERSION {
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
        self.declarations();
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
        for index in 0..pack.nodes.len() {
            self.node(index);
        }
        if self.errors.list.is_empty() {
            self.lints();
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
            if let Some(condition) = &choice.when {
                let mut budget = MAX_CONDITION_NODES;
                self.condition(&format!("{path}.when"), condition, 0, &mut budget);
            }
            if let Some(target) = &choice.target {
                self.node_ref(&format!("{path}.target"), target);
            }
            let total = count_effects(&choice.effects, 0);
            if total > MAX_EFFECTS_PER_CHOICE {
                self.error(
                    format!("{path}.effects"),
                    format!("at most {MAX_EFFECTS_PER_CHOICE} effects including nested ones"),
                );
            } else {
                self.effects(&format!("{path}.effects"), &choice.effects, 0);
            }
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
            Effect::Cue(Cue::Music { asset } | Cue::Sfx { asset }) => {
                if !valid_id(asset) {
                    self.error(path, "cue asset must be a reviewed asset id, not a URL");
                }
            }
            Effect::Cue(Cue::StopMusic) => {}
            Effect::Score(delta) => {
                if delta.unsigned_abs() > MAX_SCORE_DELTA.unsigned_abs() {
                    self.error(path, "score delta is out of range");
                }
            }
            Effect::End(ending) => self.ending(path, ending),
        }
    }

    /// Reachability (ignoring conditions), dead ends and missing endings.
    fn lints(&mut self) {
        let pack = self.pack;
        let mut edges: Vec<Vec<usize>> = vec![Vec::new(); pack.nodes.len()];
        for (index, node) in pack.nodes.iter().enumerate() {
            for choice in &node.choices {
                let mut targets = Vec::new();
                if let Some(target) = &choice.target {
                    targets.push(target.as_str());
                }
                collect_targets(&choice.effects, &mut targets);
                for target in targets {
                    if let Some(next) = self.nodes.get(target) {
                        edges[index].push(*next);
                    }
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
            if reachable[index] && pack.nodes[index].choices.iter().any(has_end_effect) {
                ending_reachable = true;
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

fn has_end_effect(choice: &crate::Choice) -> bool {
    fn any_end(effects: &[Effect]) -> bool {
        effects.iter().any(|effect| match effect {
            Effect::End(_) => true,
            Effect::If {
                then, otherwise, ..
            } => any_end(then) || any_end(otherwise),
            _ => false,
        })
    }
    any_end(&choice.effects)
}
