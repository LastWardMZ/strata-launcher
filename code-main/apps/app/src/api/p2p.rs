//! Amigos, chat, invitaciones y túneles P2P de Strata, sin servidor propio.
//!
//! Cada instalación tiene una identidad (clave Ed25519) y se conecta a otras por
//! su clave pública con iroh (QUIC, hole punching y relay como alternativa). La
//! identidad del otro extremo la autentica el propio handshake QUIC, así que
//! `remote_id()` es fiable para decidir qué puede hacer cada amigo.
//!
//! Protocolo (ALPN `strata/p2p/1`): cada stream bidireccional empieza con una
//! petición JSON con prefijo de longitud y se responde con otra. En `Tunnel`, tras
//! la respuesta `ok`, el stream pasa a transportar bytes TCP en bruto hacia un
//! puerto local del anfitrión (un mundo LAN o un servidor personal).

use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey, endpoint::{Connection, presets}};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::AbortHandle,
};

pub const ALPN: &[u8] = b"strata/p2p/1";
const MAX_FRAME: usize = 64 * 1024;
const MAX_NAME: usize = 32;
const MAX_TEXT: usize = 2000;
const MAX_BIO: usize = 500;
const MAX_MESSAGES: usize = 1000;
const MAX_TUNNELS_PER_FRIEND: usize = 16;
const MAX_INCOMING_PENDING: usize = 50;
const MAX_SHARES: usize = 16;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(12);

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum FriendStatus {
    Outgoing,
    Incoming,
    Accepted,
    Blocked,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Friend {
    pub id: String,
    pub name: String,
    pub status: FriendStatus,
    /// `false` mientras el otro extremo no haya confirmado nuestro último paso
    /// (solicitud enviada o aceptación); se reintenta en segundo plano.
    #[serde(default)]
    pub synced: bool,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct InviteInfo {
    pub share_id: String,
    /// `world` (mundo LAN) o `server` (servidor personal).
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub game_version: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ChatMessage {
    pub id: String,
    pub from_me: bool,
    pub text: String,
    pub ts: u64,
    pub delivered: bool,
    #[serde(default)]
    pub invite: Option<InviteInfo>,
}

#[derive(Clone, Serialize, Debug)]
pub struct Share {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub game_version: String,
    pub port: u16,
}

#[derive(Serialize)]
pub struct P2pInfo {
    pub id: String,
    pub name: String,
    pub bio: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Request {
    FriendRequest { name: String },
    FriendAccept { name: String },
    Chat { id: String, text: String, ts: u64 },
    Invite { id: String, info: InviteInfo, ts: u64 },
    Tunnel { share_id: String },
    Ping,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Response {
    Ok {
        #[serde(default)]
        accepted: bool,
        #[serde(default)]
        name: String,
    },
    Error { message: String },
}

#[derive(Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    name: String,
    /// Biografía del perfil. Es local: no se envía a otros launchers.
    #[serde(default)]
    bio: String,
    #[serde(default)]
    friends: Vec<Friend>,
}

type Notify = Arc<dyn Fn(serde_json::Value) + Send + Sync>;

pub struct Core {
    endpoint: Endpoint,
    dir: PathBuf,
    store: Mutex<Store>,
    chats: Mutex<HashMap<String, Vec<ChatMessage>>>,
    shares: Mutex<HashMap<String, Share>>,
    joins: Mutex<HashMap<String, (u16, AbortHandle)>>,
    tunnels: Mutex<HashMap<String, usize>>,
    conns: tokio::sync::Mutex<HashMap<String, Connection>>,
    hints: Mutex<HashMap<String, EndpointAddr>>,
    notify: Notify,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn clean_name(name: &str) -> String {
    let cleaned: String = name.chars().filter(|c| !c.is_control()).collect();
    cleaned.trim().chars().take(MAX_NAME).collect()
}

async fn write_frame<W: AsyncWriteExt + Unpin, T: Serialize>(w: &mut W, value: &T) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_FRAME {
        return Err("Mensaje demasiado grande".into());
    }
    w.write_all(&(bytes.len() as u32).to_be_bytes()).await.map_err(|e| e.to_string())?;
    w.write_all(&bytes).await.map_err(|e| e.to_string())
}

async fn read_frame<R: AsyncReadExt + Unpin, T: DeserializeOwned>(r: &mut R) -> Result<T, String> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).await.map_err(|e| e.to_string())?;
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err("Mensaje demasiado grande".into());
    }
    let mut bytes = vec![0u8; len];
    r.read_exact(&mut bytes).await.map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

fn parse_id(code: &str) -> Result<EndpointId, String> {
    code.trim().parse::<EndpointId>().map_err(|_| "Código de amigo no válido".to_string())
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn load_or_create_key(dir: &Path) -> Result<SecretKey, String> {
    let path = dir.join("identity.key");
    if let Ok(text) = fs::read_to_string(&path) {
        let array = from_hex(text.trim()).ok_or("Identidad P2P corrupta")?;
        return Ok(SecretKey::from_bytes(&array));
    }
    let key = SecretKey::generate();
    fs::write(&path, to_hex(&key.to_bytes())).map_err(|e| e.to_string())?;
    Ok(key)
}

impl Core {
    /// Arranca el motor. `relay` permite desactivar los relays públicos (pruebas).
    pub async fn start(dir: PathBuf, notify: Notify, relay: bool) -> Result<Arc<Self>, String> {
        fs::create_dir_all(dir.join("chats")).map_err(|e| e.to_string())?;
        let key = load_or_create_key(&dir)?;
        let builder = if relay {
            Endpoint::builder(presets::N0)
        } else {
            Endpoint::builder(presets::Minimal)
        };
        let endpoint = builder
            .secret_key(key)
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .map_err(|e| e.to_string())?;
        let mut store: Store = fs::read(dir.join("friends.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        if store.name.is_empty() {
            store.name = format!("Strata-{}", &endpoint.id().to_string()[..6]);
        }
        let core = Arc::new(Self {
            endpoint,
            dir,
            store: Mutex::new(store),
            chats: Mutex::new(HashMap::new()),
            shares: Mutex::new(HashMap::new()),
            joins: Mutex::new(HashMap::new()),
            tunnels: Mutex::new(HashMap::new()),
            conns: tokio::sync::Mutex::new(HashMap::new()),
            hints: Mutex::new(HashMap::new()),
            notify,
        });
        let accepting = core.clone();
        tokio::spawn(async move { accepting.accept_loop().await });
        let retrying = core.clone();
        tokio::spawn(async move { retrying.retry_loop().await });
        Ok(core)
    }

    pub async fn shutdown(&self) {
        for (_, (_, handle)) in self.joins.lock().unwrap().drain() {
            handle.abort();
        }
        self.endpoint.close().await;
    }

    /// Dirección conocida de un amigo (útil en pruebas y redes locales).
    #[allow(dead_code)]
    pub fn add_hint(&self, addr: EndpointAddr) {
        self.hints.lock().unwrap().insert(addr.id.to_string(), addr);
    }

    #[allow(dead_code)]
    pub fn addr(&self) -> EndpointAddr {
        self.endpoint.addr()
    }

    fn emit(&self, value: serde_json::Value) {
        (self.notify)(value);
    }

    fn save_store(&self) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(&*self.store.lock().unwrap()).map_err(|e| e.to_string())?;
        let path = self.dir.join("friends.json");
        let temp = path.with_extension("json.tmp");
        fs::write(&temp, bytes).map_err(|e| e.to_string())?;
        fs::rename(temp, path).map_err(|e| e.to_string())
    }

    fn chat_path(&self, friend: &str) -> PathBuf {
        self.dir.join("chats").join(format!("{friend}.json"))
    }

    fn with_chat<T>(&self, friend: &str, f: impl FnOnce(&mut Vec<ChatMessage>) -> T) -> T {
        let mut chats = self.chats.lock().unwrap();
        let messages = chats.entry(friend.to_string()).or_insert_with(|| {
            fs::read(self.chat_path(friend)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
        });
        let result = f(messages);
        if messages.len() > MAX_MESSAGES {
            let extra = messages.len() - MAX_MESSAGES;
            messages.drain(..extra);
        }
        if let Ok(bytes) = serde_json::to_vec(&*messages) {
            let _ = fs::write(self.chat_path(friend), bytes);
        }
        result
    }

    fn friend_status(&self, id: &str) -> Option<FriendStatus> {
        self.store.lock().unwrap().friends.iter().find(|f| f.id == id).map(|f| f.status)
    }

    fn require_friend(&self, id: &str) -> Result<(), String> {
        match self.friend_status(id) {
            Some(FriendStatus::Accepted) => Ok(()),
            _ => Err("No autorizado".into()),
        }
    }

    // ---------------------------------------------------------------- red

    async fn accept_loop(self: Arc<Self>) {
        while let Some(incoming) = self.endpoint.accept().await {
            let core = self.clone();
            tokio::spawn(async move {
                let Ok(connection) = incoming.await else { return };
                let remote = connection.remote_id().to_string();
                loop {
                    let Ok((send, recv)) = connection.accept_bi().await else { break };
                    let core = core.clone();
                    let remote = remote.clone();
                    tokio::spawn(async move {
                        let _ = core.handle_stream(&remote, send, recv).await;
                    });
                }
            });
        }
    }

    async fn handle_stream<W, R>(self: &Arc<Self>, remote: &str, mut send: W, mut recv: R) -> Result<(), String>
    where
        W: AsyncWriteExt + Unpin + Send + 'static,
        R: AsyncReadExt + Unpin + Send + 'static,
    {
        let request: Request = tokio::time::timeout(REQUEST_TIMEOUT, read_frame(&mut recv))
            .await
            .map_err(|_| "Tiempo agotado".to_string())??;
        if let Request::Tunnel { share_id } = request {
            return self.serve_tunnel(remote, &share_id, send, recv).await;
        }
        let response = match self.handle_request(remote, request) {
            Ok(response) => response,
            Err(message) => Response::Error { message },
        };
        write_frame(&mut send, &response).await?;
        let _ = send.shutdown().await;
        Ok(())
    }

    fn handle_request(&self, remote: &str, request: Request) -> Result<Response, String> {
        if self.friend_status(remote) == Some(FriendStatus::Blocked) {
            return Err("No autorizado".into());
        }
        match request {
            Request::Ping => {
                self.require_friend(remote)?;
                Ok(Response::Ok { accepted: true, name: self.my_name() })
            }
            Request::FriendRequest { name } => {
                let name = clean_name(&name);
                if name.is_empty() {
                    return Err("Nombre no válido".into());
                }
                let accepted = {
                    let mut store = self.store.lock().unwrap();
                    let pending = store.friends.iter().filter(|f| f.status == FriendStatus::Incoming).count();
                    match store.friends.iter_mut().find(|f| f.id == remote) {
                        Some(friend) => {
                            friend.name = name;
                            if friend.status == FriendStatus::Outgoing {
                                friend.status = FriendStatus::Accepted;
                                friend.synced = true;
                            }
                            friend.status == FriendStatus::Accepted
                        }
                        None => {
                            if pending >= MAX_INCOMING_PENDING {
                                return Err("Demasiadas solicitudes pendientes".into());
                            }
                            store.friends.push(Friend { id: remote.to_string(), name, status: FriendStatus::Incoming, synced: true });
                            false
                        }
                    }
                };
                self.save_store()?;
                self.emit(serde_json::json!({"type": "friends"}));
                Ok(Response::Ok { accepted, name: self.my_name() })
            }
            Request::FriendAccept { name } => {
                {
                    let mut store = self.store.lock().unwrap();
                    let friend = store.friends.iter_mut().find(|f| f.id == remote).ok_or("No hay solicitud")?;
                    match friend.status {
                        FriendStatus::Outgoing => {
                            friend.status = FriendStatus::Accepted;
                            friend.synced = true;
                            let name = clean_name(&name);
                            if !name.is_empty() {
                                friend.name = name;
                            }
                        }
                        FriendStatus::Accepted => {}
                        _ => return Err("No hay solicitud".into()),
                    }
                }
                self.save_store()?;
                self.emit(serde_json::json!({"type": "friends"}));
                Ok(Response::Ok { accepted: true, name: self.my_name() })
            }
            Request::Chat { id, text, ts } => {
                self.require_friend(remote)?;
                let text = text.trim().to_string();
                if text.is_empty() || text.chars().count() > MAX_TEXT {
                    return Err("Mensaje no válido".into());
                }
                self.store_incoming(remote, ChatMessage { id, from_me: false, text, ts, delivered: true, invite: None });
                Ok(Response::Ok { accepted: true, name: String::new() })
            }
            Request::Invite { id, info, ts } => {
                self.require_friend(remote)?;
                if !["world", "server"].contains(&info.kind.as_str()) {
                    return Err("Invitación no válida".into());
                }
                let info = InviteInfo {
                    title: clean_name(&info.title),
                    game_version: clean_name(&info.game_version),
                    ..info
                };
                self.store_incoming(
                    remote,
                    ChatMessage { id, from_me: false, text: String::new(), ts, delivered: true, invite: Some(info) },
                );
                Ok(Response::Ok { accepted: true, name: String::new() })
            }
            Request::Tunnel { .. } => Err("No autorizado".into()),
        }
    }

    fn store_incoming(&self, friend: &str, message: ChatMessage) {
        let added = self.with_chat(friend, |messages| {
            if messages.iter().any(|m| m.id == message.id) {
                false
            } else {
                messages.push(message);
                true
            }
        });
        if added {
            self.emit(serde_json::json!({"type": "message", "friend": friend}));
        }
    }

    fn my_name(&self) -> String {
        self.store.lock().unwrap().name.clone()
    }

    async fn get_conn(&self, friend: &str) -> Result<Connection, String> {
        let mut conns = self.conns.lock().await;
        if let Some(conn) = conns.get(friend) {
            if conn.close_reason().is_none() {
                return Ok(conn.clone());
            }
            conns.remove(friend);
        }
        let hint = self.hints.lock().unwrap().get(friend).cloned();
        let addr = match hint {
            Some(addr) => addr,
            None => EndpointAddr::from(parse_id(friend)?),
        };
        let conn = tokio::time::timeout(Duration::from_secs(15), self.endpoint.connect(addr, ALPN))
            .await
            .map_err(|_| "No se pudo conectar con tu amigo (¿está conectado?)".to_string())?
            .map_err(|e| e.to_string())?;
        conns.insert(friend.to_string(), conn.clone());
        Ok(conn)
    }

    async fn request(&self, friend: &str, request: &Request) -> Result<Response, String> {
        let result = async {
            let conn = self.get_conn(friend).await?;
            let (mut send, mut recv) = conn.open_bi().await.map_err(|e| e.to_string())?;
            write_frame(&mut send, request).await?;
            let _ = send.finish();
            tokio::time::timeout(REQUEST_TIMEOUT, read_frame::<_, Response>(&mut recv))
                .await
                .map_err(|_| "Tiempo agotado".to_string())?
        }
        .await;
        if result.is_err() {
            self.conns.lock().await.remove(friend);
        }
        match result? {
            Response::Error { message } => Err(message),
            ok => Ok(ok),
        }
    }

    async fn retry_loop(self: Arc<Self>) {
        loop {
            tokio::time::sleep(Duration::from_secs(15)).await;
            let _ = self.sync_once().await;
        }
    }

    /// Reintenta lo pendiente: solicitudes/aceptaciones sin confirmar y mensajes sin entregar.
    pub async fn sync_once(self: &Arc<Self>) -> Result<(), String> {
        let friends: Vec<Friend> = self.store.lock().unwrap().friends.clone();
        let mut tasks = Vec::new();
        for friend in friends {
            let core = self.clone();
            tasks.push(tokio::spawn(async move {
                match friend.status {
                    FriendStatus::Outgoing if !friend.synced => {
                        let _ = core.deliver_friend_request(&friend.id).await;
                    }
                    FriendStatus::Accepted => {
                        if !friend.synced {
                            let _ = core.deliver_accept(&friend.id).await;
                        }
                        let _ = core.deliver_pending_messages(&friend.id).await;
                    }
                    _ => {}
                }
            }));
        }
        for task in tasks {
            let _ = task.await;
        }
        Ok(())
    }

    async fn deliver_friend_request(&self, id: &str) -> Result<(), String> {
        let response = self.request(id, &Request::FriendRequest { name: self.my_name() }).await?;
        if let Response::Ok { accepted, name } = response {
            let mut changed = false;
            {
                let mut store = self.store.lock().unwrap();
                if let Some(friend) = store.friends.iter_mut().find(|f| f.id == id) {
                    friend.synced = true;
                    if accepted && friend.status == FriendStatus::Outgoing {
                        friend.status = FriendStatus::Accepted;
                    }
                    let name = clean_name(&name);
                    if !name.is_empty() {
                        friend.name = name;
                    }
                    changed = true;
                }
            }
            if changed {
                self.save_store()?;
                self.emit(serde_json::json!({"type": "friends"}));
            }
        }
        Ok(())
    }

    async fn deliver_accept(&self, id: &str) -> Result<(), String> {
        self.request(id, &Request::FriendAccept { name: self.my_name() }).await?;
        if let Some(friend) = self.store.lock().unwrap().friends.iter_mut().find(|f| f.id == id) {
            friend.synced = true;
        }
        self.save_store()
    }

    async fn deliver_pending_messages(&self, id: &str) -> Result<(), String> {
        let pending: Vec<ChatMessage> =
            self.with_chat(id, |m| m.iter().filter(|m| m.from_me && !m.delivered).cloned().collect());
        for message in pending {
            let request = match &message.invite {
                Some(info) => Request::Invite { id: message.id.clone(), info: info.clone(), ts: message.ts },
                None => Request::Chat { id: message.id.clone(), text: message.text.clone(), ts: message.ts },
            };
            self.request(id, &request).await?;
            self.with_chat(id, |m| {
                if let Some(found) = m.iter_mut().find(|m| m.id == message.id) {
                    found.delivered = true;
                }
            });
            self.emit(serde_json::json!({"type": "message", "friend": id}));
        }
        Ok(())
    }

    // ------------------------------------------------------------- túneles

    async fn serve_tunnel<W, R>(self: &Arc<Self>, remote: &str, share_id: &str, mut send: W, recv: R) -> Result<(), String>
    where
        W: AsyncWriteExt + Unpin + Send + 'static,
        R: AsyncReadExt + Unpin + Send + 'static,
    {
        let port = match self.authorize_tunnel(remote, share_id) {
            Ok(port) => port,
            Err(message) => {
                write_frame(&mut send, &Response::Error { message: message.clone() }).await?;
                let _ = send.shutdown().await;
                return Err(message);
            }
        };
        let guard = TunnelGuard::new(self.clone(), remote.to_string());
        let mut tcp = match TcpStream::connect(("127.0.0.1", port)).await {
            Ok(tcp) => tcp,
            Err(_) => {
                let message = "El mundo o servidor no está disponible".to_string();
                write_frame(&mut send, &Response::Error { message: message.clone() }).await?;
                let _ = send.shutdown().await;
                return Err(message);
            }
        };
        write_frame(&mut send, &Response::Ok { accepted: true, name: String::new() }).await?;
        let mut stream = tokio::io::join(recv, send);
        let _ = tokio::io::copy_bidirectional(&mut stream, &mut tcp).await;
        drop(guard);
        Ok(())
    }

    fn authorize_tunnel(&self, remote: &str, share_id: &str) -> Result<u16, String> {
        self.require_friend(remote)?;
        let port = self.shares.lock().unwrap().get(share_id).map(|s| s.port).ok_or("Ese recurso ya no se comparte")?;
        if *self.tunnels.lock().unwrap().get(remote).unwrap_or(&0) >= MAX_TUNNELS_PER_FRIEND {
            return Err("Demasiadas conexiones simultáneas".into());
        }
        Ok(port)
    }

    async fn tunnel_connection(&self, friend: &str, share_id: &str, mut tcp: TcpStream) -> Result<(), String> {
        let conn = self.get_conn(friend).await?;
        let (mut send, mut recv) = conn.open_bi().await.map_err(|e| e.to_string())?;
        write_frame(&mut send, &Request::Tunnel { share_id: share_id.to_string() }).await?;
        match tokio::time::timeout(REQUEST_TIMEOUT, read_frame::<_, Response>(&mut recv))
            .await
            .map_err(|_| "Tiempo agotado".to_string())??
        {
            Response::Error { message } => return Err(message),
            Response::Ok { .. } => {}
        }
        let mut stream = tokio::io::join(recv, send);
        let _ = tokio::io::copy_bidirectional(&mut stream, &mut tcp).await;
        Ok(())
    }

    // ------------------------------------------------------------ API pública

    pub fn info(&self) -> P2pInfo {
        P2pInfo { id: self.endpoint.id().to_string(), name: self.my_name(), bio: self.store.lock().unwrap().bio.clone() }
    }

    pub fn set_bio(&self, bio: &str) -> Result<(), String> {
        let bio: String = bio.chars().filter(|c| !c.is_control() || *c == '\n').collect();
        if bio.chars().count() > MAX_BIO {
            return Err(format!("La biografía admite hasta {MAX_BIO} caracteres"));
        }
        self.store.lock().unwrap().bio = bio.trim().to_string();
        self.save_store()
    }

    pub fn set_name(&self, name: &str) -> Result<(), String> {
        let name = clean_name(name);
        if name.is_empty() {
            return Err("El nombre no puede estar vacío".into());
        }
        self.store.lock().unwrap().name = name;
        self.save_store()
    }

    pub fn friends(&self) -> Vec<Friend> {
        self.store.lock().unwrap().friends.clone()
    }

    pub async fn add_friend(self: &Arc<Self>, code: &str) -> Result<Friend, String> {
        let id = parse_id(code)?.to_string();
        if id == self.endpoint.id().to_string() {
            return Err("Ese es tu propio código".into());
        }
        let friend = {
            let mut store = self.store.lock().unwrap();
            if let Some(existing) = store.friends.iter().find(|f| f.id == id) {
                return Err(match existing.status {
                    FriendStatus::Blocked => "Tienes bloqueado a ese usuario".to_string(),
                    FriendStatus::Accepted => "Ya sois amigos".to_string(),
                    FriendStatus::Incoming => "Ese usuario ya te envió una solicitud: acéptala".to_string(),
                    FriendStatus::Outgoing => "Ya enviaste una solicitud".to_string(),
                });
            }
            let friend = Friend { name: format!("Strata-{}", &id[..6]), id, status: FriendStatus::Outgoing, synced: false };
            store.friends.push(friend.clone());
            friend
        };
        self.save_store()?;
        self.emit(serde_json::json!({"type": "friends"}));
        let _ = self.deliver_friend_request(&friend.id).await;
        Ok(self.friends().into_iter().find(|f| f.id == friend.id).unwrap_or(friend))
    }

    pub async fn accept_friend(self: &Arc<Self>, id: &str) -> Result<(), String> {
        {
            let mut store = self.store.lock().unwrap();
            let friend = store.friends.iter_mut().find(|f| f.id == id).ok_or("Amigo desconocido")?;
            if friend.status != FriendStatus::Incoming {
                return Err("No hay solicitud pendiente".into());
            }
            friend.status = FriendStatus::Accepted;
            friend.synced = false;
        }
        self.save_store()?;
        self.emit(serde_json::json!({"type": "friends"}));
        let _ = self.deliver_accept(id).await;
        Ok(())
    }

    pub async fn remove_friend(&self, id: &str) -> Result<(), String> {
        self.store.lock().unwrap().friends.retain(|f| f.id != id);
        self.save_store()?;
        self.chats.lock().unwrap().remove(id);
        let _ = fs::remove_file(self.chat_path(id));
        self.drop_friend_access(id).await;
        self.emit(serde_json::json!({"type": "friends"}));
        Ok(())
    }

    pub async fn set_blocked(&self, id: &str, blocked: bool) -> Result<(), String> {
        {
            let mut store = self.store.lock().unwrap();
            if blocked {
                match store.friends.iter_mut().find(|f| f.id == id) {
                    Some(friend) => friend.status = FriendStatus::Blocked,
                    None => store.friends.push(Friend { id: parse_id(id)?.to_string(), name: format!("Strata-{}", &id[..6.min(id.len())]), status: FriendStatus::Blocked, synced: true }),
                }
            } else {
                store.friends.retain(|f| !(f.id == id && f.status == FriendStatus::Blocked));
            }
        }
        self.save_store()?;
        if blocked {
            self.drop_friend_access(id).await;
        }
        self.emit(serde_json::json!({"type": "friends"}));
        Ok(())
    }

    /// Cierra conexiones y túneles locales hacia un amigo retirado o bloqueado.
    async fn drop_friend_access(&self, id: &str) {
        if let Some(conn) = self.conns.lock().await.remove(id) {
            conn.close(0u32.into(), b"removed");
        }
        let mut joins = self.joins.lock().unwrap();
        let keys: Vec<String> = joins.keys().filter(|k| k.starts_with(&format!("{id}:"))).cloned().collect();
        for key in keys {
            if let Some((_, handle)) = joins.remove(&key) {
                handle.abort();
            }
        }
    }

    pub async fn is_online(&self, id: &str) -> bool {
        self.request(id, &Request::Ping).await.is_ok()
    }

    pub fn messages(&self, id: &str) -> Vec<ChatMessage> {
        self.with_chat(id, |m| m.clone())
    }

    pub async fn send_chat(self: &Arc<Self>, id: &str, text: &str) -> Result<ChatMessage, String> {
        self.require_friend(id).map_err(|_| "Solo puedes escribir a tus amigos".to_string())?;
        let text = text.trim().to_string();
        if text.is_empty() || text.chars().count() > MAX_TEXT {
            return Err(format!("El mensaje debe tener entre 1 y {MAX_TEXT} caracteres"));
        }
        let message = ChatMessage { id: uuid::Uuid::new_v4().to_string(), from_me: true, text, ts: now(), delivered: false, invite: None };
        self.with_chat(id, |m| m.push(message.clone()));
        let _ = self.deliver_pending_messages(id).await;
        Ok(self.messages(id).into_iter().find(|m| m.id == message.id).unwrap_or(message))
    }

    pub fn share_start(&self, kind: &str, title: &str, game_version: &str, port: u16) -> Result<Share, String> {
        if !["world", "server"].contains(&kind) || port == 0 {
            return Err("Recurso no válido".into());
        }
        let mut shares = self.shares.lock().unwrap();
        if let Some(existing) = shares.values().find(|s| s.port == port) {
            return Ok(existing.clone());
        }
        if shares.len() >= MAX_SHARES {
            return Err("Demasiados recursos compartidos".into());
        }
        let share = Share {
            id: uuid::Uuid::new_v4().to_string(),
            kind: kind.to_string(),
            title: clean_name(title),
            game_version: clean_name(game_version),
            port,
        };
        shares.insert(share.id.clone(), share.clone());
        Ok(share)
    }

    pub fn share_stop(&self, id: &str) {
        self.shares.lock().unwrap().remove(id);
    }

    pub fn shares(&self) -> Vec<Share> {
        self.shares.lock().unwrap().values().cloned().collect()
    }

    pub async fn invite(self: &Arc<Self>, friend: &str, share_id: &str) -> Result<ChatMessage, String> {
        self.require_friend(friend).map_err(|_| "Solo puedes invitar a tus amigos".to_string())?;
        let share = self.shares.lock().unwrap().get(share_id).cloned().ok_or("Ese recurso ya no se comparte")?;
        let info = InviteInfo { share_id: share.id, kind: share.kind, title: share.title, game_version: share.game_version };
        let message = ChatMessage {
            id: uuid::Uuid::new_v4().to_string(),
            from_me: true,
            text: String::new(),
            ts: now(),
            delivered: false,
            invite: Some(info),
        };
        self.with_chat(friend, |m| m.push(message.clone()));
        let _ = self.deliver_pending_messages(friend).await;
        Ok(self.messages(friend).into_iter().find(|m| m.id == message.id).unwrap_or(message))
    }

    /// Abre un puerto local que lleva al recurso compartido de un amigo y devuelve ese puerto.
    pub async fn join(self: &Arc<Self>, friend: &str, share_id: &str) -> Result<u16, String> {
        self.require_friend(friend).map_err(|_| "Solo puedes unirte a recursos de tus amigos".to_string())?;
        let key = format!("{friend}:{share_id}");
        if let Some((port, _)) = self.joins.lock().unwrap().get(&key) {
            return Ok(*port);
        }
        // Comprueba antes de abrir nada que el amigo está disponible.
        self.get_conn(friend).await?;
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let core = self.clone();
        let (friend_id, share) = (friend.to_string(), share_id.to_string());
        let task = tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let core = core.clone();
                let (friend_id, share) = (friend_id.clone(), share.clone());
                tokio::spawn(async move {
                    let _ = core.tunnel_connection(&friend_id, &share, tcp).await;
                });
            }
        });
        self.joins.lock().unwrap().insert(key, (port, task.abort_handle()));
        Ok(port)
    }

    pub fn join_stop(&self, friend: &str, share_id: &str) {
        if let Some((_, handle)) = self.joins.lock().unwrap().remove(&format!("{friend}:{share_id}")) {
            handle.abort();
        }
    }
}

/// Cuenta los túneles activos de un amigo y los libera al terminar.
struct TunnelGuard {
    core: Arc<Core>,
    friend: String,
}

impl TunnelGuard {
    fn new(core: Arc<Core>, friend: String) -> Self {
        *core.tunnels.lock().unwrap().entry(friend.clone()).or_insert(0) += 1;
        Self { core, friend }
    }
}

impl Drop for TunnelGuard {
    fn drop(&mut self) {
        if let Some(count) = self.core.tunnels.lock().unwrap().get_mut(&self.friend) {
            *count = count.saturating_sub(1);
        }
    }
}

// ------------------------------------------------------------------ Tauri

use tauri::{Emitter, Manager};

#[derive(Default)]
pub struct P2pState {
    core: tokio::sync::OnceCell<Arc<Core>>,
}

impl P2pState {
    /// Cierra túneles y conexiones al salir de la aplicación.
    pub fn close(&self) {
        if let Some(core) = self.core.get() {
            let core = core.clone();
            tauri::async_runtime::block_on(async move { core.shutdown().await });
        }
    }
}

async fn core<R: tauri::Runtime>(app: &tauri::AppHandle<R>, state: &tauri::State<'_, P2pState>) -> Result<Arc<Core>, String> {
    state
        .core
        .get_or_try_init(|| async {
            let dir = app.path().app_data_dir().map_err(|e| e.to_string())?.join("p2p");
            let handle = app.clone();
            let notify: Notify = Arc::new(move |value| {
                let _ = handle.emit("p2p-event", value);
            });
            Core::start(dir, notify, true).await
        })
        .await
        .cloned()
}

/// Frases con las que Minecraft anuncia el puerto al abrir un mundo a LAN. Las dos primeras
/// salen del registro del servidor integrado (no dependen del idioma del juego); las demás
/// son el mensaje de chat en inglés de distintas versiones.
const LAN_PORT_MARKERS: &[&str] = &[
    "Published LAN server on port ",
    "Started serving on ",
    "Local game hosted on port ",
    "The port number is [",
];

/// Último puerto anunciado en el texto de un log, o `None` si no hay ninguno.
fn lan_port_from_log(text: &str) -> Option<u16> {
    text.lines().rev().find_map(|line| {
        LAN_PORT_MARKERS.iter().find_map(|marker| {
            let rest = line.split(marker).nth(1)?;
            rest.trim_start().split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
        })
    })
}

#[derive(Serialize)]
pub struct LanWorld {
    pub instance_id: String,
    pub port: u16,
}

/// Mundos abiertos a LAN entre las instancias indicadas. Ignora las que no existen.
#[tauri::command]
pub async fn p2p_detect_lan_ports(instance_ids: Vec<String>) -> Result<Vec<LanWorld>, String> {
    let mut worlds = Vec::new();
    for instance_id in instance_ids.into_iter().take(200) {
        if let Ok(Some(port)) = detect_lan_port(&instance_id).await {
            worlds.push(LanWorld { instance_id, port });
        }
    }
    Ok(worlds)
}

/// Detecta el puerto LAN del mundo abierto en una instancia. Devuelve `None` si el
/// juego no tiene ningún mundo abierto a LAN (o el puerto ya no está escuchando).
#[tauri::command]
pub async fn p2p_detect_lan_port(instance_id: String) -> Result<Option<u16>, String> {
    detect_lan_port(&instance_id).await
}

async fn detect_lan_port(instance_id: &str) -> Result<Option<u16>, String> {
    let dir = theseus::instance::get_full_path(instance_id).await.map_err(|e| e.to_string())?;
    let path = dir.join("logs").join("latest.log");
    let Ok(mut file) = fs::File::open(&path) else { return Ok(None) };
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(size.saturating_sub(262_144))).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut file, &mut bytes).map_err(|e| e.to_string())?;
    let Some(port) = lan_port_from_log(&String::from_utf8_lossy(&bytes)) else { return Ok(None) };
    let listening = tokio::time::timeout(Duration::from_millis(800), TcpStream::connect(("127.0.0.1", port)))
        .await
        .map(|r| r.is_ok())
        .unwrap_or(false);
    Ok(listening.then_some(port))
}

