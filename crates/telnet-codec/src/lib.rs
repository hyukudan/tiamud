//! Internal, transport-independent Telnet byte framing for Tiamud.
//!
//! This crate exists so the production server and fuzz harness execute one
//! parser implementation. Its API is intentionally limited to byte-level
//! framing: option policy, authentication, commands, and rendering remain in
//! `tiamud-server`. The crate is workspace-internal and cannot be published.

#![forbid(unsafe_code)]

/// Interpret As Command.
pub const IAC: u8 = 255;
/// Telnet DONT negotiation command.
pub const DONT: u8 = 254;
/// Telnet DO negotiation command.
pub const DO: u8 = 253;
/// Telnet WONT negotiation command.
pub const WONT: u8 = 252;
/// Telnet WILL negotiation command.
pub const WILL: u8 = 251;
/// Telnet subnegotiation begin command.
pub const SB: u8 = 250;
/// Telnet subnegotiation end command.
pub const SE: u8 = 240;

/// Maximum retained payload for one Telnet subnegotiation frame.
pub const MAX_SUBNEGOTIATION_BYTES: usize = 16 * 1024;

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

/// Incremental Telnet byte parser with bounded subnegotiation storage.
///
/// [`TelnetParser::feed`] returns application bytes and queues protocol events
/// retrievable with [`TelnetParser::take_events`]. Incomplete frames remain
/// internal until more bytes arrive.
#[derive(Default)]
pub struct TelnetParser {
    state: ParserState,
    events: Vec<TelnetEvent>,
    subnegotiation_option: u8,
    subnegotiation_data: Vec<u8>,
    subnegotiation_overflowed: bool,
}

impl TelnetParser {
    /// Feeds one arbitrary network fragment and returns only application data.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<u8> {
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

    /// Drains complete protocol events accumulated by previous feeds.
    pub fn take_events(&mut self) -> Vec<TelnetEvent> {
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

/// A complete Telnet control event emitted by [`TelnetParser`].
#[derive(Debug, Eq, PartialEq)]
pub enum TelnetEvent {
    Negotiation { command: u8, option: u8 },
    Subnegotiation { option: u8, data: Vec<u8> },
    SubnegotiationTooLong { option: u8 },
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPT_SUPPRESS_GO_AHEAD: u8 = 3;
    const OPT_EOR: u8 = 25;
    const OPT_TTYPE: u8 = 24;
    const OPT_NAWS: u8 = 31;
    const OPT_GMCP: u8 = 201;
    const TTYPE_IS: u8 = 0;

    #[test]
    fn separates_commands_from_telnet_negotiation() {
        let mut parser = TelnetParser::default();
        let data = parser.feed(&[IAC, DO, OPT_EOR, b'l', b'o', b'o', b'k', b'\n']);
        assert_eq!(data, b"look\n");
        assert_eq!(
            parser.take_events(),
            vec![TelnetEvent::Negotiation {
                command: DO,
                option: OPT_EOR,
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
                option: OPT_NAWS,
            }]
        );
    }

    #[test]
    fn golden_plaintext_bot_transcript_keeps_iac_out_of_commands() {
        let mut parser = TelnetParser::default();
        let transcript = [
            &[IAC][..],
            &[DO, OPT_SUPPRESS_GO_AHEAD, IAC, DO][..],
            &[OPT_EOR, IAC, WONT, OPT_TTYPE, b'r', b'e'][..],
            b"gister Telnet-run-1 password\r",
            b"\nlook\r\n",
        ];
        let mut commands = Vec::new();
        for chunk in transcript {
            commands.extend(parser.feed(chunk));
        }

        assert_eq!(commands, b"register Telnet-run-1 password\r\nlook\r\n");
        assert_eq!(
            parser.take_events(),
            vec![
                TelnetEvent::Negotiation {
                    command: DO,
                    option: OPT_SUPPRESS_GO_AHEAD,
                },
                TelnetEvent::Negotiation {
                    command: DO,
                    option: OPT_EOR,
                },
                TelnetEvent::Negotiation {
                    command: WONT,
                    option: OPT_TTYPE,
                },
            ]
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
                data: vec![TTYPE_IS, b'x', IAC, b'y'],
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
                data: vec![b'a', SE, b'b'],
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
    fn arbitrary_telnet_bytes_are_fragmentation_independent_and_bounded() {
        let mut seed = 0x5449_414d_5544_u64;
        for _case in 0..512 {
            let length = (next_random(&mut seed) as usize) % 2048;
            let bytes = (0..length)
                .map(|_| next_random(&mut seed) as u8)
                .collect::<Vec<_>>();

            let mut whole = TelnetParser::default();
            let whole_data = whole.feed(&bytes);
            let whole_events = whole.take_events();

            let mut fragmented = TelnetParser::default();
            let mut fragmented_data = Vec::new();
            let mut fragmented_events = Vec::new();
            let mut offset = 0;
            while offset < bytes.len() {
                let chunk = 1 + (next_random(&mut seed) as usize % 31);
                let end = (offset + chunk).min(bytes.len());
                fragmented_data.extend(fragmented.feed(&bytes[offset..end]));
                fragmented_events.extend(fragmented.take_events());
                offset = end;
            }

            assert_eq!(fragmented_data, whole_data);
            assert_eq!(fragmented_events, whole_events);
            assert_eq!(fragmented.state, whole.state);
            assert_eq!(
                fragmented.subnegotiation_option,
                whole.subnegotiation_option
            );
            assert_eq!(fragmented.subnegotiation_data, whole.subnegotiation_data);
            assert_eq!(
                fragmented.subnegotiation_overflowed,
                whole.subnegotiation_overflowed
            );
            assert!(fragmented.subnegotiation_data.len() <= MAX_SUBNEGOTIATION_BYTES);
        }
    }

    fn next_random(seed: &mut u64) -> u64 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        *seed
    }
}
