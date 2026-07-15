use crate::{
    operations::{
        ActiveSessionLimit, ActiveSessionLimiter, AuthRateLimiter, CommandRateLimiter,
        OutputTransport, RuntimeMetrics,
    },
    persistence::StoreError,
    social::{SocialChannel, SocialMessage},
    telnet_transport::{TelnetStream, TelnetTransport},
    world_actor::{GameError, WorldHandle},
};
use flate2::{Compression, write::ZlibEncoder};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io::{self, Write as _},
    sync::{Arc, Mutex},
    time::Instant,
};
use tiamud_cell_frame::{AnsiProfile, Cell, CellAttributes, CellFrame, NativeProtocol, RgbColor};
use tiamud_core::{CharacterSnapshot, PlayerStateView, SceneAudio, SceneView, parse_action};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, ReadHalf},
    net::TcpListener,
    sync::{Notify, broadcast, watch},
    task::JoinHandle,
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
const OPT_ECHO: u8 = 1;
const OPT_SUPPRESS_GO_AHEAD: u8 = 3;
const OPT_EOR: u8 = 25;
const OPT_TTYPE: u8 = 24;
const OPT_NAWS: u8 = 31;
const OPT_CHARSET: u8 = 42;
const OPT_MSSP: u8 = 70;
const OPT_MCCP2: u8 = 86;
const OPT_MXP: u8 = 91;
const OPT_GMCP: u8 = 201;

const MSSP_VAR: u8 = 1;
const MSSP_VAL: u8 = 2;

const TTYPE_IS: u8 = 0;
const TTYPE_SEND: u8 = 1;
const CHARSET_REQUEST: u8 = 1;
const CHARSET_ACCEPTED: u8 = 2;
const CHARSET_REJECTED: u8 = 3;

const MAX_COMMAND_BYTES: usize = 4096;
const MAX_SUBNEGOTIATION_BYTES: usize = 16 * 1024;
const MAX_GMCP_BYTES: usize = 8 * 1024;
const MAX_GMCP_PACKAGE_BYTES: usize = 64;
const MAX_GMCP_SUPPORTS: usize = 64;
const MAX_GMCP_CLIENT_VALUE_BYTES: usize = 128;
const MAX_AUTH_ATTEMPTS: u8 = 5;
const MAX_TTYPE_REQUESTS: u8 = 8;
const MAX_RATE_VIOLATIONS: u8 = 3;
const TELNET_OUTPUT_MAX_FRAMES: usize = 64;
const TELNET_OUTPUT_MAX_BYTES: usize = 512 * 1024;
const TELNET_OUTPUT_MAX_FRAME_BYTES: usize = 128 * 1024;
const TELNET_CELL_FRAME_MAX_CELLS: usize = 512;
const TELNET_CELL_FRAME_MAX_BYTES: usize = 64 * 1024;

const MTTS_ANSI: u16 = 1;
const MTTS_VT100: u16 = 2;
const MTTS_UTF8: u16 = 4;
const MTTS_256_COLORS: u16 = 8;
const MTTS_MOUSE_TRACKING: u16 = 16;
const MTTS_OSC_COLOR_PALETTE: u16 = 32;
const MTTS_SCREEN_READER: u16 = 64;
const MTTS_PROXY: u16 = 128;
const MTTS_TRUECOLOR: u16 = 256;
const MTTS_MNES: u16 = 512;
const MTTS_MSLP: u16 = 1024;
const MTTS_TLS: u16 = 2048;

#[derive(Default)]
struct CompressionBuffer(Vec<u8>);

impl io::Write for CompressionBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Default)]
struct TelnetWriter {
    compressor: Option<ZlibEncoder<CompressionBuffer>>,
}

impl TelnetWriter {
    async fn write_all(
        &mut self,
        stream: &mut (impl tokio::io::AsyncWrite + Unpin + ?Sized),
        bytes: &[u8],
    ) -> io::Result<()> {
        if let Some(compressor) = &mut self.compressor {
            compressor.write_all(bytes)?;
            compressor.flush()?;
            let compressed = std::mem::take(&mut compressor.get_mut().0);
            stream.write_all(&compressed).await
        } else {
            stream.write_all(bytes).await
        }
    }

    fn start_compression(&mut self) {
        if self.compressor.is_none() {
            self.compressor = Some(ZlibEncoder::new(
                CompressionBuffer::default(),
                Compression::default(),
            ));
        }
    }

    async fn finish_compression(
        &mut self,
        stream: &mut (impl tokio::io::AsyncWrite + Unpin + ?Sized),
    ) -> io::Result<()> {
        let Some(mut compressor) = self.compressor.take() else {
            return Ok(());
        };
        compressor.try_finish()?;
        let compressed = std::mem::take(&mut compressor.get_mut().0);
        stream.write_all(&compressed).await
    }
}

struct TelnetConnection {
    stream: TelnetStream,
    writer: TelnetWriter,
    peer_ip: std::net::IpAddr,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutputPriority {
    Snapshot,
    Interactive,
    Critical,
}

#[derive(Debug)]
enum OutputFrame {
    Bytes {
        priority: OutputPriority,
        bytes: Vec<u8>,
    },
    StartCompression,
    StopCompression,
}

impl OutputFrame {
    fn byte_len(&self) -> usize {
        match self {
            Self::Bytes { bytes, .. } => bytes.len(),
            Self::StartCompression | Self::StopCompression => 0,
        }
    }

    fn priority(&self) -> OutputPriority {
        match self {
            Self::Bytes { priority, .. } => *priority,
            Self::StartCompression | Self::StopCompression => OutputPriority::Critical,
        }
    }
}

#[derive(Debug)]
struct OutputQueueState {
    frames: VecDeque<OutputFrame>,
    byte_len: usize,
    closed: bool,
}

#[derive(Debug)]
struct OutputQueueShared {
    state: Mutex<OutputQueueState>,
    notify: Notify,
    max_frames: usize,
    max_bytes: usize,
    metrics: Option<Arc<RuntimeMetrics>>,
}

#[derive(Clone, Debug)]
struct TelnetOutputQueue {
    shared: Arc<OutputQueueShared>,
}

impl TelnetOutputQueue {
    fn new(max_frames: usize, max_bytes: usize) -> Self {
        Self::with_metrics(max_frames, max_bytes, None)
    }

    fn with_metrics(
        max_frames: usize,
        max_bytes: usize,
        metrics: Option<Arc<RuntimeMetrics>>,
    ) -> Self {
        Self {
            shared: Arc::new(OutputQueueShared {
                state: Mutex::new(OutputQueueState {
                    frames: VecDeque::new(),
                    byte_len: 0,
                    closed: false,
                }),
                notify: Notify::new(),
                max_frames,
                max_bytes,
                metrics,
            }),
        }
    }

    fn bytes(&self, priority: OutputPriority, bytes: Vec<u8>) -> io::Result<()> {
        if bytes.len() > TELNET_OUTPUT_MAX_FRAME_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Telnet output frame exceeds its byte limit",
            ));
        }
        self.push(OutputFrame::Bytes { priority, bytes })
    }

    fn start_compression(&self) -> io::Result<()> {
        self.push(OutputFrame::StartCompression)
    }

    fn stop_compression(&self) -> io::Result<()> {
        self.push(OutputFrame::StopCompression)
    }

    fn push(&self, frame: OutputFrame) -> io::Result<()> {
        let frame_bytes = frame.byte_len();
        let frame_priority = frame.priority();
        let mut state = self.shared.state.lock().unwrap();
        let before = state.frames.len();
        if state.closed {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Telnet output writer is closed",
            ));
        }

        if frame_priority == OutputPriority::Snapshot
            && let Some(index) = state
                .frames
                .iter()
                .position(|queued| queued.priority() == OutputPriority::Snapshot)
            && let Some(stale) = state.frames.remove(index)
        {
            state.byte_len = state.byte_len.saturating_sub(stale.byte_len());
            if let Some(metrics) = &self.shared.metrics {
                metrics.output_coalesced(OutputTransport::Telnet);
            }
        }

        while state.frames.len() >= self.shared.max_frames
            || state.byte_len.saturating_add(frame_bytes) > self.shared.max_bytes
        {
            let evictable = match frame_priority {
                OutputPriority::Snapshot | OutputPriority::Interactive => state
                    .frames
                    .iter()
                    .position(|queued| queued.priority() == OutputPriority::Snapshot),
                OutputPriority::Critical => state.frames.iter().position(|queued| {
                    matches!(
                        queued.priority(),
                        OutputPriority::Snapshot | OutputPriority::Interactive
                    )
                }),
            };
            let Some(index) = evictable else {
                if let Some(metrics) = &self.shared.metrics {
                    metrics.output_dropped(OutputTransport::Telnet);
                    metrics.slow_client_disconnected(OutputTransport::Telnet);
                }
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "Telnet client output queue is full",
                ));
            };
            if let Some(evicted) = state.frames.remove(index) {
                state.byte_len = state.byte_len.saturating_sub(evicted.byte_len());
                if let Some(metrics) = &self.shared.metrics {
                    metrics.output_dropped(OutputTransport::Telnet);
                }
            }
        }

        state.byte_len = state.byte_len.saturating_add(frame_bytes);
        state.frames.push_back(frame);
        let after = state.frames.len();
        drop(state);
        if let Some(metrics) = &self.shared.metrics {
            for _ in before..after {
                metrics.output_frame_enqueued(OutputTransport::Telnet);
            }
            for _ in after..before {
                metrics.output_frame_removed(OutputTransport::Telnet);
            }
        }
        self.shared.notify.notify_one();
        Ok(())
    }

    async fn next(&self) -> Option<OutputFrame> {
        loop {
            let notified = self.shared.notify.notified();
            {
                let mut state = self.shared.state.lock().unwrap();
                if let Some(frame) = state.frames.pop_front() {
                    state.byte_len = state.byte_len.saturating_sub(frame.byte_len());
                    if let Some(metrics) = &self.shared.metrics {
                        metrics.output_frame_removed(OutputTransport::Telnet);
                    }
                    return Some(frame);
                }
                if state.closed {
                    return None;
                }
            }
            notified.await;
        }
    }

    fn close(&self) {
        let mut state = self.shared.state.lock().unwrap();
        state.closed = true;
        drop(state);
        self.shared.notify.notify_waiters();
    }
}

impl Drop for OutputQueueShared {
    fn drop(&mut self) {
        if let Some(metrics) = &self.metrics {
            let remaining = self
                .state
                .get_mut()
                .unwrap_or_else(|error| error.into_inner())
                .frames
                .len();
            for _ in 0..remaining {
                metrics.output_frame_removed(OutputTransport::Telnet);
            }
        }
    }
}

struct QueuedTelnetConnection {
    reader: ReadHalf<TelnetStream>,
    output: TelnetOutputQueue,
    writer_finished: watch::Receiver<bool>,
    writer_task: JoinHandle<io::Result<()>>,
}

impl QueuedTelnetConnection {
    async fn finish(self) -> io::Result<()> {
        self.output.close();
        self.writer_task.await.map_err(io::Error::other)?
    }
}

impl TelnetConnection {
    fn new(stream: TelnetStream, peer_ip: std::net::IpAddr) -> Self {
        Self {
            stream,
            writer: TelnetWriter::default(),
            peer_ip,
        }
    }

    async fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.writer.write_all(&mut self.stream, bytes).await
    }

    async fn start_compression(&mut self) -> io::Result<()> {
        self.stream
            .write_all(&[IAC, SB, OPT_MCCP2, IAC, SE])
            .await?;
        self.writer.start_compression();
        Ok(())
    }

    async fn finish_compression(&mut self) -> io::Result<()> {
        self.writer.finish_compression(&mut self.stream).await
    }

    async fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.stream.read(bytes).await
    }

    const fn peer_ip(&self) -> std::net::IpAddr {
        self.peer_ip
    }

    fn into_queued(self, metrics: Option<Arc<RuntimeMetrics>>) -> QueuedTelnetConnection {
        let (reader, mut writer_half) = tokio::io::split(self.stream);
        let output = metrics.map_or_else(
            || TelnetOutputQueue::new(TELNET_OUTPUT_MAX_FRAMES, TELNET_OUTPUT_MAX_BYTES),
            |metrics| {
                TelnetOutputQueue::with_metrics(
                    TELNET_OUTPUT_MAX_FRAMES,
                    TELNET_OUTPUT_MAX_BYTES,
                    Some(metrics),
                )
            },
        );
        let writer_output = output.clone();
        let (writer_finished_tx, writer_finished) = watch::channel(false);
        let mut writer = self.writer;
        let writer_task = tokio::spawn(async move {
            let result = async {
                while let Some(frame) = writer_output.next().await {
                    match frame {
                        OutputFrame::Bytes { bytes, .. } => {
                            writer.write_all(&mut writer_half, &bytes).await?;
                        }
                        OutputFrame::StartCompression => {
                            writer_half
                                .write_all(&[IAC, SB, OPT_MCCP2, IAC, SE])
                                .await?;
                            writer.start_compression();
                        }
                        OutputFrame::StopCompression => {
                            writer.finish_compression(&mut writer_half).await?;
                        }
                    }
                }
                writer.finish_compression(&mut writer_half).await
            }
            .await;
            writer_output.close();
            let _ = writer_finished_tx.send(true);
            result
        });
        QueuedTelnetConnection {
            reader,
            output,
            writer_finished,
            writer_task,
        }
    }
}

#[derive(Clone)]
pub(crate) struct TelnetRuntime {
    world: WorldHandle,
    command_rate_per_second: u32,
    command_burst: u32,
    auth_rate_limiter: Arc<AuthRateLimiter>,
    active_session_limiter: Arc<ActiveSessionLimiter>,
    metrics: Arc<RuntimeMetrics>,
}

impl TelnetRuntime {
    pub(crate) fn new(
        world: WorldHandle,
        command_rate_per_second: u32,
        command_burst: u32,
        auth_rate_limiter: Arc<AuthRateLimiter>,
        active_session_limiter: Arc<ActiveSessionLimiter>,
        metrics: Arc<RuntimeMetrics>,
    ) -> Self {
        Self {
            world,
            command_rate_per_second,
            command_burst,
            auth_rate_limiter,
            active_session_limiter,
            metrics,
        }
    }
}

