//! Static authoring lints layered on top of the core validator.
//!
//! The core validator already reports unreachable nodes (ignoring
//! conditions), nodes without choices or ending, nodes whose every choice is
//! conditional and packs where no ending is reachable. These lints add a cheap,
//! *sound* over-approximation of what a playthrough can ever observe:
//!
//! - an integer variable that no effect ever changes keeps its initial value;
//!   otherwise it may take any value inside its declared bounds (or, if it is
//!   only ever `set`, the hull of its initial and assigned values);
//! - a boolean variable can only hold its initial value and values assigned
//!   by `set`;
//! - an item can only be held if it is in the initial inventory or given, and
//!   is held forever if it starts held and is never taken;
//! - a node can only be visited if it is reachable from the start node.
//!
//! A condition judged `Never` really can never hold; one judged `Always`
//! really always holds. Anything uncertain is `Maybe` and produces no lint.
//! The lints never read prose and only run on packs that already validated.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use tiamud_story_core::{
    CmpOp, Condition, Effect, MAX_REPORTED_ISSUES, StoryPack, Value, VariableKind,
};

/// One authoring lint located by pack path and authored ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lint {
    pub path: String,
    pub message: String,
}

impl std::fmt::Display for Lint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

/// Lints of one pack. At most [`MAX_REPORTED_ISSUES`] are retained.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LintReport {
    pub lints: Vec<Lint>,
    pub truncated: bool,
}

impl LintReport {
    fn push(&mut self, path: String, message: impl Into<String>) {
        if self.lints.len() >= MAX_REPORTED_ISSUES {
            self.truncated = true;
            return;
        }
        self.lints.push(Lint {
            path,
            message: message.into(),
        });
    }
}

/// Three-valued truth of a condition over every reachable state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Truth {
    Never,
    Always,
    Maybe,
}

impl Truth {
    fn from_possible(can_true: bool, can_false: bool) -> Self {
        match (can_true, can_false) {
            (true, false) => Self::Always,
            (false, true) => Self::Never,
            _ => Self::Maybe,
        }
    }

    fn not(self) -> Self {
        match self {
            Self::Never => Self::Always,
            Self::Always => Self::Never,
            Self::Maybe => Self::Maybe,
        }
    }
}

/// What any playthrough may observe, computed once per pack.
struct Facts<'a> {
    pack: &'a StoryPack,
    int_ranges: BTreeMap<&'a str, (i64, i64)>,
    bool_values: BTreeMap<&'a str, (bool, bool)>,
    initial_items: BTreeSet<&'a str>,
    given_items: BTreeSet<&'a str>,
    taken_items: BTreeSet<&'a str>,
    /// Nodes reachable when every choice is assumed takeable.
    loose_reachable: BTreeSet<&'a str>,
}

#[derive(Default)]
struct Mutations<'a> {
    /// Integer variables changed by `add` or a stored roll.
    any_int: BTreeSet<&'a str>,
    /// Values assigned by `set`.
    sets: BTreeMap<&'a str, Vec<Value>>,
    given: BTreeSet<&'a str>,
    taken: BTreeSet<&'a str>,
}

fn collect_mutations<'a>(effects: &'a [Effect], out: &mut Mutations<'a>) {
    for effect in effects {
        match effect {
            Effect::Set { var, value } => out.sets.entry(var).or_default().push(*value),
            Effect::Add { var, .. } => {
                out.any_int.insert(var);
            }
            Effect::Give(item) => {
                out.given.insert(item);
            }
            Effect::Take(item) => {
                out.taken.insert(item);
            }
            Effect::Roll {
                store: Some(var), ..
            } => {
                out.any_int.insert(var);
            }
            Effect::If {
                then, otherwise, ..
            } => {
                collect_mutations(then, out);
                collect_mutations(otherwise, out);
            }
            _ => {}
        }
    }
}

