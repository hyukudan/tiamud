//! Player projection of a state: the full [`StoryView`] and its localized,
//! byte-bounded pages.

use serde::{Deserialize, Serialize};

use crate::engine::{
    active_dialogue, available_choices, choice_token, current_node, dialogue_node, menu_verbs,
    option_token, target_token, verb_targets, verb_token,
};
use crate::{
    Cue, Ending, LastResponse, LocalizedText, MAX_VIEW_BYTES, StoryError, StoryPack, StoryState,
    TargetKind, TextBlock, Value,
};

/// What a player may see: resolved node text, available choices, menu verbs
/// and targets, and dialogue options behind opaque tokens, the visible stat
/// sheet and inventory, and presentation asset ids. Hidden variables, authored
/// ids, unavailable choices, use counters and RNG state are never included.
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
    /// Background asset id of the scene.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// Cues of the scene, so a client that joins late can resynchronize.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cues: Vec<Cue>,
    /// Characters present in the scene.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub portraits: Vec<ViewPortrait>,
    /// Message window: the last menu response followed by the current
    /// dialogue lines.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<ViewBlock>,
    /// Verb menu; absent while a dialogue must be answered or after the end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menu: Option<ViewMenu>,
    /// Active dialogue options.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dialogue: Option<ViewDialogue>,
}

/// A selectable choice or dialogue option. `token` is bound to the view's
/// turn and the ordinal among the *available* entries; it never encodes the
/// authored id or label, and reveals nothing about hidden entries.
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewPortrait {
    pub name: LocalizedText,
    /// Reviewed portrait asset id; absent when the expression has none.
    pub asset: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewBlock {
    pub speaker: Option<LocalizedText>,
    pub portrait: Option<String>,
    pub text: LocalizedText,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewMenu {
    pub verbs: Vec<ViewVerb>,
}

/// A menu verb. `needs_target` tells the client to pick one of `targets`;
/// an empty target list with `needs_target` means nothing to apply it to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewVerb {
    pub token: String,
    pub label: LocalizedText,
    pub needs_target: bool,
    pub targets: Vec<ViewTarget>,
}

/// A verb target. Its token is bound to the turn and to its verb.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewTarget {
    pub token: String,
    pub label: LocalizedText,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewDialogue {
    pub options: Vec<ViewChoice>,
}

/// Builds the player view of `state`.
pub fn view(pack: &StoryPack, state: &StoryState) -> Result<StoryView, StoryError> {
    state.check_against(pack)?;
    let node = current_node(pack, state)?;
    let open = state.ended.is_none();
    let dialogue = if open {
        active_dialogue(pack, state)?
    } else {
        None
    };
    let choices = if open && dialogue.is_none() {
        available_choices(state, node)?
            .into_iter()
            .enumerate()
            .map(|(ordinal, choice)| ViewChoice {
                token: choice_token(state.turn, ordinal),
                label: choice.label.clone(),
            })
            .collect()
    } else {
        Vec::new()
    };
    let menu = if open && dialogue.is_none() && node.menu.is_some() {
        let mut verbs = Vec::new();
        for (ordinal, verb) in menu_verbs(pack, state, node)?.into_iter().enumerate() {
            let targets = verb_targets(pack, state, node, verb)?
                .into_iter()
                .enumerate()
                .map(|(target, (_, label))| ViewTarget {
                    token: target_token(state.turn, ordinal, target),
                    label: label.clone(),
                })
                .collect();
            verbs.push(ViewVerb {
                token: verb_token(state.turn, ordinal),
                label: verb.label.clone(),
                needs_target: verb.targets != TargetKind::None,
                targets,
            });
        }
        Some(ViewMenu { verbs })
    } else {
        None
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
    let mut portraits = Vec::new();
    for presence in &node.present {
        if crate::engine::holds(state, presence.when.as_ref())?
            && let Some(character) = pack.character(&presence.character)
        {
            portraits.push(ViewPortrait {
                name: character.name.clone(),
                asset: character.portrait(presence.expression.as_deref()).cloned(),
            });
        }
    }
    let mut blocks = Vec::new();
    for block in last_blocks(pack, state) {
        blocks.push(view_block(pack, block));
    }
    if let Some((_, dialogue_node)) = dialogue_node(pack, state)? {
        for line in &dialogue_node.lines {
            blocks.push(view_block(pack, line));
        }
    }
    let dialogue = dialogue.map(|active| ViewDialogue {
        options: active
            .options
            .iter()
            .enumerate()
            .map(|(ordinal, option)| ViewChoice {
                token: option_token(state.turn, ordinal),
                label: option.label.clone(),
            })
            .collect(),
    });
    Ok(StoryView {
        title: pack.title.clone(),
        text: node.text.clone(),
        turn: state.turn,
        choices,
        stats,
        inventory,
        score: state.score,
        ending: state.ended.clone(),
        background: node.background.clone(),
        cues: node.cues.clone(),
        portraits,
        blocks,
        menu,
        dialogue,
    })
}

fn view_block(pack: &StoryPack, block: &TextBlock) -> ViewBlock {
    let character = block
        .speaker
        .as_deref()
        .and_then(|speaker| pack.character(speaker));
    ViewBlock {
        speaker: character.map(|character| character.name.clone()),
        portrait: character
            .and_then(|character| character.portrait(block.expression.as_deref()))
            .cloned(),
        text: block.text.clone(),
    }
}

/// Text blocks of the last menu response; empty when there is none.
fn last_blocks<'a>(pack: &'a StoryPack, state: &StoryState) -> &'a [TextBlock] {
    let Some(last) = &state.last else {
        return &[];
    };
    let menu_of = |node: &str| pack.node(node).and_then(|node| node.menu.as_ref());
    let blocks = match last {
        LastResponse::Entry {
            node,
            entry,
            response,
        } => menu_of(node)
            .and_then(|menu| menu.entry(entry))
            .and_then(|entry| entry.response(response))
            .map(|response| response.blocks.as_slice()),
        LastResponse::Exhausted { node, entry } => menu_of(node)
            .and_then(|menu| menu.entry(entry))
            .map(|entry| entry.exhausted.as_slice()),
        LastResponse::SceneDefault { node, verb } => menu_of(node)
            .and_then(|menu| menu.defaults.iter().find(|default| default.verb == *verb))
            .map(|default| default.blocks.as_slice()),
        LastResponse::VerbDefault { verb } => pack.verb(verb).map(|verb| verb.default.as_slice()),
    };
    blocks.unwrap_or(&[])
}