pub(crate) async fn run(
    listener: TcpListener,
    runtime: TelnetRuntime,
    transport: TelnetTransport,
    mut shutdown: watch::Receiver<bool>,
) -> io::Result<()> {
    loop {
        let permit = tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return Ok(());
                }
                continue;
            }
            permit = transport.reserve_handshake() => permit?,
        };
        let (stream, address) = tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return Ok(());
                }
                continue;
            }
            accepted = listener.accept() => accepted?,
        };
        let runtime = runtime.clone();
        let mut session_shutdown = shutdown.clone();
        let transport = transport.clone();
        tokio::spawn(async move {
            let stream = tokio::select! {
                _ = session_shutdown.changed() => {
                    drop(permit);
                    return;
                }
                result = transport.upgrade(stream) => result,
            };
            drop(permit);
            let stream = match stream {
                Ok(stream) => stream,
                Err(error) => {
                    warn!(%address, %error, transport = transport.mode_name(), "Telnet transport handshake failed");
                    return;
                }
            };
            if *session_shutdown.borrow() {
                return;
            }
            let secure_transport = transport.is_tls();
            let metrics = runtime.metrics.clone();
            metrics.telnet_opened();
            if let Err(error) = serve(
                stream,
                address.ip(),
                secure_transport,
                runtime,
                session_shutdown,
            )
            .await
            {
                warn!(%address, %error, "telnet session ended with an error");
            }
            metrics.telnet_closed();
        });
    }
}

async fn serve(
    stream: TelnetStream,
    peer_ip: std::net::IpAddr,
    secure_transport: bool,
    runtime: TelnetRuntime,
    shutdown: watch::Receiver<bool>,
) -> io::Result<()> {
    let mut connection = TelnetConnection::new(stream, peer_ip);
    let mut input = TelnetInput::default();
    connection
        .write_all(&input.negotiation.begin_initial_negotiation())
        .await?;
    if !secure_transport {
        connection
            .write_all(
                b"\r\nWARNING: This Telnet connection is not encrypted. Credentials and game traffic can be intercepted.\r\n",
            )
            .await?;
    }
    connection
        .write_all(b"\r\nTiamud\r\nA world of adventures awaits.\r\n\r\n")
        .await?;

    let peer_address = connection.peer_ip();
    let Some(entered) = authenticate(
        &mut connection,
        &runtime.world,
        &mut input,
        &runtime.auth_rate_limiter,
        &runtime.metrics,
        peer_address,
    )
    .await?
    else {
        connection.finish_compression().await?;
        return Ok(());
    };
    let session_id = entered.session_id;
    let _active_session_permit = match runtime
        .active_session_limiter
        .acquire(entered.account_id, peer_address)
    {
        Ok(permit) => permit,
        Err(limit) => {
            runtime.world.leave(session_id).await;
            let message = match limit {
                ActiveSessionLimit::Account => {
                    b"This account has reached its concurrent session limit.\r\n".as_slice()
                }
                ActiveSessionLimit::Address => {
                    b"This network address has reached its concurrent session limit.\r\n".as_slice()
                }
            };
            connection.write_all(message).await?;
            connection.finish_compression().await?;
            return Ok(());
        }
    };
    runtime.metrics.session_opened();
    let mut queued = connection.into_queued(Some(runtime.metrics.clone()));
    let result = serve_entered(
        EnteredTelnetIo {
            reader: &mut queued.reader,
            output: &queued.output,
            writer_finished: &mut queued.writer_finished,
            input: &mut input,
        },
        &runtime.world,
        entered,
        runtime.command_rate_per_second,
        runtime.command_burst,
        shutdown,
    )
    .await;

    info!(session = session_id.0, "telnet client disconnected");
    runtime.world.leave(session_id).await;
    runtime.metrics.session_closed();
    let compression_result = queued.finish().await;
    result.and(compression_result)
}

async fn authenticate(
    connection: &mut TelnetConnection,
    world: &WorldHandle,
    input: &mut TelnetInput,
    auth_rate_limiter: &AuthRateLimiter,
    metrics: &RuntimeMetrics,
    peer_address: std::net::IpAddr,
) -> io::Result<Option<crate::world_actor::EnteredWorld>> {
    for _ in 0..MAX_AUTH_ATTEMPTS {
        connection.write_all(b"Access [login/register]: ").await?;
        let Some(mode) = read_required_line(connection, input).await? else {
            return Ok(None);
        };
        if mode.eq_ignore_ascii_case("quit") {
            connection.write_all(b"Goodbye.\r\n").await?;
            return Ok(None);
        }
        if !mode.eq_ignore_ascii_case("login") && !mode.eq_ignore_ascii_case("register") {
            connection
                .write_all(b"Please type login, register, or quit.\r\n")
                .await?;
            continue;
        }

        connection.write_all(b"Account: ").await?;
        let Some(account) = read_required_line(connection, input).await? else {
            return Ok(None);
        };
        connection.write_all(&input.set_server_echo(true)).await?;
        connection.write_all(b"Password: ").await?;
        let password = read_required_line(connection, input).await?;
        connection.write_all(&input.set_server_echo(false)).await?;
        connection.write_all(b"\r\n").await?;
        let Some(password) = password else {
            return Ok(None);
        };

        if !auth_rate_limiter.check(peer_address, Instant::now()) {
            metrics.auth_rate_limited();
            connection
                .write_all(b"Too many authentication attempts. Try again later.\r\n")
                .await?;
            return Ok(None);
        }

        let authenticated = if mode.eq_ignore_ascii_case("register") {
            world.register_account(account, password).await
        } else {
            world.login_account(account, password).await
        };
        let authenticated = match authenticated {
            Ok(authenticated) => authenticated,
            Err(error) => {
                write_auth_error(connection, &error).await?;
                continue;
            }
        };
        let token = authenticated.token.as_str().to_owned();
        let character = match choose_character(connection, world, input, &token).await? {
            Some(character) => character,
            None => return Ok(None),
        };
        return match world
            .enter_authenticated_character(token, character.id)
            .await
        {
            Ok(entered) => Ok(Some(entered)),
            Err(error) => {
                connection
                    .write_all(
                        format!("Could not enter Tiamud: {}\r\n", public_error(&error)).as_bytes(),
                    )
                    .await?;
                Ok(None)
            }
        };
    }
    connection
        .write_all(b"Too many failed access attempts. Please reconnect later.\r\n")
        .await?;
    Ok(None)
}

async fn choose_character(
    connection: &mut TelnetConnection,
    world: &WorldHandle,
    input: &mut TelnetInput,
    token: &str,
) -> io::Result<Option<CharacterSnapshot>> {
    let roster = world
        .authenticated_roster(token.to_owned())
        .await
        .map_err(io::Error::other)?;
    if roster.is_empty() {
        connection.write_all(b"Name your first character: ").await?;
        let Some(name) = read_required_line(connection, input).await? else {
            return Ok(None);
        };
        return world
            .create_authenticated_character(token.to_owned(), name)
            .await
            .map(Some)
            .map_err(io::Error::other);
    }

    connection.write_all(b"Characters:\r\n").await?;
    for (index, character) in roster.iter().enumerate() {
        connection
            .write_all(format!("  {}. {}\r\n", index + 1, character.name).as_bytes())
            .await?;
    }
    connection
        .write_all(b"Choose a number, or type a new character name: ")
        .await?;
    let Some(selection) = read_required_line(connection, input).await? else {
        return Ok(None);
    };
    if let Ok(index) = selection.parse::<usize>()
        && let Some(character) = index.checked_sub(1).and_then(|index| roster.get(index))
    {
        return Ok(Some(character.clone()));
    }
    world
        .create_authenticated_character(token.to_owned(), selection)
        .await
        .map(Some)
        .map_err(io::Error::other)
}

async fn write_auth_error(connection: &mut TelnetConnection, error: &GameError) -> io::Result<()> {
    let message = match error {
        GameError::Persistence(
            StoreError::InvalidName { .. }
            | StoreError::InvalidPassword
            | StoreError::AccountAlreadyExists
            | StoreError::InvalidCredentials,
        ) => error.to_string(),
        _ => "The account service is temporarily unavailable.".to_owned(),
    };
    connection.write_all(message.as_bytes()).await?;
    connection.write_all(b"\r\n").await
}

fn public_error(error: &GameError) -> String {
    match error {
        GameError::World(
            error @ (tiamud_core::WorldError::NoExit
            | tiamud_core::WorldError::ItemNotFound(_)
            | tiamud_core::WorldError::AmbiguousItem { .. }),
        ) => error.to_string(),
        GameError::Persistence(
            StoreError::InvalidSession | StoreError::SessionExpired | StoreError::SessionRevoked,
        ) => "Your account session is no longer valid.".to_owned(),
        _ => "The world is temporarily unavailable.".to_owned(),
    }
}

async fn read_required_line(
    connection: &mut TelnetConnection,
    input: &mut TelnetInput,
) -> io::Result<Option<String>> {
    loop {
        match input.read_line(connection).await? {
            Some(LineEvent::Command(line)) => {
                let line = String::from_utf8_lossy(&line).trim().to_owned();
                if !line.is_empty() {
                    return Ok(Some(line));
                }
                connection.write_all(b"Please enter a value: ").await?;
            }
            Some(LineEvent::TooLong) => {
                connection.write_all(b"Input too long; try again: ").await?;
            }
            None => return Ok(None),
        }
    }
}

struct EnteredTelnetIo<'a> {
    reader: &'a mut ReadHalf<TelnetStream>,
    output: &'a TelnetOutputQueue,
    writer_finished: &'a mut watch::Receiver<bool>,
    input: &'a mut TelnetInput,
}

async fn serve_entered(
    io: EnteredTelnetIo<'_>,
    world: &WorldHandle,
    entered: crate::world_actor::EnteredWorld,
    command_rate_per_second: u32,
    command_burst: u32,
    mut shutdown: watch::Receiver<bool>,
) -> io::Result<()> {
    let EnteredTelnetIo {
        reader,
        output,
        writer_finished,
        input,
    } = io;
    write_scene(output, input, &entered.scene, &entered.player)?;
    write_prompt(output, input)?;
    let mut last_revision = entered.scene.revision;

    let mut social = world.subscribe_social();
    let mut world_updates = world.subscribe_world();
    let mut command_limiter =
        CommandRateLimiter::per_second(command_rate_per_second, command_burst, Instant::now());
    let mut rate_violations = 0_u8;
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    output.bytes(
                        OutputPriority::Critical,
                        b"\r\nTiamud is shutting down safely. Please reconnect shortly.\r\n".to_vec(),
                    )?;
                    return Ok(());
                }
            }
            changed = writer_finished.changed() => {
                if changed.is_err() || *writer_finished.borrow() {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "Telnet output writer stopped",
                    ));
                }
            }
            delivery = social.recv() => {
                match delivery {
                    Ok(delivery) if delivery.recipient == entered.character_id => {
                        write_social_message(output, input, &delivery.message)?;
                        write_prompt(output, input)?;
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        output.bytes(
                            OutputPriority::Critical,
                            b"\r\nSome chat messages were skipped because this client is too slow.\r\n".to_vec(),
                        )?;
                        write_prompt(output, input)?;
                    }
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                }
            }
            delivery = world_updates.recv() => {
                match delivery {
                    Ok(delivery) if delivery.session_id == entered.session_id => {
                        if !delivery.events.is_empty() {
                            let mut bytes = Vec::new();
                            for event in delivery.events {
                                bytes.extend_from_slice(b"\r\n");
                                bytes.extend_from_slice(event.text.as_bytes());
                                bytes.extend_from_slice(b"\r\n");
                            }
                            output.bytes(OutputPriority::Interactive, bytes)?;
                        }
                        if delivery.scene.revision <= last_revision {
                            continue;
                        }
                        last_revision = delivery.scene.revision;
                        write_scene(output, input, &delivery.scene, &delivery.player)?;
                        write_prompt(output, input)?;
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        output.bytes(
                            OutputPriority::Critical,
                            b"\r\nWorld updates were coalesced; an authoritative snapshot follows.\r\n".to_vec(),
                        )?;
                        let (scene, player) = world
                            .current_state(entered.session_id)
                            .await
                            .map_err(io::Error::other)?;
                        last_revision = scene.revision;
                        write_scene(output, input, &scene, &player)?;
                        write_prompt(output, input)?;
                    }
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                }
            }
            event = input.read_line_queued(reader, output) => {
                let Some(event) = event? else {
                    return Ok(());
                };
                match event {
                    LineEvent::TooLong => {
                        output.bytes(
                            OutputPriority::Interactive,
                            b"Command too long.\r\n".to_vec(),
                        )?;
                        write_prompt(output, input)?;
                    }
                    LineEvent::Command(line) => {
                        let command = String::from_utf8_lossy(&line).trim().to_owned();
                        if command.eq_ignore_ascii_case("quit") {
                            output.bytes(OutputPriority::Critical, b"Goodbye.\r\n".to_vec())?;
                            return Ok(());
                        }
                        if !command_limiter.check(Instant::now()) {
                            rate_violations += 1;
                            output.bytes(
                                OutputPriority::Interactive,
                                b"Command rate limit exceeded.\r\n".to_vec(),
                            )?;
                            if rate_violations >= MAX_RATE_VIOLATIONS {
                                output.bytes(
                                    OutputPriority::Critical,
                                    b"Too many commands; reconnect after a short pause.\r\n".to_vec(),
                                )?;
                                return Ok(());
                            }
                            write_prompt(output, input)?;
                            continue;
                        }
                        rate_violations = 0;
                        match parse_action(&command) {
                            Ok(action) => match world.apply(entered.session_id, action).await {
                                Ok(outcome) => {
                                    last_revision = outcome.scene.revision;
                                    if let Some(text) = outcome.output {
                                        let mut bytes = text.into_bytes();
                                        bytes.extend_from_slice(b"\r\n");
                                        output_queue_bytes(output, bytes)?;
                                    }
                                    write_scene(output, input, &outcome.scene, &outcome.player)?;
                                }
                                Err(error) => {
                                    output.bytes(
                                        OutputPriority::Interactive,
                                        format!("{}\r\n", public_error(&error)).into_bytes(),
                                    )?;
                                }
                            },
                            Err(error) => {
                                output.bytes(
                                    OutputPriority::Interactive,
                                    format!("{error}\r\n").into_bytes(),
                                )?;
                            }
                        }
                        write_prompt(output, input)?;
                    }
                }
            }
        }
    }
}