impl<'a> Facts<'a> {
    fn new(pack: &'a StoryPack) -> Self {
        let mut mutations = Mutations::default();
        for node in &pack.nodes {
            for choice in &node.choices {
                collect_mutations(&choice.effects, &mut mutations);
            }
        }
        let mut int_ranges = BTreeMap::new();
        let mut bool_values = BTreeMap::new();
        for decl in &pack.variables {
            let assigned = mutations.sets.get(decl.id.as_str());
            match decl.kind {
                VariableKind::Int { min, max, initial } => {
                    let range = if mutations.any_int.contains(decl.id.as_str()) {
                        (min, max)
                    } else {
                        let mut range = (initial, initial);
                        for value in assigned.into_iter().flatten() {
                            if let Value::Int(value) = value {
                                let value = (*value).clamp(min, max);
                                range = (range.0.min(value), range.1.max(value));
                            }
                        }
                        range
                    };
                    int_ranges.insert(decl.id.as_str(), range);
                }
                VariableKind::Bool { initial } => {
                    let mut values = (!initial, initial);
                    for value in assigned.into_iter().flatten() {
                        match value {
                            Value::Bool(true) => values.1 = true,
                            Value::Bool(false) => values.0 = true,
                            Value::Int(_) => {}
                        }
                    }
                    bool_values.insert(decl.id.as_str(), values);
                }
            }
        }
        let mut facts = Self {
            pack,
            int_ranges,
            bool_values,
            initial_items: pack.initial_inventory.iter().map(String::as_str).collect(),
            given_items: mutations.given,
            taken_items: mutations.taken,
            loose_reachable: BTreeSet::new(),
        };
        let graph = Graph::build(&facts, false);
        facts.loose_reachable = graph.reachable_ids(pack);
        facts
    }

    /// Bounded evaluation; validated packs nest at most a few levels.
    fn truth(&self, condition: &Condition) -> Truth {
        match condition {
            Condition::All(children) => {
                let mut result = Truth::Always;
                for child in children {
                    match self.truth(child) {
                        Truth::Never => return Truth::Never,
                        Truth::Maybe => result = Truth::Maybe,
                        Truth::Always => {}
                    }
                }
                result
            }
            Condition::Any(children) => {
                let mut result = Truth::Never;
                for child in children {
                    match self.truth(child) {
                        Truth::Always => return Truth::Always,
                        Truth::Maybe => result = Truth::Maybe,
                        Truth::Never => {}
                    }
                }
                result
            }
            Condition::Not(child) => self.truth(child).not(),
            Condition::VarCmp { var, op, value } => match self.int_ranges.get(var.as_str()) {
                Some(&(low, high)) => compare_range(low, high, *op, *value),
                None => Truth::Maybe,
            },
            Condition::HasItem(item) => {
                let starts = self.initial_items.contains(item.as_str());
                let given = self.given_items.contains(item.as_str());
                let taken = self.taken_items.contains(item.as_str());
                if starts && !taken {
                    Truth::Always
                } else if !starts && !given {
                    Truth::Never
                } else {
                    Truth::Maybe
                }
            }
            Condition::Visited(node) => {
                if *node == self.pack.start {
                    Truth::Always
                } else if !self.loose_reachable.contains(node.as_str()) {
                    Truth::Never
                } else {
                    Truth::Maybe
                }
            }
            Condition::Flag(var) => match self.bool_values.get(var.as_str()) {
                Some(&(can_false, can_true)) => Truth::from_possible(can_true, can_false),
                None => Truth::Maybe,
            },
        }
    }

    fn choice_open(&self, when: Option<&Condition>) -> Truth {
        when.map_or(Truth::Always, |condition| self.truth(condition))
    }

    /// Inclusive range of a roll total.
    fn roll_range(&self, effect: &Effect) -> Option<(i64, i64)> {
        let Effect::Roll {
            dice, modifier_var, ..
        } = effect
        else {
            return None;
        };
        let count = i128::from(dice.count);
        let mut low = count + i128::from(dice.modifier);
        let mut high = count * i128::from(dice.sides) + i128::from(dice.modifier);
        if let Some(var) = modifier_var {
            let (var_low, var_high) = *self.int_ranges.get(var.as_str())?;
            low += i128::from(var_low);
            high += i128::from(var_high);
        }
        let clamp = |value: i128| {
            i64::try_from(value).unwrap_or(if value < 0 { i64::MIN } else { i64::MAX })
        };
        Some((clamp(low), clamp(high)))
    }
}

fn compare_range(low: i64, high: i64, op: CmpOp, value: i64) -> Truth {
    let (can_true, can_false) = match op {
        CmpOp::Eq => (
            low <= value && value <= high,
            !(low == value && high == value),
        ),
        CmpOp::Ne => (
            !(low == value && high == value),
            low <= value && value <= high,
        ),
        CmpOp::Lt => (low < value, high >= value),
        CmpOp::Le => (low <= value, high > value),
        CmpOp::Gt => (high > value, low <= value),
        CmpOp::Ge => (high >= value, low < value),
    };
    Truth::from_possible(can_true, can_false)
}

