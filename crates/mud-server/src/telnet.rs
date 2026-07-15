use crate::world_actor::WorldHandle;
use mud_core::{SceneView, parse_action};
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tracing::{debug, info, warn};

const IAC: u8 = 255;
const DONT: u8 = 254;
const DO: u8 = 253;
const WONT: u8 = 252;
const WILL: u8 = 251;
const SB: u8 = 250;
const SE: u8 = 240;
const EOR_COMMAND: u8 = 239;
const OPT_SUPPRESS_GO_AHEAD: u8 = 3;
const OPT_EOR: u8 = 25;
const OPT_TTYPE: u8 = 24;
const OPT_NAWS: u8 = 31;
const OPT_CHARSET: u8 = 42;
const OPT_GMCP: u8 = 201;

const TTYPE_IS: u8 = 0;
const TTYPE_SEND: u8 = 1;
const CHARSET_REQUEST: u8 = 1;
const CHARSET_ACCEPTED: u8 = 2;
const CHARSET_REJECTED: u8 = 3;

const MAX_COMMAND_BYTES: usize = 4096;
const MAX_SUBNEGOTIATION_BYTES: usize = 16 * 1024;

const INITIAL_NEGOTIATION: &[u8] = &[
    IAC,
    WILL,
    OPT_SUPPRESS_GO_AHEAD,
    IAC,
    WILL,
    OPT_EOR,
    IAC,
    DO,
    OPT_TTYPE,
    IAC,
    DO,
    OPT_NAWS,
    IAC,
    DO,
    OPT_CHARSET,
];

pub async fn run(
    listener: TcpListener,
    world: WorldHandle,
    guest_counter: Arc<AtomicU64>,
) -> io::Result<()> {
    loop {
        let (stream, address) = listener.accept().await?;
        let world = world.clone();
        let guest_counter = guest_counter.clone();
        tokio::spawn(async move {
            if let Err(error) = serve(stream, world, guest_counter).await {
                warn!(%address, %error, "telnet session ended with an error");
            }
        });
    }
}

async fn serve(
    mut stream: TcpStream,
    world: WorldHandle,
    guest_counter: Arc<AtomicU64>,
) -> io::Result<()> {
    stream.write_all(INITIAL_NEGOTIATION).await?;
    stream.write_all(b"\r\nProject Envy prototype\r\n").await?;

    let guest = guest_counter.fetch_add(1, Ordering::Relaxed);
    let entered = world
        .enter(format!("TelnetGuest-{guest}"))
        .await
        .map_err(io::Error::other)?;
    let session_id = entered.session_id;
    let result = serve_entered(&mut stream, &world, entered).await;

    info!(session = session_id.0, "telnet client disconnected");
    world.leave(session_id).await;
    result
}

async fn serve_entered(
    stream: &mut TcpStream,
    world: &WorldHandle,
    entered: crate::world_actor::EnteredWorld,
) -> io::Result<()> {
    write_scene(stream, &entered.scene).await?;
    write_prompt(stream).await?;

    let mut parser = TelnetParser::default();
    let mut negotiation = TelnetSession::default();
    let mut read_buffer = [0_u8; 4096];
    let mut command_buffer = CommandBuffer::default();
    loop {
        let count = stream.read(&mut read_buffer).await?;
        if count == 0 {
            break;
        }

        let data = parser.feed(&read_buffer[..count]);
        let response = negotiation.handle(parser.take_events());
        if !response.is_empty() {
            stream.write_all(&response).await?;
        }

        for byte in data {
            match command_buffer.push(byte) {
                None => {}
                Some(LineEvent::TooLong) => {
                    stream.write_all(b"Command too long.\r\n").await?;
                    write_prompt(stream).await?;
                }
                Some(LineEvent::Command(line)) => {
                    let command = String::from_utf8_lossy(&line).trim().to_owned();
                    if command.eq_ignore_ascii_case("quit") {
                        stream.write_all(b"Goodbye.\r\n").await?;
                        return Ok(());
                    }
                    match parse_action(&command) {
                        Ok(action) => match world.apply(entered.session_id, action).await {
                            Ok(outcome) => write_scene(stream, &outcome.scene).await?,
                            Err(error) => {
                                stream.write_all(format!("{error}\r\n").as_bytes()).await?;
                            }
                        },
                        Err(error) => {
                            stream.write_all(format!("{error}\r\n").as_bytes()).await?;
                        }
                    }
                    write_prompt(stream).await?;
                }
            }
        }
    }

    Ok(())
}

