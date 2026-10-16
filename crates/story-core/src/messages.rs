//! Default library messages (Spanish and English) and rendering of parser
//! transcript replies into localized text (format v3).

use std::collections::BTreeMap;

use crate::world::{Gender, LibraryMessage as M, ObjectDecl};
use crate::{LocalizedText, Reply, StoryPack};

/// Built-in template of a library message. Unknown languages use Spanish.
pub(crate) fn template(message: M, language: &str) -> &'static str {
    if language == "en" {
        english(message)
    } else {
        spanish(message)
    }
}

fn spanish(message: M) -> &'static str {
    match message {
        M::Dark => "Está oscuro como boca de lobo. No ves nada.",
        M::DarkName => "Oscuridad",
        M::YouSee => "Puedes ver {list}.",
        M::InsideYouSee => "Dentro de {obj} ves {list}.",
        M::Empty => "{Obj} está vací{o}.",
        M::Inventory => "Llevas {list}.",
        M::InventoryEmpty => "No llevas nada.",
        M::Wearing => "Llevas puesto: {list}.",
        M::Taken => "Coges {obj}.",
        M::AlreadyCarried => "Ya tienes {obj}.",
        M::NotPortable => "No puedes llevarte {obj}.",
        M::TakeAnimate => "{Obj} no se deja coger.",
        M::NothingToTake => "No hay nada que coger.",
        M::Dropped => "Dejas {obj}.",
        M::NotCarried => "No llevas {obj}.",
        M::NothingToDrop => "No llevas nada que dejar.",
        M::TakeOffFirst => "Primero tendrás que quitarte {obj}.",
        M::Opened => "Abres {obj}.",
        M::NotOpenable => "No puedes abrir {obj}.",
        M::AlreadyOpen => "{Obj} ya está abiert{o}.",
        M::Locked => "{Obj} está cerrad{o} con llave.",
        M::Closed => "Cierras {obj}.",
        M::NotClosable => "No puedes cerrar {obj}.",
        M::AlreadyClosed => "{Obj} ya está cerrad{o}.",
        M::Unlocked => "Abres la cerradura de {obj} con {obj2}.",
        M::NoLock => "{Obj} no tiene cerradura.",
        M::NotLocked => "{Obj} no está cerrad{o} con llave.",
        M::WrongKey => "{Obj2} no encaja en la cerradura de {obj}.",
        M::LockedWith => "Echas la llave de {obj} con {obj2}.",
        M::AlreadyLocked => "{Obj} ya está cerrad{o} con llave.",
        M::CloseFirst => "Primero tendrás que cerrar {obj}.",
        M::PutIn => "Metes {obj} en {obj2}.",
        M::NotContainer => "No puedes meter nada en {obj2}.",
        M::ContainerClosed => "{Obj2} está cerrad{o2}.",
        M::PutInSelf => "No puedes meter {obj} dentro de sí mism{o}.",
        M::PutOn => "No puedes poner nada encima de {obj2}.",
        M::NoExit => "No puedes ir por ahí.",
        M::DoorClosed => "{Obj} está cerrad{o}.",
        M::Wait => "Pasa el tiempo.",
        M::CantEnter => "No puedes entrar en {obj}.",
        M::GiveNotAnimate => "No puedes darle nada a {obj2}.",
        M::GiveDeclined => "{Obj2} no parece interesad{o2}.",
        M::ShowDeclined => "{Obj2} no muestra interés.",
        M::TalkNotAnimate => "No obtienes respuesta de {obj}.",
        M::TalkNothing => "{Obj} no tiene nada que decirte.",
        M::NothingWritten => "No hay nada escrito en {obj}.",
        M::Worn => "Te pones {obj}.",
        M::NotWearable => "No puedes ponerte {obj}.",
        M::AlreadyWorn => "Ya llevas puest{o} {obj}.",
        M::Removed => "Te quitas {obj}.",
        M::NotWorn => "No llevas puest{o} {obj}.",
        M::SwitchedOn => "Enciendes {obj}.",
        M::NotLightable => "No puedes encender ni apagar {obj}.",
        M::AlreadyOn => "{Obj} ya está encendid{o}.",
        M::SwitchedOff => "Apagas {obj}.",
        M::AlreadyOff => "{Obj} ya está apagad{o}.",
        M::GoesDark => "Todo queda a oscuras.",
        M::SearchNothing => "No encuentras nada interesante.",
        M::NothingHappens => "No ocurre nada.",
        M::Score => "Llevas {score} puntos en {turns} turnos.",
        M::ScoreOf => "Llevas {score} de {max} puntos en {turns} turnos.",
        M::Help => {
            "Escribe órdenes como «mira», «examina la mesa», «coge la lámpara», «norte», «abre la puerta con la llave» o «habla con el guarda». También: inventario, esperar, puntos y deshacer."
        }
    }
}

