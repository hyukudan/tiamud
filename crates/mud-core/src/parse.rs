//! Text command parsing shared by browser and Telnet transports.

use super::*;

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ParseActionError {
    #[error("Please enter a command.")]
    Empty,
    #[error("Unknown command: {0}")]
    Unknown(String),
    #[error("{command} what?")]
    MissingTarget { command: &'static str },
}

pub fn parse_action(input: &str) -> Result<PlayerAction, ParseActionError> {
    let input = input.trim();
    let normalised = input.to_ascii_lowercase();
    if normalised.is_empty() {
        return Err(ParseActionError::Empty);
    }
    if matches!(normalised.as_str(), "look" | "l") {
        return Ok(PlayerAction::Look);
    }
    if matches!(normalised.as_str(), "inventory" | "inv" | "i") {
        return Ok(PlayerAction::Inventory);
    }
    if normalised == "weather" {
        return Ok(PlayerAction::Weather);
    }
    if normalised == "time" {
        return Ok(PlayerAction::Time);
    }
    if let Some(class_id) = normalised.strip_prefix("promote ").map(str::trim) {
        if class_id.is_empty() {
            return Err(ParseActionError::MissingTarget { command: "promote" });
        }
        return Ok(PlayerAction::Promote {
            class_id: class_id.to_owned(),
        });
    }
    if normalised == "promote" {
        return Err(ParseActionError::MissingTarget { command: "promote" });
    }
    if let Some(rest) = normalised.strip_prefix("talent learn ").map(str::trim) {
        let mut parts = rest.split_whitespace();
        let (Some(graph_id), Some(node_id), None) = (parts.next(), parts.next(), parts.next())
        else {
            return Err(ParseActionError::MissingTarget { command: "talent" });
        };
        return Ok(PlayerAction::AcquireTalent {
            graph_id: graph_id.to_owned(),
            node_id: node_id.to_owned(),
        });
    }
    if normalised == "balance" {
        return Ok(PlayerAction::Economy(EconomyAction::Balance));
    }
    if normalised == "shop" {
        return Ok(PlayerAction::Economy(EconomyAction::Shop));
    }
    if matches!(normalised.as_str(), "loot" | "loot status") {
        return Ok(PlayerAction::Loot(LootAction::Status));
    }
    if matches!(normalised.as_str(), "loot roll" | "loot decisions") {
        return Ok(PlayerAction::Loot(LootAction::Roll));
    }
    if let Some(target) = command_tail_phrase(input, "loot pass") {
        return Ok(PlayerAction::Loot(LootAction::Pass(target)));
    }
    if normalised == "loot pass" {
        return Err(ParseActionError::MissingTarget { command: "loot" });
    }
    if let Some(rest) = command_tail_phrase(input, "loot mode") {
        let mode = match rest
            .trim()
            .to_ascii_lowercase()
            .replace([' ', '-'], "_")
            .as_str()
        {
            "round_robin" | "roundrobin" | "rr" => {
                loot_distribution::LootDistributionMode::RoundRobin
            }
            "need_greed" | "needgreed" | "ng" => loot_distribution::LootDistributionMode::NeedGreed,
            "master_loot" | "masterloot" | "master" | "ml" => {
                loot_distribution::LootDistributionMode::MasterLoot
            }
            _ => return Err(ParseActionError::Unknown(input.to_owned())),
        };
        return Ok(PlayerAction::Loot(LootAction::Mode(mode)));
    }
    if normalised == "loot mode" {
        return Err(ParseActionError::MissingTarget { command: "loot" });
    }
    if let Some(rest) = command_tail_phrase(input, "loot master") {
        // Syntax: `loot master <item> to <member>`. The final ` to ` splits the
        // item query from the recipient so multi-word item queries survive.
        let lowered = rest.to_ascii_lowercase();
        if let Some(split) = lowered.rfind(" to ") {
            let query = rest[..split].trim().to_owned();
            let member = rest[split + 4..].trim().to_owned();
            if !query.is_empty() && !member.is_empty() {
                return Ok(PlayerAction::Loot(LootAction::Master { query, member }));
            }
        }
        return Err(ParseActionError::MissingTarget { command: "loot" });
    }
    if normalised == "loot master" {
        return Err(ParseActionError::MissingTarget { command: "loot" });
    }
    for (verb, decision) in [
        ("need", loot_distribution::LootDecision::Need),
        ("greed", loot_distribution::LootDecision::Greed),
        ("pass", loot_distribution::LootDecision::Pass),
    ] {
        if let Some(query) = command_tail_phrase(input, verb) {
            return Ok(PlayerAction::Loot(LootAction::Declare { decision, query }));
        }
        if normalised == verb {
            return Err(ParseActionError::MissingTarget { command: "loot" });
        }
    }
    match normalised.as_str() {
        "trade accept" => return Ok(PlayerAction::Trade(TradeAction::Accept)),
        "trade decline" => return Ok(PlayerAction::Trade(TradeAction::Decline)),
        "trade status" | "trade" => return Ok(PlayerAction::Trade(TradeAction::Status)),
        "trade cancel" => return Ok(PlayerAction::Trade(TradeAction::Cancel)),
        _ => {}
    }
    if let Some(target) = command_tail_phrase(input, "trade request") {
        return Ok(PlayerAction::Trade(TradeAction::Request(target)));
    }
    if normalised == "trade request" {
        return Err(ParseActionError::MissingTarget { command: "trade" });
    }
    if let Some(value) = command_tail_phrase(input, "trade offer item") {
        let selection = parse_item_selection(&value, input)?;
        if selection.quantity > MAX_TRADE_ITEM_QUANTITY {
            return Err(ParseActionError::Unknown(input.to_owned()));
        }
        return Ok(PlayerAction::Trade(TradeAction::OfferItem(selection)));
    }
    if normalised == "trade offer item" {
        return Err(ParseActionError::MissingTarget { command: "trade" });
    }
    if let Some(target) = command_tail_phrase(input, "trade remove item") {
        return Ok(PlayerAction::Trade(TradeAction::RemoveItem(target)));
    }
    if normalised == "trade remove item" {
        return Err(ParseActionError::MissingTarget { command: "trade" });
    }
    if let Some(value) = command_tail_phrase(input, "trade offer coins") {
        let coins = value
            .parse::<u64>()
            .ok()
            .filter(|coins| *coins <= MAX_TRADE_COINS)
            .ok_or_else(|| ParseActionError::Unknown(input.to_owned()))?;
        return Ok(PlayerAction::Trade(TradeAction::OfferCoins(coins)));
    }
    if normalised == "trade offer coins" {
        return Err(ParseActionError::MissingTarget { command: "trade" });
    }
    if let Some(value) = command_tail_phrase(input, "trade confirm") {
        let state_revision = value
            .parse::<u64>()
            .ok()
            .filter(|revision| *revision > 0)
            .ok_or_else(|| ParseActionError::Unknown(input.to_owned()))?;
        return Ok(PlayerAction::Trade(TradeAction::Confirm { state_revision }));
    }
    if normalised == "trade confirm" {
        return Err(ParseActionError::MissingTarget { command: "trade" });
    }
    if let Some(value) = command_tail_phrase(input, "trade contents") {
        let mut parts = value.rsplitn(2, char::is_whitespace);
        let last = parts.next().unwrap_or_default().trim();
        let previous = parts
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let (container, page) = match (previous, last.parse::<u16>().ok()) {
            (Some(container), Some(page)) if page > 0 => (container.to_owned(), page),
            _ => (value, 1),
        };
        return Ok(PlayerAction::Trade(TradeAction::Contents {
            container,
            page,
        }));
    }
    if normalised == "trade contents" {
        return Err(ParseActionError::MissingTarget { command: "trade" });
    }
    for (command, constructor) in [
        ("buy", EconomyAction::Buy as fn(String) -> EconomyAction),
        ("sell", EconomyAction::Sell),
    ] {
        if normalised == command {
            return Err(ParseActionError::MissingTarget { command });
        }
        if let Some(target) = command_tail_phrase(input, command) {
            return Ok(PlayerAction::Economy(constructor(target)));
        }
    }
    match normalised.as_str() {
        "party create" => return Ok(PlayerAction::Party(PartyAction::Create)),
        "party disband" => return Ok(PlayerAction::Party(PartyAction::Disband)),
        "party leave" => return Ok(PlayerAction::Party(PartyAction::Leave)),
        "party" | "party status" => return Ok(PlayerAction::Party(PartyAction::Status)),
        "quest" | "quests" | "quest list" => {
            return Ok(PlayerAction::Quest(QuestAction::List));
        }
        "quest status" => return Ok(PlayerAction::Quest(QuestAction::Status)),
        _ => {}
    }
    for (command, constructor) in [
        (
            "party invite",
            PartyAction::Invite as fn(String) -> PartyAction,
        ),
        ("party accept", PartyAction::Accept),
        ("party revoke", PartyAction::Revoke),
        ("party kick", PartyAction::Kick),
        ("party transfer", PartyAction::Transfer),
    ] {
        if normalised == command {
            return Err(ParseActionError::MissingTarget { command: "party" });
        }
        if let Some(target) = command_tail_phrase(input, command) {
            return Ok(PlayerAction::Party(constructor(target)));
        }
    }
    for (command, constructor) in [
        (
            "quest accept",
            QuestAction::Accept as fn(String) -> QuestAction,
        ),
        ("quest complete", QuestAction::Complete),
    ] {
        if normalised == command {
            return Err(ParseActionError::MissingTarget { command: "quest" });
        }
        if let Some(target) = command_tail_phrase(input, command) {
            return Ok(PlayerAction::Quest(constructor(target)));
        }
    }
    if normalised == "talk" || normalised == "talk to" {
        return Err(ParseActionError::MissingTarget { command: "talk" });
    }
    for command in ["talk to", "talk"] {
        if let Some(target) = command_tail_phrase(input, command) {
            return Ok(PlayerAction::Quest(QuestAction::Talk(target)));
        }
    }
    if normalised == "who" {
        return Ok(PlayerAction::Social(SocialAction::Who));
    }
    if normalised == "help" {
        return Ok(PlayerAction::Social(SocialAction::Help(None)));
    }
    if let Some(topic) = command_tail(input, "help") {
        return Ok(PlayerAction::Social(SocialAction::Help(Some(topic))));
    }
    if let Some(message) = command_tail(input, "say") {
        return Ok(PlayerAction::Social(SocialAction::Say(message)));
    }
    if normalised == "say" {
        return Err(ParseActionError::MissingTarget { command: "say" });
    }
    if let Some(message) = command_tail(input, "yell") {
        return Ok(PlayerAction::Social(SocialAction::Yell(message)));
    }
    if normalised == "yell" {
        return Err(ParseActionError::MissingTarget { command: "yell" });
    }
    if let Some(message) = command_tail(input, "shout") {
        return Ok(PlayerAction::Social(SocialAction::Shout(message)));
    }
    if normalised == "shout" {
        return Err(ParseActionError::MissingTarget { command: "shout" });
    }
    for command in ["party say", "gsay", "gtell"] {
        if let Some(message) = command_tail_phrase(input, command) {
            return Ok(PlayerAction::Social(SocialAction::PartySay(message)));
        }
        if normalised == command {
            return Err(ParseActionError::MissingTarget {
                command: "party say",
            });
        }
    }
    if let Some(rest) = command_tail(input, "tell") {
        let Some((target, message)) = rest.split_once(char::is_whitespace) else {
            return Err(ParseActionError::MissingTarget { command: "tell" });
        };
        let message = message.trim();
        if message.is_empty() {
            return Err(ParseActionError::MissingTarget { command: "tell" });
        }
        return Ok(PlayerAction::Social(SocialAction::Tell {
            target: target.to_owned(),
            message: message.to_owned(),
        }));
    }
    if normalised == "tell" {
        return Err(ParseActionError::MissingTarget { command: "tell" });
    }
    if let Some(message) = command_tail(input, "reply") {
        return Ok(PlayerAction::Social(SocialAction::Reply(message)));
    }
    if normalised == "reply" {
        return Err(ParseActionError::MissingTarget { command: "reply" });
    }
    if normalised == "board" {
        return Ok(PlayerAction::Board(BoardAction::Catalog));
    }
    if normalised == "board list" {
        return Err(ParseActionError::MissingTarget { command: "board" });
    }
    if let Some(rest) = command_tail_phrase(input, "board list") {
        let mut fields = rest.split_whitespace();
        let board = fields.next().and_then(BoardKey::parse);
        let page = fields.next().map_or(Some(1), |page| {
            page.parse::<u16>()
                .ok()
                .filter(|page| (1..=MAX_BOARD_PAGE).contains(page))
        });
        let (None, Some(board), Some(page)) = (fields.next(), board, page) else {
            return Err(ParseActionError::Unknown(input.to_owned()));
        };
        return Ok(PlayerAction::Board(BoardAction::List { board, page }));
    }
    for (command, read) in [("board read", true), ("board delete", false)] {
        if normalised == command {
            return Err(ParseActionError::MissingTarget { command: "board" });
        }
        if let Some(post_id) = command_tail_phrase(input, command) {
            if !valid_board_public_id(&post_id) {
                return Err(ParseActionError::Unknown(input.to_owned()));
            }
            return Ok(PlayerAction::Board(if read {
                BoardAction::Read { post_id }
            } else {
                BoardAction::Delete { post_id }
            }));
        }
    }
    if normalised == "board post" {
        return Err(ParseActionError::MissingTarget { command: "board" });
    }
    if let Some(rest) = command_tail_phrase(input, "board post") {
        let fields = rest.splitn(3, '|').map(str::trim).collect::<Vec<_>>();
        if fields.len() != 3 {
            return Err(ParseActionError::MissingTarget { command: "board" });
        }
        let Some(board) = BoardKey::parse(fields[0]) else {
            return Err(ParseActionError::Unknown(input.to_owned()));
        };
        let (subject, body) = (fields[1], fields[2]);
        if !valid_board_subject(subject) || !valid_board_visible_text(body, MAX_BOARD_BODY_BYTES) {
            return Err(ParseActionError::Unknown(input.to_owned()));
        }
        return Ok(PlayerAction::Board(BoardAction::Post {
            board,
            subject: subject.to_owned(),
            body: body.to_owned(),
        }));
    }
    if normalised == "board report" {
        return Err(ParseActionError::MissingTarget { command: "board" });
    }
    if let Some(rest) = command_tail_phrase(input, "board report") {
        let Some((post_id, reason)) = rest.split_once('|') else {
            return Err(ParseActionError::MissingTarget { command: "board" });
        };
        let post_id = post_id.trim();
        let reason = reason.trim();
        if !valid_board_public_id(post_id)
            || !valid_board_visible_text(reason, MAX_BOARD_REPORT_REASON_BYTES)
        {
            return Err(ParseActionError::Unknown(input.to_owned()));
        }
        return Ok(PlayerAction::Board(BoardAction::Report {
            post_id: post_id.to_owned(),
            reason: reason.to_owned(),
        }));
    }
    if matches!(normalised.as_str(), "mail" | "mail list") {
        return Ok(PlayerAction::Mail(MailAction::List { page: 1 }));
    }
    if let Some(page) = normalised.strip_prefix("mail list ").map(str::trim) {
        let page = page
            .parse::<u16>()
            .ok()
            .filter(|page| (1..=MAX_MAIL_PAGE).contains(page))
            .ok_or_else(|| ParseActionError::Unknown(input.to_owned()))?;
        return Ok(PlayerAction::Mail(MailAction::List { page }));
    }
    for (command, read) in [("mail read", true), ("mail delete", false)] {
        if normalised == command {
            return Err(ParseActionError::MissingTarget { command: "mail" });
        }
        if let Some(mail_id) = command_tail_phrase(input, command) {
            if !valid_mail_public_id(&mail_id) {
                return Err(ParseActionError::Unknown(input.to_owned()));
            }
            return Ok(PlayerAction::Mail(if read {
                MailAction::Read { mail_id }
            } else {
                MailAction::Delete { mail_id }
            }));
        }
    }
    if normalised == "mail send" {
        return Err(ParseActionError::MissingTarget { command: "mail" });
    }
    if let Some(rest) = command_tail_phrase(input, "mail send") {
        let fields = rest.splitn(3, '|').map(str::trim).collect::<Vec<_>>();
        if fields.len() != 3 {
            return Err(ParseActionError::MissingTarget { command: "mail" });
        }
        let (target, subject, body) = (fields[0], fields[1], fields[2]);
        if !valid_mail_target(target)
            || !valid_mail_subject(subject)
            || !valid_mail_visible_text(body, MAX_MAIL_BODY_BYTES)
        {
            return Err(ParseActionError::Unknown(input.to_owned()));
        }
        return Ok(PlayerAction::Mail(MailAction::Send {
            target: target.to_owned(),
            subject: subject.to_owned(),
            body: body.to_owned(),
        }));
    }
    if normalised == "mail report" {
        return Err(ParseActionError::MissingTarget { command: "mail" });
    }
    if let Some(rest) = command_tail_phrase(input, "mail report") {
        let Some((mail_id, reason)) = rest.split_once('|') else {
            return Err(ParseActionError::MissingTarget { command: "mail" });
        };
        let mail_id = mail_id.trim();
        let reason = reason.trim();
        if !valid_mail_public_id(mail_id)
            || !valid_mail_visible_text(reason, MAX_MAIL_REPORT_REASON_BYTES)
        {
            return Err(ParseActionError::Unknown(input.to_owned()));
        }
        return Ok(PlayerAction::Mail(MailAction::Report {
            mail_id: mail_id.to_owned(),
            reason: reason.to_owned(),
        }));
    }
    if normalised == "emotes" {
        return Ok(PlayerAction::Social(SocialAction::EmoteList));
    }
    if normalised == "emote" {
        return Err(ParseActionError::MissingTarget { command: "emote" });
    }
    if let Some(rest) = command_tail(input, "emote") {
        let (command, target) = rest
            .split_once(char::is_whitespace)
            .map_or((rest.as_str(), None), |(command, target)| {
                (command.trim(), Some(target.trim()))
            });
        if !emote::valid_command(command)
            || target.is_some_and(|target| !emote::valid_target_query(target))
        {
            return Err(ParseActionError::Unknown(input.to_owned()));
        }
        return Ok(PlayerAction::Social(SocialAction::Emote {
            command: command.to_owned(),
            target: target.map(str::to_owned),
        }));
    }
    if matches!(
        normalised.as_str(),
        "channel" | "channels" | "channel list" | "channels list"
    ) {
        return Ok(PlayerAction::Social(SocialAction::ChannelList));
    }
    for (command, join) in [("channel join", true), ("channel leave", false)] {
        if normalised == command {
            return Err(ParseActionError::MissingTarget { command: "channel" });
        }
        if let Some(channel) = command_tail_phrase(input, command) {
            let channel = SocialChannelName::parse(&channel)
                .ok_or_else(|| ParseActionError::Unknown(input.trim().to_owned()))?;
            return Ok(PlayerAction::Social(if join {
                SocialAction::ChannelJoin(channel)
            } else {
                SocialAction::ChannelLeave(channel)
            }));
        }
    }
    if let Some(rest) = command_tail_phrase(input, "channel say") {
        let Some((channel, message)) = rest.split_once(char::is_whitespace) else {
            return Err(ParseActionError::MissingTarget { command: "channel" });
        };
        let channel = SocialChannelName::parse(channel)
            .ok_or_else(|| ParseActionError::Unknown(input.trim().to_owned()))?;
        let message = message.trim();
        if message.is_empty() {
            return Err(ParseActionError::MissingTarget { command: "channel" });
        }
        return Ok(PlayerAction::Social(SocialAction::ChannelSay {
            channel,
            message: message.to_owned(),
        }));
    }
    if normalised == "channel say" {
        return Err(ParseActionError::MissingTarget { command: "channel" });
    }
    if let Some(rest) = command_tail(input, "channel") {
        let Some((channel, message)) = rest.split_once(char::is_whitespace) else {
            return Err(ParseActionError::MissingTarget { command: "channel" });
        };
        let channel = SocialChannelName::parse(channel)
            .ok_or_else(|| ParseActionError::Unknown(input.trim().to_owned()))?;
        let message = message.trim();
        if message.is_empty() {
            return Err(ParseActionError::MissingTarget { command: "channel" });
        }
        return Ok(PlayerAction::Social(SocialAction::ChannelSay {
            channel,
            message: message.to_owned(),
        }));
    }
    for (command, channel) in [
        ("gossip", SocialChannelName::General),
        ("newbie", SocialChannelName::Newbie),
        ("wiznet", SocialChannelName::Staff),
    ] {
        if let Some(message) = command_tail(input, command) {
            return Ok(PlayerAction::Social(SocialAction::ChannelSay {
                channel,
                message,
            }));
        }
        if normalised == command {
            return Err(ParseActionError::MissingTarget { command: "channel" });
        }
    }
    for (command, constructor) in [
        ("ignore", SocialAction::Ignore as fn(String) -> SocialAction),
        ("unignore", SocialAction::Unignore),
        ("block", SocialAction::Block),
        ("unblock", SocialAction::Unblock),
    ] {
        if normalised == command {
            return Err(ParseActionError::MissingTarget { command });
        }
        if let Some(target) = command_tail(input, command) {
            return Ok(PlayerAction::Social(constructor(target)));
        }
    }
    for (command, is_unban) in [("unmute", false), ("unban", true)] {
        if normalised == command {
            return Err(ParseActionError::MissingTarget { command });
        }
        if let Some(rest) = command_tail(input, command) {
            let Some((target, reason)) = rest.split_once(char::is_whitespace) else {
                return Err(ParseActionError::MissingTarget { command });
            };
            let reason = reason.trim();
            if target.is_empty() || reason.is_empty() {
                return Err(ParseActionError::MissingTarget { command });
            }
            return Ok(PlayerAction::Social(if is_unban {
                SocialAction::Unban {
                    target: target.to_owned(),
                    reason: reason.to_owned(),
                }
            } else {
                SocialAction::Unmute {
                    target: target.to_owned(),
                    reason: reason.to_owned(),
                }
            }));
        }
    }
    for (command, is_ban) in [("mute", false), ("ban", true)] {
        if normalised == command {
            return Err(ParseActionError::MissingTarget { command });
        }
        if let Some(rest) = command_tail(input, command) {
            let mut parts = rest.splitn(3, char::is_whitespace);
            let target = parts.next().unwrap_or_default().trim();
            let duration = parts.next().unwrap_or_default().trim();
            let reason = parts.next().unwrap_or_default().trim();
            if target.is_empty() || duration.is_empty() || reason.is_empty() {
                return Err(ParseActionError::MissingTarget { command });
            }
            let duration_seconds = parse_moderation_duration(duration)
                .ok_or_else(|| ParseActionError::Unknown(input.to_owned()))?;
            let action = if is_ban {
                SocialAction::Ban {
                    target: target.to_owned(),
                    duration_seconds,
                    reason: reason.to_owned(),
                }
            } else {
                SocialAction::Mute {
                    target: target.to_owned(),
                    duration_seconds,
                    reason: reason.to_owned(),
                }
            };
            return Ok(PlayerAction::Social(action));
        }
    }
    if let Some(rest) = command_tail(input, "report") {
        let Some((target, reason)) = rest.split_once(char::is_whitespace) else {
            return Err(ParseActionError::MissingTarget { command: "report" });
        };
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(ParseActionError::MissingTarget { command: "report" });
        }
        return Ok(PlayerAction::Social(SocialAction::Report {
            target: target.to_owned(),
            reason: reason.to_owned(),
        }));
    }
    if normalised == "report" {
        return Err(ParseActionError::MissingTarget { command: "report" });
    }
    if let Some(rest) = command_tail(input, "role") {
        let mut parts = rest.splitn(3, char::is_whitespace);
        let target = parts.next().unwrap_or_default().trim();
        let role = parts.next().unwrap_or_default().trim();
        let reason = parts.next().unwrap_or_default().trim();
        if target.is_empty() || role.is_empty() || reason.is_empty() {
            return Err(ParseActionError::MissingTarget { command: "role" });
        }
        let role = match role.to_ascii_lowercase().as_str() {
            "player" => AccountRole::Player,
            "moderator" | "mod" => AccountRole::Moderator,
            "admin" => AccountRole::Admin,
            _ => return Err(ParseActionError::Unknown(input.to_owned())),
        };
        return Ok(PlayerAction::Social(SocialAction::SetRole {
            target: target.to_owned(),
            role,
            reason: reason.to_owned(),
        }));
    }
    if normalised == "role" {
        return Err(ParseActionError::MissingTarget { command: "role" });
    }
    if normalised == "telepathy" {
        return Err(ParseActionError::MissingTarget {
            command: "telepathy",
        });
    }
    if let Some(rest) = command_tail(input, "telepathy") {
        let (ability_id, rest) = rest
            .split_once(char::is_whitespace)
            .map(|(head, tail)| (head.trim(), tail.trim_start()))
            .unwrap_or_default();
        let (target, message) = rest
            .split_once(char::is_whitespace)
            .map(|(head, tail)| (head.trim(), tail.trim()))
            .unwrap_or_default();
        if ability_id.is_empty() || target.is_empty() || message.is_empty() {
            return Err(ParseActionError::MissingTarget {
                command: "telepathy",
            });
        }
        if !valid_telepathy_fields(ability_id, target, message) {
            return Err(ParseActionError::Unknown(input.to_owned()));
        }
        return Ok(PlayerAction::Telepathy {
            ability_id: ability_id.to_owned(),
            target: target.to_owned(),
            message: message.to_owned(),
        });
    }
    if matches!(normalised.as_str(), "staff" | "staff status") {
        return Ok(PlayerAction::Staff(StaffAction::Status));
    }
    if let Some(reason) = command_tail_phrase(input, "staff reports") {
        return Ok(PlayerAction::Staff(StaffAction::Reports { reason }));
    }
    if normalised == "staff reports" {
        return Err(ParseActionError::MissingTarget { command: "staff" });
    }
    if let Some(rest) = command_tail_phrase(input, "staff resolve") {
        let mut parts = rest.splitn(3, char::is_whitespace);
        let report_id = parts.next().unwrap_or_default().trim();
        let resolution = parts.next().unwrap_or_default().trim();
        let reason = parts.next().unwrap_or_default().trim();
        if report_id.is_empty() || resolution.is_empty() || reason.is_empty() {
            return Err(ParseActionError::MissingTarget { command: "staff" });
        }
        let report_id = report_id
            .parse::<i64>()
            .ok()
            .filter(|report_id| *report_id > 0)
            .ok_or_else(|| ParseActionError::Unknown(input.to_owned()))?;
        let resolution = StaffReportResolution::parse(resolution)
            .ok_or_else(|| ParseActionError::Unknown(input.to_owned()))?;
        return Ok(PlayerAction::Staff(StaffAction::ResolveReport {
            report_id,
            resolution,
            reason: reason.to_owned(),
        }));
    }
    if normalised == "staff resolve" {
        return Err(ParseActionError::MissingTarget { command: "staff" });
    }
    type StaffTargetConstructor = fn(String, String) -> StaffAction;
    let staff_target_commands: [(&str, StaffTargetConstructor); 5] = [
        ("staff inspect", |target, reason| StaffAction::Inspect {
            target,
            reason,
        }),
        ("staff where", |target, reason| StaffAction::Where {
            target,
            reason,
        }),
        ("staff goto", |target, reason| StaffAction::Goto {
            target,
            reason,
        }),
        ("staff transfer", |target, reason| StaffAction::Transfer {
            target,
            reason,
        }),
        ("staff restore", |target, reason| StaffAction::Restore {
            target,
            reason,
        }),
    ];
    for (command, constructor) in staff_target_commands {
        if let Some(rest) = command_tail_phrase(input, command) {
            let (target, reason) = split_staff_reason(&rest)
                .ok_or(ParseActionError::MissingTarget { command: "staff" })?;
            return Ok(PlayerAction::Staff(constructor(target, reason)));
        }
        if normalised == command {
            return Err(ParseActionError::MissingTarget { command: "staff" });
        }
    }
    if let Some(rest) = command_tail_phrase(input, "staff announce") {
        let (message, reason) = split_staff_reason(&rest)
            .ok_or(ParseActionError::MissingTarget { command: "staff" })?;
        return Ok(PlayerAction::Staff(StaffAction::Announce {
            message,
            reason,
        }));
    }
    if normalised == "staff announce" {
        return Err(ParseActionError::MissingTarget { command: "staff" });
    }
    if let Some(reason) = command_tail_phrase(input, "staff audit") {
        return Ok(PlayerAction::Staff(StaffAction::Audit { reason }));
    }
    if normalised == "staff audit" {
        return Err(ParseActionError::MissingTarget { command: "staff" });
    }
    if let Some(rest) = command_tail_phrase(input, "staff profile") {
        let (body, reason) = split_staff_reason(&rest)
            .ok_or(ParseActionError::MissingTarget { command: "staff" })?;
        let mut parts = body.splitn(3, char::is_whitespace);
        let target = parts.next().unwrap_or_default().trim();
        let profile = parts.next().unwrap_or_default().trim();
        let lore_title = parts.next().unwrap_or_default().trim();
        if target.is_empty() || profile.is_empty() || lore_title.is_empty() {
            return Err(ParseActionError::MissingTarget { command: "staff" });
        }
        let profile = if profile.eq_ignore_ascii_case("none") {
            None
        } else {
            Some(
                StaffProfileKey::parse(profile)
                    .ok_or_else(|| ParseActionError::Unknown(input.to_owned()))?,
            )
        };
        let lore_title = (!lore_title.eq_ignore_ascii_case("none")).then(|| lore_title.to_owned());
        return Ok(PlayerAction::Staff(StaffAction::SetProfile {
            target: target.to_owned(),
            profile,
            lore_title,
            reason,
        }));
    }
    if normalised == "staff profile" {
        return Err(ParseActionError::MissingTarget { command: "staff" });
    }
    if let Some(rest) = command_tail_phrase(input, "staff capability") {
        let (body, reason) = split_staff_reason(&rest)
            .ok_or(ParseActionError::MissingTarget { command: "staff" })?;
        let mut parts = body.split_whitespace();
        let operation = parts.next().unwrap_or_default();
        let target = parts.next().unwrap_or_default();
        let capability = parts.next().unwrap_or_default();
        if target.is_empty() || capability.is_empty() || parts.next().is_some() {
            return Err(ParseActionError::MissingTarget { command: "staff" });
        }
        let enabled = match operation.to_ascii_lowercase().as_str() {
            "grant" => true,
            "revoke" => false,
            _ => return Err(ParseActionError::Unknown(input.to_owned())),
        };
        let capability = StaffCapability::parse(capability)
            .ok_or_else(|| ParseActionError::Unknown(input.to_owned()))?;
        return Ok(PlayerAction::Staff(StaffAction::SetCapability {
            target: target.to_owned(),
            capability,
            enabled,
            reason,
        }));
    }
    type StaffAreaConstructor = fn(String, String) -> StaffAction;
    let staff_area_commands: [(&str, StaffAreaConstructor); 2] = [
        ("staff area retire", |area, reason| {
            StaffAction::RetireArea { area, reason }
        }),
        ("staff area rehydrate", |area, reason| {
            StaffAction::RehydrateArea { area, reason }
        }),
    ];
    for (command, constructor) in staff_area_commands {
        if let Some(rest) = command_tail_phrase(input, command) {
            let (area, reason) = split_staff_reason(&rest)
                .ok_or(ParseActionError::MissingTarget { command: "staff" })?;
            return Ok(PlayerAction::Staff(constructor(area, reason)));
        }
        if normalised == command {
            return Err(ParseActionError::MissingTarget { command: "staff" });
        }
    }
    if normalised == "staff capability" || normalised.starts_with("staff ") {
        return Err(ParseActionError::MissingTarget { command: "staff" });
    }
    if let Some(direction) = Direction::parse(&normalised) {
        return Ok(PlayerAction::Move(direction));
    }
    if let Some(rest) = normalised.strip_prefix("go ")
        && let Some(direction) = Direction::parse(rest.trim())
    {
        return Ok(PlayerAction::Move(direction));
    }
    for command in ["ability", "skill", "cast"] {
        if normalised == command {
            return Err(ParseActionError::MissingTarget { command: "ability" });
        }
        if let Some(rest) = normalised.strip_prefix(&format!("{command} ")) {
            let mut parts = rest.trim().splitn(2, char::is_whitespace);
            let ability_id = parts.next().unwrap_or_default().trim();
            if ability_id.is_empty() {
                return Err(ParseActionError::MissingTarget { command: "ability" });
            }
            let target = parts
                .next()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
            return Ok(PlayerAction::Ability {
                ability_id: ability_id.to_owned(),
                target,
            });
        }
    }
    if let Some(rest) = command_tail(input, "contents") {
        let rest = rest.to_ascii_lowercase();
        let (container, page) = if let Some((container, page)) = rest.rsplit_once(" page ") {
            let page = page
                .trim()
                .parse::<u16>()
                .ok()
                .filter(|page| *page > 0)
                .ok_or_else(|| ParseActionError::Unknown(input.to_owned()))?;
            (container.trim(), page)
        } else if let Some((container, page)) = rest.rsplit_once(char::is_whitespace)
            && let Ok(page) = page.trim().parse::<u16>()
        {
            if page == 0 {
                return Err(ParseActionError::Unknown(input.to_owned()));
            }
            (container.trim(), page)
        } else {
            (rest.trim(), 1)
        };
        if container.is_empty() {
            return Err(ParseActionError::MissingTarget {
                command: "contents",
            });
        }
        return Ok(PlayerAction::Contents {
            container: container.to_owned(),
            page,
        });
    }
    if normalised == "contents" {
        return Err(ParseActionError::MissingTarget {
            command: "contents",
        });
    }
    if let Some(rest) = command_tail(input, "put") {
        let rest = rest.to_ascii_lowercase();
        let (selection, container) = rest
            .rsplit_once(" in ")
            .ok_or(ParseActionError::MissingTarget { command: "put" })?;
        let selection = parse_item_selection(selection, input)?;
        let container = container.trim();
        if container.is_empty() {
            return Err(ParseActionError::MissingTarget { command: "put" });
        }
        return Ok(PlayerAction::Put {
            selection,
            container: container.to_owned(),
        });
    }
    if normalised == "put" {
        return Err(ParseActionError::MissingTarget { command: "put" });
    }
    if let Some(rest) = command_tail(input, "give") {
        let separator = rest
            .to_ascii_lowercase()
            .rfind(" to ")
            .ok_or(ParseActionError::MissingTarget { command: "give" })?;
        let (selection, recipient) = rest.split_at(separator);
        let recipient = recipient
            .get(" to ".len()..)
            .ok_or(ParseActionError::MissingTarget { command: "give" })?;
        let selection = parse_item_selection(selection, input)?;
        let recipient = recipient.trim();
        if recipient.is_empty() {
            return Err(ParseActionError::MissingTarget { command: "give" });
        }
        return Ok(PlayerAction::Give {
            selection,
            recipient: recipient.to_owned(),
        });
    }
    if normalised == "give" {
        return Err(ParseActionError::MissingTarget { command: "give" });
    }
    // Container retrieval must be parsed before the historical `get` alias
    // for room pickup.
    if let Some(rest) = command_tail(input, "get").map(|rest| rest.to_ascii_lowercase())
        && let Some((selection, container)) = rest.rsplit_once(" from ")
    {
        let selection = parse_item_selection(selection, input)?;
        let container = container.trim();
        if container.is_empty() {
            return Err(ParseActionError::MissingTarget { command: "get" });
        }
        return Ok(PlayerAction::Get {
            selection,
            container: container.to_owned(),
        });
    }
    for (command, aliases, take) in [
        ("take", &["take", "get"][..], true),
        ("drop", &["drop"][..], false),
    ] {
        for alias in aliases {
            if normalised == *alias {
                return Err(ParseActionError::MissingTarget { command });
            }
            if let Some(target) = command_tail(input, alias) {
                let selection = parse_item_selection(&target.to_ascii_lowercase(), input)?;
                return Ok(if take {
                    PlayerAction::Take(selection)
                } else {
                    PlayerAction::Drop(selection)
                });
            }
        }
    }
    if normalised == "reward odds" {
        return Err(ParseActionError::MissingTarget {
            command: "reward odds",
        });
    }
    if let Some(rest) = normalised.strip_prefix("reward odds ") {
        let (target, page) = if let Some((target, page)) = rest.rsplit_once(char::is_whitespace)
            && let Ok(page) = page.parse::<u16>()
        {
            if page == 0 {
                return Err(ParseActionError::Unknown(input.to_owned()));
            }
            (target.trim(), page)
        } else {
            (rest.trim(), 1)
        };
        if target.is_empty() {
            return Err(ParseActionError::MissingTarget {
                command: "reward odds",
            });
        }
        return Ok(PlayerAction::RewardOdds {
            target: target.to_owned(),
            page,
        });
    }
    for (command, aliases, action) in [
        (
            "examine",
            &["examine", "x"][..],
            PlayerAction::Examine as fn(String) -> PlayerAction,
        ),
        ("attack", &["attack", "kill"][..], PlayerAction::Attack),
        ("use", &["use"][..], PlayerAction::Use),
        ("open", &["open"][..], PlayerAction::OpenReward),
        ("equip", &["equip", "wear"][..], PlayerAction::Equip),
        ("unequip", &["unequip", "remove"][..], PlayerAction::Unequip),
    ] {
        for alias in aliases {
            if normalised == *alias {
                return Err(ParseActionError::MissingTarget { command });
            }
            if let Some(target) = normalised.strip_prefix(&format!("{alias} ")) {
                let target = target.trim();
                if target.is_empty() {
                    return Err(ParseActionError::MissingTarget { command });
                }
                return Ok(action(target.to_owned()));
            }
        }
    }
    Err(ParseActionError::Unknown(input.trim().to_owned()))
}