/// One localized page of a view. Its JSON encoding never exceeds
/// [`MAX_VIEW_BYTES`]; pages are numbered `1..=pages` and share the turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewPage {
    pub turn: u32,
    pub page: u16,
    pub pages: u16,
    pub items: Vec<PageItem>,
}

/// A localized, self-contained piece of a view. Items keep their order across
/// pages, and no item is ever split or truncated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageItem {
    Scene {
        title: String,
        text: String,
        score: i64,
        background: Option<String>,
        ending: Option<Ending>,
    },
    Cue(Cue),
    Portrait {
        name: String,
        asset: Option<String>,
    },
    Block {
        speaker: Option<String>,
        portrait: Option<String>,
        text: String,
    },
    Stat {
        label: String,
        value: Value,
    },
    Item {
        name: String,
    },
    Choice {
        token: String,
        label: String,
    },
    Verb {
        token: String,
        label: String,
        needs_target: bool,
    },
    Target {
        verb: String,
        token: String,
        label: String,
    },
    DialogueOption {
        token: String,
        label: String,
    },
}

/// Projects the view for one locale and splits it into pages whose JSON is
/// at most [`MAX_VIEW_BYTES`] each. Content is paginated, never truncated: an
/// item that alone cannot fit (impossible for a validated pack) is an error.
pub fn view_pages(
    pack: &StoryPack,
    state: &StoryState,
    locale: &str,
) -> Result<Vec<ViewPage>, StoryError> {
    let full = view(pack, state)?;
    let text = |value: &LocalizedText| value.resolve(locale).unwrap_or_default().to_owned();
    let mut items = vec![PageItem::Scene {
        title: text(&full.title),
        text: text(&full.text),
        score: full.score,
        background: full.background.clone(),
        ending: full.ending.clone(),
    }];
    items.extend(full.cues.iter().cloned().map(PageItem::Cue));
    items.extend(full.portraits.iter().map(|portrait| PageItem::Portrait {
        name: text(&portrait.name),
        asset: portrait.asset.clone(),
    }));
    items.extend(full.blocks.iter().map(|block| PageItem::Block {
        speaker: block.speaker.as_ref().map(text),
        portrait: block.portrait.clone(),
        text: text(&block.text),
    }));
    items.extend(full.stats.iter().map(|stat| PageItem::Stat {
        label: text(&stat.label),
        value: stat.value,
    }));
    items.extend(
        full.inventory
            .iter()
            .map(|name| PageItem::Item { name: text(name) }),
    );
    items.extend(full.choices.iter().map(|choice| PageItem::Choice {
        token: choice.token.clone(),
        label: text(&choice.label),
    }));
    if let Some(menu) = &full.menu {
        for verb in &menu.verbs {
            items.push(PageItem::Verb {
                token: verb.token.clone(),
                label: text(&verb.label),
                needs_target: verb.needs_target,
            });
            items.extend(verb.targets.iter().map(|target| PageItem::Target {
                verb: verb.token.clone(),
                token: target.token.clone(),
                label: text(&target.label),
            }));
        }
    }
    if let Some(dialogue) = &full.dialogue {
        items.extend(
            dialogue
                .options
                .iter()
                .map(|option| PageItem::DialogueOption {
                    token: option.token.clone(),
                    label: text(&option.label),
                }),
        );
    }
    paginate(full.turn, items)
}

