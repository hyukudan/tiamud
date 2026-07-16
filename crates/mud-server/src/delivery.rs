//! Bounded, session-addressed delivery between the world actor and transports.
//!
//! A Tokio broadcast channel makes every connected transport receive and
//! discard every other session's private projection. Sending one update to
//! each of `S` sessions therefore performs `S * S` receiver wake-ups. This
//! router owns one bounded mailbox per live world session and performs one
//! hash lookup plus one `try_send` for each intended recipient.

use crate::{
    operations::{RoutedDeliveryKind, RuntimeMetrics},
    social::SocialMessage,
    world_actor::WorldDelivery,
};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
};
use tiamud_core::SessionId;
use tokio::sync::mpsc;

pub const SESSION_DELIVERY_CAPACITY: usize = 256;
const WORLD_LAGGED: u8 = 1;
const SOCIAL_LAGGED: u8 = 2;

#[derive(Debug)]
enum QueuedDelivery {
    World(Box<WorldDelivery>),
    Social(SocialMessage),
}

/// A transport-visible delivery. Lag is reported by class so world state can
/// be repaired from an authoritative snapshot without treating lossy chat as
/// a state-consistency failure.
#[derive(Debug)]
pub enum SessionDelivery {
    World(Box<WorldDelivery>),
    Social(SocialMessage),
    WorldLagged,
    SocialLagged,
}

#[derive(Debug)]
struct Mailbox {
    tx: mpsc::Sender<QueuedDelivery>,
    lagged: Arc<AtomicU8>,
}

/// The receiving half is created by the world actor before the successful
/// entry reply is released, closing the subscribe-after-enter race.
#[derive(Debug)]
pub struct DeliveryInbox {
    rx: mpsc::Receiver<QueuedDelivery>,
    lagged: Arc<AtomicU8>,
    pending_lagged: u8,
    buffered: Option<QueuedDelivery>,
}

impl DeliveryInbox {
    pub async fn recv(&mut self) -> Option<SessionDelivery> {
        loop {
            if let Some(delivery) = self.take_lagged() {
                return Some(delivery);
            }
            if let Some(delivery) = self.buffered.take() {
                return Some(delivery.into());
            }
            self.buffered = Some(self.rx.recv().await?);
            // Check the flag again after waking. If the mailbox overflowed
            // concurrently, the repair notice must precede queued stale data.
        }
    }

    #[cfg(test)]
    pub(crate) fn try_recv(&mut self) -> Result<SessionDelivery, mpsc::error::TryRecvError> {
        if let Some(delivery) = self.take_lagged() {
            return Ok(delivery);
        }
        if let Some(delivery) = self.buffered.take() {
            return Ok(delivery.into());
        }
        self.rx.try_recv().map(Into::into)
    }

    fn take_lagged(&mut self) -> Option<SessionDelivery> {
        self.pending_lagged |= self.lagged.swap(0, Ordering::AcqRel);
        if self.pending_lagged & WORLD_LAGGED != 0 {
            self.pending_lagged &= !WORLD_LAGGED;
            return Some(SessionDelivery::WorldLagged);
        }
        if self.pending_lagged & SOCIAL_LAGGED != 0 {
            self.pending_lagged &= !SOCIAL_LAGGED;
            return Some(SessionDelivery::SocialLagged);
        }
        None
    }
}

impl From<QueuedDelivery> for SessionDelivery {
    fn from(value: QueuedDelivery) -> Self {
        match value {
            QueuedDelivery::World(delivery) => Self::World(delivery),
            QueuedDelivery::Social(delivery) => Self::Social(delivery),
        }
    }
}

#[derive(Debug)]
pub struct DeliveryRouter {
    mailboxes: HashMap<SessionId, Mailbox>,
    capacity: usize,
    metrics: Option<Arc<RuntimeMetrics>>,
}

impl DeliveryRouter {
    pub fn new(metrics: Option<Arc<RuntimeMetrics>>) -> Self {
        Self::with_capacity(SESSION_DELIVERY_CAPACITY, metrics)
    }

    fn with_capacity(capacity: usize, metrics: Option<Arc<RuntimeMetrics>>) -> Self {
        assert!(capacity > 0, "delivery mailbox capacity must be positive");
        Self {
            mailboxes: HashMap::new(),
            capacity,
            metrics,
        }
    }