fn output_queue_bytes(output: &TelnetOutputQueue, bytes: Vec<u8>) -> io::Result<()> {
    output.bytes(OutputPriority::Interactive, bytes)
}

fn write_social_message(
    output: &TelnetOutputQueue,
    input: &TelnetInput,
    message: &SocialMessage,
) -> io::Result<()> {
    let channel = match message.channel {
        SocialChannel::Say => "say",
        SocialChannel::Tell => "tell",
    };
    let mut bytes =
        format!("\r\n[{channel}] {}: {}\r\n", message.sender, message.text).into_bytes();
    let gmcp = input.negotiation.social_gmcp(message);
    if !gmcp.is_empty() {
        bytes.extend_from_slice(&gmcp);
    }
    output.bytes(OutputPriority::Interactive, bytes)
}

fn write_scene(
    output: &TelnetOutputQueue,
    input: &mut TelnetInput,
    scene: &SceneView,
    player: &PlayerStateView,
) -> io::Result<()> {
    let mut bytes = render_scene(scene, &input.negotiation).into_bytes();
    let gmcp = input.negotiation.scene_gmcp(scene);
    let media_transition = gmcp
        .windows(b"Client.Media.".len())
        .any(|window| window == b"Client.Media.");
    if !gmcp.is_empty() {
        bytes.extend_from_slice(&gmcp);
    }
    let player_gmcp = input.negotiation.player_gmcp(player);
    if !player_gmcp.is_empty() {
        bytes.extend_from_slice(&player_gmcp);
    }
    if let Some(cell_frame) = render_scene_cell_frame(scene, &input.negotiation) {
        bytes.extend_from_slice(&cell_frame);
    }
    output.bytes(
        if media_transition {
            OutputPriority::Critical
        } else {
            OutputPriority::Snapshot
        },
        bytes,
    )
}

