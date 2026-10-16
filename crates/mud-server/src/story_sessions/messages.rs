//! Server text of story sessions in the session's story locale (ADR 0029).
//!
//! The story pack localizes its own prose; this module localizes what the
//! server itself says around it: confirmations, help, failures, slot and
//! lobby listings and the Telnet rendering labels. English sessions (`en`,
//! `en-*`) get English; every other locale keeps Spanish, the server's
//! default language. Text never names authored ids.

use tiamud_protocol::{StoryDecisionMode, StoryEndingKind, StoryLeftReason, StorySeatRole};

/// Language of the server's own story text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Lang {
    Es,
    En,
}

impl Lang {
    /// `en` and `en-*` (any case) are English; everything else is Spanish.
    pub fn of(locale: &str) -> Self {
        let primary = locale.split('-').next().unwrap_or_default();
        if primary.eq_ignore_ascii_case("en") {
            Self::En
        } else {
            Self::Es
        }
    }

    pub const fn pick(self, es: &'static str, en: &'static str) -> &'static str {
        match self {
            Self::Es => es,
            Self::En => en,
        }
    }

    pub const fn is_english(self) -> bool {
        matches!(self, Self::En)
    }
}

/// Help of a parser story.
pub fn parser_help(locale: &str) -> &'static str {
    Lang::of(locale).pick(
        super::PARSER_STORY_HELP,
        "Parser story: type commands such as \"look\", \"inventory\", \"take the key\", \"n\" or \"open the door with the key\". Also \"undo\" (deshacer), \"save N\" (guardar), \"load N\" (cargar), \"saves\" (partidas) or \"leave\" (salir).",
    )
}

/// Help of a solo gamebook or verb-menu story.
pub fn input_help(locale: &str) -> &'static str {
    Lang::of(locale).pick(
        super::STORY_INPUT_HELP,
        "You are absorbed in a story: type a number to choose, \"look\" (ver), \"undo\" (deshacer), \"save N\" (guardar), \"load N\" (cargar), \"saves\" (partidas), \"delete N\" (borrar), \"share\" (compartir) or \"leave\" (salir).",
    )
}

/// Help of a shared instance.
pub fn shared_help(locale: &str) -> &'static str {
    Lang::of(locale).pick(
        super::SHARED_STORY_HELP,
        "Shared story: a number or \"vote N\" (votar N) chooses or votes, \"vote 0\" withdraws your vote, \"seats\" (asientos) shows the room; the host may \"invite <name>\" (invitar), \"kick <name>\" (expulsar), \"mode host|vote|turns [seconds]\" (modo anfitrion|votacion|turnos), \"undo\" (deshacer) and save or load games. \"leave\" (salir) gives up your seat.",
    )
}

pub fn entered(locale: &str, title: &str) -> String {
    match Lang::of(locale) {
        Lang::Es => format!("Entras en «{title}»."),
        Lang::En => format!("You enter «{title}»."),
    }
}

pub fn resumed(locale: &str, title: &str) -> String {
    match Lang::of(locale) {
        Lang::Es => format!("Continúas la historia «{title}»."),
        Lang::En => format!("You resume the story «{title}»."),
    }
}

pub fn hosting(locale: &str, title: &str) -> String {
    match Lang::of(locale) {
        Lang::Es => format!("Compartes «{title}»."),
        Lang::En => format!("You share «{title}»."),
    }
}

pub fn joined(locale: &str, title: &str, spectate: bool) -> String {
    match (Lang::of(locale), spectate) {
        (Lang::Es, true) => format!("Miras «{title}» como espectador."),
        (Lang::Es, false) => format!("Te unes a «{title}»."),
        (Lang::En, true) => format!("You watch «{title}» as a spectator."),
        (Lang::En, false) => format!("You join «{title}»."),
    }
}

pub fn chose(locale: &str, label: &str, voting: bool) -> String {
    match (Lang::of(locale), voting) {
        (Lang::Es, true) => format!("Votas por: {label}"),
        (Lang::Es, false) => format!("Eliges: {label}"),
        (Lang::En, true) => format!("You vote for: {label}"),
        (Lang::En, false) => format!("You choose: {label}"),
    }
}

pub fn voted(locale: &str, withdrawn: bool) -> &'static str {
    let lang = Lang::of(locale);
    if withdrawn {
        lang.pick("Retiras tu voto.", "You withdraw your vote.")
    } else {
        lang.pick("Votas.", "You vote.")
    }
}

pub fn undone(locale: &str) -> &'static str {
    Lang::of(locale).pick("Deshaces la última elección.", "You undo the last choice.")
}

pub fn leaving(locale: &str) -> &'static str {
    Lang::of(locale).pick("Sales de la historia.", "You leave the story.")
}

