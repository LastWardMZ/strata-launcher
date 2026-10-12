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

use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey, endpoint::{Connection, RecvStream, SendStream, presets}};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
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
/// Los identificadores de mensaje y de recurso compartido son UUID: nada más largo es legítimo.
const MAX_ID: usize = 64;
/// Mensajes (chat o invitaciones) que se aceptan de un mismo amigo por minuto.
const MAX_INCOMING_PER_MINUTE: u32 = 60;
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
    /// `true` si es amigo por la cuenta de Strata (y no por haber intercambiado códigos a mano):
    /// su acceso depende de que la amistad de la cuenta siga existiendo.
    #[serde(default, alias = "via_account")]
    pub via_account: bool,
}

/// Modpack de Modrinth del que viene una instancia (para que el amigo instale el mismo).
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Default)]
pub struct ModpackRef {
    pub project_id: String,
    pub version_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub icon_url: Option<String>,
}

/// Lo justo para reconocer o reconstruir una instancia en el launcher de un amigo.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Default)]
pub struct InstanceRecipe {
    pub name: String,
    pub game_version: String,
    /// `vanilla`, `forge`, `fabric`, `quilt` o `neoforge`.
    pub loader: String,
    #[serde(default)]
    pub loader_version: Option<String>,
    #[serde(default)]
    pub modpack: Option<ModpackRef>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct InviteInfo {
    /// Vacío cuando `kind` es `instance` (no hay nada a lo que conectarse).
    pub share_id: String,
    /// `world` (mundo LAN), `server` (servidor personal) o `instance` (instancia compartida por el chat).
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub game_version: String,
    /// Instancia con la que juega quien invita. Los launchers antiguos no la envían.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<InstanceRecipe>,
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
    pub instance: Option<InstanceRecipe>,
    /// Instancia local desde la que se juega (solo en mundos): permite seguir el puerto LAN.
    #[serde(skip_serializing)]
    pub instance_id: Option<String>,
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

/// Devuelve el puerto LAN que tiene abierto ahora una instancia (el puerto cambia cada vez que se abre el mundo).
pub type PortResolver = Arc<dyn Fn(String) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<u16>> + Send>> + Send + Sync>;

pub struct Core {
    port_resolver: Mutex<Option<PortResolver>>,
    endpoint: Endpoint,
    dir: PathBuf,
    store: Mutex<Store>,
    chats: Mutex<HashMap<String, Vec<ChatMessage>>>,
    shares: Mutex<HashMap<String, Share>>,
    joins: Mutex<HashMap<String, (u16, AbortHandle)>>,
    tunnels: Mutex<HashMap<String, usize>>,
    /// Mensajes recibidos por amigo en el minuto en curso: (inicio de la ventana en segundos, cuántos).
    incoming_rate: Mutex<HashMap<String, (u64, u32)>>,
    conns: tokio::sync::Mutex<HashMap<String, Connection>>,
    hints: Mutex<HashMap<String, EndpointAddr>>,
    /// Códigos de los amigos de la cuenta de Strata: se aceptan sin pedir confirmación.
    trusted: Mutex<HashSet<String>>,
    /// Ajustes de privacidad de la cuenta que se aplican aquí, en el extremo que recibe.
    friend_requests_none: AtomicBool,
    invites_none: AtomicBool,
    notify: Notify,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn clean_name(name: &str) -> String {
    let cleaned: String = name.chars().filter(|c| !c.is_control()).collect();
    cleaned.trim().chars().take(MAX_NAME).collect()
}

/// Identificadores de Modrinth, versiones y cargadores: solo caracteres inofensivos.
fn clean_token(value: &str, max: usize) -> String {
    value.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+')).take(max).collect()
}

/// Un id de mensaje o de recurso válido: corto y solo con letras, números y guiones (los launchers envían UUID v4).
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn clean_recipe(recipe: InstanceRecipe) -> Result<InstanceRecipe, String> {
    let loader = clean_token(&recipe.loader, 16).to_lowercase();
    if !["vanilla", "forge", "fabric", "quilt", "neoforge"].contains(&loader.as_str()) {
        return Err("Instancia no válida".into());
    }
    let game_version = clean_token(&recipe.game_version, 32);
    if game_version.is_empty() {
        return Err("Instancia no válida".into());
    }
    let modpack = recipe.modpack.and_then(|m| {
        let (project_id, version_id) = (clean_token(&m.project_id, 32), clean_token(&m.version_id, 32));
        if project_id.is_empty() || version_id.is_empty() {
            return None;
        }
        // La imagen solo se admite si viene del CDN de Modrinth (nunca se descarga desde otro sitio).
        let icon_url = m.icon_url.filter(|u| u.len() <= 300 && u.starts_with("https://cdn.modrinth.com/"));
        Some(ModpackRef { project_id, version_id, title: clean_name(&m.title), icon_url })
    });
    Ok(InstanceRecipe {
        name: clean_name(&recipe.name),
        game_version,
        loader,
        loader_version: recipe.loader_version.map(|v| clean_token(&v, 48)).filter(|v| !v.is_empty()),
        modpack,
    })
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
            port_resolver: Mutex::new(None),
            endpoint,
            dir,
            store: Mutex::new(store),
            chats: Mutex::new(HashMap::new()),
            shares: Mutex::new(HashMap::new()),
            joins: Mutex::new(HashMap::new()),
            tunnels: Mutex::new(HashMap::new()),
            incoming_rate: Mutex::new(HashMap::new()),
            conns: tokio::sync::Mutex::new(HashMap::new()),
            hints: Mutex::new(HashMap::new()),
            trusted: Mutex::new(HashSet::new()),
            friend_requests_none: AtomicBool::new(false),
            invites_none: AtomicBool::new(false),
            notify,
        });
        let accepting = core.clone();
        tokio::spawn(async move { accepting.accept_loop().await });
        let retrying = core.clone();
        tokio::spawn(async move { retrying.retry_loop().await });
        Ok(core)
    }