pub(crate) fn parse_item_selection(
    value: &str,
    original: &str,
) -> Result<ItemSelection, ParseActionError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ParseActionError::MissingTarget { command: "item" });
    }
    let Some((first, rest)) = value.split_once(char::is_whitespace) else {
        return Ok(ItemSelection::one(value));
    };
    let Some(quantity) = first.parse::<u16>().ok() else {
        return Ok(ItemSelection::one(value));
    };
    let query = rest.trim();
    if quantity == 0 || quantity > MAX_ITEM_SELECTION_QUANTITY || query.is_empty() {
        return Err(ParseActionError::Unknown(original.trim().to_owned()));
    }
    Ok(ItemSelection {
        query: query.to_owned(),
        quantity,
    })
}

pub(crate) fn split_staff_reason(value: &str) -> Option<(String, String)> {
    let (body, reason) = value.rsplit_once('|')?;
    let body = body.trim();
    let reason = reason.trim();
    (!body.is_empty() && !reason.is_empty()).then(|| (body.to_owned(), reason.to_owned()))
}

pub(crate) fn command_tail(input: &str, command: &str) -> Option<String> {
    let (head, tail) = input.split_once(char::is_whitespace)?;
    if !head.eq_ignore_ascii_case(command) {
        return None;
    }
    let tail = tail.trim();
    (!tail.is_empty()).then(|| tail.to_owned())
}

pub(crate) fn command_tail_phrase(input: &str, command: &str) -> Option<String> {
    let head = input.get(..command.len())?;
    if !head.eq_ignore_ascii_case(command) {
        return None;
    }
    let rest = input.get(command.len()..)?;
    if !rest.chars().next().is_some_and(char::is_whitespace) {
        return None;
    }
    let tail = rest.trim();
    (!tail.is_empty()).then(|| tail.to_owned())
}

pub(crate) fn parse_moderation_duration(value: &str) -> Option<Option<u64>> {
    if matches!(value.to_ascii_lowercase().as_str(), "permanent" | "perm") {
        return Some(None);
    }
    let (number, multiplier) = match value.as_bytes().last().copied()? {
        b'm' | b'M' => (&value[..value.len() - 1], 60_u64),
        b'h' | b'H' => (&value[..value.len() - 1], 60 * 60),
        b'd' | b'D' => (&value[..value.len() - 1], 24 * 60 * 60),
        _ => (value, 1),
    };
    number
        .parse::<u64>()
        .ok()
        .and_then(|number| number.checked_mul(multiplier))
        .map(Some)
}