async fn write_scene(stream: &mut TcpStream, scene: &SceneView) -> io::Result<()> {
    stream.write_all(scene.as_plain_text().as_bytes()).await
}

async fn write_prompt(stream: &mut TcpStream) -> io::Result<()> {
    stream.write_all(b"> ").await?;
    stream.write_all(&[IAC, EOR_COMMAND]).await
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum ParserState {
    #[default]
    Data,
    Iac,
    Negotiation(u8),
    SubnegotiationOption,
    Subnegotiation,
    SubnegotiationIac,
}

#[derive(Default)]
struct TelnetParser {
    state: ParserState,
    events: Vec<TelnetEvent>,
    subnegotiation_option: u8,
    subnegotiation_data: Vec<u8>,
    subnegotiation_overflowed: bool,
}

impl TelnetParser {
    fn feed(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut data = Vec::with_capacity(bytes.len());
        for &byte in bytes {
            self.state = match self.state {
                ParserState::Data if byte == IAC => ParserState::Iac,
                ParserState::Data => {
                    data.push(byte);
                    ParserState::Data
                }
                ParserState::Iac if byte == IAC => {
                    data.push(IAC);
                    ParserState::Data
                }
                ParserState::Iac if matches!(byte, WILL | WONT | DO | DONT) => {
                    ParserState::Negotiation(byte)
                }
                ParserState::Iac if byte == SB => ParserState::SubnegotiationOption,
                ParserState::Iac => ParserState::Data,
                ParserState::Negotiation(command) => {
                    self.events.push(TelnetEvent::Negotiation {
                        command,
                        option: byte,
                    });
                    ParserState::Data
                }
                ParserState::SubnegotiationOption if byte == IAC => {
                    // An option byte is mandatory. Discard a malformed empty SB frame,
                    // but remain in Telnet command mode for the byte that follows IAC.
                    ParserState::Iac
                }
                ParserState::SubnegotiationOption => {
                    self.begin_subnegotiation(byte);
                    ParserState::Subnegotiation
                }
                ParserState::Subnegotiation if byte == IAC => ParserState::SubnegotiationIac,
                ParserState::Subnegotiation => {
                    self.push_subnegotiation_byte(byte);
                    ParserState::Subnegotiation
                }
                ParserState::SubnegotiationIac if byte == SE => {
                    self.finish_subnegotiation();
                    ParserState::Data
                }
                ParserState::SubnegotiationIac if byte == IAC => {
                    self.push_subnegotiation_byte(IAC);
                    ParserState::Subnegotiation
                }
                ParserState::SubnegotiationIac => ParserState::Subnegotiation,
            };
        }
        data
    }

    fn take_events(&mut self) -> Vec<TelnetEvent> {
        std::mem::take(&mut self.events)
    }

    fn begin_subnegotiation(&mut self, option: u8) {
        self.subnegotiation_option = option;
        self.subnegotiation_data.clear();
        self.subnegotiation_overflowed = false;
    }

    fn push_subnegotiation_byte(&mut self, byte: u8) {
        if self.subnegotiation_overflowed {
            return;
        }
        if self.subnegotiation_data.len() == MAX_SUBNEGOTIATION_BYTES {
            self.subnegotiation_data.clear();
            self.subnegotiation_overflowed = true;
            return;
        }
        self.subnegotiation_data.push(byte);
    }

    fn finish_subnegotiation(&mut self) {
        let option = self.subnegotiation_option;
        let event = if self.subnegotiation_overflowed {
            TelnetEvent::SubnegotiationTooLong { option }
        } else {
            TelnetEvent::Subnegotiation {
                option,
                data: std::mem::take(&mut self.subnegotiation_data),
            }
        };
        self.events.push(event);
        self.subnegotiation_data.clear();
        self.subnegotiation_overflowed = false;
    }
}

#[derive(Debug, Eq, PartialEq)]
enum TelnetEvent {
    Negotiation { command: u8, option: u8 },
    Subnegotiation { option: u8, data: Vec<u8> },
    SubnegotiationTooLong { option: u8 },
}

#[derive(Debug, Default, Eq, PartialEq)]
struct ClientCapabilities {
    terminal_type: Option<String>,
    columns: Option<u16>,
    rows: Option<u16>,
    charset: Option<String>,
}

#[derive(Default)]
struct TelnetSession {
    capabilities: ClientCapabilities,
    ttype_requested: bool,
    charset_requested: bool,
}

impl TelnetSession {
    fn handle(&mut self, events: Vec<TelnetEvent>) -> Vec<u8> {
        let mut response = Vec::new();
        for event in events {
            match event {
                TelnetEvent::Negotiation { command, option } => {
                    self.handle_negotiation(command, option, &mut response);
                }
                TelnetEvent::Subnegotiation { option, data } => {
                    self.handle_subnegotiation(option, &data);
                }
                TelnetEvent::SubnegotiationTooLong { option } => {
                    warn!(option, "discarded oversized Telnet subnegotiation");
                }
            }
        }
        response
    }

    fn handle_negotiation(&mut self, command: u8, option: u8, response: &mut Vec<u8>) {
        match (command, option) {
            (WILL, OPT_TTYPE) if !self.ttype_requested => {
                self.ttype_requested = true;
                response.extend_from_slice(&[IAC, SB, OPT_TTYPE, TTYPE_SEND, IAC, SE]);
            }
            (WILL, OPT_CHARSET) if !self.charset_requested => {
                self.charset_requested = true;
                response.extend_from_slice(&[
                    IAC,
                    SB,
                    OPT_CHARSET,
                    CHARSET_REQUEST,
                    b' ',
                    b'U',
                    b'T',
                    b'F',
                    b'-',
                    b'8',
                    IAC,
                    SE,
                ]);
            }
            (WILL, OPT_NAWS | OPT_TTYPE | OPT_CHARSET) => {}
            (WONT, OPT_TTYPE) => {
                self.ttype_requested = false;
                self.capabilities.terminal_type = None;
            }
            (WONT, OPT_CHARSET) => {
                self.charset_requested = false;
                self.capabilities.charset = None;
            }
            (WONT, OPT_NAWS) => {
                self.capabilities.columns = None;
                self.capabilities.rows = None;
            }
            (DO, OPT_SUPPRESS_GO_AHEAD | OPT_EOR) => {}
            // Do not claim GMCP support until the server can process packages. Its
            // frames are nevertheless safely bounded and parsed by this state machine.
            (DO, _) => response.extend_from_slice(&[IAC, WONT, option]),
            (WILL, _) => response.extend_from_slice(&[IAC, DONT, option]),
            (DONT | WONT, _) => {}
            _ => {}
        }
    }

    fn handle_subnegotiation(&mut self, option: u8, data: &[u8]) {
        match option {
            OPT_TTYPE if data.first() == Some(&TTYPE_IS) => {
                self.capabilities.terminal_type = sanitize_token(&data[1..], 64);
            }
            OPT_NAWS if data.len() == 4 => {
                let columns = u16::from_be_bytes([data[0], data[1]]);
                let rows = u16::from_be_bytes([data[2], data[3]]);
                if columns > 0 && rows > 0 {
                    self.capabilities.columns = Some(columns);
                    self.capabilities.rows = Some(rows);
                }
            }
            OPT_CHARSET if data.first() == Some(&CHARSET_ACCEPTED) => {
                self.capabilities.charset = sanitize_token(&data[1..], 32);
            }
            OPT_CHARSET if data.first() == Some(&CHARSET_REJECTED) => {
                self.capabilities.charset = None;
            }
            // Keep GMCP framed away from player input. Package dispatch will be added
            // only when the option is intentionally enabled by the server.
            OPT_GMCP => {}
            _ => {}
        }
        debug!(?self.capabilities, "updated Telnet client capabilities");
    }
}

fn sanitize_token(bytes: &[u8], max_len: usize) -> Option<String> {
    let value: String = bytes
        .iter()
        .copied()
        .take(max_len)
        .filter(|byte| byte.is_ascii_graphic() || *byte == b' ')
        .map(char::from)
        .collect();
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

#[derive(Default)]
struct CommandBuffer {
    bytes: Vec<u8>,
    discarding: bool,
}

impl CommandBuffer {
    fn push(&mut self, byte: u8) -> Option<LineEvent> {
        if self.discarding {
            if byte == b'\n' {
                self.discarding = false;
                return Some(LineEvent::TooLong);
            }
            return None;
        }

        if byte == b'\n' {
            if self.bytes.last() == Some(&b'\r') {
                self.bytes.pop();
            }
            return Some(LineEvent::Command(std::mem::take(&mut self.bytes)));
        }

        if self.bytes.len() == MAX_COMMAND_BYTES {
            self.bytes.clear();
            self.discarding = true;
        } else {
            self.bytes.push(byte);
        }
        None
    }
}

#[derive(Debug, Eq, PartialEq)]
enum LineEvent {
    Command(Vec<u8>),
    TooLong,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separates_commands_from_telnet_negotiation() {
        let mut parser = TelnetParser::default();
        let data = parser.feed(&[IAC, DO, OPT_EOR, b'l', b'o', b'o', b'k', b'\n']);
        assert_eq!(data, b"look\n");
        assert_eq!(
            parser.take_events(),
            vec![TelnetEvent::Negotiation {
                command: DO,
                option: OPT_EOR
            }]
        );
    }

    #[test]
    fn parses_fragmented_negotiation_without_leaking_it_into_commands() {
        let mut parser = TelnetParser::default();
        assert!(parser.feed(&[IAC]).is_empty());
        assert!(parser.feed(&[WILL]).is_empty());
        assert_eq!(parser.feed(&[OPT_NAWS, b'l']), b"l");
        assert_eq!(
            parser.take_events(),
            vec![TelnetEvent::Negotiation {
                command: WILL,
                option: OPT_NAWS
            }]
        );
    }

    #[test]
    fn decodes_fragmented_escaped_iac_in_application_data() {
        let mut parser = TelnetParser::default();
        assert_eq!(parser.feed(&[b'a', IAC]), b"a");
        assert_eq!(parser.feed(&[IAC, b'b']), &[IAC, b'b']);
        assert!(parser.take_events().is_empty());
    }

    #[test]
    fn frames_subnegotiation_and_unescapes_iac() {
        let mut parser = TelnetParser::default();
        assert!(
            parser
                .feed(&[IAC, SB, OPT_TTYPE, TTYPE_IS, b'x', IAC])
                .is_empty()
        );
        let data = parser.feed(&[IAC, b'y', IAC, SE, b'n', b'\n']);
        assert_eq!(data, b"n\n");
        assert_eq!(
            parser.take_events(),
            vec![TelnetEvent::Subnegotiation {
                option: OPT_TTYPE,
                data: vec![TTYPE_IS, b'x', IAC, b'y']
            }]
        );
    }

    #[test]
    fn a_bare_se_byte_does_not_end_subnegotiation() {
        let mut parser = TelnetParser::default();
        let data = parser.feed(&[IAC, SB, OPT_GMCP, b'a', SE, b'b', IAC, SE, b'l']);
        assert_eq!(data, b"l");
        assert_eq!(
            parser.take_events(),
            vec![TelnetEvent::Subnegotiation {
                option: OPT_GMCP,
                data: vec![b'a', SE, b'b']
            }]
        );
    }

    #[test]
    fn oversized_subnegotiation_is_bounded_and_discarded_until_real_end() {
        let mut parser = TelnetParser::default();
        let mut frame = vec![IAC, SB, OPT_GMCP];
        frame.extend(std::iter::repeat_n(b'x', MAX_SUBNEGOTIATION_BYTES + 1));
        frame.extend_from_slice(b"look");
        assert!(parser.feed(&frame).is_empty());
        assert!(parser.take_events().is_empty());

        assert_eq!(parser.feed(&[IAC, SE, b'n', b'\n']), b"n\n");
        assert_eq!(
            parser.take_events(),
            vec![TelnetEvent::SubnegotiationTooLong { option: OPT_GMCP }]
        );
    }

    #[test]
    fn parses_naws_with_telnet_escaped_width_byte() {
        let mut parser = TelnetParser::default();
        parser.feed(&[IAC, SB, OPT_NAWS, 0, IAC, IAC, 0, 24, IAC, SE]);
        let mut session = TelnetSession::default();
        assert!(session.handle(parser.take_events()).is_empty());
        assert_eq!(session.capabilities.columns, Some(255));
        assert_eq!(session.capabilities.rows, Some(24));
    }

    #[test]
    fn requests_ttype_and_utf8_only_after_client_accepts_options() {
        let mut session = TelnetSession::default();
        let response = session.handle(vec![
            TelnetEvent::Negotiation {
                command: WILL,
                option: OPT_TTYPE,
            },
            TelnetEvent::Negotiation {
                command: WILL,
                option: OPT_CHARSET,
            },
        ]);
        assert_eq!(
            response,
            [
                &[IAC, SB, OPT_TTYPE, TTYPE_SEND, IAC, SE][..],
                &[
                    IAC,
                    SB,
                    OPT_CHARSET,
                    CHARSET_REQUEST,
                    b' ',
                    b'U',
                    b'T',
                    b'F',
                    b'-',
                    b'8',
                    IAC,
                    SE,
                ][..],
            ]
            .concat()
        );

        let duplicate = session.handle(vec![TelnetEvent::Negotiation {
            command: WILL,
            option: OPT_TTYPE,
        }]);
        assert!(duplicate.is_empty());
    }

    #[test]
    fn parses_sanitized_ttype_and_charset_responses() {
        let mut session = TelnetSession::default();
        session.handle(vec![
            TelnetEvent::Subnegotiation {
                option: OPT_TTYPE,
                data: [TTYPE_IS, b'x', b't', b'e', b'r', b'm', 0, 27]
                    .into_iter()
                    .collect(),
            },
            TelnetEvent::Subnegotiation {
                option: OPT_CHARSET,
                data: [CHARSET_ACCEPTED, b'U', b'T', b'F', b'-', b'8']
                    .into_iter()
                    .collect(),
            },
        ]);
        assert_eq!(session.capabilities.terminal_type.as_deref(), Some("xterm"));
        assert_eq!(session.capabilities.charset.as_deref(), Some("UTF-8"));
    }

    #[test]
    fn does_not_claim_unimplemented_gmcp_support() {
        let mut session = TelnetSession::default();
        let response = session.handle(vec![TelnetEvent::Negotiation {
            command: DO,
            option: OPT_GMCP,
        }]);
        assert_eq!(response, [IAC, WONT, OPT_GMCP]);
    }

    #[test]
    fn discards_an_entire_overlong_command_before_accepting_another() {
        let mut commands = CommandBuffer::default();
        for _ in 0..=MAX_COMMAND_BYTES {
            assert_eq!(commands.push(b'x'), None);
        }
        assert_eq!(commands.push(b'l'), None);
        assert_eq!(commands.push(b'\n'), Some(LineEvent::TooLong));

        for byte in b"look\r\n" {
            if let Some(event) = commands.push(*byte) {
                assert_eq!(event, LineEvent::Command(b"look".to_vec()));
            }
        }
    }
}