/// Arranca el motor al abrir Strata, para recibir solicitudes y mensajes aunque
/// el usuario no tenga abierta la pantalla de Amigos.
pub fn start_in_background<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        let state = app.state::<P2pState>();
        if let Err(error) = core(&app, &state).await {
            tracing::warn!("No se pudo iniciar el motor P2P: {error}");
        }
    });
}

#[tauri::command]
pub async fn p2p_info<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, P2pState>) -> Result<P2pInfo, String> {
    Ok(core(&app, &state).await?.info())
}

#[tauri::command]
pub async fn p2p_set_name<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, P2pState>, name: String) -> Result<(), String> {
    core(&app, &state).await?.set_name(&name)
}

#[tauri::command]
pub async fn p2p_set_bio<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, P2pState>, bio: String) -> Result<(), String> {
    core(&app, &state).await?.set_bio(&bio)
}

#[tauri::command]
pub async fn p2p_friends<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, P2pState>) -> Result<Vec<Friend>, String> {
    Ok(core(&app, &state).await?.friends())
}

#[tauri::command]
pub async fn p2p_add_friend<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, P2pState>, code: String) -> Result<Friend, String> {
    core(&app, &state).await?.add_friend(&code).await
}

#[tauri::command]
pub async fn p2p_accept_friend<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, P2pState>, id: String) -> Result<(), String> {
    core(&app, &state).await?.accept_friend(&id).await
}

