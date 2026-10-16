//! Telnet and GMCP coverage of shared story instances (ADR 0033).

use super::super::{DO, OPT_GMCP, OutputFrame, TelnetEvent, TelnetParser};
use super::*;
use crate::story_sessions::{StoryCatalog, StoryLimits};
use std::{collections::BTreeMap, time::Duration};
use tiamud_story_core::StoryPack;

const CRIPTA: &str = "crypt-of-the-necromancer";

fn stories() -> StoryManager {
    let bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../story-core/fixtures/crypt-of-the-necromancer.story.json"),
    )
    .unwrap();
    StoryManager::new(
        StoryCatalog::from_packs(
            vec![StoryPack::from_json_slice(&bytes).unwrap()],
            BTreeMap::new(),
        )
        .unwrap(),
        StoryLimits::default(),
    )
}

fn gmcp_input() -> TelnetInput {
    let mut input = TelnetInput::default();
    input.negotiation.begin_initial_negotiation();
    input.negotiation.handle(vec![TelnetEvent::Negotiation {
        command: DO,
        option: OPT_GMCP,
    }]);
    input.negotiation.handle(vec![TelnetEvent::Subnegotiation {
        option: OPT_GMCP,
        data: b"Core.Supports.Set [\"Tiamud.Story 1\"]".to_vec(),
    }]);
    input
}

fn drain(output: &TelnetOutputQueue) -> (String, Vec<String>) {
    let mut state = output.shared.state.lock().unwrap();
    let mut bytes = Vec::new();
    for frame in state.frames.drain(..) {
        if let OutputFrame::Bytes { bytes: frame, .. } = frame {
            bytes.extend(frame);
        }
    }
    state.byte_len = 0;
    let mut parser = TelnetParser::default();
    let text = parser.feed(&bytes);
    let gmcp = parser
        .take_events()
        .into_iter()
        .filter_map(|event| match event {
            TelnetEvent::Subnegotiation {
                option: OPT_GMCP,
                data,
            } => Some(String::from_utf8(data).unwrap()),
            _ => None,
        })
        .collect();
    (String::from_utf8(text).unwrap(), gmcp)
}

struct Client {
    story: TelnetStory,
    output: TelnetOutputQueue,
    input: TelnetInput,
}

impl Client {
    fn new(manager: &StoryManager, account: AccountId, name: &str) -> Self {
        Self {
            story: TelnetStory::new(Some(manager.clone()), account).with_name(name),
            output: TelnetOutputQueue::new(256, 1024 * 1024),
            input: gmcp_input(),
        }
    }

    async fn line(&mut self, line: &str) -> StoryLineOutcome {
        self.story
            .line(&self.output, &mut self.input, line)
            .await
            .unwrap()
    }

    /// Handles every ready story event; returns the last outcome.
    async fn pump(&mut self) -> Option<StoryLineOutcome> {
        let mut last = None;
        while let Ok(event) =
            tokio::time::timeout(Duration::from_millis(1), self.story.next_event()).await
        {
            last = Some(
                self.story
                    .handle_event(&self.output, &mut self.input, event)
                    .unwrap(),
            );
        }
        last
    }

    fn drain(&self) -> (String, Vec<String>) {
        drain(&self.output)
    }
}

