//! Topology queries of the parser world (format v3): containment, reach,
//! light and the parser's scope. Every walk is bounded by the number of
//! declared objects, so a corrupted state yields `None` instead of looping.

use crate::world::{Location, ObjectDecl};
use crate::{StoryPack, StoryState};

/// The outermost holder of an object and whether every container between
/// them is open.
pub(crate) struct Reach<'a> {
    /// Outermost object of the containment chain (the object itself when it
    /// is not inside anything).
    pub top: &'a str,
    /// Location of `top`; never [`Location::In`].
    pub at: &'a Location,
    /// True when every container enclosing the object is open.
    pub open: bool,
}

/// Follows `in` links to the outermost holder. `None` on a cycle or when a
/// link names an object without state.
pub(crate) fn reach<'a>(state: &'a StoryState, id: &'a str) -> Option<Reach<'a>> {
    let mut current = id;
    let mut open = true;
    for _ in 0..=state.objects.len() {
        let object = state.objects.get(current)?;
        match &object.at {
            Location::In(container) => {
                let holder = state.objects.get(container.as_str())?;
                open &= holder.open;
                current = container.as_str();
            }
            at => {
                return Some(Reach {
                    top: current,
                    at,
                    open,
                });
            }
        }
    }
    None
}

/// Location of the outermost holder; `None` on a containment cycle.
pub(crate) fn root<'a>(state: &'a StoryState, id: &'a str) -> Option<&'a Location> {
    reach(state, id).map(|reach| reach.at)
}

/// True when the object is held, directly or inside held containers.
pub(crate) fn held(state: &StoryState, id: &str) -> bool {
    matches!(root(state, id), Some(Location::Carried))
}

/// True when `ancestor` encloses `id` (directly or transitively).
pub(crate) fn inside(state: &StoryState, id: &str, ancestor: &str) -> bool {
    let mut current = id;
    for _ in 0..=state.objects.len() {
        match state.objects.get(current).map(|object| &object.at) {
            Some(Location::In(container)) => {
                if container == ancestor {
                    return true;
                }
                current = container.as_str();
            }
            _ => return false,
        }
    }
    // A cycle: report enclosure so callers refuse to deepen it.
    true
}

/// True when the object's outermost holder stands in room `node`, either on
/// its floor or as a fixed object also found there.
fn in_room(pack: &StoryPack, reach: &Reach<'_>, node: &str) -> bool {
    match reach.at {
        Location::Room(room) => {
            room == node
                || pack
                    .object(reach.top)
                    .is_some_and(|decl| decl.found_in.iter().any(|other| other == node))
        }
        _ => false,
    }
}

/// True when the current node is lit: not a room, a room that is not dark,
/// or a dark room with a lit object held or standing there in reach.
pub(crate) fn lit(pack: &StoryPack, state: &StoryState) -> bool {
    let Some(room) = pack.room(&state.node) else {
        return true;
    };
    if !room.dark {
        return true;
    }
    state.objects.iter().any(|(id, object)| {
        object.lit
            && reach(state, id).is_some_and(|reach| {
                reach.open
                    && (matches!(reach.at, Location::Carried) || in_room(pack, &reach, &state.node))
            })
    })
}

/// True when the parser can refer to the object now: held, or in the current
/// room while it is lit, with every enclosing container open.
pub(crate) fn visible(pack: &StoryPack, state: &StoryState, id: &str) -> bool {
    let Some(reach) = reach(state, id) else {
        return false;
    };
    reach.open
        && match reach.at {
            Location::Carried => true,
            Location::Room(_) => in_room(pack, &reach, &state.node) && lit(pack, state),
            _ => false,
        }
}

/// Visible objects, in pack order.
pub(crate) fn visible_objects<'a>(pack: &'a StoryPack, state: &StoryState) -> Vec<&'a ObjectDecl> {
    let Some(world) = &pack.world else {
        return Vec::new();
    };
    world
        .objects
        .iter()
        .filter(|decl| visible(pack, state, &decl.id))
        .collect()
}

/// Objects directly inside `container`, in pack order.
pub(crate) fn contents<'a>(
    pack: &'a StoryPack,
    state: &StoryState,
    container: &str,
) -> Vec<&'a str> {
    let Some(world) = &pack.world else {
        return Vec::new();
    };
    world
        .objects
        .iter()
        .filter(|decl| {
            state
                .objects
                .get(&decl.id)
                .is_some_and(|object| matches!(&object.at, Location::In(c) if c == container))
        })
        .map(|decl| decl.id.as_str())
        .collect()
}

/// Objects directly on the floor of `node` or fixed there, in pack order.
pub(crate) fn room_objects<'a>(
    pack: &'a StoryPack,
    state: &StoryState,
    node: &str,
) -> Vec<&'a ObjectDecl> {
    let Some(world) = &pack.world else {
        return Vec::new();
    };
    world
        .objects
        .iter()
        .filter(
            |decl| match state.objects.get(&decl.id).map(|object| &object.at) {
                Some(Location::Room(room)) => {
                    room == node || decl.found_in.iter().any(|other| other == node)
                }
                _ => false,
            },
        )
        .collect()
}