fn json_len<T: Serialize>(value: &T) -> Result<usize, StoryError> {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .map_err(|error| StoryError::Malformed(crate::bounded_message(error.to_string())))
}

/// Greedy, order-preserving packing by measured JSON size.
fn paginate(turn: u32, items: Vec<PageItem>) -> Result<Vec<ViewPage>, StoryError> {
    // Envelope of an empty page with the widest page numbers.
    let overhead = json_len(&ViewPage {
        turn,
        page: u16::MAX,
        pages: u16::MAX,
        items: Vec::new(),
    })?;
    let mut groups: Vec<Vec<PageItem>> = Vec::new();
    let mut current: Vec<PageItem> = Vec::new();
    let mut used = overhead;
    for item in items {
        let size = json_len(&item)?;
        if overhead.saturating_add(size) > MAX_VIEW_BYTES {
            return Err(StoryError::ViewTooLarge {
                limit: MAX_VIEW_BYTES,
            });
        }
        // One comma separates every item after the first.
        let added = size.saturating_add(usize::from(!current.is_empty()));
        if used.saturating_add(added) > MAX_VIEW_BYTES {
            groups.push(std::mem::take(&mut current));
            used = overhead.saturating_add(size);
        } else {
            used = used.saturating_add(added);
        }
        current.push(item);
    }
    if !current.is_empty() || groups.is_empty() {
        groups.push(current);
    }
    let pages = u16::try_from(groups.len()).map_err(|_| StoryError::ViewTooLarge {
        limit: MAX_VIEW_BYTES,
    })?;
    let mut out = Vec::with_capacity(groups.len());
    for (index, items) in groups.into_iter().enumerate() {
        let page = ViewPage {
            turn,
            // `index < pages <= u16::MAX`, so this cannot fail.
            page: u16::try_from(index + 1).unwrap_or(u16::MAX),
            pages,
            items,
        };
        if json_len(&page)? > MAX_VIEW_BYTES {
            return Err(StoryError::ViewTooLarge {
                limit: MAX_VIEW_BYTES,
            });
        }
        out.push(page);
    }
    Ok(out)
}
