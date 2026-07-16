#![no_main]

use libfuzzer_sys::fuzz_target;
use tiamud_telnet_codec::{IAC, MAX_SUBNEGOTIATION_BYTES, SE, TelnetEvent, TelnetParser};

const MAX_INPUT_BYTES: usize = 256 * 1024;

fuzz_target!(|input: &[u8]| {
    if input.len() > MAX_INPUT_BYTES {
        return;
    }

    let mut whole = TelnetParser::default();
    let mut whole_data = whole.feed(input);
    let mut whole_events = whole.take_events();

    let mut fragmented = TelnetParser::default();
    let mut fragmented_data = Vec::new();
    let mut fragmented_events = Vec::new();
    let mut offset = 0;
    while offset < input.len() {
        let chunk_length = 1 + usize::from(input[offset] % 31);
        let end = (offset + chunk_length).min(input.len());
        fragmented_data.extend(fragmented.feed(&input[offset..end]));
        fragmented_events.extend(fragmented.take_events());
        offset = end;
    }

    assert_eq!(fragmented_data, whole_data);
    assert_eq!(fragmented_events, whole_events);
    assert!(whole_events.iter().all(bounded_event));

    // Make any incomplete final parser state observable without adding a
    // parser-state inspection API solely for fuzzing.
    let suffix = [IAC, SE, IAC, SE, b'\n'];
    whole_data.extend(whole.feed(&suffix));
    whole_events.extend(whole.take_events());
    fragmented_data.extend(fragmented.feed(&suffix));
    fragmented_events.extend(fragmented.take_events());
    assert_eq!(fragmented_data, whole_data);
    assert_eq!(fragmented_events, whole_events);
    assert!(whole_events.iter().all(bounded_event));
});

fn bounded_event(event: &TelnetEvent) -> bool {
    match event {
        TelnetEvent::Subnegotiation { data, .. } => data.len() <= MAX_SUBNEGOTIATION_BYTES,
        TelnetEvent::Negotiation { .. } | TelnetEvent::SubnegotiationTooLong { .. } => true,
    }
}