#[tauri::command]
pub async fn p2p_remove_friend<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, P2pState>, id: String) -> Result<(), String> {
    core(&app, &state).await?.remove_friend(&id).await
}

#[tauri::command]
pub async fn p2p_block_friend<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, P2pState>, id: String, blocked: bool) -> Result<(), String> {
    core(&app, &state).await?.set_blocked(&id, blocked).await
}

#[tauri::command]
pub async fn p2p_is_online<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, P2pState>, id: String) -> Result<bool, String> {
    Ok(core(&app, &state).await?.is_online(&id).await)
}

#[tauri::command]
pub async fn p2p_messages<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, P2pState>, id: String) -> Result<Vec<ChatMessage>, String> {
    Ok(core(&app, &state).await?.messages(&id))
}

#[tauri::command]
pub async fn p2p_send_message<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, P2pState>, id: String, text: String) -> Result<ChatMessage, String> {
    core(&app, &state).await?.send_chat(&id, &text).await
}

#[tauri::command]
pub async fn p2p_share_start<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, P2pState>, kind: String, title: String, game_version: String, port: u16) -> Result<Share, String> {
    core(&app, &state).await?.share_start(&kind, &title, &game_version, port)
}

#[tauri::command]
pub async fn p2p_share_stop<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, P2pState>, id: String) -> Result<(), String> {
    core(&app, &state).await?.share_stop(&id);
    Ok(())
}