fn english(message: M) -> &'static str {
    match message {
        M::Dark => "It is pitch dark. You can't see a thing.",
        M::DarkName => "Darkness",
        M::YouSee => "You can see {list}.",
        M::InsideYouSee => "In {obj} you see {list}.",
        M::Empty => "{Obj} is empty.",
        M::Inventory => "You are carrying {list}.",
        M::InventoryEmpty => "You are carrying nothing.",
        M::Wearing => "You are wearing {list}.",
        M::Taken => "You take {obj}.",
        M::AlreadyCarried => "You already have {obj}.",
        M::NotPortable => "You can't take {obj}.",
        M::TakeAnimate => "{Obj} won't let you.",
        M::NothingToTake => "There is nothing to take.",
        M::Dropped => "You drop {obj}.",
        M::NotCarried => "You aren't holding {obj}.",
        M::NothingToDrop => "You have nothing to drop.",
        M::TakeOffFirst => "You'll have to take off {obj} first.",
        M::Opened => "You open {obj}.",
        M::NotOpenable => "You can't open {obj}.",
        M::AlreadyOpen => "{Obj} is already open.",
        M::Locked => "{Obj} is locked.",
        M::Closed => "You close {obj}.",
        M::NotClosable => "You can't close {obj}.",
        M::AlreadyClosed => "{Obj} is already closed.",
        M::Unlocked => "You unlock {obj} with {obj2}.",
        M::NoLock => "{Obj} has no lock.",
        M::NotLocked => "{Obj} isn't locked.",
        M::WrongKey => "{Obj2} doesn't fit the lock of {obj}.",
        M::LockedWith => "You lock {obj} with {obj2}.",
        M::AlreadyLocked => "{Obj} is already locked.",
        M::CloseFirst => "You'll have to close {obj} first.",
        M::PutIn => "You put {obj} in {obj2}.",
        M::NotContainer => "You can't put anything in {obj2}.",
        M::ContainerClosed => "{Obj2} is closed.",
        M::PutInSelf => "You can't put {obj} inside itself.",
        M::PutOn => "You can't put anything on {obj2}.",
        M::NoExit => "You can't go that way.",
        M::DoorClosed => "{Obj} is closed.",
        M::Wait => "Time passes.",
        M::CantEnter => "You can't enter {obj}.",
        M::GiveNotAnimate => "You can't give anything to {obj2}.",
        M::GiveDeclined => "{Obj2} doesn't seem interested.",
        M::ShowDeclined => "{Obj2} shows no interest.",
        M::TalkNotAnimate => "{Obj} doesn't answer.",
        M::TalkNothing => "{Obj} has nothing to say to you.",
        M::NothingWritten => "There is nothing written on {obj}.",
        M::Worn => "You put on {obj}.",
        M::NotWearable => "You can't wear {obj}.",
        M::AlreadyWorn => "You are already wearing {obj}.",
        M::Removed => "You take off {obj}.",
        M::NotWorn => "You aren't wearing {obj}.",
        M::SwitchedOn => "You light {obj}.",
        M::NotLightable => "You can't light or put out {obj}.",
        M::AlreadyOn => "{Obj} is already lit.",
        M::SwitchedOff => "You put out {obj}.",
        M::AlreadyOff => "{Obj} isn't lit.",
        M::GoesDark => "Everything goes dark.",
        M::SearchNothing => "You find nothing of interest.",
        M::NothingHappens => "Nothing happens.",
        M::Score => "You have scored {score} points in {turns} turns.",
        M::ScoreOf => "You have scored {score} out of {max} points in {turns} turns.",
        M::Help => {
            "Type commands such as \"look\", \"examine the desk\", \"take the lamp\", \"north\", \"open the door with the key\" or \"talk to the keeper\". Also: inventory, wait, score and undo."
        }
    }
}