/// Node graph over choice destinations.
struct Graph {
    edges: Vec<Vec<usize>>,
    /// Nodes where a story may end: ending nodes, or nodes offering a
    /// takeable choice with a reachable `end` effect.
    ends: Vec<bool>,
}

impl Graph {
    /// With `prune`, choices and branches that can never apply are skipped.
    fn build(facts: &Facts<'_>, prune: bool) -> Self {
        let pack = facts.pack;
        let index: BTreeMap<&str, usize> = pack
            .nodes
            .iter()
            .enumerate()
            .map(|(position, node)| (node.id.as_str(), position))
            .collect();
        let mut edges = vec![Vec::new(); pack.nodes.len()];
        let mut ends = vec![false; pack.nodes.len()];
        for (position, node) in pack.nodes.iter().enumerate() {
            if node.ending.is_some() {
                ends[position] = true;
            }
            for choice in &node.choices {
                if prune && facts.choice_open(choice.when.as_ref()) == Truth::Never {
                    continue;
                }
                let mut targets = Vec::new();
                let mut can_end = false;
                if let Some(target) = &choice.target {
                    targets.push(target.as_str());
                }
                walk_targets(facts, prune, &choice.effects, &mut targets, &mut can_end);
                ends[position] |= can_end;
                for target in targets {
                    if let Some(next) = index.get(target) {
                        edges[position].push(*next);
                    }
                }
            }
        }
        Self { edges, ends }
    }

    fn reachable_from(&self, start: usize) -> Vec<bool> {
        let mut seen = vec![false; self.edges.len()];
        let mut queue = VecDeque::new();
        if start < seen.len() {
            seen[start] = true;
            queue.push_back(start);
        }
        while let Some(current) = queue.pop_front() {
            for next in &self.edges[current] {
                if !seen[*next] {
                    seen[*next] = true;
                    queue.push_back(*next);
                }
            }
        }
        seen
    }

    /// Nodes from which some ending can be reached (reverse search).
    fn can_finish(&self) -> Vec<bool> {
        let mut reverse = vec![Vec::new(); self.edges.len()];
        for (from, targets) in self.edges.iter().enumerate() {
            for to in targets {
                reverse[*to].push(from);
            }
        }
        let mut seen = self.ends.clone();
        let mut queue: VecDeque<usize> = (0..seen.len()).filter(|node| seen[*node]).collect();
        while let Some(current) = queue.pop_front() {
            for previous in &reverse[current] {
                if !seen[*previous] {
                    seen[*previous] = true;
                    queue.push_back(*previous);
                }
            }
        }
        seen
    }

    fn reachable_ids<'a>(&self, pack: &'a StoryPack) -> BTreeSet<&'a str> {
        let Some(start) = pack.nodes.iter().position(|node| node.id == pack.start) else {
            return BTreeSet::new();
        };
        self.reachable_from(start)
            .into_iter()
            .zip(&pack.nodes)
            .filter_map(|(seen, node)| seen.then_some(node.id.as_str()))
            .collect()
    }
}

fn walk_targets<'a>(
    facts: &Facts<'_>,
    prune: bool,
    effects: &'a [Effect],
    targets: &mut Vec<&'a str>,
    can_end: &mut bool,
) {
    for effect in effects {
        match effect {
            Effect::Goto(node) => targets.push(node),
            Effect::End(_) => *can_end = true,
            Effect::Roll {
                test: Some(test), ..
            } => {
                let (low, high) = facts.roll_range(effect).unwrap_or((i64::MIN, i64::MAX));
                if let Some(success) = &test.success
                    && (!prune || high >= test.at_least)
                {
                    targets.push(success);
                }
                if let Some(failure) = &test.failure
                    && (!prune || low < test.at_least)
                {
                    targets.push(failure);
                }
            }
            Effect::If {
                when,
                then,
                otherwise,
            } => {
                let truth = if prune {
                    facts.truth(when)
                } else {
                    Truth::Maybe
                };
                if truth != Truth::Never {
                    walk_targets(facts, prune, then, targets, can_end);
                }
                if truth != Truth::Always {
                    walk_targets(facts, prune, otherwise, targets, can_end);
                }
            }
            _ => {}
        }
    }
}