#[tokio::test(start_paused = true)]
async fn telnet_players_share_vote_by_number_and_see_text_tallies() {
    let manager = stories();
    let mut aria = Client::new(&manager, AccountId(1), "Aria");
    let mut bruno = Client::new(&manager, AccountId(2), "Bruno");

    assert_eq!(
        aria.line(&format!("historia compartir {CRIPTA} es")).await,
        StoryLineOutcome::Handled
    );
    let (text, gmcp) = aria.drain();
    assert!(text.contains("Compartes «"), "{text}");
    assert!(text.contains("Historia compartida · votación por mayoría · plazo 60 s"));
    let code = text
        .split("Código para unirse: ")
        .nth(1)
        .unwrap()
        .chars()
        .take(8)
        .collect::<String>();
    assert!(
        gmcp.iter()
            .any(|package| package.starts_with("Tiamud.Story.Lobby ")
                && package.contains(&format!("\"code\":\"{code}\"")))
    );
    assert!(
        gmcp.iter()
            .any(|package| package.starts_with("Tiamud.Story.View "))
    );

    assert!(aria.line("invitar Bruno").await == StoryLineOutcome::Handled);
    assert!(aria.drain().0.contains("Invitación enviada a Bruno"));
    assert_eq!(bruno.pump().await, Some(StoryLineOutcome::Handled));
    let (text, gmcp) = bruno.drain();
    assert!(text.contains(&format!("historia unirse {code}")), "{text}");
    assert!(
        gmcp.iter()
            .any(|package| package.starts_with("Tiamud.Story.Invitation "))
    );

    assert_eq!(
        bruno.line(&format!("story join {code}")).await,
        StoryLineOutcome::Handled
    );
    let (text, _) = bruno.drain();
    assert!(text.contains("Te unes a «"));
    assert!(text.contains("2. Bruno"));
    assert!(text.contains("tu número cuenta como voto"));
    assert!(!text.contains("Código para unirse"));
    aria.pump().await;
    aria.drain();

    // Aria votes by number: a text tally reaches both players.
    assert_eq!(aria.line("1").await, StoryLineOutcome::Handled);
    aria.pump().await;
    let (text, gmcp) = aria.drain();
    assert!(
        text.contains("Votos (1/2): 1) 1 · cierra en 60 s · tu voto: 1"),
        "{text}"
    );
    let tally = gmcp
        .iter()
        .find(|package| package.starts_with("Tiamud.Story.Tally "))
        .unwrap();
    assert!(
        tally.contains("\"votes\":[{\"number\":1,\"votes\":1}]"),
        "{tally}"
    );
    assert!(tally.contains("\"yourVote\":1"));
    assert!(!tally.contains("\"c"), "no opaque ids in GMCP: {tally}");
    bruno.pump().await;
    let (text, _) = bruno.drain();
    assert!(
        text.contains("Votos (1/2): 1) 1 · cierra en 60 s"),
        "{text}"
    );
    assert!(!text.contains("tu voto"));

    // Bruno votes with `votar 1`: the decision renders once for each.
    assert_eq!(bruno.line("votar 1").await, StoryLineOutcome::Handled);
    bruno.pump().await;
    let (bruno_text, _) = bruno.drain();
    assert_eq!(bruno_text.matches("[Turno 1").count(), 1, "{bruno_text}");
    aria.pump().await;
    let (aria_text, _) = aria.drain();
    assert_eq!(aria_text.matches("[Turno 1").count(), 1, "{aria_text}");

    // Seats, host-only commands and malformed input.
    assert_eq!(bruno.line("asientos").await, StoryLineOutcome::Handled);
    let (text, gmcp) = bruno.drain();
    assert!(text.contains("Asientos (2/8):") && text.contains("Tú: jugador."));
    assert!(
        gmcp.iter()
            .any(|package| package.starts_with("Tiamud.Story.Lobby "))
    );
    bruno.line("deshacer").await;
    assert!(bruno.drain().0.contains("Solo el anfitrión"));
    bruno.line("modo turnos").await;
    assert!(bruno.drain().0.contains("Solo el anfitrión"));
    aria.line("modo caos").await;
    assert!(aria.drain().0.contains("Valor no válido"));
    bruno.line("votar").await;
    assert!(bruno.drain().0.contains("Historia compartida:"));
    bruno.line("xyzzy").await;
    assert!(bruno.drain().0.contains("«votar N» (vote N)"));

    // The host switches to turns; the turn line names the holder.
    aria.line("modo turnos").await;
    assert!(
        aria.drain()
            .0
            .contains("Modo de decisión: turnos rotatorios.")
    );
    bruno.pump().await;
    assert!(bruno.drain().0.contains("Turno de Aria"));

    // Kicked: Bruno's session returns to the world.
    aria.line("expulsar bruno").await;
    assert!(aria.drain().0.contains("bruno ya no participa"));
    assert_eq!(bruno.pump().await, Some(StoryLineOutcome::Left));
    let (text, gmcp) = bruno.drain();
    assert!(text.contains("El anfitrión te ha retirado"));
    assert!(
        gmcp.iter()
            .any(|package| package == "Tiamud.Story.Left {\"reason\":\"kicked\"}")
    );
    assert!(!bruno.story.active());
    assert_eq!(
        bruno.line(&format!("historia mirar {code}")).await,
        StoryLineOutcome::Handled
    );
    assert!(bruno.drain().0.contains("Ese código no corresponde"));
}

#[tokio::test(start_paused = true)]
async fn a_solo_telnet_story_can_be_shared_and_ends_for_spectators() {
    let manager = stories();
    let mut aria = Client::new(&manager, AccountId(1), "Aria");
    let mut carla = Client::new(&manager, AccountId(3), "Carla");
    aria.line(&format!("historia {CRIPTA} es")).await;
    aria.drain();
    aria.line("asientos").await;
    assert!(aria.drain().0.contains("no está compartida"));
    assert_eq!(aria.line("compartir").await, StoryLineOutcome::Handled);
    let (text, _) = aria.drain();
    let code = text
        .split("Código para unirse: ")
        .nth(1)
        .unwrap()
        .chars()
        .take(8)
        .collect::<String>();
    carla.line(&format!("historia mirar {code}")).await;
    let (text, _) = carla.drain();
    assert!(text.contains("Miras «") && text.contains("Miras como espectador."));
    carla.line("1").await;
    assert!(
        carla
            .drain()
            .0
            .contains("Los espectadores solo pueden mirar.")
    );
    assert_eq!(aria.line("salir").await, StoryLineOutcome::Left);
    assert_eq!(carla.pump().await, Some(StoryLineOutcome::Left));
    assert!(
        carla
            .drain()
            .0
            .contains("La historia compartida ha terminado")
    );
}