    pub fn register(&mut self, session: SessionId) -> DeliveryInbox {
        let (tx, rx) = mpsc::channel(self.capacity);
        let lagged = Arc::new(AtomicU8::new(0));
        self.mailboxes.insert(
            session,
            Mailbox {
                tx,
                lagged: lagged.clone(),
            },
        );
        self.record_mailbox_count();
        DeliveryInbox {
            rx,
            lagged,
            pending_lagged: 0,
            buffered: None,
        }
    }

    pub fn unregister(&mut self, session: SessionId) {
        self.mailboxes.remove(&session);
        self.record_mailbox_count();
    }

    pub fn send_world(&mut self, session: SessionId, delivery: WorldDelivery) {
        self.send(
            session,
            QueuedDelivery::World(Box::new(delivery)),
            RoutedDeliveryKind::World,
            WORLD_LAGGED,
        );
    }

    pub fn send_social(&mut self, session: SessionId, delivery: SocialMessage) -> bool {
        self.send(
            session,
            QueuedDelivery::Social(delivery),
            RoutedDeliveryKind::Social,
            SOCIAL_LAGGED,
        )
    }

    fn send(
        &mut self,
        session: SessionId,
        delivery: QueuedDelivery,
        kind: RoutedDeliveryKind,
        lagged_flag: u8,
    ) -> bool {
        let Some(mailbox) = self.mailboxes.get(&session) else {
            return false;
        };
        match mailbox.tx.try_send(delivery) {
            Ok(()) => {
                if let Some(metrics) = &self.metrics {
                    metrics.delivery_enqueued(kind);
                }
                true
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                mailbox.lagged.fetch_or(lagged_flag, Ordering::Release);
                if let Some(metrics) = &self.metrics {
                    metrics.delivery_dropped(kind);
                }
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.mailboxes.remove(&session);
                self.record_mailbox_count();
                false
            }
        }
    }

    fn record_mailbox_count(&self) {
        if let Some(metrics) = &self.metrics {
            metrics.set_delivery_mailboxes(self.mailboxes.len());
        }
    }
}

impl Drop for DeliveryRouter {
    fn drop(&mut self) {
        if let Some(metrics) = &self.metrics {
            metrics.set_delivery_mailboxes(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::social::{SocialChannel, SocialMessage};

    fn message(index: usize) -> SocialMessage {
        SocialMessage {
            channel: SocialChannel::Tell,
            sender: "Router".into(),
            text: format!("message {index}"),
        }
    }

    #[test]
    fn a_thousand_targeted_deliveries_create_exactly_a_thousand_queue_entries() {
        let metrics = Arc::new(RuntimeMetrics::default());
        let mut router = DeliveryRouter::with_capacity(2, Some(metrics.clone()));
        let mut inboxes = (1..=1_000_u64)
            .map(|id| {
                let session = SessionId(id);
                (session, router.register(session))
            })
            .collect::<Vec<_>>();

        for (index, (session, _)) in inboxes.iter().enumerate() {
            router.send_social(*session, message(index));
        }

        for (index, (_, inbox)) in inboxes.iter_mut().enumerate() {
            let SessionDelivery::Social(delivery) = inbox.try_recv().unwrap() else {
                panic!("targeted social delivery expected");
            };
            assert_eq!(delivery.text, format!("message {index}"));
            assert!(matches!(
                inbox.try_recv(),
                Err(mpsc::error::TryRecvError::Empty)
            ));
        }
        let metrics = metrics.prometheus(0);
        assert!(metrics.contains("tiamud_delivery_mailboxes 1000"));
        assert!(metrics.contains("tiamud_delivery_enqueued_total{kind=\"social\"} 1000"));
    }

    #[tokio::test]
    async fn overflow_is_bounded_and_reports_each_delivery_class_once() {
        let mut router = DeliveryRouter::with_capacity(1, None);
        let session = SessionId(1);
        let mut inbox = router.register(session);
        router.send_social(session, message(1));
        router.send_social(session, message(2));
        router.send_social(session, message(3));

        assert!(matches!(
            inbox.recv().await,
            Some(SessionDelivery::SocialLagged)
        ));
        let Some(SessionDelivery::Social(delivery)) = inbox.recv().await else {
            panic!("the already queued message must be preserved");
        };
        assert_eq!(delivery.text, "message 1");
        assert!(matches!(
            inbox.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }
}