/// Name of an object in `language` without article.
fn bare<'a>(decl: &'a ObjectDecl, language: &str) -> &'a str {
    decl.name.resolve(language).unwrap_or(decl.id.as_str())
}

/// Definite noun phrase: `la lámpara`, `the lamp`, or the proper name.
pub(crate) fn definite(decl: &ObjectDecl, language: &str) -> String {
    let name = bare(decl, language);
    if decl.proper {
        return name.to_owned();
    }
    let article = match (language, decl.gender) {
        ("en", _) => "the",
        (_, Gender::Masculine) => "el",
        (_, Gender::Feminine) => "la",
    };
    format!("{article} {name}")
}

/// Indefinite noun phrase: `una lámpara`, `a lamp`, `an oil can`.
pub(crate) fn indefinite(decl: &ObjectDecl, language: &str) -> String {
    let name = bare(decl, language);
    if decl.proper {
        return name.to_owned();
    }
    let article = match (language, decl.gender) {
        ("en", _) => {
            if name
                .chars()
                .next()
                .is_some_and(|c| matches!(c.to_ascii_lowercase(), 'a' | 'e' | 'i' | 'o' | 'u'))
            {
                "an"
            } else {
                "a"
            }
        }
        (_, Gender::Masculine) => "un",
        (_, Gender::Feminine) => "una",
    };
    format!("{article} {name}")
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn ending(decl: Option<&ObjectDecl>, language: &str) -> &'static str {
    match (language, decl.map(|decl| decl.gender)) {
        ("en", _) => "",
        (_, Some(Gender::Feminine)) => "a",
        _ => "o",
    }
}

/// `a, b y c` / `a, b and c`.
fn join(names: &[String], language: &str) -> String {
    let and = if language == "en" { "and" } else { "y" };
    match names {
        [] => String::new(),
        [only] => only.clone(),
        [init @ .., last] => format!("{} {and} {last}", init.join(", ")),
    }
}

/// Spanish contractions `de el` → `del` and `a el` → `al`.
fn contract(text: String, language: &str) -> String {
    if language == "en" {
        return text;
    }
    let mut out = String::with_capacity(text.len());
    let words: Vec<&str> = text.split(' ').collect();
    let mut index = 0;
    while let Some(word) = words.get(index) {
        if !out.is_empty() {
            out.push(' ');
        }
        let next = words.get(index + 1).copied();
        match (*word, next) {
            ("de", Some("el")) => {
                out.push_str("del");
                index += 2;
            }
            ("De", Some("el")) => {
                out.push_str("Del");
                index += 2;
            }
            ("a", Some("el")) => {
                out.push_str("al");
                index += 2;
            }
            ("A", Some("el")) => {
                out.push_str("Al");
                index += 2;
            }
            _ => {
                out.push_str(word);
                index += 1;
            }
        }
    }
    out
}

/// Values available to a template.
pub(crate) struct Fill<'a> {
    pub obj: Option<&'a ObjectDecl>,
    pub obj2: Option<&'a ObjectDecl>,
    pub list: Vec<&'a ObjectDecl>,
    pub score: i64,
    pub turns: u32,
    pub max: i64,
}