#[tauri::command]
pub async fn p2p_shares<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, P2pState>) -> Result<Vec<Share>, String> {
    Ok(core(&app, &state).await?.shares())
}

#[tauri::command]
pub async fn p2p_invite<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, P2pState>, friend: String, share_id: String) -> Result<ChatMessage, String> {
    core(&app, &state).await?.invite(&friend, &share_id).await
}

#[tauri::command]
pub async fn p2p_join<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, P2pState>, friend: String, share_id: String) -> Result<u16, String> {
    core(&app, &state).await?.join(&friend, &share_id).await
}

#[tauri::command]
pub async fn p2p_join_stop<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, P2pState>, friend: String, share_id: String) -> Result<(), String> {
    core(&app, &state).await?.join_stop(&friend, &share_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn silent() -> Notify {
        Arc::new(|_| {})
    }

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("strata-p2p-{name}-{}", uuid::Uuid::new_v4()))
    }

    async fn settle() {
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    async fn pair() -> (Arc<Core>, Arc<Core>) {
        let a = Core::start(temp("a"), silent(), false).await.unwrap();
        let b = Core::start(temp("b"), silent(), false).await.unwrap();
        settle().await;
        a.add_hint(b.addr());
        b.add_hint(a.addr());
        (a, b)
    }

    async fn befriend(a: &Arc<Core>, b: &Arc<Core>) {
        a.add_friend(&b.info().id).await.unwrap();
        assert_eq!(b.friends()[0].status, FriendStatus::Incoming);
        b.accept_friend(&a.info().id).await.unwrap();
        assert_eq!(a.friends()[0].status, FriendStatus::Accepted);
        assert_eq!(b.friends()[0].status, FriendStatus::Accepted);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn friends_chat_and_invites_work_between_two_instances() {
        let (a, b) = pair().await;
        assert!(a.add_friend(&a.info().id).await.is_err(), "no puedes añadirte a ti mismo");
        assert!(a.add_friend("no-es-un-codigo").await.is_err());
        befriend(&a, &b).await;
        assert!(a.add_friend(&b.info().id).await.is_err(), "ya sois amigos");

        a.send_chat(&b.info().id, "hola desde A").await.unwrap();
        b.send_chat(&a.info().id, "hola desde B").await.unwrap();
        assert!(b.messages(&a.info().id).iter().any(|m| !m.from_me && m.text == "hola desde A"));
        assert!(a.messages(&b.info().id).iter().any(|m| !m.from_me && m.text == "hola desde B"));
        assert!(a.messages(&b.info().id).iter().all(|m| !m.from_me || m.delivered));
        assert!(a.is_online(&b.info().id).await);

        let share = a.share_start("world", "Mi mundo", "1.21.1", 25565).unwrap();
        a.invite(&b.info().id, &share.id).await.unwrap();
        let invite = b.messages(&a.info().id).into_iter().find_map(|m| m.invite).expect("invitación recibida");
        assert_eq!((invite.kind.as_str(), invite.title.as_str()), ("world", "Mi mundo"));
        a.shutdown().await;
        b.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn undelivered_messages_arrive_after_the_friend_comes_back() {
        let a = Core::start(temp("a"), silent(), false).await.unwrap();
        let b_dir = temp("b");
        let b = Core::start(b_dir.clone(), silent(), false).await.unwrap();
        settle().await;
        a.add_hint(b.addr());
        b.add_hint(a.addr());
        befriend(&a, &b).await;
        let b_id = b.info().id;
        b.shutdown().await;
        drop(b);

        let message = a.send_chat(&b_id, "¿estás ahí?").await.unwrap();
        assert!(!message.delivered, "con el amigo desconectado queda pendiente");

        let b2 = Core::start(b_dir, silent(), false).await.unwrap();
        assert_eq!(b2.info().id, b_id, "misma identidad tras reiniciar");
        settle().await;
        a.add_hint(b2.addr());
        b2.add_hint(a.addr());
        a.sync_once().await.unwrap();
        assert!(b2.messages(&a.info().id).iter().any(|m| m.text == "¿estás ahí?"));
        assert!(a.messages(&b_id).iter().all(|m| m.delivered));
        a.shutdown().await;
        b2.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tunnel_carries_tcp_between_friends_and_rejects_strangers() {
        let echo = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let echo_port = echo.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut tcp, _)) = echo.accept().await {
                tokio::spawn(async move {
                    let (mut r, mut w) = tcp.split();
                    let _ = tokio::io::copy(&mut r, &mut w).await;
                });
            }
        });

        let (a, b) = pair().await;
        befriend(&a, &b).await;
        let share = a.share_start("server", "Servidor", "1.21.1", echo_port).unwrap();
        let port = b.join(&a.info().id, &share.id).await.unwrap();
        assert_eq!(b.join(&a.info().id, &share.id).await.unwrap(), port, "reutiliza el puerto");

        let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        client.write_all(b"ping por el tunel").await.unwrap();
        let mut buffer = [0u8; 17];
        tokio::time::timeout(Duration::from_secs(10), client.read_exact(&mut buffer)).await.unwrap().unwrap();
        assert_eq!(&buffer, b"ping por el tunel");

        // Un desconocido (no aceptado por A) no puede abrir el túnel aunque conozca el id del recurso.
        let c = Core::start(temp("c"), silent(), false).await.unwrap();
        settle().await;
        c.add_hint(a.addr());
        a.add_hint(c.addr());
        c.store.lock().unwrap().friends.push(Friend { id: a.info().id, name: "A".into(), status: FriendStatus::Accepted, synced: true });
        let stolen = c.join(&a.info().id, &share.id).await.unwrap();
        let mut intruder = TcpStream::connect(("127.0.0.1", stolen)).await.unwrap();
        let _ = intruder.write_all(b"hola").await;
        let mut received = Vec::new();
        let read = tokio::time::timeout(Duration::from_secs(10), intruder.read_to_end(&mut received)).await;
        assert!(read.is_ok(), "la conexión del desconocido debe cerrarse");
        assert!(received.is_empty(), "el desconocido no recibe datos del servidor");

        // Dejar de compartir corta las conexiones nuevas.
        a.share_stop(&share.id);
        let mut late = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let _ = late.write_all(b"x").await;
        let mut late_data = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(10), late.read_to_end(&mut late_data)).await;
        assert!(late_data.is_empty());
        a.shutdown().await;
        b.shutdown().await;
        c.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn blocked_users_cannot_message_and_identity_persists() {
        let dir = temp("persist");
        let first = Core::start(dir.clone(), silent(), false).await.unwrap();
        let id = first.info().id;
        first.set_name("Erik").unwrap();
        first.set_bio("Jugador de supervivencia\ny redstone").unwrap();
        assert!(first.set_bio(&"x".repeat(MAX_BIO + 1)).is_err(), "la biografía tiene un límite");
        first.shutdown().await;
        drop(first);
        let second = Core::start(dir, silent(), false).await.unwrap();
        assert_eq!(second.info().id, id, "la identidad se conserva entre arranques");
        assert_eq!(second.info().name, "Erik");
        assert_eq!(second.info().bio, "Jugador de supervivencia\ny redstone", "la biografía persiste");
        second.shutdown().await;

        let (a, b) = pair().await;
        befriend(&a, &b).await;
        a.set_blocked(&b.info().id, true).await.unwrap();
        let sent = b.send_chat(&a.info().id, "¿me oyes?").await.unwrap();
        assert!(!sent.delivered, "un bloqueado no puede entregar mensajes");
        assert!(a.messages(&b.info().id).is_empty());
        assert!(a.send_chat(&b.info().id, "no").await.is_err(), "no se escribe a un bloqueado");
        a.shutdown().await;
        b.shutdown().await;
    }

    #[test]
    fn lan_port_is_read_from_minecraft_logs() {
        let log = "[12:00:01] [Server thread/INFO]: Starting\n[12:01:10] [Server thread/INFO]: [System] [CHAT] Local game hosted on port 41234\n[12:05:00] [Server thread/INFO]: [System] [CHAT] Local game hosted on port 50555\n[12:06:00] [Server thread/INFO]: hola";
        assert_eq!(lan_port_from_log(log), Some(50555), "usa el último anuncio");
        // Líneas reales de Minecraft 26.3 (registro del servidor y chat).
        let real = "[18:00:01] [Render thread/INFO]: Published LAN server on port 55535\n[18:00:01] [Render thread/INFO]: [System] [CHAT] This world is now open to LAN. The port number is [55535]\n";
        assert_eq!(lan_port_from_log(real), Some(55535));
        assert_eq!(lan_port_from_log("[x] [CHAT] This world is now open to LAN. The port number is [41000]"), Some(41000));
        assert_eq!(lan_port_from_log("[x] Started serving on 42000"), Some(42000));
        assert_eq!(lan_port_from_log("sin nada"), None);
        assert_eq!(lan_port_from_log("Local game hosted on port abc"), None);
        assert_eq!(lan_port_from_log("Local game hosted on port 99999"), None, "fuera de rango");
    }

    #[test]
    fn hex_helpers_roundtrip_and_reject_garbage() {
        let bytes = [7u8; 32];
        assert_eq!(from_hex(&to_hex(&bytes)), Some(bytes));
        assert_eq!(from_hex("zz"), None);
        assert_eq!(from_hex(&"é".repeat(32)), None);
    }
}
