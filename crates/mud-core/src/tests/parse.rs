use super::*;

#[test]
fn parses_legacy_friendly_movement_commands() {
    assert_eq!(parse_action("l"), Ok(PlayerAction::Look));
    assert_eq!(
        parse_action("go north"),
        Ok(PlayerAction::Move(Direction::North))
    );
}

#[test]
fn parses_inventory_and_item_commands_with_safe_missing_targets() {
    assert_eq!(parse_action("inventory"), Ok(PlayerAction::Inventory));
    assert_eq!(
        parse_action("get Blue Lantern"),
        Ok(PlayerAction::Take("blue lantern".into()))
    );
    assert_eq!(
        parse_action("drop lamp"),
        Ok(PlayerAction::Drop("lamp".into()))
    );
    assert_eq!(
        parse_action("x lantern"),
        Ok(PlayerAction::Examine("lantern".into()))
    );
    assert_eq!(
        parse_action("open o0123456789abcdef"),
        Ok(PlayerAction::OpenReward("o0123456789abcdef".into()))
    );
    assert_eq!(
        parse_action("reward odds o0123456789abcdef"),
        Ok(PlayerAction::RewardOdds {
            target: "o0123456789abcdef".into(),
            page: 1,
        })
    );
    assert_eq!(
        parse_action("reward odds sealed cache 12"),
        Ok(PlayerAction::RewardOdds {
            target: "sealed cache".into(),
            page: 12,
        })
    );
    assert!(parse_action("reward odds sealed cache 0").is_err());
    assert_eq!(
        parse_action("take   "),
        Err(ParseActionError::MissingTarget { command: "take" })
    );
    assert_eq!(
        parse_action("examine"),
        Err(ParseActionError::MissingTarget { command: "examine" })
    );
}

#[test]
fn parses_social_and_moderation_commands_without_lowercasing_messages() {
    assert_eq!(
        parse_action("say Hold the North Gate!"),
        Ok(PlayerAction::Social(SocialAction::Say(
            "Hold the North Gate!".into()
        )))
    );
    assert_eq!(
        parse_action("yell Hold the Near Gate!"),
        Ok(PlayerAction::Social(SocialAction::Yell(
            "Hold the Near Gate!".into()
        )))
    );
    assert_eq!(
        parse_action("shout Hold Every Gate!"),
        Ok(PlayerAction::Social(SocialAction::Shout(
            "Hold Every Gate!".into()
        )))
    );
    assert_eq!(
        parse_action("tell Aria Meet Me There"),
        Ok(PlayerAction::Social(SocialAction::Tell {
            target: "Aria".into(),
            message: "Meet Me There".into(),
        }))
    );
    assert_eq!(
        parse_action("party say Hold the North Gate!"),
        Ok(PlayerAction::Social(SocialAction::PartySay(
            "Hold the North Gate!".into()
        )))
    );
    assert_eq!(
        parse_action("reply I Am On My Way"),
        Ok(PlayerAction::Social(SocialAction::Reply(
            "I Am On My Way".into()
        )))
    );
    assert_eq!(
        parse_action("channel list"),
        Ok(PlayerAction::Social(SocialAction::ChannelList))
    );
    assert_eq!(
        parse_action("channel join newbie"),
        Ok(PlayerAction::Social(SocialAction::ChannelJoin(
            SocialChannelName::Newbie
        )))
    );
    assert_eq!(
        parse_action("channel leave general"),
        Ok(PlayerAction::Social(SocialAction::ChannelLeave(
            SocialChannelName::General
        )))
    );
    assert_eq!(
        parse_action("channel say general Keep This Case"),
        Ok(PlayerAction::Social(SocialAction::ChannelSay {
            channel: SocialChannelName::General,
            message: "Keep This Case".into(),
        }))
    );
    assert_eq!(
        parse_action("party revoke Aria"),
        Ok(PlayerAction::Party(PartyAction::Revoke("Aria".into())))
    );
    assert_eq!(
        parse_action("party kick Aria"),
        Ok(PlayerAction::Party(PartyAction::Kick("Aria".into())))
    );
    assert_eq!(
        parse_action("party transfer Aria"),
        Ok(PlayerAction::Party(PartyAction::Transfer("Aria".into())))
    );
    assert_eq!(
        parse_action("party disband"),
        Ok(PlayerAction::Party(PartyAction::Disband))
    );
    for command in [
        "yell",
        "shout",
        "party say",
        "reply",
        "channel join",
        "channel say general",
    ] {
        assert!(matches!(
            parse_action(command),
            Err(ParseActionError::MissingTarget { .. })
        ));
    }
    assert_eq!(
        parse_action("mute Griefer 2h repeated flooding"),
        Ok(PlayerAction::Social(SocialAction::Mute {
            target: "Griefer".into(),
            duration_seconds: Some(7_200),
            reason: "repeated flooding".into(),
        }))
    );
    assert_eq!(
        parse_action("ban Griefer permanent severe abuse"),
        Ok(PlayerAction::Social(SocialAction::Ban {
            target: "Griefer".into(),
            duration_seconds: None,
            reason: "severe abuse".into(),
        }))
    );
    assert_eq!(
        parse_action("role Aria moderator operational promotion"),
        Ok(PlayerAction::Social(SocialAction::SetRole {
            target: "Aria".into(),
            role: AccountRole::Moderator,
            reason: "operational promotion".into(),
        }))
    );
    assert_eq!(
        parse_action("unmute Griefer appeal accepted"),
        Ok(PlayerAction::Social(SocialAction::Unmute {
            target: "Griefer".into(),
            reason: "appeal accepted".into(),
        }))
    );
}