fn write_prompt(output: &TelnetOutputQueue, input: &TelnetInput) -> io::Result<()> {
    let mut bytes = b"> ".to_vec();
    if input.negotiation.local_enabled(OPT_EOR) {
        bytes.extend_from_slice(&[IAC, EOR_COMMAND]);
    }
    output.bytes(OutputPriority::Interactive, bytes)
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum QState {
    #[default]
    No,
    Yes,
    WantNo,
    WantNoQueued,
    WantYes,
    WantYesQueued,
}

impl QState {
    const fn enabled(self) -> bool {
        matches!(self, Self::Yes)
    }

    /// Apply a local request to change one side of an option. The returned
    /// boolean is the request to put on the wire (`true` for enable). At most
    /// the opposite of an in-flight request is queued, as prescribed by RFC
    /// 1143; repeated requests never generate repeated negotiation traffic.
    fn request(&mut self, enable: bool) -> Option<bool> {
        match (enable, *self) {
            (true, Self::No) => {
                *self = Self::WantYes;
                Some(true)
            }
            (true, Self::WantNo) => {
                *self = Self::WantNoQueued;
                None
            }
            (true, Self::WantYesQueued) => {
                *self = Self::WantYes;
                None
            }
            (false, Self::Yes) => {
                *self = Self::WantNo;
                Some(false)
            }
            (false, Self::WantNoQueued) => {
                *self = Self::WantNo;
                None
            }
            (false, Self::WantYes) => {
                *self = Self::WantYesQueued;
                None
            }
            _ => None,
        }
    }

    /// Apply an incoming positive (`WILL`/`DO`) or negative (`WONT`/`DONT`)
    /// negotiation. `accept` is consulted only for a new unsolicited enable.
    /// The returned boolean is the acknowledgement or queued request to send.
    fn receive(&mut self, positive: bool, accept: bool) -> Option<bool> {
        if positive {
            match *self {
                Self::No if accept => {
                    *self = Self::Yes;
                    Some(true)
                }
                Self::No => Some(false),
                Self::Yes => None,
                Self::WantNo => {
                    // A non-compliant peer answered a disable with enable. Do
                    // not feed the disagreement back into a negotiation loop.
                    *self = Self::No;
                    None
                }
                Self::WantNoQueued => {
                    *self = Self::Yes;
                    None
                }
                Self::WantYes => {
                    *self = Self::Yes;
                    None
                }
                Self::WantYesQueued => {
                    *self = Self::WantNo;
                    Some(false)
                }
            }
        } else {
            match *self {
                Self::No => None,
                Self::Yes => {
                    *self = Self::No;
                    Some(false)
                }
                Self::WantNo => {
                    *self = Self::No;
                    None
                }
                Self::WantNoQueued => {
                    *self = Self::WantYes;
                    Some(true)
                }
                Self::WantYes | Self::WantYesQueued => {
                    *self = Self::No;
                    None
                }
            }
        }
    }
}

#[derive(Debug)]
struct QMethod {
    local: [QState; 256],
    remote: [QState; 256],
}

impl Default for QMethod {
    fn default() -> Self {
        Self {
            local: [QState::No; 256],
            remote: [QState::No; 256],
        }
    }
}

impl QMethod {
    fn request_local(&mut self, option: u8, enable: bool, output: &mut Vec<u8>) {
        if let Some(positive) = self.local[usize::from(option)].request(enable) {
            append_negotiation(output, if positive { WILL } else { WONT }, option);
        }
    }

    fn request_remote(&mut self, option: u8, enable: bool, output: &mut Vec<u8>) {
        if let Some(positive) = self.remote[usize::from(option)].request(enable) {
            append_negotiation(output, if positive { DO } else { DONT }, option);
        }
    }

    fn receive_local(&mut self, command: u8, option: u8, accept: bool, output: &mut Vec<u8>) {
        let positive = command == DO;
        if let Some(reply) = self.local[usize::from(option)].receive(positive, accept) {
            append_negotiation(output, if reply { WILL } else { WONT }, option);
        }
    }

    fn receive_remote(&mut self, command: u8, option: u8, accept: bool, output: &mut Vec<u8>) {
        let positive = command == WILL;
        if let Some(reply) = self.remote[usize::from(option)].receive(positive, accept) {
            append_negotiation(output, if reply { DO } else { DONT }, option);
        }
    }

    fn local_enabled(&self, option: u8) -> bool {
        self.local[usize::from(option)].enabled()
    }

    fn remote_enabled(&self, option: u8) -> bool {
        self.remote[usize::from(option)].enabled()
    }
}

fn append_negotiation(output: &mut Vec<u8>, command: u8, option: u8) {
    output.extend_from_slice(&[IAC, command, option]);
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum ColorPolicy {
    #[default]
    Plain,
    Ansi16,
    Ansi256,
    TrueColor,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PresentationPolicy {
    utf8: bool,
    color: ColorPolicy,
    screen_reader: bool,
    clickable_links: bool,
    decorative_cell_art: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct MttsCapabilities {
    bitvector: u16,
    ansi: bool,
    vt100: bool,
    utf8: bool,
    colors_256: bool,
    mouse_tracking: bool,
    osc_color_palette: bool,
    screen_reader: bool,
    proxy: bool,
    truecolor: bool,
    mnes: bool,
    mslp: bool,
    tls: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum LinkMode {
    #[default]
    Plain,
    Mslp,
    Mxp,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum GraphicsMode {
    #[default]
    None,
    Ansi,
    Kitty,
    Sixel,
    Iterm2,
}

#[derive(Debug, Eq, PartialEq)]
struct GmcpClient {
    name: Option<String>,
    version: Option<String>,
    supports: BTreeSet<String>,
    media_opt_in: bool,
    assets_opt_in: bool,
    links: LinkMode,
    graphics: GraphicsMode,
    plain_mode: bool,
    reduced_motion: bool,
    screen_reader: bool,
    decorative_enabled: bool,
    comm_stub_sent: bool,
    group_stub_sent: bool,
    active_media: Option<SceneAudio>,
}

impl Default for GmcpClient {
    fn default() -> Self {
        Self {
            name: None,
            version: None,
            supports: BTreeSet::new(),
            media_opt_in: false,
            assets_opt_in: false,
            links: LinkMode::Plain,
            graphics: GraphicsMode::None,
            plain_mode: false,
            reduced_motion: false,
            screen_reader: false,
            decorative_enabled: true,
            comm_stub_sent: false,
            group_stub_sent: false,
            active_media: None,
        }
    }
}

impl GmcpClient {
    fn supports(&self, package: &str) -> bool {
        let package = package.to_ascii_lowercase();
        self.supports.iter().any(|supported| {
            supported == &package
                || supported
                    .strip_prefix(&package)
                    .is_some_and(|suffix| suffix.starts_with('.'))
        })
    }

    fn blocks_rich_output(&self) -> bool {
        self.plain_mode || self.reduced_motion || self.screen_reader || !self.decorative_enabled
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GmcpHello {
    #[serde(default)]
    client: Option<String>,
    #[serde(default)]
    version: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TiamudClientSet {
    #[serde(default)]
    media: bool,
    #[serde(default)]
    assets: bool,
    #[serde(default)]
    links: Option<String>,
    #[serde(default)]
    graphics: Option<String>,
    #[serde(default)]
    plain: bool,
    #[serde(default, rename = "reducedMotion")]
    reduced_motion: bool,
    #[serde(default, rename = "screenReader")]
    screen_reader: bool,
    #[serde(default)]
    decorative: Option<bool>,
}

impl MttsCapabilities {
    const fn from_bitvector(bitvector: u16) -> Self {
        Self {
            bitvector,
            ansi: bitvector & MTTS_ANSI != 0,
            vt100: bitvector & MTTS_VT100 != 0,
            utf8: bitvector & MTTS_UTF8 != 0,
            colors_256: bitvector & MTTS_256_COLORS != 0,
            mouse_tracking: bitvector & MTTS_MOUSE_TRACKING != 0,
            osc_color_palette: bitvector & MTTS_OSC_COLOR_PALETTE != 0,
            screen_reader: bitvector & MTTS_SCREEN_READER != 0,
            proxy: bitvector & MTTS_PROXY != 0,
            truecolor: bitvector & MTTS_TRUECOLOR != 0,
            mnes: bitvector & MTTS_MNES != 0,
            mslp: bitvector & MTTS_MSLP != 0,
            tls: bitvector & MTTS_TLS != 0,
        }
    }
}

#[derive(Debug, Default, Eq, PartialEq)]
struct ClientCapabilities {
    client_name: Option<String>,
    terminal_type: Option<String>,
    terminal_types: Vec<String>,
    mtts: Option<MttsCapabilities>,
    columns: Option<u16>,
    rows: Option<u16>,
    charset: Option<String>,
    presentation: PresentationPolicy,
}

impl ClientCapabilities {
    fn refresh_presentation_policy(&mut self) {
        let mtts = self.mtts.unwrap_or_default();
        let charset_is_utf8 = self
            .charset
            .as_deref()
            .is_some_and(|charset| charset.eq_ignore_ascii_case("UTF-8"));
        let screen_reader = mtts.screen_reader;
        let color = if screen_reader {
            ColorPolicy::Plain
        } else if mtts.truecolor {
            ColorPolicy::TrueColor
        } else if mtts.colors_256 {
            ColorPolicy::Ansi256
        } else if mtts.ansi || mtts.vt100 {
            ColorPolicy::Ansi16
        } else {
            ColorPolicy::Plain
        };
        self.presentation = PresentationPolicy {
            utf8: mtts.utf8 || charset_is_utf8,
            color,
            screen_reader,
            clickable_links: mtts.mslp && !screen_reader,
            // A TTYPE such as KITTY, XTERM, or ITERM is not consent to native
            // graphics. A future explicit opt-in protocol must set this.
            decorative_cell_art: false,
        };
    }

    fn reset_ttype(&mut self) {
        self.client_name = None;
        self.terminal_type = None;
        self.terminal_types.clear();
        self.mtts = None;
        self.refresh_presentation_policy();
    }
}

#[derive(Default)]
struct TelnetSession {
    capabilities: ClientCapabilities,
    gmcp: GmcpClient,
    q: QMethod,
    allow_server_echo: bool,
    charset_requested: bool,
    ttype_requests_sent: u8,
    ttype_awaiting_response: bool,
    last_ttype_response: Option<String>,
    mccp_started: bool,
    mccp_start_requested: bool,
    mccp_stop_requested: bool,
}

impl TelnetSession {
    fn begin_initial_negotiation(&mut self) -> Vec<u8> {
        let mut output = Vec::new();
        self.q
            .request_local(OPT_SUPPRESS_GO_AHEAD, true, &mut output);
        self.q.request_local(OPT_EOR, true, &mut output);
        self.q.request_remote(OPT_TTYPE, true, &mut output);
        self.q.request_remote(OPT_NAWS, true, &mut output);
        self.q.request_remote(OPT_CHARSET, true, &mut output);
        self.q.request_local(OPT_GMCP, true, &mut output);
        self.q.request_local(OPT_MSSP, true, &mut output);
        self.q.request_local(OPT_MCCP2, true, &mut output);
        output
    }

    fn request_server_echo(&mut self, enabled: bool) -> Vec<u8> {
        self.allow_server_echo = enabled;
        let mut output = Vec::new();
        self.q.request_local(OPT_ECHO, enabled, &mut output);
        output
    }

    fn local_enabled(&self, option: u8) -> bool {
        self.q.local_enabled(option)
    }

    fn take_mccp_start_requested(&mut self) -> bool {
        std::mem::take(&mut self.mccp_start_requested)
    }

    fn take_mccp_stop_requested(&mut self) -> bool {
        std::mem::take(&mut self.mccp_stop_requested)
    }

    fn handle(&mut self, events: Vec<TelnetEvent>) -> Vec<u8> {
        let mccp_was_started = self.mccp_started;
        let media_was_enabled = self.gmcp.media_opt_in;
        self.mccp_start_requested = false;
        self.mccp_stop_requested = false;
        let mut response = Vec::new();
        for event in events {
            match event {
                TelnetEvent::Negotiation { command, option } => {
                    self.handle_negotiation(command, option, &mut response);
                }
                TelnetEvent::Subnegotiation { option, data } => {
                    self.handle_subnegotiation(option, &data, &mut response);
                }
                TelnetEvent::SubnegotiationTooLong { option } => {
                    warn!(option, "discarded oversized Telnet subnegotiation");
                }
            }
        }
        let mccp_is_enabled = self.q.local_enabled(OPT_MCCP2);
        self.mccp_start_requested = !mccp_was_started && mccp_is_enabled;
        self.mccp_stop_requested = mccp_was_started && !mccp_is_enabled;
        self.mccp_started = mccp_is_enabled;
        self.revalidate_rich_opt_ins();
        self.stop_media_after_opt_out(media_was_enabled, &mut response);
        self.refresh_decorative_policy();
        response
    }

    fn handle_negotiation(&mut self, command: u8, option: u8, response: &mut Vec<u8>) {
        let ttype_was_enabled = self.q.remote_enabled(OPT_TTYPE);
        let charset_was_enabled = self.q.remote_enabled(OPT_CHARSET);
        let naws_was_enabled = self.q.remote_enabled(OPT_NAWS);
        let gmcp_was_enabled = self.q.local_enabled(OPT_GMCP);
        let mssp_was_enabled = self.q.local_enabled(OPT_MSSP);

        match command {
            WILL | WONT => self.q.receive_remote(
                command,
                option,
                matches!(option, OPT_TTYPE | OPT_NAWS | OPT_CHARSET),
                response,
            ),
            DO | DONT => self.q.receive_local(
                command,
                option,
                matches!(option, OPT_SUPPRESS_GO_AHEAD | OPT_EOR)
                    || matches!(option, OPT_GMCP | OPT_MSSP | OPT_MCCP2)
                    || (option == OPT_ECHO && self.allow_server_echo)
                    || (option == OPT_MXP && self.gmcp.links == LinkMode::Mxp),
                response,
            ),
            _ => return,
        }

        let ttype_is_enabled = self.q.remote_enabled(OPT_TTYPE);
        if !ttype_was_enabled && ttype_is_enabled {
            self.reset_ttype_discovery();
            self.request_next_ttype(response);
        } else if ttype_was_enabled && !ttype_is_enabled {
            self.reset_ttype_discovery();
        }

        let charset_is_enabled = self.q.remote_enabled(OPT_CHARSET);
        if !charset_was_enabled && charset_is_enabled {
            if !self.charset_requested {
                self.charset_requested = true;
                append_charset_request(response);
            }
        } else if charset_was_enabled && !charset_is_enabled {
            self.charset_requested = false;
            self.capabilities.charset = None;
            self.capabilities.refresh_presentation_policy();
        }

        if naws_was_enabled && !self.q.remote_enabled(OPT_NAWS) {
            self.capabilities.columns = None;
            self.capabilities.rows = None;
        }

        let gmcp_is_enabled = self.q.local_enabled(OPT_GMCP);
        if !gmcp_was_enabled && gmcp_is_enabled {
            append_initial_gmcp(response);
        } else if gmcp_was_enabled && !gmcp_is_enabled {
            self.gmcp = GmcpClient::default();
        }

        if !mssp_was_enabled && self.q.local_enabled(OPT_MSSP) {
            append_mssp(response);
        }
    }

    fn handle_subnegotiation(&mut self, option: u8, data: &[u8], response: &mut Vec<u8>) {
        match option {
            OPT_TTYPE if data.first() == Some(&TTYPE_IS) => {
                self.handle_ttype_response(&data[1..], response);
            }
            OPT_NAWS if self.q.remote_enabled(OPT_NAWS) && data.len() == 4 => {
                let columns = u16::from_be_bytes([data[0], data[1]]);
                let rows = u16::from_be_bytes([data[2], data[3]]);
                if columns > 0 && rows > 0 {
                    self.capabilities.columns = Some(columns);
                    self.capabilities.rows = Some(rows);
                }
            }
            OPT_CHARSET
                if self.q.remote_enabled(OPT_CHARSET)
                    && data.first() == Some(&CHARSET_ACCEPTED) =>
            {
                self.capabilities.charset = sanitize_token(&data[1..], 32);
                self.capabilities.refresh_presentation_policy();
            }
            OPT_CHARSET
                if self.q.remote_enabled(OPT_CHARSET)
                    && data.first() == Some(&CHARSET_REJECTED) =>
            {
                self.capabilities.charset = None;
                self.capabilities.refresh_presentation_policy();
            }
            OPT_GMCP if self.q.local_enabled(OPT_GMCP) => {
                self.handle_gmcp(data, response);
            }
            _ => {}
        }
        debug!(?self.capabilities, "updated Telnet client capabilities");
    }

    fn handle_gmcp(&mut self, bytes: &[u8], response: &mut Vec<u8>) {
        let Some((package, data)) = parse_gmcp(bytes) else {
            return;
        };
        match package.as_str() {
            "core.hello" => {
                let Ok(hello) = serde_json::from_value::<GmcpHello>(data) else {
                    return;
                };
                self.gmcp.name = hello
                    .client
                    .and_then(|value| sanitize_text(&value, MAX_GMCP_CLIENT_VALUE_BYTES));
                self.gmcp.version = hello
                    .version
                    .and_then(|value| sanitize_text(&value, MAX_GMCP_CLIENT_VALUE_BYTES));
            }
            "core.supports.set" | "core.supports.add" => {
                let was_media_enabled = self.gmcp.media_opt_in;
                let Some(packages) = parse_gmcp_supports(&data) else {
                    return;
                };
                if package == "core.supports.set" {
                    self.gmcp.supports.clear();
                    self.gmcp.comm_stub_sent = false;
                    self.gmcp.group_stub_sent = false;
                }
                for package in packages {
                    if self.gmcp.supports.len() == MAX_GMCP_SUPPORTS {
                        break;
                    }
                    self.gmcp.supports.insert(package);
                }
                self.append_supported_stubs(response);
                self.revalidate_rich_opt_ins();
                self.stop_media_after_opt_out(was_media_enabled, response);
            }
            "core.supports.remove" => {
                let was_media_enabled = self.gmcp.media_opt_in;
                let Some(packages) = parse_gmcp_supports(&data) else {
                    return;
                };
                for package in packages {
                    self.gmcp.supports.remove(&package);
                }
                self.revalidate_rich_opt_ins();
                self.stop_media_after_opt_out(was_media_enabled, response);
            }
            "tiamud.client.set" => {
                let was_media_enabled = self.gmcp.media_opt_in;
                let Ok(settings) = serde_json::from_value::<TiamudClientSet>(data) else {
                    return;
                };
                self.gmcp.plain_mode = settings.plain;
                self.gmcp.reduced_motion = settings.reduced_motion;
                self.gmcp.screen_reader = settings.screen_reader;
                self.gmcp.decorative_enabled = settings.decorative.unwrap_or(true);
                self.gmcp.graphics = if self.gmcp.supports("tiamud.cellframe") {
                    match settings.graphics.as_deref() {
                        Some("ansi") => GraphicsMode::Ansi,
                        Some("kitty") => GraphicsMode::Kitty,
                        Some("sixel") => GraphicsMode::Sixel,
                        Some("iterm2") => GraphicsMode::Iterm2,
                        _ => GraphicsMode::None,
                    }
                } else {
                    GraphicsMode::None
                };
                self.gmcp.media_opt_in = settings.media && self.gmcp.supports("client.media");
                self.gmcp.assets_opt_in = settings.assets && self.gmcp.supports("tiamud.assets");
                self.gmcp.links = match settings.links.as_deref() {
                    Some("mslp") => LinkMode::Mslp,
                    Some("mxp") => LinkMode::Mxp,
                    _ => LinkMode::Plain,
                };
                self.revalidate_rich_opt_ins();
                self.stop_media_after_opt_out(was_media_enabled, response);
                self.q
                    .request_local(OPT_MXP, self.gmcp.links == LinkMode::Mxp, response);
            }
            _ => {}
        }
    }

    fn stop_media_after_opt_out(&mut self, was_enabled: bool, response: &mut Vec<u8>) {
        if was_enabled
            && !self.gmcp.media_opt_in
            && let Some(active) = self.gmcp.active_media.take()
            && safe_audio_channel(&active.channel)
        {
            append_gmcp(
                response,
                "Client.Media.Stop",
                &json!({"channel": active.channel}),
            );
        }
    }

    fn append_supported_stubs(&mut self, response: &mut Vec<u8>) {
        if self.gmcp.supports("comm") && !self.gmcp.comm_stub_sent {
            append_gmcp(
                response,
                "Comm.Channel.List",
                &json!({"channels": ["say", "tell"]}),
            );
            self.gmcp.comm_stub_sent = true;
        }
        if self.gmcp.supports("group") && !self.gmcp.group_stub_sent {
            append_gmcp(response, "Group.Info", &json!({"members": []}));
            self.gmcp.group_stub_sent = true;
        }
    }

    fn scene_gmcp(&mut self, scene: &SceneView) -> Vec<u8> {
        if !self.q.local_enabled(OPT_GMCP) {
            return Vec::new();
        }
        let mut output = Vec::new();
        let exits = scene
            .exits
            .iter()
            .map(|exit| (exit.command.clone(), Value::String(exit.id.clone())))
            .collect::<BTreeMap<_, _>>();
        append_gmcp(
            &mut output,
            "Room.Info",
            &json!({
                "num": scene.view_id,
                "name": scene.title,
                "zone": "Tiamud",
                "revision": scene.revision,
                "exits": exits,
            }),
        );
        append_gmcp(&mut output, "Tiamud.Scene", &json!(scene));
        if self.gmcp.assets_opt_in
            && !self.rich_output_blocked()
            && let Some(image) = &scene.image
            && safe_scene_image(image)
        {
            append_gmcp(
                &mut output,
                "Tiamud.Assets",
                &json!({"viewId": scene.view_id, "image": image}),
            );
        }
        let next_media = scene.audio.as_ref().filter(|audio| safe_scene_audio(audio));
        if self.gmcp.media_opt_in
            && !self.rich_output_blocked()
            && self.gmcp.active_media.as_ref() != next_media
        {
            if let Some(active) = self.gmcp.active_media.take()
                && safe_audio_channel(&active.channel)
            {
                append_gmcp(
                    &mut output,
                    "Client.Media.Stop",
                    &json!({"channel": active.channel}),
                );
            }
            if let Some(audio) = next_media {
                append_gmcp(
                    &mut output,
                    "Client.Media.Play",
                    &json!({
                        "channel": audio.channel,
                        "url": audio.url,
                        "loop": audio.looped,
                    }),
                );
                self.gmcp.active_media = Some(audio.clone());
            }
        }
        output
    }

    fn player_gmcp(&self, player: &PlayerStateView) -> Vec<u8> {
        if !self.q.local_enabled(OPT_GMCP) {
            return Vec::new();
        }
        let mut output = Vec::new();
        if self.gmcp.supports("char.vitals") {
            append_gmcp(
                &mut output,
                "Char.Vitals",
                &json!({
                    "hp": player.character.health,
                    "maxhp": player.character.max_health,
                    "level": player.character.level,
                    "xp": player.character.experience,
                    "tnl": player.character.next_level_experience.saturating_sub(player.character.experience),
                    "strength": player.character.strength,
                    "defence": player.character.defence,
                    "skills": player.character.skills,
                    "revision": player.revision,
                }),
            );
        }
        if self.gmcp.supports("char.items") {
            let inventory = player
                .inventory
                .iter()
                .map(|item| json!({"id": item.id, "name": item.label}))
                .collect::<Vec<_>>();
            append_gmcp(
                &mut output,
                "Char.Items.List",
                &json!({
                    "location": "inv",
                    "items": inventory,
                    "revision": player.revision,
                }),
            );
            let equipment = player
                .equipment
                .iter()
                .map(|item| json!({"id": item.id, "name": item.label, "slot": item.slot}))
                .collect::<Vec<_>>();
            append_gmcp(
                &mut output,
                "Char.Items.List",
                &json!({
                    "location": "worn",
                    "items": equipment,
                    "revision": player.revision,
                }),
            );
        }
        if self.gmcp.supports("group") {
            let members = player.party.as_ref().map_or_else(Vec::new, |party| {
                party
                    .members
                    .iter()
                    .map(|member| {
                        json!({
                            "id": member.id,
                            "name": member.name,
                            "leader": member.leader,
                            "online": member.online,
                        })
                    })
                    .collect::<Vec<_>>()
            });
            append_gmcp(
                &mut output,
                "Group.Info",
                &json!({"members": members, "revision": player.revision}),
            );
        }
        output
    }

    fn social_gmcp(&self, message: &SocialMessage) -> Vec<u8> {
        if !self.q.local_enabled(OPT_GMCP) || !self.gmcp.supports("comm") {
            return Vec::new();
        }
        let channel = match message.channel {
            SocialChannel::Say => "say",
            SocialChannel::Tell => "tell",
        };
        let mut output = Vec::new();
        append_gmcp(
            &mut output,
            "Comm.Channel.Text",
            &json!({
                "channel": channel,
                "talker": message.sender,
                "text": message.text,
            }),
        );
        output
    }

    fn negotiated_link_mode(&self) -> LinkMode {
        if self.rich_output_blocked() {
            return LinkMode::Plain;
        }
        match self.gmcp.links {
            LinkMode::Mxp if self.q.local_enabled(OPT_MXP) => LinkMode::Mxp,
            LinkMode::Mslp
                if self
                    .capabilities
                    .mtts
                    .is_some_and(|capabilities| capabilities.mslp) =>
            {
                LinkMode::Mslp
            }
            _ => LinkMode::Plain,
        }
    }

    fn rich_output_blocked(&self) -> bool {
        self.capabilities.presentation.screen_reader || self.gmcp.blocks_rich_output()
    }

    fn effective_color_policy(&self) -> ColorPolicy {
        if self.rich_output_blocked() {
            ColorPolicy::Plain
        } else {
            self.capabilities.presentation.color
        }
    }

    fn negotiated_graphics_mode(&self) -> Option<GraphicsMode> {
        (self.q.local_enabled(OPT_GMCP)
            && self.gmcp.supports("tiamud.cellframe")
            && self.gmcp.graphics != GraphicsMode::None
            && !self.rich_output_blocked())
        .then_some(self.gmcp.graphics)
    }

    fn revalidate_rich_opt_ins(&mut self) {
        if !self.gmcp.supports("client.media") {
            self.gmcp.media_opt_in = false;
        }
        if !self.gmcp.supports("tiamud.assets") {
            self.gmcp.assets_opt_in = false;
        }
        if !self.gmcp.supports("tiamud.cellframe") {
            self.gmcp.graphics = GraphicsMode::None;
        }
        if self.rich_output_blocked() {
            self.gmcp.media_opt_in = false;
            self.gmcp.assets_opt_in = false;
            self.gmcp.links = LinkMode::Plain;
        }
    }

    fn refresh_decorative_policy(&mut self) {
        self.capabilities.presentation.decorative_cell_art = self.q.local_enabled(OPT_GMCP)
            && self.gmcp.supports("tiamud.cellframe")
            && self.gmcp.graphics != GraphicsMode::None
            && !self.rich_output_blocked();
    }

    fn reset_ttype_discovery(&mut self) {
        self.ttype_requests_sent = 0;
        self.ttype_awaiting_response = false;
        self.last_ttype_response = None;
        self.capabilities.reset_ttype();
    }

    fn request_next_ttype(&mut self, response: &mut Vec<u8>) {
        if self.q.remote_enabled(OPT_TTYPE)
            && !self.ttype_awaiting_response
            && self.ttype_requests_sent < MAX_TTYPE_REQUESTS
        {
            response.extend_from_slice(&[IAC, SB, OPT_TTYPE, TTYPE_SEND, IAC, SE]);
            self.ttype_requests_sent += 1;
            self.ttype_awaiting_response = true;
        }
    }

    fn handle_ttype_response(&mut self, bytes: &[u8], response: &mut Vec<u8>) {
        if !self.q.remote_enabled(OPT_TTYPE) || !self.ttype_awaiting_response {
            return;
        }
        self.ttype_awaiting_response = false;
        let Some(value) = sanitize_token(bytes, 64) else {
            self.request_next_ttype(response);
            return;
        };
        if self
            .last_ttype_response
            .as_deref()
            .is_some_and(|previous| previous.eq_ignore_ascii_case(&value))
        {
            return;
        }

        if self.capabilities.client_name.is_none() {
            self.capabilities.client_name = Some(value.clone());
        }
        if let Some(mtts) = parse_mtts(&value) {
            self.capabilities.mtts = Some(mtts);
        } else {
            self.capabilities.terminal_type = Some(value.clone());
            self.capabilities.terminal_types.push(value.clone());
        }
        self.capabilities.refresh_presentation_policy();
        self.last_ttype_response = Some(value);
        self.request_next_ttype(response);
    }
}

const GMCP_PACKAGES: &[&str] = &[
    "Core 1",
    "Room 1",
    "Comm 1",
    "Group 1",
    "Client.Media 1",
    "Char.Vitals 1",
    "Char.Items 1",
    "Tiamud.Scene 1",
    "Tiamud.Assets 1",
    "Tiamud.CellFrame 1",
    "Tiamud.Client 1",
];

fn append_initial_gmcp(output: &mut Vec<u8>) {
    append_gmcp(
        output,
        "Core.Hello",
        &json!({"client": "Tiamud", "version": env!("CARGO_PKG_VERSION")}),
    );
    append_gmcp(output, "Core.Supports.Set", &json!(GMCP_PACKAGES));
}

fn append_gmcp(output: &mut Vec<u8>, package: &str, data: &Value) -> bool {
    if !valid_gmcp_package(package) {
        return false;
    }
    let Ok(json) = serde_json::to_vec(data) else {
        return false;
    };
    if package.len() + 1 + json.len() > MAX_GMCP_BYTES {
        return false;
    }
    output.extend_from_slice(&[IAC, SB, OPT_GMCP]);
    append_telnet_escaped(output, package.as_bytes());
    output.push(b' ');
    append_telnet_escaped(output, &json);
    output.extend_from_slice(&[IAC, SE]);
    true
}

fn append_telnet_escaped(output: &mut Vec<u8>, bytes: &[u8]) {
    for byte in bytes {
        output.push(*byte);
        if *byte == IAC {
            output.push(IAC);
        }
    }
}

fn parse_gmcp(bytes: &[u8]) -> Option<(String, Value)> {
    if bytes.is_empty() || bytes.len() > MAX_GMCP_BYTES {
        return None;
    }
    let separator = bytes.iter().position(|byte| byte.is_ascii_whitespace());
    let (package, data) = separator.map_or((bytes, &[][..]), |index| {
        (&bytes[..index], bytes[index..].trim_ascii_start())
    });
    let package = std::str::from_utf8(package).ok()?;
    if !valid_gmcp_package(package) {
        return None;
    }
    let data = if data.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(data).ok()?
    };
    Some((package.to_ascii_lowercase(), data))
}

fn valid_gmcp_package(package: &str) -> bool {
    !package.is_empty()
        && package.len() <= MAX_GMCP_PACKAGE_BYTES
        && package
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_alphabetic()))
}

fn parse_gmcp_supports(data: &Value) -> Option<Vec<String>> {
    let packages = data.as_array()?;
    if packages.len() > MAX_GMCP_SUPPORTS {
        return None;
    }
    packages
        .iter()
        .map(|package| {
            let package = package.as_str()?;
            if package.is_empty()
                || package.len() > MAX_GMCP_PACKAGE_BYTES
                || package.chars().any(char::is_control)
            {
                return None;
            }
            let mut fields = package.split_ascii_whitespace();
            let name = fields.next()?;
            if let Some(version) = fields.next()
                && (version.is_empty()
                    || !version.bytes().all(|byte| byte.is_ascii_digit())
                    || fields.next().is_some())
            {
                return None;
            }
            valid_gmcp_package(name).then(|| name.to_ascii_lowercase())
        })
        .collect()
}

fn append_mssp(output: &mut Vec<u8>) {
    const VALUES: &[(&str, &str)] = &[
        ("NAME", "Tiamud"),
        ("CODEBASE", "Tiamud"),
        ("FAMILY", "Custom"),
        ("GENRE", "Fantasy"),
        ("GAMEPLAY", "Adventure"),
        ("STATUS", "Alpha"),
        ("ANSI", "1"),
        ("UTF-8", "1"),
        ("VT100", "1"),
        ("XTERM 256 COLORS", "1"),
        ("XTERM TRUE COLORS", "1"),
        ("GMCP", "1"),
        ("MCCP", "1"),
        ("MSSP", "1"),
        ("MXP", "1"),
        ("MSLP", "1"),
        ("MCMP", "1"),
        ("PAY TO PLAY", "0"),
        ("PAY FOR PERKS", "0"),
    ];
    output.extend_from_slice(&[IAC, SB, OPT_MSSP]);
    for (name, value) in VALUES {
        output.push(MSSP_VAR);
        output.extend_from_slice(name.as_bytes());
        output.push(MSSP_VAL);
        output.extend_from_slice(value.as_bytes());
    }
    output.extend_from_slice(&[IAC, SE]);
}

fn sanitize_text(value: &str, max_len: usize) -> Option<String> {
    let value = value
        .chars()
        .filter(|character| !character.is_control())
        .take(max_len)
        .collect::<String>();
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn safe_scene_image(image: &tiamud_core::SceneImage) -> bool {
    image
        .manifest
        .as_deref()
        .is_some_and(safe_content_addressed_asset_path)
        && [&image.avif, &image.webp, &image.jpeg]
            .into_iter()
            .flatten()
            .all(|path| safe_content_addressed_asset_path(path))
}

fn safe_scene_audio(audio: &SceneAudio) -> bool {
    safe_audio_channel(&audio.channel)
        && safe_content_addressed_asset_path(&audio.url)
        && audio.url.rsplit_once('/').is_some_and(|(_, file)| {
            [".opus", ".ogg", ".mp3"].iter().any(|extension| {
                file.strip_prefix("audio-")
                    .and_then(|name| name.strip_suffix(extension))
                    .is_some_and(|name| !name.is_empty())
            })
        })
}

fn safe_audio_channel(channel: &str) -> bool {
    matches!(channel, "ambience" | "effects" | "music")
}

fn safe_content_addressed_asset_path(value: &str) -> bool {
    if value.len() > 256 || value.contains(['?', '#', '\\']) {
        return false;
    }
    let Some(path) = value.strip_prefix("/assets/") else {
        return false;
    };
    let Some((digest, file)) = path.split_once('/') else {
        return false;
    };
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && !file.is_empty()
        && file.len() <= 128
        && !file.contains('/')
        && file
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn render_scene_cell_frame(scene: &SceneView, session: &TelnetSession) -> Option<Vec<u8>> {
    let mode = session.negotiated_graphics_mode()?;
    let frame = atmospheric_cell_frame(scene);
    if frame
        .validate_with_max_cells(TELNET_CELL_FRAME_MAX_CELLS)
        .is_err()
    {
        return Some(b"\r\n[Decorative atmosphere omitted: invalid frame.]\r\n".to_vec());
    }

    // The complete playable scene has already been emitted as text. This alt
    // line describes only the redundant decorative layer and is always sent
    // before a terminal-native payload.
    let mut output = format!("\r\n[Decorative: {}]\r\n", frame.alt).into_bytes();
    let rendered = match mode {
        GraphicsMode::None => return None,
        GraphicsMode::Ansi => {
            let profile = match session.effective_color_policy() {
                ColorPolicy::Plain | ColorPolicy::Ansi16 => AnsiProfile::Ansi16,
                ColorPolicy::Ansi256 => AnsiProfile::Ansi256,
                ColorPolicy::TrueColor => AnsiProfile::TrueColor,
            };
            frame
                .render_ansi_bounded(profile, TELNET_CELL_FRAME_MAX_BYTES)
                .map(String::into_bytes)
        }
        GraphicsMode::Kitty => {
            frame.render_native(NativeProtocol::Kitty, true, TELNET_CELL_FRAME_MAX_BYTES)
        }
        GraphicsMode::Sixel => {
            frame.render_native(NativeProtocol::Sixel, true, TELNET_CELL_FRAME_MAX_BYTES)
        }
        GraphicsMode::Iterm2 => {
            frame.render_native(NativeProtocol::Iterm2, true, TELNET_CELL_FRAME_MAX_BYTES)
        }
    };
    match rendered {
        Ok(rendered) => {
            output.extend_from_slice(&rendered);
            output.extend_from_slice(b"\r\n");
        }
        Err(error) => {
            debug!(%error, "Telnet CellFrame renderer used its textual fallback");
            output.extend_from_slice(b"[Terminal graphic omitted safely.]\r\n");
        }
    }
    Some(output)
}

fn atmospheric_cell_frame(scene: &SceneView) -> CellFrame {
    const WIDTH: u16 = 32;
    const HEIGHT: u16 = 6;
    let title = sanitize_text(&scene.title, 96).unwrap_or_else(|| "current location".to_owned());
    let weather =
        sanitize_text(&scene.weather.condition, 48).unwrap_or_else(|| "unknown weather".to_owned());
    let accent = match weather.to_ascii_lowercase().as_str() {
        "rain" | "storm" => RgbColor {
            red: 46,
            green: 105,
            blue: 148,
        },
        "snow" | "frost" => RgbColor {
            red: 164,
            green: 202,
            blue: 218,
        },
        "sun" | "clear" => RgbColor {
            red: 194,
            green: 124,
            blue: 45,
        },
        "fog" | "mist" => RgbColor {
            red: 104,
            green: 116,
            blue: 125,
        },
        _ => RgbColor {
            red: 112,
            green: 74,
            blue: 139,
        },
    };
    let palette = vec![
        RgbColor {
            red: 10,
            green: 14,
            blue: 20,
        },
        accent,
        RgbColor {
            red: 225,
            green: 190,
            blue: 102,
        },
    ];
    let weather_seed = weather.bytes().fold(0_u8, u8::wrapping_add);
    let mut cells = Vec::with_capacity(usize::from(WIDTH) * usize::from(HEIGHT));
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let border = y == 0 || y + 1 == HEIGHT || x == 0 || x + 1 == WIDTH;
            let glyph = if border {
                if (x == 0 || x + 1 == WIDTH) && (y == 0 || y + 1 == HEIGHT) {
                    '+'
                } else if y == 0 || y + 1 == HEIGHT {
                    '-'
                } else {
                    '|'
                }
            } else if (x as u8)
                .wrapping_mul(17)
                .wrapping_add((y as u8).wrapping_mul(31))
                .wrapping_add(weather_seed)
                % 13
                == 0
            {
                '*'
            } else {
                ' '
            };
            cells.push(Cell {
                glyph,
                foreground: if border { 2 } else { 1 },
                background: if y % 2 == 0 { 0 } else { 1 },
                attributes: CellAttributes {
                    bold: border,
                    ..CellAttributes::default()
                },
            });
        }
    }
    CellFrame {
        width: WIDTH,
        height: HEIGHT,
        palette,
        cells,
        alt: format!("Atmospheric decoration for {title} in {weather}."),
    }
}

fn render_scene(scene: &SceneView, session: &TelnetSession) -> String {
    let presentation = session.capabilities.presentation;
    let mut output = String::from("\r\n");
    match session.effective_color_policy() {
        ColorPolicy::Plain => output.push_str(&scene.title),
        ColorPolicy::Ansi16 => {
            output.push_str("\x1b[1;36m");
            output.push_str(&scene.title);
            output.push_str("\x1b[0m");
        }
        ColorPolicy::Ansi256 => {
            output.push_str("\x1b[1;38;5;45m");
            output.push_str(&scene.title);
            output.push_str("\x1b[0m");
        }
        ColorPolicy::TrueColor => {
            output.push_str("\x1b[1;38;2;99;220;255m");
            output.push_str(&scene.title);
            output.push_str("\x1b[0m");
        }
    }
    output.push_str("\r\n");
    for paragraph in &scene.prose {
        output.push_str(paragraph);
        output.push_str("\r\n");
    }
    if !scene.items.is_empty() {
        output.push_str("You see here: ");
        output.push_str(
            &scene
                .items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        );
        output.push_str(".\r\n");
    }
    for npc in &scene.npcs {
        output.push_str(&format!(
            "{} is here ({}/{} health).\r\n",
            npc.label, npc.health, npc.max_health
        ));
    }
    output.push_str(&format!(
        "Weather: {}; day {}, hour {}.\r\n",
        scene.weather.condition, scene.weather.day, scene.weather.hour
    ));
    output.push_str("Exits: ");
    if scene.exits.is_empty() {
        output.push_str("none");
    } else {
        let link_mode = if presentation.screen_reader {
            LinkMode::Plain
        } else {
            session.negotiated_link_mode()
        };
        for (index, exit) in scene.exits.iter().enumerate() {
            if index > 0 {
                output.push_str(", ");
            }
            output.push_str(&render_link(&exit.label, &exit.command, link_mode));
        }
    }
    output.push_str("\r\n");
    output
}

fn render_link(label: &str, command: &str, mode: LinkMode) -> String {
    let Some(label) = sanitize_text(label, 64) else {
        return String::new();
    };
    let Some(command) = sanitize_link_command(command) else {
        return label;
    };
    match mode {
        LinkMode::Plain => label,
        // MSLP simple links deliberately send their visible direction. Scene
        // exit labels are server-authored movement commands, never arbitrary
        // user markup.
        LinkMode::Mslp => format!("\x1b[4m{}\x1b[24m", sanitize_mslp_label(&label)),
        LinkMode::Mxp => format!(
            "\x1b[1z<SEND HREF=\"{}\">{}</SEND>\x1b[7z",
            escape_mxp(&command),
            escape_mxp(&label)
        ),
    }
}

fn sanitize_link_command(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b' ' | b'.' | b':' | b'_' | b'-')
        }))
    .then(|| value.to_owned())
}

fn sanitize_mslp_label(value: &str) -> String {
    value
        .chars()
        .filter(|character| !matches!(character, '\x1b' | '\x07' | '\0'))
        .collect()
}

fn escape_mxp(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn append_charset_request(output: &mut Vec<u8>) {
    output.extend_from_slice(&[
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

fn parse_mtts(value: &str) -> Option<MttsCapabilities> {
    let mut fields = value.split_ascii_whitespace();
    let label = fields.next()?;
    let bitvector = fields.next()?.parse::<u16>().ok()?;
    (label.eq_ignore_ascii_case("MTTS") && fields.next().is_none())
        .then(|| MttsCapabilities::from_bitvector(bitvector))
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

#[derive(Default)]
struct TelnetInput {
    parser: TelnetParser,
    negotiation: TelnetSession,
    command_buffer: CommandBuffer,
    queued: VecDeque<LineEvent>,
}

impl TelnetInput {
    fn set_server_echo(&mut self, enabled: bool) -> Vec<u8> {
        self.negotiation.request_server_echo(enabled)
    }

    async fn read_line(
        &mut self,
        connection: &mut TelnetConnection,
    ) -> io::Result<Option<LineEvent>> {
        loop {
            if let Some(event) = self.queued.pop_front() {
                return Ok(Some(event));
            }

            let mut read_buffer = [0_u8; 4096];
            let count = connection.read(&mut read_buffer).await?;
            if count == 0 {
                return Ok(None);
            }
            let data = self.parser.feed(&read_buffer[..count]);
            let response = self.negotiation.handle(self.parser.take_events());
            if !response.is_empty() {
                connection.write_all(&response).await?;
            }
            if self.negotiation.take_mccp_stop_requested() {
                connection.finish_compression().await?;
            }
            if self.negotiation.take_mccp_start_requested() {
                connection.start_compression().await?;
            }
            for byte in data {
                if let Some(event) = self.command_buffer.push(byte) {
                    self.queued.push_back(event);
                }
            }
        }
    }

    async fn read_line_queued(
        &mut self,
        reader: &mut ReadHalf<TelnetStream>,
        output: &TelnetOutputQueue,
    ) -> io::Result<Option<LineEvent>> {
        loop {
            if let Some(event) = self.queued.pop_front() {
                return Ok(Some(event));
            }

            let mut read_buffer = [0_u8; 4096];
            let count = reader.read(&mut read_buffer).await?;
            if count == 0 {
                return Ok(None);
            }
            let data = self.parser.feed(&read_buffer[..count]);
            let response = self.negotiation.handle(self.parser.take_events());
            if !response.is_empty() {
                output.bytes(OutputPriority::Critical, response)?;
            }
            if self.negotiation.take_mccp_stop_requested() {
                output.stop_compression()?;
            }
            if self.negotiation.take_mccp_start_requested() {
                output.start_compression()?;
            }
            for byte in data {
                if let Some(event) = self.command_buffer.push(byte) {
                    self.queued.push_back(event);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpStream;

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
        let mut session = TelnetSession::default();
        session.begin_initial_negotiation();
        assert!(
            session
                .handle(vec![TelnetEvent::Negotiation {
                    command: WILL,
                    option: OPT_NAWS,
                }])
                .is_empty()
        );

        let mut parser = TelnetParser::default();
        parser.feed(&[IAC, SB, OPT_NAWS, 0, IAC, IAC, 0, 24, IAC, SE]);
        assert!(session.handle(parser.take_events()).is_empty());
        assert_eq!(session.capabilities.columns, Some(255));
        assert_eq!(session.capabilities.rows, Some(24));
    }

    #[test]
    fn requests_ttype_and_utf8_only_after_client_accepts_options() {
        let mut session = TelnetSession::default();
        assert_eq!(
            session.begin_initial_negotiation(),
            [
                &[IAC, WILL, OPT_SUPPRESS_GO_AHEAD][..],
                &[IAC, WILL, OPT_EOR][..],
                &[IAC, DO, OPT_TTYPE][..],
                &[IAC, DO, OPT_NAWS][..],
                &[IAC, DO, OPT_CHARSET][..],
                &[IAC, WILL, OPT_GMCP][..],
                &[IAC, WILL, OPT_MSSP][..],
                &[IAC, WILL, OPT_MCCP2][..],
            ]
            .concat()
        );
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
        session.begin_initial_negotiation();
        session.handle(vec![
            TelnetEvent::Negotiation {
                command: WILL,
                option: OPT_TTYPE,
            },
            TelnetEvent::Negotiation {
                command: WILL,
                option: OPT_CHARSET,
            },
        ]);
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
        assert!(session.capabilities.presentation.utf8);
    }

    #[test]
    fn cycles_ttype_until_repeat_and_derives_a_conservative_mtts_policy() {
        let mut session = TelnetSession::default();
        session.begin_initial_negotiation();
        let first_request = session.handle(vec![TelnetEvent::Negotiation {
            command: WILL,
            option: OPT_TTYPE,
        }]);
        assert_eq!(first_request, [IAC, SB, OPT_TTYPE, TTYPE_SEND, IAC, SE]);

        for value in ["MUDLET 4.17", "XTERM-256COLOR", "MTTS 1357"] {
            assert_eq!(
                session.handle(vec![TelnetEvent::Subnegotiation {
                    option: OPT_TTYPE,
                    data: [vec![TTYPE_IS], value.as_bytes().to_vec()].concat(),
                }]),
                [IAC, SB, OPT_TTYPE, TTYPE_SEND, IAC, SE]
            );
        }
        // 1357 = ANSI + UTF-8 + 256 colors + SCREEN READER + TRUECOLOR + MSLP.
        // Screen-reader policy deliberately suppresses decorative color/link output.
        let mtts = session.capabilities.mtts.expect("MTTS bitvector");
        assert!(mtts.ansi && mtts.utf8 && mtts.colors_256 && mtts.truecolor);
        assert!(mtts.screen_reader && mtts.mslp);
        assert_eq!(
            session.capabilities.presentation,
            PresentationPolicy {
                utf8: true,
                color: ColorPolicy::Plain,
                screen_reader: true,
                clickable_links: false,
                decorative_cell_art: false,
            }
        );
        assert_eq!(
            session.capabilities.client_name.as_deref(),
            Some("MUDLET 4.17")
        );
        assert_eq!(
            session.capabilities.terminal_type.as_deref(),
            Some("XTERM-256COLOR")
        );

        let repeated = session.handle(vec![TelnetEvent::Subnegotiation {
            option: OPT_TTYPE,
            data: [vec![TTYPE_IS], b"MTTS 1357".to_vec()].concat(),
        }]);
        assert!(repeated.is_empty());
        assert_eq!(session.ttype_requests_sent, 4);
        assert!(!session.ttype_awaiting_response);
    }

    #[test]
    fn ttype_is_bounded_and_unsolicited_responses_do_not_restart_the_cycle() {
        let mut session = TelnetSession::default();
        session.begin_initial_negotiation();
        session.handle(vec![TelnetEvent::Negotiation {
            command: WILL,
            option: OPT_TTYPE,
        }]);

        for index in 0..MAX_TTYPE_REQUESTS {
            let response = session.handle(vec![TelnetEvent::Subnegotiation {
                option: OPT_TTYPE,
                data: [vec![TTYPE_IS], format!("TERM-{index}").into_bytes()].concat(),
            }]);
            if index + 1 == MAX_TTYPE_REQUESTS {
                assert!(response.is_empty());
            } else {
                assert_eq!(response, [IAC, SB, OPT_TTYPE, TTYPE_SEND, IAC, SE]);
            }
        }
        assert_eq!(session.ttype_requests_sent, MAX_TTYPE_REQUESTS);
        assert!(!session.ttype_awaiting_response);
        assert!(
            session
                .handle(vec![TelnetEvent::Subnegotiation {
                    option: OPT_TTYPE,
                    data: [vec![TTYPE_IS], b"EXTRA".to_vec()].concat(),
                }])
                .is_empty()
        );
    }

    #[test]
    fn ttype_name_alone_never_opts_into_native_graphics() {
        let mut session = TelnetSession::default();
        session.begin_initial_negotiation();
        session.handle(vec![TelnetEvent::Negotiation {
            command: WILL,
            option: OPT_TTYPE,
        }]);
        session.handle(vec![TelnetEvent::Subnegotiation {
            option: OPT_TTYPE,
            data: [vec![TTYPE_IS], b"KITTY".to_vec()].concat(),
        }]);
        assert!(!session.capabilities.presentation.decorative_cell_art);
        assert_eq!(session.capabilities.presentation.color, ColorPolicy::Plain);
    }

    #[test]
    fn mtts_capabilities_never_replace_explicit_graphics_consent() {
        let mut session = TelnetSession::default();
        session.capabilities.mtts = Some(MttsCapabilities::from_bitvector(
            MTTS_ANSI | MTTS_256_COLORS | MTTS_TRUECOLOR,
        ));
        session.capabilities.refresh_presentation_policy();

        assert_eq!(session.negotiated_graphics_mode(), None);
        assert!(!session.capabilities.presentation.decorative_cell_art);
        assert!(render_scene_cell_frame(&sample_scene(), &session).is_none());
    }

    #[test]
    fn every_cell_frame_renderer_requires_an_explicit_gmcp_choice() {
        let expectations = [
            ("ansi", b"\x1b[0".as_slice()),
            ("kitty", b"\x1b_G".as_slice()),
            ("sixel", b"\x1bPq".as_slice()),
            ("iterm2", b"\x1b]1337".as_slice()),
        ];
        for (mode, marker) in expectations {
            let mut session = gmcp_cell_frame_session();
            session.handle(vec![TelnetEvent::Subnegotiation {
                option: OPT_GMCP,
                data: format!(r#"Tiamud.Client.Set {{"graphics":"{mode}","decorative":true}}"#)
                    .into_bytes(),
            }]);

            assert!(session.capabilities.presentation.decorative_cell_art);
            let output = render_scene_cell_frame(&sample_scene(), &session).unwrap();
            assert!(output.windows(marker.len()).any(|window| window == marker));
            assert!(output.starts_with(b"\r\n[Decorative: "));
            assert!(output.len() <= TELNET_CELL_FRAME_MAX_BYTES + 256);
        }
    }

    #[test]
    fn accessibility_preferences_override_graphics_assets_media_and_links() {
        let blockers = [
            r#""plain":true"#,
            r#""reducedMotion":true"#,
            r#""screenReader":true"#,
            r#""decorative":false"#,
        ];
        for blocker in blockers {
            let mut session = gmcp_cell_frame_session();
            session.handle(vec![TelnetEvent::Subnegotiation {
                option: OPT_GMCP,
                data: format!(
                    r#"Tiamud.Client.Set {{"graphics":"kitty","media":true,"assets":true,"links":"mxp",{blocker}}}"#
                )
                .into_bytes(),
            }]);

            assert_eq!(session.negotiated_graphics_mode(), None);
            assert!(!session.capabilities.presentation.decorative_cell_art);
            assert!(!session.gmcp.media_opt_in);
            assert!(!session.gmcp.assets_opt_in);
            assert_eq!(session.negotiated_link_mode(), LinkMode::Plain);
            assert!(render_scene_cell_frame(&sample_scene(), &session).is_none());
        }

        let mut session = gmcp_cell_frame_session();
        session.handle(vec![TelnetEvent::Subnegotiation {
            option: OPT_GMCP,
            data: br#"Tiamud.Client.Set {"graphics":"sixel","media":true,"assets":true}"#.to_vec(),
        }]);
        session.capabilities.mtts = Some(MttsCapabilities::from_bitvector(MTTS_SCREEN_READER));
        session.capabilities.refresh_presentation_policy();
        session.handle(Vec::new());
        assert_eq!(session.negotiated_graphics_mode(), None);
        assert!(!session.gmcp.media_opt_in);
        assert!(!session.gmcp.assets_opt_in);
    }

    #[test]
    fn malformed_graphics_values_and_unsafe_asset_urls_fail_closed() {
        let mut session = gmcp_cell_frame_session();
        session.handle(vec![TelnetEvent::Subnegotiation {
            option: OPT_GMCP,
            data: br#"Tiamud.Client.Set {"graphics":"kitty\u001b_G","assets":true}"#.to_vec(),
        }]);
        assert_eq!(session.negotiated_graphics_mode(), None);

        for path in [
            "https://attacker.invalid/media.ogg",
            "/assets/../../secret",
            "/assets/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA/a.ogg",
            "/assets/aaaaaaaa/a.ogg?run=true",
        ] {
            assert!(!safe_content_addressed_asset_path(path));
        }
        assert!(safe_content_addressed_asset_path(&format!(
            "/assets/{}/ambience.ogg",
            "a".repeat(64)
        )));

        let mut scene = sample_scene();
        scene.image.as_mut().unwrap().webp = Some("https://attacker.invalid/image.webp".into());
        let payloads = gmcp_payloads(&session.scene_gmcp(&scene));
        assert!(
            !payloads
                .iter()
                .any(|payload| payload.starts_with(b"Tiamud.Assets "))
        );
    }

    #[test]
    fn negotiates_gmcp_and_announces_the_bounded_package_set() {
        let mut session = TelnetSession::default();
        session.begin_initial_negotiation();
        let response = session.handle(vec![TelnetEvent::Negotiation {
            command: DO,
            option: OPT_GMCP,
        }]);
        assert!(session.local_enabled(OPT_GMCP));
        let payloads = gmcp_payloads(&response);
        assert_eq!(payloads.len(), 2);
        assert!(payloads[0].starts_with(b"Core.Hello "));
        assert!(payloads[1].starts_with(b"Core.Supports.Set "));
        assert!(
            payloads
                .iter()
                .all(|payload| payload.len() <= MAX_GMCP_BYTES)
        );
    }

    #[test]
    fn gmcp_core_messages_are_strict_bounded_and_fragment_safe() {
        let wire = [
            &[IAC, SB, OPT_GMCP][..],
            br#"Core.Hello {"client":"TinTin++","version":"2.02"}"#,
            &[IAC, SE][..],
            &[IAC, SB, OPT_GMCP][..],
            br#"Core.Supports.Set ["Room 1","Tiamud.Scene 1","Comm 1","Group 1"]"#,
            &[IAC, SE][..],
        ]
        .concat();
        let mut parser = TelnetParser::default();
        let mut session = TelnetSession::default();
        session.begin_initial_negotiation();
        session.handle(vec![TelnetEvent::Negotiation {
            command: DO,
            option: OPT_GMCP,
        }]);
        let mut response = Vec::new();
        for byte in wire {
            assert!(parser.feed(&[byte]).is_empty());
            response.extend(session.handle(parser.take_events()));
        }

        assert_eq!(session.gmcp.name.as_deref(), Some("TinTin++"));
        assert_eq!(session.gmcp.version.as_deref(), Some("2.02"));
        assert!(session.gmcp.supports("room"));
        let payloads = gmcp_payloads(&response);
        assert!(
            payloads
                .iter()
                .any(|payload| payload.starts_with(b"Comm.Channel.List "))
        );
        assert!(
            payloads
                .iter()
                .any(|payload| payload.starts_with(b"Group.Info "))
        );

        let previous = session.gmcp.name.clone();
        assert!(
            session
                .handle(vec![TelnetEvent::Subnegotiation {
                    option: OPT_GMCP,
                    data: br#"Core.Hello {"client":"bad","unexpected":true}"#.to_vec(),
                }])
                .is_empty()
        );
        assert_eq!(session.gmcp.name, previous);
        assert!(parse_gmcp(&vec![b'x'; MAX_GMCP_BYTES + 1]).is_none());
        let mut oversized = Vec::new();
        assert!(!append_gmcp(
            &mut oversized,
            "Tiamud.Scene",
            &Value::String("x".repeat(MAX_GMCP_BYTES))
        ));
        assert!(oversized.is_empty());
    }

    #[test]
    fn gmcp_scene_snapshot_contains_only_available_safe_views() {
        let mut session = TelnetSession::default();
        session.begin_initial_negotiation();
        session.handle(vec![TelnetEvent::Negotiation {
            command: DO,
            option: OPT_GMCP,
        }]);
        session.handle(vec![TelnetEvent::Subnegotiation {
            option: OPT_GMCP,
            data: br#"Core.Supports.Set ["Room 1","Tiamud.Scene 1","Tiamud.Assets 1","Client.Media 1","Char.Vitals 1","Char.Items 1"]"#.to_vec(),
        }]);
        session.handle(vec![TelnetEvent::Subnegotiation {
            option: OPT_GMCP,
            data: br#"Tiamud.Client.Set {"media":true,"assets":true}"#.to_vec(),
        }]);
        let mut scene = sample_scene();
        scene.audio = Some(SceneAudio {
            channel: "ambience".into(),
            url: format!("/assets/{}/audio-rain.ogg", "a".repeat(64)),
            looped: true,
        });
        let payloads = gmcp_payloads(&session.scene_gmcp(&scene));

        assert!(
            payloads
                .iter()
                .any(|payload| payload.starts_with(b"Room.Info "))
        );
        assert!(
            payloads
                .iter()
                .any(|payload| payload.starts_with(b"Tiamud.Scene "))
        );
        assert!(
            payloads
                .iter()
                .any(|payload| payload.starts_with(b"Tiamud.Assets "))
        );
        assert!(
            payloads
                .iter()
                .any(|payload| payload.starts_with(b"Client.Media.Play "))
        );
        assert!(
            !payloads
                .iter()
                .any(|payload| payload.starts_with(b"Char.Vitals "))
        );
        assert!(
            !payloads
                .iter()
                .any(|payload| payload.starts_with(b"Char.Items "))
        );
        assert!(session.gmcp.media_opt_in);

        let stopped = session.handle(vec![TelnetEvent::Subnegotiation {
            option: OPT_GMCP,
            data: br#"Tiamud.Client.Set {"media":false,"assets":true}"#.to_vec(),
        }]);
        assert!(
            gmcp_payloads(&stopped)
                .iter()
                .any(|payload| payload.starts_with(b"Client.Media.Stop "))
        );
        assert!(!session.gmcp.media_opt_in);
    }

    #[test]
    fn gmcp_media_is_inert_without_opt_in_and_unsafe_urls_fail_closed() {
        let mut session = TelnetSession::default();
        session.begin_initial_negotiation();
        session.handle(vec![TelnetEvent::Negotiation {
            command: DO,
            option: OPT_GMCP,
        }]);
        session.handle(vec![TelnetEvent::Subnegotiation {
            option: OPT_GMCP,
            data: br#"Core.Supports.Set ["Client.Media 1"]"#.to_vec(),
        }]);
        let mut scene = sample_scene();
        scene.audio = Some(SceneAudio {
            channel: "ambience".into(),
            url: format!("/assets/{}/audio-rain.opus", "a".repeat(64)),
            looped: true,
        });
        assert!(
            !gmcp_payloads(&session.scene_gmcp(&scene))
                .iter()
                .any(|payload| payload.starts_with(b"Client.Media.Play "))
        );

        session.handle(vec![TelnetEvent::Subnegotiation {
            option: OPT_GMCP,
            data: br#"Tiamud.Client.Set {"media":true}"#.to_vec(),
        }]);
        scene.audio.as_mut().unwrap().url = "https://tracker.invalid/rain.mp3".into();
        assert!(
            !gmcp_payloads(&session.scene_gmcp(&scene))
                .iter()
                .any(|payload| payload.starts_with(b"Client.Media.Play "))
        );
    }

    #[test]
    fn gmcp_player_snapshot_emits_standard_vitals_items_and_group_views() {
        let mut session = TelnetSession::default();
        session.begin_initial_negotiation();
        session.handle(vec![TelnetEvent::Negotiation {
            command: DO,
            option: OPT_GMCP,
        }]);
        session.handle(vec![TelnetEvent::Subnegotiation {
            option: OPT_GMCP,
            data: br#"Core.Supports.Set ["Char.Vitals 1","Char.Items 1","Group 1"]"#.to_vec(),
        }]);

        let mut world = tiamud_core::World::demo();
        let session_id = tiamud_core::SessionId(17);
        world
            .connect(
                session_id,
                tiamud_core::CharacterSnapshot {
                    id: tiamud_core::CharacterId(5),
                    account_id: tiamud_core::AccountId(3),
                    name: "Aria".to_owned(),
                    room_id: "demo:lantern-square".to_owned(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let player = world.player_state_view(session_id).unwrap();
        let payloads = gmcp_payloads(&session.player_gmcp(&player));

        assert!(payloads.iter().any(|payload| {
            payload.starts_with(b"Char.Vitals ")
                && String::from_utf8_lossy(payload).contains("\"maxhp\"")
        }));
        assert_eq!(
            payloads
                .iter()
                .filter(|payload| payload.starts_with(b"Char.Items.List "))
                .count(),
            2
        );
        assert!(payloads.iter().any(|payload| {
            payload.starts_with(b"Group.Info ")
                && String::from_utf8_lossy(payload).contains("\"revision\"")
        }));
        let wire = String::from_utf8_lossy(
            &payloads
                .iter()
                .flat_map(|payload| payload.iter().copied())
                .collect::<Vec<_>>(),
        )
        .into_owned();
        assert!(!wire.contains("demo:lantern-square"));
        assert!(!wire.contains("characterId"));
    }

    #[test]
    fn gmcp_social_delivery_is_bounded_to_the_safe_comm_projection() {
        let mut session = TelnetSession::default();
        session.begin_initial_negotiation();
        session.handle(vec![TelnetEvent::Negotiation {
            command: DO,
            option: OPT_GMCP,
        }]);
        session.handle(vec![TelnetEvent::Subnegotiation {
            option: OPT_GMCP,
            data: br#"Core.Supports.Set ["Comm 1"]"#.to_vec(),
        }]);

        let message = SocialMessage {
            channel: SocialChannel::Tell,
            sender: "Mara".to_owned(),
            text: "Meet me by the north gate.".to_owned(),
        };
        let payloads = gmcp_payloads(&session.social_gmcp(&message));

        assert_eq!(payloads.len(), 1);
        let payload = String::from_utf8(payloads[0].clone()).unwrap();
        assert!(payload.starts_with("Comm.Channel.Text "));
        assert!(payload.contains("\"channel\":\"tell\""));
        assert!(payload.contains("\"talker\":\"Mara\""));
        assert!(payload.contains("north gate"));
        assert!(!payload.contains("characterId"));
        assert!(!payload.contains("recipient"));
        assert!(!payload.contains("moderation"));
    }

    #[test]
    fn mssp_reports_only_concrete_protocol_and_game_facts() {
        let mut session = TelnetSession::default();
        session.begin_initial_negotiation();
        let response = session.handle(vec![TelnetEvent::Negotiation {
            command: DO,
            option: OPT_MSSP,
        }]);
        assert!(response.starts_with(&[IAC, SB, OPT_MSSP, MSSP_VAR]));
        assert!(response.ends_with(&[IAC, SE]));
        assert!(
            response
                .windows(b"NAME".len())
                .any(|window| window == b"NAME")
        );
        assert!(
            response
                .windows(b"GMCP".len())
                .any(|window| window == b"GMCP")
        );
        assert!(
            !response
                .windows(b"PLAYERS".len())
                .any(|window| window == b"PLAYERS")
        );
    }

    #[test]
    fn rich_links_require_explicit_opt_in_and_are_sanitized() {
        let mut session = TelnetSession::default();
        session.capabilities.mtts = Some(MttsCapabilities::from_bitvector(MTTS_MSLP));
        session.capabilities.refresh_presentation_policy();
        assert_eq!(
            render_link("north", "north", session.negotiated_link_mode()),
            "north"
        );

        session.begin_initial_negotiation();
        session.handle(vec![TelnetEvent::Negotiation {
            command: DO,
            option: OPT_GMCP,
        }]);
        session.handle(vec![TelnetEvent::Subnegotiation {
            option: OPT_GMCP,
            data: br#"Core.Supports.Set ["Tiamud.Client 1"]"#.to_vec(),
        }]);
        session.handle(vec![TelnetEvent::Subnegotiation {
            option: OPT_GMCP,
            data: br#"Tiamud.Client.Set {"links":"mslp"}"#.to_vec(),
        }]);
        assert_eq!(
            render_link("north", "north", session.negotiated_link_mode()),
            "\x1b[4mnorth\x1b[24m"
        );
        assert_eq!(
            render_link("click", "north;quit", session.negotiated_link_mode()),
            "click"
        );

        let mxp_request = session.handle(vec![TelnetEvent::Subnegotiation {
            option: OPT_GMCP,
            data: br#"Tiamud.Client.Set {"links":"mxp"}"#.to_vec(),
        }]);
        assert_eq!(mxp_request, [IAC, WILL, OPT_MXP]);
        assert_eq!(session.negotiated_link_mode(), LinkMode::Plain);
        assert!(
            session
                .handle(vec![TelnetEvent::Negotiation {
                    command: DO,
                    option: OPT_MXP,
                }])
                .is_empty()
        );
        assert_eq!(session.negotiated_link_mode(), LinkMode::Mxp);
        assert!(!render_link("<north>", "north", LinkMode::Mxp).contains("<north>"));

        session.capabilities.mtts = Some(MttsCapabilities::from_bitvector(
            MTTS_MSLP | MTTS_SCREEN_READER,
        ));
        session.capabilities.refresh_presentation_policy();
        let rendered = render_scene(&sample_scene(), &session);
        assert!(!rendered.contains('\x1b'));
    }

    #[tokio::test]
    async fn mccp2_compresses_every_byte_after_the_start_marker() {
        use flate2::read::ZlibDecoder;
        use std::io::Read as _;

        let mut negotiation = TelnetSession::default();
        negotiation.begin_initial_negotiation();
        assert!(
            negotiation
                .handle(vec![TelnetEvent::Negotiation {
                    command: DO,
                    option: OPT_MCCP2,
                }])
                .is_empty()
        );
        assert!(negotiation.take_mccp_start_requested());
        assert!(
            negotiation
                .handle(vec![TelnetEvent::Negotiation {
                    command: DO,
                    option: OPT_MCCP2,
                }])
                .is_empty()
        );
        assert!(!negotiation.take_mccp_start_requested());

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let client = tokio::spawn(TcpStream::connect(address));
        let (server, _) = listener.accept().await.unwrap();
        let mut client = client.await.unwrap().unwrap();
        let mut connection = TelnetConnection::new(Box::new(server), address.ip());

        connection.start_compression().await.unwrap();
        connection.write_all(b"compressed scene\r\n").await.unwrap();
        connection.finish_compression().await.unwrap();
        drop(connection);

        let mut wire = Vec::new();
        client.read_to_end(&mut wire).await.unwrap();
        assert_eq!(&wire[..5], &[IAC, SB, OPT_MCCP2, IAC, SE]);
        let mut decoder = ZlibDecoder::new(&wire[5..]);
        let mut decoded = Vec::new();
        decoder.read_to_end(&mut decoded).unwrap();
        assert_eq!(decoded, b"compressed scene\r\n");
    }

    #[test]
    fn bounded_output_queue_coalesces_snapshots_without_losing_interactive_text() {
        let metrics = Arc::new(RuntimeMetrics::default());
        let output = TelnetOutputQueue::with_metrics(3, 64, Some(metrics.clone()));
        output
            .bytes(OutputPriority::Snapshot, b"old scene".to_vec())
            .unwrap();
        output
            .bytes(OutputPriority::Interactive, b"command result".to_vec())
            .unwrap();
        output
            .bytes(OutputPriority::Snapshot, b"new scene".to_vec())
            .unwrap();

        let state = output.shared.state.lock().unwrap();
        assert_eq!(state.frames.len(), 2);
        assert!(matches!(
            &state.frames[0],
            OutputFrame::Bytes { priority: OutputPriority::Interactive, bytes }
                if bytes == b"command result"
        ));
        assert!(matches!(
            &state.frames[1],
            OutputFrame::Bytes { priority: OutputPriority::Snapshot, bytes }
                if bytes == b"new scene"
        ));
        assert_eq!(state.byte_len, b"command result".len() + b"new scene".len());
        assert!(
            metrics
                .prometheus(0)
                .contains("tiamud_output_coalesced_total{transport=\"telnet\"} 1")
        );
    }

    #[test]
    fn bounded_output_queue_rejects_a_slow_client_but_retains_critical_output() {
        let metrics = Arc::new(RuntimeMetrics::default());
        let output = TelnetOutputQueue::with_metrics(1, 16, Some(metrics.clone()));
        output
            .bytes(OutputPriority::Interactive, b"ordinary".to_vec())
            .unwrap();
        assert_eq!(
            output
                .bytes(OutputPriority::Interactive, b"another".to_vec())
                .unwrap_err()
                .kind(),
            io::ErrorKind::WouldBlock
        );

        output
            .bytes(OutputPriority::Critical, b"shutdown".to_vec())
            .unwrap();
        let state = output.shared.state.lock().unwrap();
        assert!(matches!(
            &state.frames[0],
            OutputFrame::Bytes { priority: OutputPriority::Critical, bytes }
                if bytes == b"shutdown"
        ));
        assert!(
            metrics
                .prometheus(0)
                .contains("tiamud_slow_client_disconnects_total{transport=\"telnet\"} 1")
        );
    }

    #[tokio::test]
    async fn queued_telnet_writer_drains_before_closing() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let client = tokio::spawn(TcpStream::connect(address));
        let (server, _) = listener.accept().await.unwrap();
        let mut client = client.await.unwrap().unwrap();
        let connection = TelnetConnection::new(Box::new(server), address.ip());
        let queued = connection.into_queued(None);

        queued
            .output
            .bytes(OutputPriority::Interactive, b"first\r\n".to_vec())
            .unwrap();
        queued
            .output
            .bytes(OutputPriority::Critical, b"last\r\n".to_vec())
            .unwrap();
        queued.finish().await.unwrap();

        let mut wire = Vec::new();
        client.read_to_end(&mut wire).await.unwrap();
        assert_eq!(wire, b"first\r\nlast\r\n");
    }

    #[test]
    fn server_echo_is_accepted_only_while_reading_a_password() {
        let mut session = TelnetSession::default();
        assert_eq!(session.request_server_echo(true), [IAC, WILL, OPT_ECHO]);
        assert!(
            session
                .handle(vec![TelnetEvent::Negotiation {
                    command: DO,
                    option: OPT_ECHO,
                }])
                .is_empty()
        );

        assert_eq!(session.request_server_echo(false), [IAC, WONT, OPT_ECHO]);
        assert!(
            session
                .handle(vec![TelnetEvent::Negotiation {
                    command: DONT,
                    option: OPT_ECHO,
                }])
                .is_empty()
        );
        assert_eq!(
            session.handle(vec![TelnetEvent::Negotiation {
                command: DO,
                option: OPT_ECHO,
            }]),
            [IAC, WONT, OPT_ECHO]
        );
    }

    #[test]
    fn q_method_queues_opposite_requests_without_emitting_a_negotiation_loop() {
        let mut q = QState::No;
        assert_eq!(q.request(true), Some(true));
        assert_eq!(q, QState::WantYes);
        assert_eq!(q.request(false), None);
        assert_eq!(q, QState::WantYesQueued);
        assert_eq!(q.request(true), None);
        assert_eq!(q, QState::WantYes);
        assert_eq!(q.receive(true, true), None);
        assert_eq!(q, QState::Yes);

        assert_eq!(q.request(false), Some(false));
        assert_eq!(q.request(true), None);
        assert_eq!(q, QState::WantNoQueued);
        assert_eq!(q.receive(false, true), Some(true));
        assert_eq!(q, QState::WantYes);
        assert_eq!(q.receive(true, true), None);
        assert_eq!(q, QState::Yes);
    }

    #[test]
    fn q_method_matches_the_rfc_1143_receive_table() {
        let positive_cases = [
            (QState::No, true, QState::Yes, Some(true)),
            (QState::No, false, QState::No, Some(false)),
            (QState::Yes, true, QState::Yes, None),
            (QState::WantNo, true, QState::No, None),
            (QState::WantNoQueued, true, QState::Yes, None),
            (QState::WantYes, true, QState::Yes, None),
            (QState::WantYesQueued, true, QState::WantNo, Some(false)),
        ];
        for (initial, accept, expected, reply) in positive_cases {
            let mut state = initial;
            assert_eq!(state.receive(true, accept), reply, "from {initial:?}");
            assert_eq!(state, expected, "from {initial:?}");
        }

        let negative_cases = [
            (QState::No, QState::No, None),
            (QState::Yes, QState::No, Some(false)),
            (QState::WantNo, QState::No, None),
            (QState::WantNoQueued, QState::WantYes, Some(true)),
            (QState::WantYes, QState::No, None),
            (QState::WantYesQueued, QState::No, None),
        ];
        for (initial, expected, reply) in negative_cases {
            let mut state = initial;
            assert_eq!(state.receive(false, true), reply, "from {initial:?}");
            assert_eq!(state, expected, "from {initial:?}");
        }
    }

    #[test]
    fn repeated_and_crossed_negotiations_are_idempotent_in_both_directions() {
        let mut session = TelnetSession::default();
        session.begin_initial_negotiation();

        assert!(
            session
                .handle(vec![TelnetEvent::Negotiation {
                    command: DO,
                    option: OPT_EOR,
                }])
                .is_empty()
        );
        assert!(session.local_enabled(OPT_EOR));
        assert!(
            session
                .handle(vec![
                    TelnetEvent::Negotiation {
                        command: DO,
                        option: OPT_EOR,
                    },
                    TelnetEvent::Negotiation {
                        command: DO,
                        option: OPT_EOR,
                    },
                ])
                .is_empty()
        );

        let first_unsolicited = session.handle(vec![TelnetEvent::Negotiation {
            command: WILL,
            option: 200,
        }]);
        assert_eq!(first_unsolicited, [IAC, DONT, 200]);
        let repeated_refusal = session.handle(vec![TelnetEvent::Negotiation {
            command: WONT,
            option: 200,
        }]);
        assert!(repeated_refusal.is_empty());
    }

    #[test]
    fn fragmented_option_stream_has_the_same_q_state_and_output_as_a_whole_frame() {
        let wire = [
            IAC,
            WILL,
            OPT_TTYPE,
            IAC,
            WILL,
            OPT_CHARSET,
            IAC,
            DO,
            OPT_EOR,
        ];

        let mut whole_parser = TelnetParser::default();
        whole_parser.feed(&wire);
        let mut whole_session = TelnetSession::default();
        whole_session.begin_initial_negotiation();
        let whole_output = whole_session.handle(whole_parser.take_events());

        let mut fragmented_parser = TelnetParser::default();
        let mut fragmented_session = TelnetSession::default();
        fragmented_session.begin_initial_negotiation();
        let mut fragmented_output = Vec::new();
        for byte in wire {
            fragmented_parser.feed(&[byte]);
            fragmented_output.extend(fragmented_session.handle(fragmented_parser.take_events()));
        }

        assert_eq!(fragmented_output, whole_output);
        assert_eq!(fragmented_session.q.local, whole_session.q.local);
        assert_eq!(fragmented_session.q.remote, whole_session.q.remote);
        assert_eq!(
            fragmented_session.ttype_requests_sent,
            whole_session.ttype_requests_sent
        );
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
            let mut offset = 0;
            while offset < bytes.len() {
                let chunk = 1 + (next_random(&mut seed) as usize % 31);
                let end = (offset + chunk).min(bytes.len());
                fragmented_data.extend(fragmented.feed(&bytes[offset..end]));
                offset = end;
            }
            let fragmented_events = fragmented.take_events();

            assert_eq!(fragmented_data, whole_data);
            assert_eq!(fragmented_events, whole_events);
            assert_eq!(fragmented.state, whole.state);
            assert!(fragmented.subnegotiation_data.len() <= MAX_SUBNEGOTIATION_BYTES);
        }
    }

    #[tokio::test]
    async fn shutdown_signal_stops_the_telnet_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let store = Arc::new(
            crate::persistence::SqliteCharacterStore::open(":memory:")
                .await
                .unwrap(),
        );
        let world = WorldHandle::spawn(tiamud_core::World::demo(), store);
        let auth = Arc::new(AuthRateLimiter::per_minute(10, 5));
        let sessions = Arc::new(ActiveSessionLimiter::new(4, 16));
        let metrics = Arc::new(RuntimeMetrics::default());
        let (shutdown, receiver) = watch::channel(false);
        let transport = TelnetTransport::plaintext_for_tests();
        let runtime = TelnetRuntime::new(world, 12, 20, auth, sessions, metrics);
        let task = tokio::spawn(run(listener, runtime, transport, receiver));

        shutdown.send(true).unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), task)
                .await
                .unwrap()
                .unwrap()
                .is_ok()
        );
    }

    fn gmcp_payloads(bytes: &[u8]) -> Vec<Vec<u8>> {
        let mut parser = TelnetParser::default();
        assert!(parser.feed(bytes).is_empty());
        parser
            .take_events()
            .into_iter()
            .filter_map(|event| match event {
                TelnetEvent::Subnegotiation {
                    option: OPT_GMCP,
                    data,
                } => Some(data),
                _ => None,
            })
            .collect()
    }

    fn gmcp_cell_frame_session() -> TelnetSession {
        let mut session = TelnetSession::default();
        session.begin_initial_negotiation();
        session.handle(vec![TelnetEvent::Negotiation {
            command: DO,
            option: OPT_GMCP,
        }]);
        session.handle(vec![TelnetEvent::Subnegotiation {
            option: OPT_GMCP,
            data: br#"Core.Supports.Set ["Tiamud.Client 1","Tiamud.CellFrame 1","Tiamud.Assets 1","Client.Media 1"]"#.to_vec(),
        }]);
        session
    }

    fn sample_scene() -> SceneView {
        SceneView {
            view_id: "opaque-room".to_owned(),
            revision: 7,
            title: "Lantern Square".to_owned(),
            prose: vec!["Rain shines on the stones.".to_owned()],
            image: Some(tiamud_core::SceneImage {
                alt: "Lanterns in the rain.".to_owned(),
                localized_alt: BTreeMap::new(),
                manifest: Some(format!("/assets/{}/manifest.json", "a".repeat(64))),
                avif: None,
                webp: None,
                jpeg: None,
                width: None,
                height: None,
            }),
            audio: None,
            exits: vec![tiamud_core::SceneExit {
                id: "opaque-road".to_owned(),
                label: "north".to_owned(),
                command: "north".to_owned(),
            }],
            items: Vec::new(),
            npcs: Vec::new(),
            weather: tiamud_core::WeatherView {
                condition: "rain".to_owned(),
                day: 2,
                hour: 21,
                next_change_in_ticks: 4,
            },
        }
    }

    #[test]
    fn official_client_profiles_consume_oob_data_without_dangerous_commands() {
        let tintin = include_str!("../../../clients/tintin/tiamud.tin");
        assert!(tintin.contains("Core.Supports.Set"));
        assert!(tintin.contains("Room.Info"));
        assert!(tintin.contains("Tiamud.Assets"));
        assert!(tintin.contains("Tiamud.CellFrame 1"));
        assert!(tintin.contains("Char.Vitals 1"));
        assert!(tintin.contains("Char.Items 1"));
        assert!(tintin.contains("tiamud-graphics-off"));
        assert!(tintin.contains("[0-9a-f]{64}"));
        assert!(tintin.contains("reducedMotion"));
        assert!(!tintin.contains("#SYSTEM"));
        assert!(!tintin.contains("#RUN"));
        assert!(!tintin.contains("#EVENT {PRESSED LINK"));

        let mudlet = include_str!("../../../clients/mudlet/tiamud.lua");
        assert!(mudlet.contains("gmcp.Room.Info"));
        assert!(mudlet.contains("gmcp.Client.Media.Play"));
        assert!(mudlet.contains("Tiamud.CellFrame 1"));
        assert!(mudlet.contains("Char.Vitals 1"));
        assert!(mudlet.contains("Char.Items 1"));
        assert!(mudlet.contains("function Tiamud.enableGraphics"));
        assert!(mudlet.contains("#digest ~= 64"));
        assert!(mudlet.contains("reducedMotion"));
        assert!(!mudlet.contains("loadstring"));
        assert!(!mudlet.contains("os.execute"));
        assert!(!mudlet.contains("send("));
        assert!(!mudlet.contains("downloadFile"));
        assert!(!mudlet.contains("playSoundFile"));
        assert!(!mudlet.contains("openUrl"));
    }

    fn next_random(seed: &mut u64) -> u64 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        *seed
    }
}
