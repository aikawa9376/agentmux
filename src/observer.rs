//! One state owner per live tmux server. PTYs and topology remain owned by tmux.
use crate::{
    config::LoadedConfig,
    control,
    model::{AgentState, ObserverInfo, PaneRecord, Snapshot},
    tmux::Tmux,
    watcher,
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::{HashMap, VecDeque},
    fs::{self, OpenOptions},
    hash::{Hash, Hasher},
    io::{BufRead, BufReader, Read, Write},
    os::unix::{
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        net::{UnixListener, UnixStream},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const VERSION: u32 = 1;
const EVENT_LIMIT: usize = 256;
const MESSAGE_LIMIT: u64 = 16 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Snapshot {
        #[serde(default)]
        client: Option<String>,
    },
    Refresh,
    Seen {
        pane: String,
        #[serde(default)]
        client: Option<String>,
    },
    Action {
        request: control::Request,
    },
    Events {
        epoch: String,
        since: u64,
    },
    Explain {
        pane: String,
    },
    Overlay {
        client: String,
        pid: i32,
        active: bool,
    },
    Status,
    Stop,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,
    pub kind: String,
    pub pane: String,
    pub generation: u64,
    pub state: AgentState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub epoch: String,
    pub pane: String,
    pub generation: u64,
    pub seq: u64,
    pub already_working: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct Response {
    version: u32,
    #[serde(default)]
    result: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

pub fn call<T: DeserializeOwned>(path: &Path, request: &Request) -> Result<T> {
    let mut stream =
        UnixStream::connect(path).context("observer unavailable; run agentmux daemon start")?;
    stream.set_read_timeout(Some(Duration::from_secs(8)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    serde_json::to_writer(&mut stream, request)?;
    stream.write_all(b"\n")?;
    let mut line = String::new();
    BufReader::new(stream.take(MESSAGE_LIMIT)).read_line(&mut line)?;
    let response: Response = serde_json::from_str(&line).context("invalid observer response")?;
    ensure!(
        response.version == VERSION,
        "observer protocol mismatch; restart the observer"
    );
    if let Some(error) = response.error {
        bail!("{error}");
    }
    Ok(serde_json::from_value(response.result)?)
}

pub fn snapshot(path: &Path, client: Option<&str>) -> Result<Snapshot> {
    call(
        path,
        &Request::Snapshot {
            client: client.map(str::to_owned),
        },
    )
}
pub fn refresh(path: &Path) -> Result<()> {
    call(path, &Request::Refresh)
}
pub fn mark_seen(path: &Path, pane: &str, client: Option<&str>) -> Result<()> {
    call(
        path,
        &Request::Seen {
            pane: pane.into(),
            client: client.map(str::to_owned),
        },
    )
}
pub fn action(path: &Path, request: control::Request) -> Result<Receipt> {
    call(path, &Request::Action { request })
}

pub fn endpoint(tmux: &Tmux) -> Result<PathBuf> {
    let identity = tmux.run(["display-message", "-p", "#{socket_path}\x1f#{pid}"])?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    identity.trim().hash(&mut hasher);
    // A short private path also works when XDG_RUNTIME_DIR exceeds sun_path.
    let uid = unsafe { libc::geteuid() };
    // Do not key service discovery on each caller's TMPDIR: popup/server
    // environments can differ while addressing the same tmux server.
    let directory = PathBuf::from("/tmp").join(format!("agentmux-{uid}"));
    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700);
    match builder.create(&directory) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let metadata = fs::symlink_metadata(&directory)?;
    ensure!(
        metadata.is_dir() && metadata.uid() == uid && metadata.mode() & 0o077 == 0,
        "observer directory must be private and owned by the current user"
    );
    Ok(directory.join(format!("{:016x}.sock", hasher.finish())))
}

pub fn ensure_running(tmux: &mut Tmux) -> Result<PathBuf> {
    let path = endpoint(tmux)?;
    if call::<serde_json::Value>(&path, &Request::Status).is_err() {
        let mut command = Command::new(std::env::current_exe()?);
        if let Some(socket) = tmux.socket_name() {
            command.args(["--socket-name", socket]);
        }
        command.args(["daemon", "run"]);
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path.with_extension("log"))?;
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log);
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command.spawn().context("could not start observer")?;
        let deadline = Instant::now() + Duration::from_secs(6);
        loop {
            if call::<serde_json::Value>(&path, &Request::Status).is_ok() {
                break;
            }
            if let Some(status) = child.try_wait()? {
                // Another concurrent starter may have won the lock.
                ensure!(
                    status.success(),
                    "observer startup failed; inspect {}",
                    path.with_extension("log").display()
                );
            }
            ensure!(
                Instant::now() < deadline,
                "observer startup timed out; inspect {}",
                path.with_extension("log").display()
            );
            thread::sleep(Duration::from_millis(25));
        }
        // Reap a child when its tmux server eventually exits, without tying its lifetime to this CLI.
        thread::spawn(move || {
            let _ = child.wait();
        });
    }
    tmux.use_observer(path.clone());
    Ok(path)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Tracked {
    identity: String,
    generation: u64,
    state: AgentState,
    state_seq: u64,
    completion_seq: Option<u64>,
    seen: u64,
    had_work: bool,
    #[serde(skip)]
    pending_idle: Option<Instant>,
    #[serde(skip)]
    clients: HashMap<String, u64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Ledger {
    seq: u64,
    generation: u64,
    tracked: HashMap<String, Tracked>,
}

struct Store {
    ledger: Ledger,
    epoch: String,
    snapshot: Snapshot,
    events: VecDeque<Event>,
    overlays: HashMap<String, i32>,
}

impl Store {
    fn new(ledger: Ledger) -> Self {
        let epoch = format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        Self {
            ledger,
            epoch,
            snapshot: Snapshot::default(),
            events: VecDeque::new(),
            overlays: HashMap::new(),
        }
    }

    fn event(&mut self, kind: &str, pane: &str, generation: u64, state: AgentState) -> u64 {
        self.ledger.seq += 1;
        self.events.push_back(Event {
            seq: self.ledger.seq,
            kind: kind.into(),
            pane: pane.into(),
            generation,
            state,
        });
        if self.events.len() > EVENT_LIMIT {
            self.events.pop_front();
        }
        self.ledger.seq
    }

    fn observe(&mut self, mut snapshot: Snapshot, now: Instant) -> Vec<PaneRecord> {
        let mut notify = Vec::new();
        for pane in &mut snapshot.panes {
            if !pane.is_agent() {
                continue;
            }
            let identity = format!(
                "{}:{}",
                pane.agent_kind.as_deref().unwrap_or_default(),
                pane.instance_id
            );
            let mut tracked = self.ledger.tracked.remove(&pane.pane_id);
            if tracked.as_ref().is_none_or(|t| t.identity != identity) {
                self.ledger.generation += 1;
                let generation = self.ledger.generation;
                let state = if pane.state == AgentState::Done {
                    AgentState::Idle
                } else {
                    pane.state
                };
                let seq = self.event("agent.started", &pane.pane_id, generation, state);
                tracked = Some(Tracked {
                    identity,
                    generation,
                    state,
                    state_seq: seq,
                    completion_seq: None,
                    seen: seq,
                    had_work: state == AgentState::Working,
                    pending_idle: None,
                    clients: HashMap::new(),
                });
                if state == AgentState::Blocked {
                    notify.push(pane.clone());
                }
            }
            let mut tracked = tracked.expect("tracked agent");
            let mut next = if pane.state == AgentState::Done {
                AgentState::Idle
            } else {
                pane.state
            };
            if pane.state_source.ends_with(":skip") {
                next = tracked.state;
            }
            // A transient missing spinner is not completion. Native idle is authoritative.
            if next == AgentState::Idle
                && tracked.had_work
                && !pane.state_source.starts_with("native:")
            {
                let start = *tracked.pending_idle.get_or_insert(now);
                if now.saturating_duration_since(start) < Duration::from_millis(300) {
                    next = tracked.state;
                }
            } else {
                tracked.pending_idle = None;
            }
            if next != tracked.state {
                let seq = self.event("agent.state", &pane.pane_id, tracked.generation, next);
                tracked.state_seq = seq;
                if next == AgentState::Idle && tracked.had_work {
                    tracked.completion_seq = Some(seq);
                    tracked.had_work = false;
                    notify.push(pane.clone());
                }
                if next == AgentState::Blocked {
                    notify.push(pane.clone());
                }
                tracked.state = next;
            }
            if next == AgentState::Working {
                tracked.had_work = true;
            }
            pane.state = tracked.state;
            pane.generation = tracked.generation;
            pane.state_seq = tracked.state_seq;
            pane.completion_seq = tracked.completion_seq;
            self.ledger.tracked.insert(pane.pane_id.clone(), tracked);
        }
        let removed: Vec<_> = self
            .ledger
            .tracked
            .keys()
            .filter(|id| !snapshot.pane(id).is_some_and(PaneRecord::is_agent))
            .cloned()
            .collect();
        for id in removed {
            if let Some(t) = self.ledger.tracked.remove(&id) {
                self.event("agent.removed", &id, t.generation, AgentState::Dead);
            }
        }
        self.snapshot = snapshot;
        notify
    }

    fn view(&self, client: Option<&str>) -> Snapshot {
        let mut snapshot = self.snapshot.clone();
        for pane in &mut snapshot.panes {
            if let Some(t) = self.ledger.tracked.get(&pane.pane_id) {
                let seen = client.map_or(t.seen, |c| *t.clients.get(c).unwrap_or(&0));
                if pane.state == AgentState::Idle && t.completion_seq.is_some_and(|seq| seq > seen)
                {
                    pane.state = AgentState::Done;
                }
            }
        }
        snapshot.spaces = Snapshot::build_spaces(&snapshot.panes);
        snapshot.observer = Some(ObserverInfo {
            version: VERSION,
            epoch: self.epoch.clone(),
            seq: self.ledger.seq,
        });
        snapshot
    }

    fn seen(&mut self, pane: &str, client: Option<&str>, shared: bool) -> Result<()> {
        let t = self
            .ledger
            .tracked
            .get_mut(pane)
            .context("agent unavailable")?;
        let seq = t.completion_seq.unwrap_or(t.state_seq);
        let changed = shared
            && t.state == AgentState::Idle
            && t.completion_seq
                .is_some_and(|completion| completion > t.seen);
        let generation = t.generation;
        if shared {
            t.seen = seq;
        }
        if let Some(client) = client {
            // Bound transient viewer identities; actual tmux clients are few.
            if t.clients.len() < 128 || t.clients.contains_key(client) {
                t.clients.insert(client.into(), seq);
            }
        }
        if changed {
            self.event("agent.seen", pane, generation, AgentState::Idle);
        }
        Ok(())
    }

    fn events_since(&self, epoch: &str, since: u64) -> Result<Vec<Event>> {
        ensure!(
            epoch == self.epoch,
            "observer_restarted: take a new snapshot"
        );
        ensure!(since <= self.ledger.seq, "invalid sequence");
        if since < self.ledger.seq {
            ensure!(
                self.events
                    .front()
                    .is_some_and(|e| since >= e.seq.saturating_sub(1)),
                "events_lost: take a new snapshot"
            );
        }
        Ok(self
            .events
            .iter()
            .filter(|e| e.seq > since)
            .cloned()
            .collect())
    }
}

struct Service {
    tmux: Tmux,
    store: Mutex<Store>,
    scan: Mutex<()>,
    actions: Mutex<()>,
    stop: AtomicBool,
    path: PathBuf,
    notifications: mpsc::SyncSender<PaneRecord>,
}

impl Service {
    fn save(&self, store: &Store) -> Result<()> {
        let path = self.path.with_extension("json");
        let temporary = self.path.with_extension("json.tmp");
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temporary)?;
        serde_json::to_writer(&mut file, &store.ledger)?;
        file.sync_all()?;
        fs::rename(temporary, path)?;
        Ok(())
    }

    fn refresh(&self) -> Result<()> {
        let _scan = self.scan.lock().unwrap();
        let snapshot = self.tmux.raw_snapshot()?;
        let mut store = self.store.lock().unwrap();
        store
            .overlays
            .retain(|_, pid| crate::tmux::process_alive(*pid));
        let seq = store.ledger.seq;
        let notifications = store.observe(snapshot, Instant::now());
        if store.ledger.seq != seq {
            self.save(&store)?;
        }
        // Viewing a real focused pane is client-specific; polling previews is passive.
        if let Ok(clients) = self.tmux.run([
            "list-clients",
            "-F",
            "#{client_name}\x1f#{pane_id}\x1f#{client_control_mode}",
        ]) {
            for line in clients.lines() {
                let fields: Vec<_> = line.split('\x1f').collect();
                let overlay = fields.first().is_some_and(|name| {
                    store
                        .overlays
                        .get(*name)
                        .is_some_and(|pid| crate::tmux::process_alive(*pid))
                });
                if fields.len() == 3 && fields[2] == "0" && !overlay {
                    let _ = store.seen(fields[1], Some(fields[0]), false);
                }
            }
        }
        drop(store);
        for pane in notifications {
            let _ = self.notifications.try_send(pane);
        }
        Ok(())
    }

    fn handle(&self, request: Request) -> Result<serde_json::Value> {
        match request {
            Request::Snapshot { client } => Ok(serde_json::to_value(
                self.store.lock().unwrap().view(client.as_deref()),
            )?),
            Request::Refresh => {
                self.refresh()?;
                Ok(serde_json::Value::Null)
            }
            Request::Seen { pane, client } => {
                let mut store = self.store.lock().unwrap();
                store.seen(&pane, client.as_deref(), true)?;
                self.save(&store)?;
                Ok(serde_json::Value::Null)
            }
            Request::Events { epoch, since } => Ok(serde_json::to_value(
                self.store.lock().unwrap().events_since(&epoch, since)?,
            )?),
            Request::Action { request } => {
                let _action = self.actions.lock().unwrap();
                self.refresh()?;
                let receipt = {
                    let store = self.store.lock().unwrap();
                    let pane = store
                        .snapshot
                        .pane(&request.pane)
                        .context("agent unavailable")?;
                    Receipt {
                        epoch: store.epoch.clone(),
                        pane: pane.pane_id.clone(),
                        generation: pane.generation,
                        seq: store.ledger.seq,
                        already_working: pane.state == AgentState::Working,
                    }
                };
                control::execute(&self.tmux, request)?;
                self.refresh()?;
                Ok(serde_json::to_value(receipt)?)
            }
            Request::Explain { pane } => {
                let snapshot = self.store.lock().unwrap().view(None);
                let pane = snapshot.pane(&pane).context("pane unavailable")?;
                let screen = self.tmux.capture_screen(&pane.pane_id)?;
                Ok(pane
                    .agent_kind
                    .as_deref()
                    .and_then(|kind| {
                        crate::detect::explain(
                            kind,
                            &crate::detect::strip_ansi(&screen),
                            &pane.title,
                        )
                    })
                    .map(serde_json::to_value)
                    .transpose()?
                    .unwrap_or_default())
            }
            Request::Status => {
                let store = self.store.lock().unwrap();
                Ok(
                    serde_json::json!({"version": VERSION, "pid": std::process::id(), "epoch": store.epoch, "seq": store.ledger.seq, "socket": self.path}),
                )
            }
            Request::Overlay {
                client,
                pid,
                active,
            } => {
                ensure!(pid > 0 && !client.is_empty(), "invalid overlay owner");
                let mut store = self.store.lock().unwrap();
                if active {
                    ensure!(
                        store.overlays.len() < 128 || store.overlays.contains_key(&client),
                        "too many overlay viewers"
                    );
                    store.overlays.insert(client, pid);
                } else if store.overlays.get(&client) == Some(&pid) {
                    store.overlays.remove(&client);
                }
                Ok(serde_json::Value::Null)
            }
            Request::Stop => Ok(serde_json::Value::Null),
        }
    }
}

pub struct OverlayGuard {
    path: PathBuf,
    client: String,
    pid: i32,
}

impl OverlayGuard {
    pub fn register(path: &Path, client: &str) -> Result<Self> {
        let guard = Self {
            path: path.into(),
            client: client.into(),
            pid: std::process::id() as i32,
        };
        call::<()>(
            path,
            &Request::Overlay {
                client: client.into(),
                pid: guard.pid,
                active: true,
            },
        )?;
        Ok(guard)
    }
}
impl Drop for OverlayGuard {
    fn drop(&mut self) {
        let _ = call::<()>(
            &self.path,
            &Request::Overlay {
                client: self.client.clone(),
                pid: self.pid,
                active: false,
            },
        );
    }
}

/// Run in the foreground; `ensure_running` is the detached launcher.
pub fn run(tmux: Tmux) -> Result<()> {
    let path = endpoint(&tmux)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path.with_extension("lock"))?;
    use std::os::fd::AsRawFd;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Ok(());
    }
    let _ = fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let ledger = match fs::read(path.with_extension("json")) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .context("invalid observer ledger; original file retained")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ledger::default(),
        Err(error) => return Err(error.into()),
    };
    let (notify_tx, notify_rx) = mpsc::sync_channel(16);
    let config = LoadedConfig::load();
    let notify_tmux = tmux.clone();
    let notification_config = config.config.notifications.clone();
    thread::spawn(move || {
        for pane in notify_rx {
            notify(&notify_tmux, &notification_config, pane);
        }
    });
    let service = Arc::new(Service {
        tmux,
        store: Mutex::new(Store::new(ledger)),
        scan: Mutex::new(()),
        actions: Mutex::new(()),
        stop: AtomicBool::new(false),
        path: path.clone(),
        notifications: notify_tx,
    });
    service.refresh()?;
    let observer = service.clone();
    let interval = Duration::from_millis(config.config.refresh.status_ms);
    thread::spawn(move || {
        let snapshot = observer.store.lock().unwrap().snapshot.clone();
        let events = snapshot.panes.first().and_then(|p| {
            watcher::start(
                &observer.tmux,
                &p.pane_id,
                snapshot.panes.iter().map(|p| p.pane_id.clone()).collect(),
            )
            .ok()
        });
        let mut pending = false;
        let mut last = Instant::now();
        let mut failures = 0;
        while !observer.stop.load(Ordering::Relaxed) {
            if let Some(events) = &events {
                for _ in 0..256 {
                    if events.try_recv().is_ok() {
                        pending = true;
                    } else {
                        break;
                    }
                }
            }
            let busy = observer
                .store
                .lock()
                .unwrap()
                .snapshot
                .agents()
                .any(|p| p.state == AgentState::Working);
            if last.elapsed()
                >= if busy || pending {
                    interval
                } else {
                    interval.max(Duration::from_secs(2))
                }
            {
                match observer.refresh() {
                    Ok(()) => failures = 0,
                    Err(error) => {
                        eprintln!("observer refresh: {error:#}");
                        failures += 1;
                    }
                }
                if failures >= 3 {
                    observer.stop.store(true, Ordering::Relaxed);
                }
                last = Instant::now();
                pending = false;
            }
            thread::sleep(Duration::from_millis(25));
        }
    });
    let (tx, rx) = mpsc::sync_channel::<UnixStream>(16);
    let rx = Arc::new(Mutex::new(rx));
    for _ in 0..4 {
        let (rx, service) = (rx.clone(), service.clone());
        thread::spawn(move || {
            loop {
                let Ok(mut stream) = rx.lock().unwrap().recv() else {
                    break;
                };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
                let mut stopping = false;
                let result = (|| -> Result<_> {
                    let mut line = String::new();
                    BufReader::new((&stream).take(64 * 1024)).read_line(&mut line)?;
                    let request = serde_json::from_str(&line)?;
                    stopping = matches!(request, Request::Stop);
                    service.handle(request)
                })();
                let response = match result {
                    Ok(result) => Response {
                        version: VERSION,
                        result,
                        error: None,
                    },
                    Err(error) => Response {
                        version: VERSION,
                        result: serde_json::Value::Null,
                        error: Some(format!("{error:#}")),
                    },
                };
                let _ = serde_json::to_writer(&mut stream, &response);
                let _ = stream.write_all(b"\n");
                if stopping {
                    service.stop.store(true, Ordering::Relaxed);
                }
            }
        });
    }
    while !service.stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = tx.try_send(stream);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10))
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => {
                eprintln!("observer accept: {error}");
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
    fs::remove_file(path)?;
    Ok(())
}