pub fn looked(locale: &str) -> &'static str {
    Lang::of(locale).pick("Vuelves a mirar.", "You look again.")
}

pub fn left_world(locale: &str, autosaved: bool) -> &'static str {
    let lang = Lang::of(locale);
    if autosaved {
        lang.pick(
            "Sales de la historia y vuelves al mundo. Tu progreso quedó en el autoguardado.",
            "You leave the story and return to the world. Your progress is in the autosave.",
        )
    } else {
        lang.pick(
            "Sales de la historia y vuelves al mundo.",
            "You leave the story and return to the world.",
        )
    }
}

/// Why a participant was removed from a story, as a sentence.
pub fn removed(locale: &str, reason: StoryLeftReason) -> &'static str {
    let lang = Lang::of(locale);
    match reason {
        StoryLeftReason::Kicked => lang.pick(
            "El anfitrión te ha retirado de la historia compartida. Vuelves al mundo.",
            "The host removed you from the shared story. You return to the world.",
        ),
        StoryLeftReason::Ended => lang.pick(
            "La historia compartida ha terminado: no queda nadie sentado. Vuelves al mundo.",
            "The shared story ended: no seated player remained. You return to the world.",
        ),
        StoryLeftReason::Left => left_world(locale, false),
        StoryLeftReason::Evicted => super::StoryFailure::Gone.message_in(locale),
    }
}

pub fn invited(locale: &str, name: &str) -> String {
    match Lang::of(locale) {
        Lang::Es => format!("Invitación enviada a {name}, si está conectado."),
        Lang::En => format!("Invitation sent to {name}, if they are connected."),
    }
}

pub fn kicked(locale: &str, name: &str) -> String {
    match Lang::of(locale) {
        Lang::Es => format!("{name} ya no participa en la historia."),
        Lang::En => format!("{name} no longer takes part in the story."),
    }
}

pub fn mode_set(locale: &str, mode: StoryDecisionMode) -> String {
    match Lang::of(locale) {
        Lang::Es => format!("Modo de decisión: {}.", mode_label(mode, locale)),
        Lang::En => format!("Decision mode: {}.", mode_label(mode, locale)),
    }
}

pub fn saved(locale: &str, slot: u8) -> String {
    match Lang::of(locale) {
        Lang::Es => format!("Partida guardada en la ranura {slot}."),
        Lang::En => format!("Game saved to slot {slot}."),
    }
}

pub fn deleted(locale: &str, slot: u8) -> String {
    match Lang::of(locale) {
        Lang::Es => format!("Partida de la ranura {slot} borrada."),
        Lang::En => format!("The game in slot {slot} was deleted."),
    }
}

pub fn restored(locale: &str, slot: u8) -> String {
    match (Lang::of(locale), slot) {
        (Lang::Es, 0) => "Continúas la partida autoguardada.".to_owned(),
        (Lang::Es, slot) => format!("Cargas la partida de la ranura {slot}."),
        (Lang::En, 0) => "You continue the autosaved game.".to_owned(),
        (Lang::En, slot) => format!("You load the game in slot {slot}."),
    }
}

/// Telnet line of the seat whose turn it is (round-robin mode).
pub fn turn_of(locale: &str, name: &str, seconds: Option<u16>) -> String {
    let lang = Lang::of(locale);
    let mut line = match lang {
        Lang::Es => format!("Turno de {name}"),
        Lang::En => format!("{name}'s turn"),
    };
    if let Some(seconds) = seconds {
        line.push_str(&match lang {
            Lang::Es => format!(" · cede el turno en {seconds} s"),
            Lang::En => format!(" · the turn passes in {seconds} s"),
        });
    }
    line
}

/// Telnet hint under a shared view: who decides and how.
pub fn shared_hint(
    locale: &str,
    mode: StoryDecisionMode,
    role: Option<StorySeatRole>,
) -> &'static str {
    let lang = Lang::of(locale);
    match (mode, role) {
        (_, Some(StorySeatRole::Spectator)) => {
            lang.pick("Miras como espectador.", "You are watching as a spectator.")
        }
        (StoryDecisionMode::Vote, _) => lang.pick(
            "Historia compartida: tu número cuenta como voto.",
            "Shared story: your number counts as a vote.",
        ),
        (StoryDecisionMode::Host, Some(StorySeatRole::Host)) => {
            lang.pick("Decides tú (anfitrión).", "You decide (host).")
        }
        (StoryDecisionMode::Host, _) => lang.pick(
            "Decide el anfitrión; «votar N» (vote N) deja un voto orientativo.",
            "The host decides; \"vote N\" (votar N) leaves an advisory vote.",
        ),
        (StoryDecisionMode::Turns, _) => lang.pick(
            "Turnos rotatorios: decide quien tenga el turno.",
            "Round-robin turns: whoever holds the turn decides.",
        ),
    }
}