/// Substitutes `{key}` placeholders in one pass; unknown keys stay literal.
pub(crate) fn fill(template: &str, values: &Fill<'_>, language: &str) -> String {
    let mut out = String::with_capacity(template.len() + 32);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            out.push_str(&rest[open..]);
            rest = "";
            break;
        };
        let key = &after[..close];
        let definite_of = |decl: Option<&ObjectDecl>| {
            decl.map(|decl| definite(decl, language))
                .unwrap_or_default()
        };
        let value = match key {
            "obj" => Some(definite_of(values.obj)),
            "Obj" => Some(capitalize(&definite_of(values.obj))),
            "aobj" => Some(
                values
                    .obj
                    .map(|decl| indefinite(decl, language))
                    .unwrap_or_default(),
            ),
            "obj2" => Some(definite_of(values.obj2)),
            "Obj2" => Some(capitalize(&definite_of(values.obj2))),
            "o" => Some(ending(values.obj, language).to_owned()),
            "o2" => Some(ending(values.obj2, language).to_owned()),
            "list" => Some(join(
                &values
                    .list
                    .iter()
                    .map(|decl| indefinite(decl, language))
                    .collect::<Vec<_>>(),
                language,
            )),
            "score" => Some(values.score.to_string()),
            "turns" => Some(values.turns.to_string()),
            "max" => Some(values.max.to_string()),
            _ => None,
        };
        match value {
            Some(value) => out.push_str(&value),
            None => {
                out.push('{');
                out.push_str(key);
                out.push('}');
            }
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    contract(out, language)
}

/// Renders one transcript reply in every lexicon language of the pack.
/// Authored text is returned as authored (all its locales).
pub(crate) fn render(pack: &StoryPack, reply: &Reply) -> LocalizedText {
    let Some(world) = &pack.world else {
        return LocalizedText::default();
    };
    let object = |id: &Option<String>| id.as_deref().and_then(|id| pack.object(id));
    let message = |key: M, values: Fill<'_>| {
        let mut map = BTreeMap::new();
        for language in world.languages() {
            let text = world
                .messages
                .get(&key)
                .and_then(|text| text.resolve(language))
                .unwrap_or_else(|| template(key, language));
            map.insert(language.to_owned(), fill(text, &values, language));
        }
        LocalizedText(map)
    };
    let authored = |text: Option<&LocalizedText>| text.cloned().unwrap_or_default();
    match reply {
        Reply::Parser { language, text } => LocalizedText::single(language, text),
        Reply::Message {
            message: key,
            obj,
            obj2,
            list,
        } => message(
            *key,
            Fill {
                obj: object(obj),
                obj2: object(obj2),
                list: list.iter().filter_map(|id| pack.object(id)).collect(),
                score: 0,
                turns: 0,
                max: 0,
            },
        ),
        Reply::Intro => authored(world.intro.as_ref()),
        Reply::RoomName { node } => authored(pack.room(node).map(|room| &room.name)),
        Reply::RoomText { node } => authored(pack.node(node).map(|node| &node.text)),
        Reply::Describe { obj } => authored(pack.object(obj).map(|decl| &decl.description)),
        Reply::Read { obj } => authored(pack.object(obj).and_then(|decl| decl.text.as_ref())),
        Reply::Rule { rule } => authored(world.rule(rule).and_then(|rule| rule.text.as_ref())),
        Reply::Blocked { node, direction } => authored(pack.room(node).and_then(|room| {
            room.exits
                .iter()
                .find(|exit| exit.direction == *direction)
                .and_then(|exit| exit.blocked.as_ref())
        })),
        Reply::Score { score, turns } => {
            let (key, max) = match world.max_score {
                Some(max) => (M::ScoreOf, max),
                None => (M::Score, 0),
            };
            message(
                key,
                Fill {
                    obj: None,
                    obj2: None,
                    list: Vec::new(),
                    score: *score,
                    turns: *turns,
                    max,
                },
            )
        }
    }
}

/// Text of a library message in every lexicon language (for the view).
pub(crate) fn library_text(pack: &StoryPack, key: M) -> LocalizedText {
    render(
        pack,
        &Reply::Message {
            message: key,
            obj: None,
            obj2: None,
            list: Vec::new(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contractions_apply_only_to_whole_words() {
        assert_eq!(
            contract("Dentro de el cofre ves a el monje.".to_owned(), "es"),
            "Dentro del cofre ves al monje."
        );
        assert_eq!(contract("de ella".to_owned(), "es"), "de ella");
        assert_eq!(contract("de el".to_owned(), "en"), "de el");
    }

    #[test]
    fn lists_join_with_the_language_conjunction() {
        let names = ["a".to_owned(), "b".to_owned(), "c".to_owned()];
        assert_eq!(join(&names, "es"), "a, b y c");
        assert_eq!(join(&names[..2], "en"), "a and b");
        assert_eq!(join(&names[..1], "en"), "a");
    }
}