    /// Permite que los mundos compartidos sigan al puerto LAN actual de su instancia.
    pub fn set_port_resolver(&self, resolver: PortResolver) {
        *self.port_resolver.lock().unwrap() = Some(resolver);
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
                let trusted = self.trusted.lock().unwrap().contains(remote);
                let accepted = {
                    let mut store = self.store.lock().unwrap();
                    let pending = store.friends.iter().filter(|f| f.status == FriendStatus::Incoming).count();
                    match store.friends.iter_mut().find(|f| f.id == remote) {
                        Some(friend) => {
                            friend.name = name;
                            if friend.status == FriendStatus::Outgoing
                                || (trusted && friend.status == FriendStatus::Incoming)
                            {
                                friend.status = FriendStatus::Accepted;
                                friend.synced = true;
                                friend.via_account = friend.via_account || trusted;
                            }
                            friend.status == FriendStatus::Accepted
                        }
                        None => {
                            if !trusted && self.friend_requests_none.load(Ordering::Relaxed) {
                                return Err("Este usuario no acepta solicitudes de amistad".into());
                            }
                            if trusted {
                                store.friends.push(Friend { id: remote.to_string(), name, status: FriendStatus::Accepted, synced: true, via_account: true });
                                true
                            } else {
                                if pending >= MAX_INCOMING_PENDING {
                                    return Err("Demasiadas solicitudes pendientes".into());
                                }
                                store.friends.push(Friend { id: remote.to_string(), name, status: FriendStatus::Incoming, synced: true, via_account: false });
                                false
                            }
                        }
                    }
                };
                self.save_store()?;
                self.emit(serde_json::json!({"type": "friends"}));
                Ok(Response::Ok { accepted, name: self.my_name() })
            }
            Request::FriendAccept { name } => {
                {
                    let trusted = self.trusted.lock().unwrap().contains(remote);
                    let mut store = self.store.lock().unwrap();
                    if trusted && !store.friends.iter().any(|f| f.id == remote) {
                        let name = clean_name(&name);
                        let name = if name.is_empty() { format!("Strata-{}", &remote[..6.min(remote.len())]) } else { name };
                        store.friends.push(Friend { id: remote.to_string(), name, status: FriendStatus::Accepted, synced: true, via_account: true });
                    }
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
                if text.is_empty() || text.chars().count() > MAX_TEXT || !valid_id(&id) {
                    return Err("Mensaje no válido".into());
                }
                self.allow_incoming(remote)?;
                // La hora la pone quien envía: nunca en el futuro (desordenaría la conversación).
                let ts = ts.min(now() + 300);
                self.store_incoming(remote, ChatMessage { id, from_me: false, text, ts, delivered: true, invite: None });
                Ok(Response::Ok { accepted: true, name: String::new() })
            }
            Request::Invite { id, info, ts } => {
                self.require_friend(remote)?;
                if self.invites_none.load(Ordering::Relaxed) {
                    return Err("Este usuario no acepta invitaciones".into());
                }
                if !["world", "server", "instance"].contains(&info.kind.as_str()) || !valid_id(&id) {
                    return Err("Invitación no válida".into());
                }
                // El id del recurso se guarda y se devuelve al unirse: acotado como cualquier otro id (vacío en instancias).
                if info.kind != "instance" && !valid_id(&info.share_id) {
                    return Err("Invitación no válida".into());
                }
                self.allow_incoming(remote)?;
                let ts = ts.min(now() + 300);
                let instance = match info.instance.clone() {
                    Some(recipe) => Some(clean_recipe(recipe)?),
                    None => None,
                };
                // Compartir una instancia no abre ningún túnel: no lleva `share_id` y exige la receta.
                let info = if info.kind == "instance" {
                    let recipe = instance.ok_or("Instancia no válida")?;
                    InviteInfo {
                        share_id: String::new(),
                        kind: info.kind,
                        title: if recipe.name.is_empty() { clean_name(&info.title) } else { recipe.name.clone() },
                        game_version: recipe.game_version.clone(),
                        instance: Some(recipe),
                    }
                } else {
                    InviteInfo {
                        title: clean_name(&info.title),
                        game_version: clean_name(&info.game_version),
                        instance,
                        ..info
                    }
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

    /// Límite de mensajes por minuto de cada amigo. Lo que se rechaza aquí queda pendiente en el origen y se reintenta.
    fn allow_incoming(&self, friend: &str) -> Result<(), String> {
        let now = now();
        let mut rates = self.incoming_rate.lock().unwrap();
        let entry = rates.entry(friend.to_string()).or_insert((now, 0));
        if now.saturating_sub(entry.0) >= 60 {
            *entry = (now, 0);
        }
        entry.1 += 1;
        if entry.1 > MAX_INCOMING_PER_MINUTE {
            return Err("Demasiados mensajes seguidos".into());
        }
        Ok(())
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
        let (port, instance_id, guard) = match self.authorize_tunnel(remote, share_id) {
            Ok(found) => found,
            Err(message) => {
                tracing::warn!("Túnel P2P rechazado ({share_id}): {message}");
                write_frame(&mut send, &Response::Error { message: message.clone() }).await?;
                let _ = send.shutdown().await;
                return Err(message);
            }
        };
        // Un mundo LAN cambia de puerto cada vez que se abre: se usa el que tiene abierto ahora la instancia.
        let resolver = self.port_resolver.lock().unwrap().clone();
        let port = match (instance_id, resolver) {
            (Some(instance_id), Some(resolver)) => resolver(instance_id).await.unwrap_or(port),
            _ => port,
        };
        let mut tcp = match TcpStream::connect(("127.0.0.1", port)).await {
            Ok(tcp) => tcp,
            Err(error) => {
                tracing::warn!("Túnel P2P: no se pudo conectar con el puerto local {port}: {error}");
                let message = "El mundo o servidor no está disponible: comprueba que sigue abierto a LAN".to_string();
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

    /// Autoriza el túnel y reserva su hueco a la vez: comprobar el límite y contar el túnel en dos pasos permitía que una
    /// ráfaga de aperturas simultáneas lo superara (el contador solo subía después de resolver el puerto).
    fn authorize_tunnel(self: &Arc<Self>, remote: &str, share_id: &str) -> Result<(u16, Option<String>, TunnelGuard), String> {
        self.require_friend(remote)?;
        let (port, instance_id) = self
            .shares
            .lock()
            .unwrap()
            .get(share_id)
            .map(|s| (s.port, s.instance_id.clone().filter(|_| s.kind == "world")))
            .ok_or("Ese recurso ya no se comparte (puede que tu amigo haya reiniciado Strata): pídele que vuelva a invitarte")?;
        let guard = TunnelGuard::reserve(self.clone(), remote.to_string()).ok_or("Demasiadas conexiones simultáneas")?;
        Ok((port, instance_id, guard))
    }

    /// Abre un canal hacia el recurso de un amigo y espera a que él lo acepte.
    async fn open_tunnel(&self, friend: &str, share_id: &str) -> Result<(SendStream, RecvStream), String> {
        let conn = self.get_conn(friend).await?;
        let (mut send, mut recv) = conn.open_bi().await.map_err(|e| e.to_string())?;
        write_frame(&mut send, &Request::Tunnel { share_id: share_id.to_string() }).await?;
        match tokio::time::timeout(REQUEST_TIMEOUT, read_frame::<_, Response>(&mut recv))
            .await
            .map_err(|_| "Tiempo agotado esperando a tu amigo".to_string())??
        {
            Response::Error { message } => Err(message),
            Response::Ok { .. } => Ok((send, recv)),
        }
    }

    async fn tunnel_connection(&self, friend: &str, share_id: &str, mut tcp: TcpStream) -> Result<(), String> {
        let (send, recv) = match self.open_tunnel(friend, share_id).await {
            Ok(channel) => channel,
            Err(error) => {
                tracing::warn!("Túnel P2P: no se pudo abrir el canal con el amigo: {error}");
                return Err(error);
            }
        };
        let mut stream = tokio::io::join(recv, send);
        let _ = tokio::io::copy_bidirectional(&mut stream, &mut tcp).await;
        Ok(())
    }

    // ------------------------------------------------------------ API pública

    pub fn info(&self) -> P2pInfo {
        P2pInfo { id: self.endpoint.id().to_string(), name: self.my_name(), bio: self.store.lock().unwrap().bio.clone() }
    }

    /// Aplica los ajustes de privacidad de la cuenta que dependen de este extremo.
    pub fn set_privacy(&self, friend_requests_none: bool, invites_none: bool) {
        self.friend_requests_none.store(friend_requests_none, Ordering::Relaxed);
        self.invites_none.store(invites_none, Ordering::Relaxed);
    }

    /// Firma un mensaje con la clave de este launcher (identidad de la cuenta de Strata).
    /// Devuelve la firma Ed25519 en hexadecimal.
    pub fn sign_message(&self, message: &[u8]) -> String {
        to_hex(&self.endpoint.secret_key().sign(message).to_bytes())
    }

    /// Archivo donde se guarda la clave de este launcher.
    pub fn identity_path(&self) -> PathBuf {
        self.dir.join("identity.key")
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
            let friend = Friend { name: format!("Strata-{}", &id[..6]), id, status: FriendStatus::Outgoing, synced: false, via_account: false };
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
                    None => store.friends.push(Friend { id: parse_id(id)?.to_string(), name: format!("Strata-{}", &id[..6.min(id.len())]), status: FriendStatus::Blocked, synced: true, via_account: false }),
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

    /// Sincroniza los amigos de la cuenta que han enlazado su código de partidas. Los de la lista
    /// pasan a ser amigos aceptados (y se les avisa en segundo plano); los que ya no estén y se
    /// habían añadido por la cuenta se retiran. Los amigos añadidos a mano con un código no se tocan,
    /// y un usuario bloqueado sigue bloqueado.
    pub async fn sync_account_friends(self: &Arc<Self>, peers: Vec<(String, String)>) -> Result<(), String> {
        let mut valid = Vec::new();
        for (code, name) in peers {
            let id = parse_id(&code)?.to_string();
            if id != self.endpoint.id().to_string() {
                valid.push((id, clean_name(&name)));
            }
        }
        let trusted: HashSet<String> = valid.iter().map(|(id, _)| id.clone()).collect();
        let mut removed = Vec::new();
        let mut changed = false;
        {
            let mut store = self.store.lock().unwrap();
            for (id, name) in &valid {
                match store.friends.iter_mut().find(|f| &f.id == id) {
                    Some(friend) if friend.status == FriendStatus::Blocked => {}
                    Some(friend) => {
                        if friend.status != FriendStatus::Accepted {
                            friend.status = FriendStatus::Accepted;
                            friend.synced = false;
                            changed = true;
                        }
                        if !friend.via_account {
                            friend.via_account = true;
                            changed = true;
                        }
                        if !name.is_empty() && &friend.name != name && friend.via_account {
                            friend.name = name.clone();
                            changed = true;
                        }
                    }
                    None => {
                        let name = if name.is_empty() { format!("Strata-{}", &id[..6]) } else { name.clone() };
                        store.friends.push(Friend { id: id.clone(), name, status: FriendStatus::Accepted, synced: false, via_account: true });
                        changed = true;
                    }
                }
            }
            store.friends.retain(|f| {
                // Los bloqueos que vienen de la cuenta se gestionan en `sync_account_blocks`, no aquí.
                let keep = !(f.via_account && f.status != FriendStatus::Blocked && !trusted.contains(&f.id));
                if !keep {
                    removed.push(f.id.clone());
                }
                keep
            });
            changed |= !removed.is_empty();
        }
        *self.trusted.lock().unwrap() = trusted;
        for id in &removed {
            self.chats.lock().unwrap().remove(id);
            self.drop_friend_access(id).await;
        }
        if changed {
            self.save_store()?;
            self.emit(serde_json::json!({"type": "friends"}));
            let core = self.clone();
            tokio::spawn(async move {
                let _ = core.sync_once().await;
            });
        }
        Ok(())
    }

    /// Lleva al P2P los bloqueos de la cuenta. El P2P es directo entre launchers y el servidor no puede impedirlo, así que
    /// sin esto un usuario bloqueado en la cuenta seguiría pudiendo mandar solicitudes P2P (su código es público para
    /// cualquier cuenta). Un bloqueo manual no se toca; los que vienen de la cuenta se quitan cuando la cuenta los quita.
    pub async fn sync_account_blocks(self: &Arc<Self>, blocked: Vec<String>) -> Result<(), String> {
        let me = self.endpoint.id().to_string();
        let wanted: HashSet<String> = blocked.iter().filter_map(|code| parse_id(code).ok()).map(|id| id.to_string()).filter(|id| *id != me).collect();
        let mut newly_blocked = Vec::new();
        let mut changed = false;
        {
            let mut store = self.store.lock().unwrap();
            for id in &wanted {
                match store.friends.iter_mut().find(|f| &f.id == id) {
                    Some(friend) if friend.status == FriendStatus::Blocked => {}
                    Some(friend) => {
                        friend.status = FriendStatus::Blocked;
                        friend.via_account = true;
                        friend.synced = true;
                        newly_blocked.push(id.clone());
                        changed = true;
                    }
                    None => {
                        store.friends.push(Friend { id: id.clone(), name: format!("Strata-{}", &id[..6.min(id.len())]), status: FriendStatus::Blocked, synced: true, via_account: true });
                        newly_blocked.push(id.clone());
                        changed = true;
                    }
                }
            }
            let before = store.friends.len();
            store.friends.retain(|f| !(f.status == FriendStatus::Blocked && f.via_account && !wanted.contains(&f.id)));
            changed |= store.friends.len() != before;
        }
        for id in &newly_blocked {
            self.drop_friend_access(id).await;
        }
        if changed {
            self.save_store()?;
            self.emit(serde_json::json!({"type": "friends"}));
        }
        Ok(())
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

    #[cfg(test)]
    pub fn share_start(&self, kind: &str, title: &str, game_version: &str, port: u16) -> Result<Share, String> {
        self.share_start_with(kind, title, game_version, port, None, None)
    }

    /// Como `share_start`, indicando además con qué instancia se juega (para que el amigo use la misma).
    pub fn share_start_with(
        &self,
        kind: &str,
        title: &str,
        game_version: &str,
        port: u16,
        instance: Option<InstanceRecipe>,
        instance_id: Option<String>,
    ) -> Result<Share, String> {
        if !["world", "server"].contains(&kind) || port == 0 {
            return Err("Recurso no válido".into());
        }
        let instance = instance.map(clean_recipe).transpose()?;
        let instance_id = instance_id.filter(|_| kind == "world").map(|id| id.chars().take(128).collect::<String>());
        let mut shares = self.shares.lock().unwrap();
        // Mismo puerto, o el mismo mundo de la misma instancia con otro puerto (se abrió a LAN de nuevo):
        // se conserva el id para que las invitaciones ya enviadas sigan valiendo.
        let same = shares.values_mut().find(|s| {
            s.port == port || (kind == "world" && s.kind == "world" && instance_id.is_some() && s.instance_id == instance_id)
        });
        if let Some(existing) = same {
            existing.port = port;
            existing.instance = instance.or(existing.instance.take());
            existing.instance_id = instance_id.or(existing.instance_id.take());
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
            instance,
            instance_id,
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
        let info = InviteInfo {
            share_id: share.id,
            kind: share.kind,
            title: share.title,
            game_version: share.game_version,
            instance: share.instance,
        };
        self.send_invite_message(friend, info).await
    }

    /// Envía por el chat una instancia (versión, cargador y modpack) para que el amigo pueda instalarla.
    pub async fn send_instance(self: &Arc<Self>, friend: &str, recipe: InstanceRecipe) -> Result<ChatMessage, String> {
        self.require_friend(friend).map_err(|_| "Solo puedes compartir instancias con tus amigos".to_string())?;
        let recipe = clean_recipe(recipe)?;
        let info = InviteInfo {
            share_id: String::new(),
            kind: "instance".into(),
            title: recipe.name.clone(),
            game_version: recipe.game_version.clone(),
            instance: Some(recipe),
        };
        self.send_invite_message(friend, info).await
    }

    async fn send_invite_message(self: &Arc<Self>, friend: &str, info: InviteInfo) -> Result<ChatMessage, String> {
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
        // Un puerto guardado solo vale si su escucha sigue viva (si murió, se abre de nuevo).
        {
            let mut joins = self.joins.lock().unwrap();
            match joins.get(&key) {
                Some((port, handle)) if !handle.is_finished() => return Ok(*port),
                Some(_) => {
                    joins.remove(&key);
                }
                None => {}
            }
        }
        // Antes de abrir nada se comprueba que el amigo está y que de verdad comparte esto: así los fallos
        // salen aquí, con un texto claro, y no como un error críptico dentro de Minecraft.
        let (send, recv) = self.open_tunnel(friend, share_id).await?;
        drop((send, recv));
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let core = self.clone();
        let (friend_id, share) = (friend.to_string(), share_id.to_string());
        let task = tokio::spawn(async move {
            let mut errors = 0u32;
            loop {
                match listener.accept().await {
                    Ok((tcp, _)) => {
                        errors = 0;
                        let core = core.clone();
                        let (friend_id, share) = (friend_id.clone(), share.clone());
                        tokio::spawn(async move {
                            let _ = core.tunnel_connection(&friend_id, &share, tcp).await;
                        });
                    }
                    // Un fallo aislado (p. ej. una conexión abortada en Windows) no debe cerrar el puerto.
                    Err(error) => {
                        errors += 1;
                        tracing::warn!("Túnel P2P: error al aceptar una conexión local: {error}");
                        if errors > 50 {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }
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
    /// Cuenta un túnel más del amigo, o `None` si ya tiene el máximo (comprobación y reserva bajo el mismo bloqueo).
    fn reserve(core: Arc<Core>, friend: String) -> Option<Self> {
        {
            let mut tunnels = core.tunnels.lock().unwrap();
            let count = tunnels.entry(friend.clone()).or_insert(0);
            if *count >= MAX_TUNNELS_PER_FRIEND {
                return None;
            }
            *count += 1;
        }
        Some(Self { core, friend })
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

pub(crate) async fn core<R: tauri::Runtime>(app: &tauri::AppHandle<R>, state: &P2pState) -> Result<Arc<Core>, String> {
    state
        .core
        .get_or_try_init(|| async {
            let dir = app.path().app_data_dir().map_err(|e| e.to_string())?.join("p2p");
            let handle = app.clone();
            let notify: Notify = Arc::new(move |value| {
                let _ = handle.emit("p2p-event", value);
            });
            let core = Core::start(dir, notify, true).await?;
            // Los mundos compartidos siguen al puerto LAN actual de su instancia.
            core.set_port_resolver(Arc::new(|instance_id: String| {
                Box::pin(async move { detect_lan_port(&instance_id).await.ok().flatten() })
            }));
            Ok::<_, String>(core)
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

/// Los puertos de un mundo LAN son efímeros: nada por debajo (SSH, HTTP, SMB…) puede ser el de Minecraft.
const MIN_LAN_PORT: u16 = 1024;

/// `true` si lo que hay antes del marcador es solo el prefijo con el que el propio juego escribe sus líneas:
/// `[hh:mm:ss] [hilo/NIVEL]: ` (y, a veces, `(registro)`, `[System]` y `[CHAT]`). Un jugador puede escribir en el chat
/// cualquier frase, incluso «Started serving on 3306»; esa línea lleva delante su nombre (`<Steve> `, `* Steve `…) y no
/// pasa este filtro, así que no puede cambiar a qué puerto reenvía el túnel.
fn is_game_log_prefix(before: &str) -> bool {
    let mut rest = before;
    let mut groups = 0;
    loop {
        rest = rest.trim_start_matches(|c: char| c == ':' || c.is_whitespace());
        if rest.is_empty() {
            return groups >= 2;
        }
        if let Some(tail) = rest.strip_prefix('[') {
            let Some((inner, after)) = tail.split_once(']') else { return false };
            let clock = !inner.is_empty() && inner.chars().all(|c| c.is_ascii_digit() || c == ':');
            let thread = inner.contains('/') && inner.chars().all(|c| c.is_ascii_alphanumeric() || " #-_/".contains(c));
            if !(clock || thread || inner == "System" || inner == "CHAT") {
                return false;
            }
            groups += 1;
            rest = after;
        } else if let Some(tail) = rest.strip_prefix('(') {
            let Some((inner, after)) = tail.split_once(')') else { return false };
            if inner.is_empty() || !inner.chars().all(|c| c.is_ascii_alphanumeric() || " _.-".contains(c)) {
                return false;
            }
            rest = after;
        } else {
            // La frase del propio juego que antecede al marcador «The port number is [».
            return groups >= 2 && rest.trim_end() == "This world is now open to LAN.";
        }
    }
}

/// Último puerto anunciado en el texto de un log, o `None` si no hay ninguno.
fn lan_port_from_log(text: &str) -> Option<u16> {
    text.lines().rev().find_map(|line| {
        LAN_PORT_MARKERS.iter().find_map(|marker| {
            let (before, rest) = line.split_once(marker)?;
            if !is_game_log_prefix(before) {
                return None;
            }
            let port: u16 = rest.trim_start().split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()?;
            (port >= MIN_LAN_PORT).then_some(port)
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

/// Guarda una copia de la clave del launcher (que es la identidad de tu cuenta) en el archivo indicado.
#[tauri::command]
pub async fn p2p_export_identity<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, P2pState>, path: String) -> Result<(), String> {
    let core = core(&app, &state).await?;
    let destination = PathBuf::from(path);
    if destination.exists() {
        return Err("Ya existe un archivo con ese nombre".into());
    }
    fs::copy(core.identity_path(), destination).map_err(|e| e.to_string())?;
    Ok(())
}

/// Sustituye la clave del launcher por la de una copia de seguridad. Surte efecto al reiniciar.
#[tauri::command]
pub async fn p2p_import_identity<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, P2pState>, path: String) -> Result<(), String> {
    let core = core(&app, &state).await?;
    let source = PathBuf::from(path);
    if !source.is_file() || source.metadata().map_err(|e| e.to_string())?.len() > 1024 {
        return Err("Ese archivo no es una copia de seguridad válida".into());
    }
    let text = fs::read_to_string(&source).map_err(|e| e.to_string())?;
    let key = from_hex(text.trim()).ok_or("Ese archivo no es una copia de seguridad válida")?;
    let target = core.identity_path();
    let backup = target.with_extension(format!("key.bak-{}", now()));
    fs::copy(&target, backup).map_err(|e| e.to_string())?;
    fs::write(&target, to_hex(&SecretKey::from_bytes(&key).to_bytes())).map_err(|e| e.to_string())?;
    Ok(())
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

#[derive(Deserialize)]
pub struct AccountPeer {
    pub code: String,
    pub name: String,
}

/// Recibe los amigos de la cuenta con su código de partidas (ver `Core::sync_account_friends`).
#[tauri::command]
pub async fn p2p_sync_account_friends<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, P2pState>, peers: Vec<AccountPeer>) -> Result<(), String> {
    if peers.len() > 500 {
        return Err("Demasiados amigos".into());
    }
    core(&app, &state).await?.sync_account_friends(peers.into_iter().map(|p| (p.code, p.name)).collect()).await
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
pub async fn p2p_share_start<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, P2pState>, kind: String, title: String, game_version: String, port: u16, instance: Option<InstanceRecipe>, instance_id: Option<String>) -> Result<Share, String> {
    core(&app, &state).await?.share_start_with(&kind, &title, &game_version, port, instance, instance_id)
}

#[tauri::command]
pub async fn p2p_send_instance<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, P2pState>, friend: String, instance: InstanceRecipe) -> Result<ChatMessage, String> {
    core(&app, &state).await?.send_instance(&friend, instance).await
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
        c.store.lock().unwrap().friends.push(Friend { id: a.info().id, name: "A".into(), status: FriendStatus::Accepted, synced: true, via_account: false });
        assert!(c.join(&a.info().id, &share.id).await.is_err(), "el desconocido ni siquiera consigue abrir el túnel");

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

    async fn echo_server() -> u16 {
        let echo = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = echo.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut tcp, _)) = echo.accept().await {
                tokio::spawn(async move {
                    let (mut r, mut w) = tcp.split();
                    let _ = tokio::io::copy(&mut r, &mut w).await;
                });
            }
        });
        port
    }

    async fn echo_through(port: u16) {
        let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        client.write_all(b"ping").await.unwrap();
        let mut buffer = [0u8; 4];
        tokio::time::timeout(Duration::from_secs(10), client.read_exact(&mut buffer)).await.unwrap().unwrap();
        assert_eq!(&buffer, b"ping");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn joining_something_that_is_no_longer_shared_fails_clearly_without_opening_a_port() {
        let (a, b) = pair().await;
        befriend(&a, &b).await;
        let error = b.join(&a.info().id, "recurso-inexistente").await.unwrap_err();
        assert!(error.contains("ya no se comparte"), "mensaje claro: {error}");
        assert!(b.joins.lock().unwrap().is_empty(), "no se abre ningún puerto local");

        // Compartido pero con el mundo cerrado: el error lo explica en vez de cortar la conexión sin más.
        let closed = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let closed_port = closed.local_addr().unwrap().port();
        drop(closed);
        let share = a.share_start("world", "Mundo", "1.21", closed_port).unwrap();
        let error = b.join(&a.info().id, &share.id).await.unwrap_err();
        assert!(error.contains("no está disponible"), "mensaje claro: {error}");
        a.shutdown().await;
        b.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn shared_worlds_follow_the_current_lan_port_of_their_instance() {
        let (a, b) = pair().await;
        befriend(&a, &b).await;
        let live_port = echo_server().await;
        let closed = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let stale_port = closed.local_addr().unwrap().port();
        drop(closed);

        // El mundo se compartió con un puerto que ya no existe; la instancia tiene ahora otro abierto.
        let share = a.share_start_with("world", "Mundo", "1.21", stale_port, None, Some("instancia-1".into())).unwrap();
        a.set_port_resolver(Arc::new(move |instance: String| {
            Box::pin(async move { (instance == "instancia-1").then_some(live_port) })
        }));
        let port = b.join(&a.info().id, &share.id).await.unwrap();
        echo_through(port).await;

        // Volver a compartir el mismo mundo con otro puerto conserva el id (las invitaciones siguen valiendo).
        let again = a.share_start_with("world", "Mundo", "1.21", live_port, None, Some("instancia-1".into())).unwrap();
        assert_eq!(again.id, share.id);
        assert_eq!(a.shares().len(), 1);
        assert_eq!(a.shares()[0].port, live_port);
        a.shutdown().await;
        b.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_dead_local_listener_is_replaced_instead_of_returning_a_dead_port() {
        let (a, b) = pair().await;
        befriend(&a, &b).await;
        let share = a.share_start("server", "Servidor", "1.21", echo_server().await).unwrap();
        let first = b.join(&a.info().id, &share.id).await.unwrap();
        // Se simula que la escucha local murió.
        let key = format!("{}:{}", a.info().id, share.id);
        b.joins.lock().unwrap().get(&key).unwrap().1.abort();
        tokio::time::sleep(Duration::from_millis(200)).await;
        let second = b.join(&a.info().id, &share.id).await.unwrap();
        echo_through(second).await;
        let _ = first;
        a.shutdown().await;
        b.shutdown().await;
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


    #[tokio::test(flavor = "multi_thread")]
    async fn account_friends_become_p2p_friends_without_manual_codes() {
        let (a, b) = pair().await;
        let (a_id, b_id) = (a.info().id, b.info().id);
        // Ambos tienen la amistad en su cuenta y el servidor les da el código del otro.
        a.sync_account_friends(vec![(b_id.clone(), "Bea".into())]).await.unwrap();
        b.sync_account_friends(vec![(a_id.clone(), "Ana".into())]).await.unwrap();
        a.sync_once().await.unwrap();
        b.sync_once().await.unwrap();
        tokio::time::sleep(Duration::from_millis(800)).await;
        let on_a = a.friends();
        assert_eq!((on_a[0].status, on_a[0].via_account, on_a[0].name.as_str()), (FriendStatus::Accepted, true, "Bea"));
        assert_eq!(b.friends()[0].status, FriendStatus::Accepted);
        a.send_chat(&b_id, "hola por la cuenta").await.unwrap();
        assert!(b.messages(&a_id).iter().any(|m| m.text == "hola por la cuenta"));

        // Si la amistad de la cuenta desaparece, también el acceso P2P (y se cierra la conversación).
        a.sync_account_friends(vec![]).await.unwrap();
        assert!(a.friends().is_empty());
        assert!(a.send_chat(&b_id, "ya no").await.is_err());
        a.shutdown().await;
        b.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn account_sync_keeps_manual_and_blocked_friends_and_only_trusts_listed_codes() {
        let (a, b) = pair().await;
        let c = Core::start(temp("c"), silent(), false).await.unwrap();
        settle().await;
        // B es amigo manual de A (código intercambiado a mano).
        befriend(&a, &b).await;
        a.sync_account_friends(vec![]).await.unwrap();
        assert_eq!(a.friends()[0].status, FriendStatus::Accepted, "los amigos manuales no se retiran");
        assert!(!a.friends()[0].via_account);

        // Un bloqueado no se desbloquea por aparecer en la lista de la cuenta.
        a.set_blocked(&c.info().id, true).await.unwrap();
        a.sync_account_friends(vec![(c.info().id, "Cris".into())]).await.unwrap();
        assert!(a.friends().iter().any(|f| f.id == c.info().id && f.status == FriendStatus::Blocked));

        // Un código que NO está en la lista no se acepta solo: queda como solicitud pendiente.
        let d = Core::start(temp("d"), silent(), false).await.unwrap();
        settle().await;
        d.add_hint(a.addr());
        a.add_hint(d.addr());
        d.add_friend(&a.info().id).await.unwrap();
        assert!(a.friends().iter().any(|f| f.id == d.info().id && f.status == FriendStatus::Incoming));

        assert!(a.sync_account_friends(vec![("no-es-un-codigo".into(), "x".into())]).await.is_err());
        for core in [&a, &b, &c, &d] {
            core.shutdown().await;
        }
    }


    /// SEC-13: el límite de túneles por amigo se reserva al autorizar, no después de resolver el puerto.
    #[tokio::test(flavor = "multi_thread")]
    async fn tunnel_limit_is_reserved_atomically_at_authorization() {
        let (a, b) = pair().await;
        befriend(&a, &b).await;
        let b_id = b.info().id;
        let share = a.share_start("server", "Servidor", "1.21", 25565).unwrap();
        let held: Vec<_> = (0..MAX_TUNNELS_PER_FRIEND).map(|_| a.authorize_tunnel(&b_id, &share.id).expect("dentro del límite")).collect();
        assert!(a.authorize_tunnel(&b_id, &share.id).is_err(), "el siguiente se rechaza aunque ninguno haya llegado a conectar");
        drop(held);
        assert!(a.authorize_tunnel(&b_id, &share.id).is_ok(), "al liberar los túneles vuelve a haber hueco");
        a.shutdown().await;
        b.shutdown().await;
    }

    /// SEC-05: un amigo no puede inflar el disco con ids enormes ni saturar con mensajes.
    #[tokio::test(flavor = "multi_thread")]
    async fn incoming_messages_have_bounded_ids_times_and_rate() {
        let (a, b) = pair().await;
        befriend(&a, &b).await;
        let b_id = b.info().id;
        let chat = |id: String, ts: u64| Request::Chat { id, text: "hola".into(), ts };

        assert!(a.handle_request(&b_id, chat("x".repeat(65_000), now())).is_err(), "id de 65 KB");
        assert!(a.handle_request(&b_id, chat(String::new(), now())).is_err(), "id vacío");
        assert!(a.handle_request(&b_id, chat("../../etc/passwd".into(), now())).is_err(), "caracteres no permitidos");
        assert!(a.messages(&b_id).is_empty(), "no se guardó nada");

        // Un UUID normal pasa, y una hora en el futuro se acota.
        assert!(a.handle_request(&b_id, chat(uuid::Uuid::new_v4().to_string(), u64::MAX)).is_ok());
        assert!(a.messages(&b_id)[0].ts <= now() + 300, "la hora no puede ir en el futuro");

        // Invitaciones: el id del recurso también se acota.
        let invite = |share_id: String| Request::Invite {
            id: uuid::Uuid::new_v4().to_string(),
            info: InviteInfo { share_id, kind: "world".into(), title: "Mundo".into(), game_version: "1.21".into(), instance: None },
            ts: now(),
        };
        assert!(a.handle_request(&b_id, invite("y".repeat(60_000))).is_err());
        assert!(a.handle_request(&b_id, invite(uuid::Uuid::new_v4().to_string())).is_ok());

        // Límite por minuto (ya se han aceptado 2 en esta ventana).
        let mut accepted = 2;
        while a.handle_request(&b_id, chat(uuid::Uuid::new_v4().to_string(), now())).is_ok() {
            accepted += 1;
            assert!(accepted <= 200, "el límite no actúa");
        }
        assert_eq!(accepted, MAX_INCOMING_PER_MINUTE as usize);
        a.shutdown().await;
        b.shutdown().await;
    }

    /// SEC-02: un usuario bloqueado en la cuenta no puede mandar solicitudes P2P directas, y el bloqueo se levanta
    /// cuando la cuenta lo levanta (salvo los bloqueos hechos a mano).
    #[tokio::test(flavor = "multi_thread")]
    async fn account_blocks_apply_to_p2p_requests_and_lift_with_the_account() {
        let (a, b) = pair().await;
        let (a_id, b_id) = (a.info().id, b.info().id);

        a.sync_account_blocks(vec![b_id.clone(), "no-es-un-codigo".into(), a_id.clone()]).await.unwrap();
        assert_eq!(a.friends().len(), 1, "solo el código válido y ajeno");
        assert_eq!(a.friends()[0].status, FriendStatus::Blocked);

        // B manda una solicitud P2P directa: A la rechaza y no la registra como entrante.
        let _ = b.add_friend(&a_id).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(a.friends().len(), 1);
        assert_eq!(a.friends()[0].status, FriendStatus::Blocked, "sigue bloqueado, no pasa a entrante");

        // Un bloqueo de la cuenta no se puede levantar con la lista de amigos de la cuenta.
        a.sync_account_friends(vec![]).await.unwrap();
        assert_eq!(a.friends()[0].status, FriendStatus::Blocked);

        // La cuenta lo desbloquea: el P2P también.
        a.sync_account_blocks(vec![]).await.unwrap();
        assert!(a.friends().is_empty());

        // Un bloqueo manual sobrevive a que la cuenta lo desbloquee.
        a.set_blocked(&b_id, true).await.unwrap();
        a.sync_account_blocks(vec![b_id.clone()]).await.unwrap();
        a.sync_account_blocks(vec![]).await.unwrap();
        assert_eq!(a.friends().len(), 1);
        assert_eq!(a.friends()[0].status, FriendStatus::Blocked);
        a.shutdown().await;
        b.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn privacy_settings_are_enforced_by_the_receiving_launcher() {
        let (a, b) = pair().await;
        let (a_id, b_id) = (a.info().id, b.info().id);
        a.set_privacy(true, true);

        // Un desconocido no puede mandar solicitudes si no se aceptan.
        let err = b.add_friend(&a_id).await;
        assert!(err.is_ok(), "enviar la solicitud no falla en el origen");
        assert!(a.friends().is_empty(), "A no registra la solicitud");

        // Un amigo de la cuenta (de confianza) sí entra y puede chatear, pero no invitar.
        a.sync_account_friends(vec![(b_id.clone(), "Bea".into())]).await.unwrap();
        b.sync_account_friends(vec![(a_id.clone(), "Ana".into())]).await.unwrap();
        a.sync_once().await.unwrap();
        b.sync_once().await.unwrap();
        tokio::time::sleep(Duration::from_millis(800)).await;
        assert!(a.friends().iter().any(|f| f.id == b_id && f.status == FriendStatus::Accepted));
        b.send_chat(&a_id, "hola").await.unwrap();
        assert!(a.messages(&b_id).iter().any(|m| m.text == "hola"));
        let share = b.share_start("world", "Mundo", "1.21", 25565).unwrap();
        let invite = b.invite(&a_id, &share.id).await.unwrap();
        assert!(!invite.delivered, "A rechaza las invitaciones");
        assert!(a.messages(&b_id).iter().all(|m| m.invite.is_none()));

        // Al volver a permitirlas, la invitación pendiente se entrega.
        a.set_privacy(false, false);
        b.sync_once().await.unwrap();
        assert!(a.messages(&b_id).iter().any(|m| m.invite.is_some()));
        a.shutdown().await;
        b.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn signatures_prove_possession_of_the_account_key() {
        let core = Core::start(temp("sign"), silent(), false).await.unwrap();
        let message = b"strata-req
GET
/sync
1700000000000
e3b0c44298fc1c149afbf4c8996fb924";
        let bytes: [u8; 64] = from_hex_n::<64>(&core.sign_message(message)).expect("firma hex de 64 bytes");
        let signature = iroh::Signature::from_bytes(&bytes);
        assert!(core.endpoint.id().verify(message, &signature).is_ok());
        assert!(core.endpoint.id().verify(b"otro mensaje", &signature).is_err(), "no vale para otro mensaje");
        assert_eq!(core.identity_path().file_name().unwrap(), "identity.key");
        core.shutdown().await;
    }

    fn from_hex_n<const N: usize>(text: &str) -> Option<[u8; N]> {
        if text.len() != N * 2 || !text.is_ascii() {
            return None;
        }
        let mut out = [0u8; N];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
        }
        Some(out)
    }

    #[test]
    fn lan_port_is_read_from_minecraft_logs() {
        let log = "[12:00:01] [Server thread/INFO]: Starting\n[12:01:10] [Server thread/INFO]: [System] [CHAT] Local game hosted on port 41234\n[12:05:00] [Server thread/INFO]: [System] [CHAT] Local game hosted on port 50555\n[12:06:00] [Server thread/INFO]: hola";
        assert_eq!(lan_port_from_log(log), Some(50555), "usa el último anuncio");
        // Líneas reales de Minecraft 26.3 (registro del servidor y chat).
        let real = "[18:00:01] [Render thread/INFO]: Published LAN server on port 55535\n[18:00:01] [Render thread/INFO]: [System] [CHAT] This world is now open to LAN. The port number is [55535]\n";
        assert_eq!(lan_port_from_log(real), Some(55535));
        assert_eq!(lan_port_from_log("[18:00:00] [Render thread/INFO]: [System] [CHAT] This world is now open to LAN. The port number is [41000]"), Some(41000));
        assert_eq!(lan_port_from_log("[18:00:00] [Server thread/INFO]: Started serving on 42000"), Some(42000));
        assert_eq!(lan_port_from_log("[18:00:00] [Server thread/INFO] (Minecraft): Started serving on 42001"), Some(42001));
        assert_eq!(lan_port_from_log("sin nada"), None);
        assert_eq!(lan_port_from_log("[18:00:00] [Render thread/INFO]: Local game hosted on port abc"), None);
        assert_eq!(lan_port_from_log("[18:00:00] [Render thread/INFO]: Local game hosted on port 99999"), None, "fuera de rango");
        assert_eq!(lan_port_from_log("[18:00:00] [Render thread/INFO]: Published LAN server on port 80"), None, "los puertos bajos no son de un mundo LAN");
    }

    /// SEC-01: lo que escriba un jugador en el chat no puede cambiar a qué puerto local reenvía el túnel.
    #[test]
    fn chat_lines_cannot_redirect_the_lan_port() {
        let honest = "[12:00:00] [Server thread/INFO]: Started serving on 61234\n";
        for forged in [
            "[12:05:10] [Server thread/INFO]: <Steve> Started serving on 3306",
            "[12:05:10] [Server thread/INFO] (Minecraft) <Steve> Started serving on 3306",
            "[12:05:10] [Render thread/INFO]: [System] [CHAT] <Steve> Local game hosted on port 5432",
            "[12:05:10] [Server thread/INFO]: * Steve Started serving on 2375",
            "[12:05:10] [Server thread/INFO]: [Steve] Started serving on 6379",
            "[12:05:10] [Server thread/INFO]: Steve: This world is now open to LAN. The port number is [8000]",
            "Started serving on 3306",
        ] {
            assert_eq!(lan_port_from_log(&format!("{honest}{forged}\n")), Some(61234), "{forged}");
            assert_eq!(lan_port_from_log(forged), None, "{forged}");
        }
    }

    fn recipe() -> InstanceRecipe {
        InstanceRecipe {
            name: "BattleArmory TACZ".into(),
            game_version: "1.20.1".into(),
            loader: "forge".into(),
            loader_version: Some("47.4.0".into()),
            modpack: Some(ModpackRef {
                project_id: "rQiXwLhB".into(),
                version_id: "TX0XBjIG".into(),
                title: "BattleArmory TACZ".into(),
                icon_url: Some("https://cdn.modrinth.com/data/rQiXwLhB/icon.jpeg".into()),
            }),
        }
    }

    #[test]
    fn old_invites_without_instance_still_parse_and_do_not_leak_the_field() {
        let old = r#"{"share_id":"abc","kind":"world","title":"Mi mundo","game_version":"1.21"}"#;
        let info: InviteInfo = serde_json::from_str(old).unwrap();
        assert!(info.instance.is_none());
        assert!(!serde_json::to_string(&info).unwrap().contains("instance"), "un launcher antiguo recibe lo mismo de siempre");
    }

    #[test]
    fn recipe_is_validated_and_cleaned() {
        assert_eq!(clean_recipe(recipe()).unwrap(), recipe());
        let mut bad = recipe();
        bad.loader = "../../etc".into();
        assert!(clean_recipe(bad).is_err(), "cargador desconocido");
        let mut bad = recipe();
        bad.game_version = "  <script> ".into();
        assert_eq!(clean_recipe(bad).unwrap().game_version, "script");
        let mut bad = recipe();
        bad.game_version = "!!".into();
        assert!(clean_recipe(bad).is_err(), "versión vacía tras limpiar");
        let mut evil = recipe();
        evil.modpack.as_mut().unwrap().icon_url = Some("https://evil.example/x.png".into());
        evil.modpack.as_mut().unwrap().version_id = "a/b?c".into();
        let cleaned = clean_recipe(evil).unwrap();
        assert_eq!(cleaned.modpack.as_ref().unwrap().icon_url, None, "solo imágenes del CDN de Modrinth");
        assert_eq!(cleaned.modpack.unwrap().version_id, "abc");
        let mut empty = recipe();
        empty.modpack.as_mut().unwrap().project_id = "///".into();
        assert!(clean_recipe(empty).unwrap().modpack.is_none(), "modpack sin identificador válido se descarta");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn instances_travel_with_invites_and_can_be_sent_through_chat() {
        let (a, b) = pair().await;
        befriend(&a, &b).await;
        let (a_id, b_id) = (a.info().id, b.info().id);

        // Mundo compartido: la invitación lleva la instancia con la que juega A.
        let share = a.share_start_with("world", "Mi mundo", "1.20.1", 25565, Some(recipe()), None).unwrap();
        a.invite(&b_id, &share.id).await.unwrap();
        let invite = b.messages(&a_id).into_iter().find_map(|m| m.invite).expect("invitación recibida");
        assert_eq!(invite.instance, Some(recipe()));

        // Instancia por el chat: sin share_id y sin túnel.
        a.send_instance(&b_id, recipe()).await.unwrap();
        let sent = b.messages(&a_id).into_iter().filter_map(|m| m.invite).find(|i| i.kind == "instance").expect("instancia recibida");
        assert_eq!((sent.share_id.as_str(), sent.title.as_str(), sent.game_version.as_str()), ("", "BattleArmory TACZ", "1.20.1"));
        assert_eq!(sent.instance.unwrap().modpack.unwrap().version_id, "TX0XBjIG");

        // No se puede enviar algo inválido, ni a quien no es amigo.
        let mut bad = recipe();
        bad.loader = "otro".into();
        assert!(a.send_instance(&b_id, bad).await.is_err());
        assert!(a.send_instance(&"0".repeat(64), recipe()).await.is_err());
        a.shutdown().await;
        b.shutdown().await;
    }

    #[test]
    fn hex_helpers_roundtrip_and_reject_garbage() {
        let bytes = [7u8; 32];
        assert_eq!(from_hex(&to_hex(&bytes)), Some(bytes));
        assert_eq!(from_hex("zz"), None);
        assert_eq!(from_hex(&"é".repeat(32)), None);
    }
}