fn notify(tmux: &Tmux, config: &crate::config::NotificationConfig, pane: PaneRecord) {
    let state = if pane.state == AgentState::Blocked {
        "blocked"
    } else {
        "done"
    };
    let message: String = format!(
        "agentmux: {} {state} ({})",
        pane.display_name(),
        pane.location()
    )
    .chars()
    .filter(|c| !c.is_control() && *c != '#')
    .take(200)
    .collect();
    if config.tmux {
        if let Ok(clients) = tmux.run([
            "list-clients",
            "-F",
            "#{client_name}\x1f#{pane_id}\x1f#{client_control_mode}",
        ]) {
            for line in clients.lines() {
                let fields: Vec<_> = line.split('\x1f').collect();
                if fields.len() == 3 && fields[2] == "0" && fields[1] != pane.pane_id {
                    let _ = tmux.run(["display-message", "-c", fields[0], "-d", "5000", &message]);
                }
            }
        }
    }
    if config.system {
        if let Ok(mut child) = Command::new("notify-send")
            .args(["agentmux", &message])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            let deadline = Instant::now() + Duration::from_millis(500);
            while child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(20));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

pub fn wait(
    tmux: &Tmux,
    target: &str,
    states: &[AgentState],
    timeout: Duration,
    receipt: Option<&Receipt>,
) -> Result<PaneRecord> {
    ensure!(
        !timeout.is_zero(),
        "timeout: submission used the caller deadline; inspect before retrying"
    );
    let initial = tmux.resolve_pane(target)?;
    ensure!(initial.is_agent(), "target is not an agent");
    let generation = receipt.map_or(initial.generation, |r| r.generation);
    let epoch = receipt
        .map(|r| r.epoch.clone())
        .or(tmux.snapshot()?.observer.map(|o| o.epoch));
    let mut activity = receipt.is_none_or(|r| r.already_working);
    let start = Instant::now();
    loop {
        let snapshot = tmux.snapshot()?;
        ensure!(
            snapshot.observer.as_ref().map(|o| &o.epoch) == epoch.as_ref(),
            "observer restarted while waiting; inspect before retrying"
        );
        let pane = snapshot.pane(&initial.pane_id).context("agent exited")?;
        ensure!(
            pane.is_agent() && pane.generation == generation,
            "agent changed while waiting"
        );
        if let Some(receipt) = receipt {
            activity |= (pane.state_seq > receipt.seq
                && matches!(pane.state, AgentState::Working | AgentState::Blocked))
                || pane.completion_seq.is_some_and(|seq| seq > receipt.seq);
            ensure!(
                activity || start.elapsed() < Duration::from_secs(5),
                "agent_prompt_stalled: input may have been delivered; inspect before retrying"
            );
        }
        if activity && states.contains(&pane.state) {
            return Ok(pane.clone());
        }
        ensure!(
            start.elapsed() < timeout,
            "timeout: last state {}; input may have been delivered",
            pane.state.label()
        );
        thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(state: AgentState) -> Snapshot {
        Snapshot {
            panes: vec![PaneRecord {
                pane_id: "%1".into(),
                session_id: "$1".into(),
                session_name: "project".into(),
                agent_kind: Some("codex".into()),
                instance_id: "process-1".into(),
                state,
                state_source: "native:test".into(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn completion_is_retained_without_reads_marking_it_seen() {
        let mut store = Store::new(Ledger::default());
        let now = Instant::now();
        store.observe(sample(AgentState::Idle), now);
        assert_eq!(store.view(None).panes[0].state, AgentState::Idle);
        store.observe(sample(AgentState::Working), now);
        assert_eq!(store.observe(sample(AgentState::Idle), now).len(), 1);
        let completion = store.view(None).panes[0].completion_seq;
        assert!(completion.is_some());
        assert_eq!(store.view(None).panes[0].state, AgentState::Done);
        assert_eq!(store.view(None).panes[0].state, AgentState::Done);
        store.seen("%1", Some("alice"), true).unwrap();
        assert_eq!(store.view(None).panes[0].state, AgentState::Idle);
        assert_eq!(store.view(Some("alice")).panes[0].state, AgentState::Idle);
        assert_eq!(store.view(Some("bob")).panes[0].state, AgentState::Done);
        let ledger: Ledger =
            serde_json::from_slice(&serde_json::to_vec(&store.ledger).unwrap()).unwrap();
        let mut restored = Store::new(ledger);
        assert!(restored.observe(sample(AgentState::Idle), now).is_empty());
        assert_eq!(restored.view(None).panes[0].completion_seq, completion);
    }

    #[test]
    fn initial_done_and_agent_replacement_do_not_fabricate_completion() {
        let mut store = Store::new(Ledger::default());
        let now = Instant::now();
        assert!(store.observe(sample(AgentState::Done), now).is_empty());
        assert_eq!(store.view(None).panes[0].state, AgentState::Idle);
        store.observe(sample(AgentState::Working), now);
        let generation = store.view(None).panes[0].generation;
        let mut replacement = sample(AgentState::Idle);
        replacement.panes[0].instance_id = "process-2".into();
        assert!(store.observe(replacement, now).is_empty());
        let pane = &store.view(None).panes[0];
        assert_ne!(generation, pane.generation);
        assert_eq!(pane.completion_seq, None);
    }

    #[test]
    fn transient_idle_and_skipped_updates_keep_the_previous_working_state() {
        let mut store = Store::new(Ledger::default());
        let now = Instant::now();
        store.observe(sample(AgentState::Working), now);
        let mut skipped = sample(AgentState::Unknown);
        skipped.panes[0].state_source = "manifest:codex:viewer:skip".into();
        store.observe(skipped, now);
        assert_eq!(store.view(None).panes[0].state, AgentState::Working);
        let mut idle = sample(AgentState::Idle);
        idle.panes[0].state_source = "manifest:codex:idle-fallback".into();
        assert!(store.observe(idle.clone(), now).is_empty());
        assert_eq!(store.view(None).panes[0].state, AgentState::Working);
        store.observe(idle, now + Duration::from_millis(350));
        assert_eq!(store.view(None).panes[0].state, AgentState::Done);
    }

    #[test]
    fn subscriptions_report_loss_and_restart_instead_of_silent_gaps() {
        let mut store = Store::new(Ledger::default());
        let epoch = store.epoch.clone();
        for _ in 0..EVENT_LIMIT + 1 {
            store.event("agent.state", "%1", 1, AgentState::Working);
        }
        assert!(
            store
                .events_since(&epoch, 0)
                .unwrap_err()
                .to_string()
                .contains("events_lost")
        );
        assert_eq!(store.events_since(&epoch, 1).unwrap().len(), EVENT_LIMIT);
        assert!(
            store
                .events_since("old", store.ledger.seq)
                .unwrap_err()
                .to_string()
                .contains("observer_restarted")
        );
    }
}