fn lint_effects(facts: &Facts<'_>, path: &str, effects: &[Effect], report: &mut LintReport) {
    for (index, effect) in effects.iter().enumerate() {
        let path = format!("{path}.effects[{index}]");
        match effect {
            Effect::If {
                when,
                then,
                otherwise,
            } => {
                match facts.truth(when) {
                    Truth::Never if !then.is_empty() => report.push(
                        path.clone(),
                        "`if` condition can never hold; its `then` branch is dead",
                    ),
                    Truth::Always if !otherwise.is_empty() => report.push(
                        path.clone(),
                        "`if` condition always holds; its `otherwise` branch is dead",
                    ),
                    _ => {}
                }
                lint_effects(facts, &format!("{path}.then"), then, report);
                lint_effects(facts, &format!("{path}.otherwise"), otherwise, report);
            }
            Effect::Roll {
                test: Some(test), ..
            } => {
                if let Some((low, high)) = facts.roll_range(effect) {
                    if low >= test.at_least {
                        report.push(
                            path,
                            format!(
                                "roll test always succeeds (total {low}..={high}, needs {})",
                                test.at_least
                            ),
                        );
                    } else if high < test.at_least {
                        report.push(
                            path,
                            format!(
                                "roll test can never succeed (total {low}..={high}, needs {})",
                                test.at_least
                            ),
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

/// Runs every authoring lint on a pack that already passed
/// [`StoryPack::validate`]. Output order is deterministic.
pub fn lint(pack: &StoryPack) -> LintReport {
    let mut report = LintReport::default();
    let facts = Facts::new(pack);
    for (node_index, node) in pack.nodes.iter().enumerate() {
        for (choice_index, choice) in node.choices.iter().enumerate() {
            let path = format!(
                "nodes[{node_index}].choices[{choice_index}] ({}/{})",
                node.id, choice.id
            );
            match facts.choice_open(choice.when.as_ref()) {
                Truth::Never => report.push(
                    path.clone(),
                    "choice condition can never be satisfied (variable bounds, item sources or reachability)",
                ),
                Truth::Always if choice.when.is_some() => report.push(
                    path.clone(),
                    "choice condition always holds; the gate is redundant",
                ),
                _ => {}
            }
            lint_effects(&facts, &path, &choice.effects, &mut report);
        }
    }

    let loose = Graph::build(&facts, false);
    let strict = Graph::build(&facts, true);
    let Some(start) = pack.nodes.iter().position(|node| node.id == pack.start) else {
        return report;
    };
    let loose_reach = loose.reachable_from(start);
    let strict_reach = strict.reachable_from(start);
    let loose_finish = loose.can_finish();
    let strict_finish = strict.can_finish();
    let any_strict_ending =
        (0..pack.nodes.len()).any(|node| strict_reach[node] && strict.ends[node]);
    let any_loose_ending = (0..pack.nodes.len()).any(|node| loose_reach[node] && loose.ends[node]);
    if any_loose_ending && !any_strict_ending {
        report.push(
            "nodes".to_owned(),
            "no ending is reachable once choices that can never be taken are discounted",
        );
    }
    for (index, node) in pack.nodes.iter().enumerate() {
        let path = format!("nodes[{index}] ({})", node.id);
        if loose_reach[index] && !strict_reach[index] {
            report.push(
                path,
                "node is only reachable through choices or branches that can never be taken",
            );
            continue;
        }
        if !strict_reach[index] || node.ending.is_some() || node.choices.is_empty() {
            // Unreachable and choice-less nodes are core validator warnings.
            continue;
        }
        let all_closed = node
            .choices
            .iter()
            .all(|choice| facts.choice_open(choice.when.as_ref()) == Truth::Never);
        if all_closed {
            report.push(
                path,
                "dead end: no choice of this node can ever be available",
            );
        } else if any_strict_ending && !strict_finish[index] {
            let message = if loose_finish[index] {
                "soft-lock: every path to an ending needs a choice that can never be taken"
            } else {
                "soft-lock: no ending can be reached from this node"
            };
            report.push(path, message);
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_comparisons_are_sound() {
        assert_eq!(compare_range(0, 10, CmpOp::Gt, 10), Truth::Never);
        assert_eq!(compare_range(0, 10, CmpOp::Ge, 10), Truth::Maybe);
        assert_eq!(compare_range(0, 10, CmpOp::Ge, 0), Truth::Always);
        assert_eq!(compare_range(0, 10, CmpOp::Lt, 0), Truth::Never);
        assert_eq!(compare_range(0, 10, CmpOp::Le, 10), Truth::Always);
        assert_eq!(compare_range(5, 5, CmpOp::Eq, 5), Truth::Always);
        assert_eq!(compare_range(5, 5, CmpOp::Ne, 5), Truth::Never);
        assert_eq!(compare_range(0, 4, CmpOp::Eq, 9), Truth::Never);
        assert_eq!(compare_range(0, 4, CmpOp::Ne, 9), Truth::Always);
    }
}
