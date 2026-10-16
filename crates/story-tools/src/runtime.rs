//! The tools' story runtime: the `story-core` entry points with the Lua
//! script host of `tiamud-story-script` attached (ADR 0034), so `preview`,
//! `test` and `play` run packs with scripts exactly like packs without them.
//! For a pack without scripts these are identical to the plain functions.

use tiamud_story_core::{
    CommandStep, MenuSelection, StoryError, StoryEvent, StoryInput, StoryPack, StoryState,
    StoryView,
};
use tiamud_story_script::LuaStoryHost;

/// [`tiamud_story_core::view_scripted`] with the Lua host.
pub fn view(pack: &StoryPack, state: &StoryState) -> Result<StoryView, StoryError> {
    tiamud_story_core::view_scripted(pack, state, &LuaStoryHost::new())
}

/// [`tiamud_story_core::step_scripted`] with the Lua host.
pub fn step(
    pack: &StoryPack,
    state: &StoryState,
    input: StoryInput,
) -> Result<(StoryState, Vec<StoryEvent>), StoryError> {
    tiamud_story_core::step_scripted(pack, state, input, &LuaStoryHost::new())
}

/// [`tiamud_story_core::step_command_scripted`] with the Lua host.
pub fn step_command(
    pack: &StoryPack,
    state: &StoryState,
    text: &str,
    language: Option<&str>,
) -> Result<CommandStep, StoryError> {
    tiamud_story_core::step_command_scripted(pack, state, text, language, &LuaStoryHost::new())
}

/// [`tiamud_story_core::resolve_menu_scripted`] with the Lua host.
pub fn resolve_menu<'a>(
    pack: &'a StoryPack,
    state: &StoryState,
    verb_token: &str,
    target_token: Option<&str>,
) -> Result<MenuSelection<'a>, StoryError> {
    tiamud_story_core::resolve_menu_scripted(
        pack,
        state,
        verb_token,
        target_token,
        &LuaStoryHost::new(),
    )
}