#[test]
fn parses_staff_commands_with_closed_values_and_mandatory_reasons() {
    assert_eq!(
        parse_action("staff inspect Aria | investigate stuck character"),
        Ok(PlayerAction::Staff(StaffAction::Inspect {
            target: "Aria".into(),
            reason: "investigate stuck character".into(),
        }))
    );
    assert_eq!(
        parse_action("staff announce Server restart in ten minutes | planned maintenance"),
        Ok(PlayerAction::Staff(StaffAction::Announce {
            message: "Server restart in ten minutes".into(),
            reason: "planned maintenance".into(),
        }))
    );
    assert_eq!(
        parse_action("staff resolve 7 action_taken verified harassment"),
        Ok(PlayerAction::Staff(StaffAction::ResolveReport {
            report_id: 7,
            resolution: StaffReportResolution::ActionTaken,
            reason: "verified harassment".into(),
        }))
    );
    assert_eq!(
        parse_action("staff profile Aria helper Angel of Welcome | onboarding assignment"),
        Ok(PlayerAction::Staff(StaffAction::SetProfile {
            target: "Aria".into(),
            profile: Some(StaffProfileKey::Helper),
            lore_title: Some("Angel of Welcome".into()),
            reason: "onboarding assignment".into(),
        }))
    );
    assert_eq!(
        parse_action("staff capability grant Aria reports.view | support rotation"),
        Ok(PlayerAction::Staff(StaffAction::SetCapability {
            target: "Aria".into(),
            capability: StaffCapability::ReportsView,
            enabled: true,
            reason: "support rotation".into(),
        }))
    );
    for invalid in [
        "staff inspect Aria",
        "staff announce notice without reason",
        "staff resolve 0 action_taken invalid id",
        "staff resolve 7 arbitrary invalid resolution",
        "staff capability grant Aria unknown | invalid capability",
    ] {
        assert!(parse_action(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn parses_staff_area_lifecycle_commands_with_mandatory_reasons() {
    assert_eq!(
        parse_action("staff area retire square | draining for maintenance"),
        Ok(PlayerAction::Staff(StaffAction::RetireArea {
            area: "square".into(),
            reason: "draining for maintenance".into(),
        }))
    );
    assert_eq!(
        parse_action("staff area rehydrate square | restoring after maintenance"),
        Ok(PlayerAction::Staff(StaffAction::RehydrateArea {
            area: "square".into(),
            reason: "restoring after maintenance".into(),
        }))
    );
    assert_eq!(
        parse_action("staff area retire square"),
        Err(ParseActionError::MissingTarget { command: "staff" })
    );
    assert_eq!(
        parse_action("STAFF AREA RETIRE Square | why"),
        Ok(PlayerAction::Staff(StaffAction::RetireArea {
            area: "Square".into(),
            reason: "why".into(),
        }))
    );
}

#[test]
fn parses_phase_two_gameplay_commands() {
    assert_eq!(
        parse_action("attack guardian"),
        Ok(PlayerAction::Attack("guardian".into()))
    );
    assert_eq!(
        parse_action("use potion"),
        Ok(PlayerAction::Use("potion".into()))
    );
    assert_eq!(
        parse_action("wear sword"),
        Ok(PlayerAction::Equip("sword".into()))
    );
    assert_eq!(
        parse_action("remove weapon"),
        Ok(PlayerAction::Unequip("weapon".into()))
    );
    assert_eq!(
        parse_action("attack"),
        Err(ParseActionError::MissingTarget { command: "attack" })
    );
}

#[test]
fn parses_adventure_commands_without_losing_targets() {
    assert_eq!(
        parse_action("party invite Aria").unwrap(),
        PlayerAction::Party(PartyAction::Invite("Aria".into()))
    );
    assert_eq!(
        parse_action("quest accept first hunt").unwrap(),
        PlayerAction::Quest(QuestAction::Accept("first hunt".into()))
    );
    assert_eq!(
        parse_action("buy healing tonic").unwrap(),
        PlayerAction::Economy(EconomyAction::Buy("healing tonic".into()))
    );
    assert_eq!(parse_action("weather").unwrap(), PlayerAction::Weather);
}

#[test]
fn parses_bounded_stack_and_container_commands() {
    assert_eq!(
        parse_action("take 3 tonic").unwrap(),
        PlayerAction::Take(ItemSelection {
            query: "tonic".into(),
            quantity: 3,
        })
    );
    assert_eq!(
        parse_action("get 2 tonic from SATCHEL").unwrap(),
        PlayerAction::Get {
            selection: ItemSelection {
                query: "tonic".into(),
                quantity: 2,
            },
            container: "satchel".into(),
        }
    );
    assert_eq!(
        parse_action("put tonic in satchel").unwrap(),
        PlayerAction::Put {
            selection: ItemSelection::one("tonic"),
            container: "satchel".into(),
        }
    );
    assert_eq!(
        parse_action("contents satchel 2").unwrap(),
        PlayerAction::Contents {
            container: "satchel".into(),
            page: 2,
        }
    );
    assert_eq!(
        parse_action("give 2 Moon Tonic to Aria").unwrap(),
        PlayerAction::Give {
            selection: ItemSelection {
                query: "Moon Tonic".into(),
                quantity: 2,
            },
            recipient: "Aria".into(),
        }
    );
    assert_eq!(
        parse_action("trade offer item 12 Moon Tonic").unwrap(),
        PlayerAction::Trade(TradeAction::OfferItem(ItemSelection {
            query: "Moon Tonic".into(),
            quantity: 12,
        }))
    );
    assert_eq!(
        parse_action("trade offer coins 1000000000").unwrap(),
        PlayerAction::Trade(TradeAction::OfferCoins(1_000_000_000))
    );
    assert_eq!(
        parse_action("trade confirm 42").unwrap(),
        PlayerAction::Trade(TradeAction::Confirm { state_revision: 42 })
    );
    assert_eq!(
        parse_action("trade contents offered bag 3").unwrap(),
        PlayerAction::Trade(TradeAction::Contents {
            container: "offered bag".into(),
            page: 3,
        })
    );
    assert!(parse_action("take 257 tonic").is_err());
    assert!(parse_action("take 0 tonic").is_err());
    assert!(parse_action("give tonic").is_err());
    assert!(parse_action("give 257 tonic to Aria").is_err());
    assert!(parse_action("trade offer item 129 tonic").is_err());
    assert!(parse_action("trade offer coins 1000000001").is_err());
    assert!(parse_action("trade confirm 0").is_err());
}

#[test]
fn parses_group_loot_distribution_commands() {
    use loot_distribution::{LootDecision, LootDistributionMode};
    assert_eq!(
        parse_action("loot mode need_greed"),
        Ok(PlayerAction::Loot(LootAction::Mode(
            LootDistributionMode::NeedGreed
        )))
    );
    assert_eq!(
        parse_action("loot mode master"),
        Ok(PlayerAction::Loot(LootAction::Mode(
            LootDistributionMode::MasterLoot
        )))
    );
    assert_eq!(
        parse_action("loot mode rr"),
        Ok(PlayerAction::Loot(LootAction::Mode(
            LootDistributionMode::RoundRobin
        )))
    );
    assert_eq!(
        parse_action("loot roll"),
        Ok(PlayerAction::Loot(LootAction::Roll))
    );
    assert_eq!(
        parse_action("need silver fang"),
        Ok(PlayerAction::Loot(LootAction::Declare {
            decision: LootDecision::Need,
            query: "silver fang".into()
        }))
    );
    assert_eq!(
        parse_action("greed fang"),
        Ok(PlayerAction::Loot(LootAction::Declare {
            decision: LootDecision::Greed,
            query: "fang".into()
        }))
    );
    assert_eq!(
        parse_action("pass fang"),
        Ok(PlayerAction::Loot(LootAction::Declare {
            decision: LootDecision::Pass,
            query: "fang".into()
        }))
    );
    assert_eq!(
        parse_action("loot master fang to Partner"),
        Ok(PlayerAction::Loot(LootAction::Master {
            query: "fang".into(),
            member: "Partner".into()
        }))
    );
    // The pre-existing reservation `loot pass` still routes to `Pass`, never
    // to a need/greed declaration.
    assert_eq!(
        parse_action("loot pass fang"),
        Ok(PlayerAction::Loot(LootAction::Pass("fang".into())))
    );
    assert!(matches!(
        parse_action("loot mode bogus"),
        Err(ParseActionError::Unknown(_))
    ));
    assert!(matches!(
        parse_action("loot mode"),
        Err(ParseActionError::MissingTarget { command: "loot" })
    ));
    assert!(matches!(
        parse_action("need"),
        Err(ParseActionError::MissingTarget { command: "loot" })
    ));
}