pub fn mode_label(mode: StoryDecisionMode, locale: &str) -> &'static str {
    let lang = Lang::of(locale);
    match mode {
        StoryDecisionMode::Host => lang.pick("decide el anfitrión", "the host decides"),
        StoryDecisionMode::Vote => lang.pick("votación por mayoría", "majority vote"),
        StoryDecisionMode::Turns => lang.pick("turnos rotatorios", "round-robin turns"),
    }
}

pub fn ending_label(kind: StoryEndingKind, locale: &str) -> &'static str {
    let lang = Lang::of(locale);
    match kind {
        StoryEndingKind::Victory => lang.pick("victoria", "victory"),
        StoryEndingKind::Defeat => lang.pick("derrota", "defeat"),
        StoryEndingKind::Neutral => lang.pick("final", "ending"),
    }
}

pub fn role_label(role: StorySeatRole, locale: &str) -> &'static str {
    let lang = Lang::of(locale);
    match role {
        StorySeatRole::Host => lang.pick("anfitrión", "host"),
        StorySeatRole::Player => lang.pick("jugador", "player"),
        StorySeatRole::Spectator => lang.pick("espectador", "spectator"),
    }
}

/// Server notes shown after the parser replies of a meta command.
pub fn restart_confirm_note(locale: &str) -> &'static str {
    Lang::of(locale).pick(
        "¿Seguro que quieres reiniciar la historia desde el principio? Escribe «reiniciar» otra vez para confirmarlo; cualquier otra orden lo cancela. Podrás «deshacer» el reinicio.",
        "Do you really want to restart the story from the beginning? Type \"restart\" again to confirm; any other command cancels it. You can \"undo\" the restart.",
    )
}

pub fn restarted_note(locale: &str) -> &'static str {
    Lang::of(locale).pick("La historia vuelve a empezar.", "The story starts again.")
}

pub fn save_note(locale: &str) -> &'static str {
    Lang::of(locale).pick(
        "Para guardar escribe «guardar N» (ranuras 1 a 9) o usa el panel de partidas guardadas.",
        "To save, type \"save N\" (slots 1 to 9) or use the saved games panel.",
    )
}

pub fn restore_note(locale: &str) -> &'static str {
    Lang::of(locale).pick(
        "Para cargar escribe «cargar N» (la ranura 0 es el autoguardado) o usa el panel de partidas guardadas.",
        "To load, type \"load N\" (slot 0 is the autosave) or use the saved games panel.",
    )
}

pub fn quit_note(locale: &str) -> &'static str {
    leaving(locale)
}

/// A rendered boolean stat.
pub fn yes_no(value: bool, locale: &str) -> &'static str {
    let lang = Lang::of(locale);
    if value {
        lang.pick("sí", "yes")
    } else {
        lang.pick("no", "no")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_is_only_chosen_for_english_locales() {
        assert_eq!(Lang::of("en"), Lang::En);
        assert_eq!(Lang::of("EN-gb"), Lang::En);
        assert_eq!(Lang::of("es"), Lang::Es);
        assert_eq!(Lang::of("eng"), Lang::Es);
        assert_eq!(Lang::of("fr"), Lang::Es);
        assert_eq!(Lang::of(""), Lang::Es);
    }

    #[test]
    fn spanish_help_is_the_documented_default() {
        assert_eq!(input_help("es"), super::super::STORY_INPUT_HELP);
        assert_eq!(parser_help("fr"), super::super::PARSER_STORY_HELP);
        assert_eq!(shared_help("es-MX"), super::super::SHARED_STORY_HELP);
        assert!(input_help("en").starts_with("You are absorbed"));
        assert!(parser_help("en-US").contains("take the key"));
        assert!(shared_help("en").contains("vote N"));
    }

    #[test]
    fn confirmations_follow_the_locale() {
        assert_eq!(entered("es", "Cripta"), "Entras en «Cripta».");
        assert_eq!(entered("en", "Crypt"), "You enter «Crypt».");
        assert_eq!(restored("en", 0), "You continue the autosaved game.");
        assert_eq!(restored("es", 3), "Cargas la partida de la ranura 3.");
        assert_eq!(saved("en", 2), "Game saved to slot 2.");
        assert_eq!(
            mode_set("en", StoryDecisionMode::Vote),
            "Decision mode: majority vote."
        );
        assert_eq!(
            turn_of("en", "Aria", Some(30)),
            "Aria's turn · the turn passes in 30 s"
        );
        assert_eq!(ending_label(StoryEndingKind::Defeat, "es"), "derrota");
        assert_eq!(yes_no(true, "en"), "yes");
    }
}
