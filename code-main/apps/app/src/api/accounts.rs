//! Cliente del servicio de cuentas de Strata (nombres únicos, búsqueda, amigos y privacidad).
//!
//! No hay contraseñas ni sesiones: la cuenta ES la clave pública del launcher (la misma identidad
//! del P2P) y cada petición va firmada con ella. Las peticiones salen de Rust, no de la interfaz
//! web, así que no dependen de CORS ni de la CSP y el servidor se puede cambiar sin recompilar. El
//! chat y las partidas NO pasan por el servidor, solo los nombres, las amistades y los ajustes.

use crate::api::p2p::{self, P2pState};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::Manager;
use tokio::sync::Mutex;

/// Servidor por defecto. Se puede fijar al compilar con la variable `STRATA_ACCOUNTS_URL`.
pub const DEFAULT_SERVER: &str = match option_env!("STRATA_ACCOUNTS_URL") {
    Some(url) => url,
    None => "http://127.0.0.1:8787",
};
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// El plan gratuito de Cloudflare admite 100.000 peticiones al día: se sincroniza poco.
const SYNC_EVERY: Duration = Duration::from_secs(180);

#[derive(Default, Serialize, Deserialize, Clone)]
struct Stored {
    #[serde(default)]
    server_url: String,
}

pub struct AccountsState {
    client: reqwest::Client,
    stored: Mutex<Option<Stored>>,
}

impl Default for AccountsState {
    fn default() -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent(format!("Strata/{}", env!("CARGO_PKG_VERSION")))
                .build()
                .expect("cliente HTTP"),
            stored: Mutex::new(None),
        }
    }
}

/// Error de una petición: el mensaje (en español, apto para mostrar) y el código que manda el servidor.
struct Failure {
    message: String,
    code: Option<String>,
}

impl Failure {
    fn plain(message: impl Into<String>) -> Self {
        Self { message: message.into(), code: None }
    }
}
impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self::plain(message)
    }
}

/// Valida la dirección del servidor: HTTPS, o HTTP solo hacia este mismo equipo.
fn normalize_server_url(input: &str) -> Result<String, String> {
    let url = url::Url::parse(input.trim()).map_err(|_| "La dirección del servidor no es válida".to_string())?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !(url.scheme() == "https" || (url.scheme() == "http" && local)) {
        return Err("El servidor debe usar https:// (http:// solo se admite para este mismo equipo)".into());
    }
    if url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
        return Err("La dirección del servidor no es válida".into());
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

/// Mensaje que se firma en cada petición; debe coincidir con `requestMessage` del servicio.
fn request_message(method: &str, path_and_query: &str, ts: u64, body: &str) -> String {
    format!("strata-req\n{method}\n{path_and_query}\n{ts}\n{}", hex(&Sha256::digest(body.as_bytes())))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn store_path<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("accounts.json"))
}

impl AccountsState {
    async fn load<R: tauri::Runtime>(&self, app: &tauri::AppHandle<R>) -> Stored {
        let mut guard = self.stored.lock().await;
        if guard.is_none() {
            let mut stored: Stored = store_path(app)
                .ok()
                .and_then(|p| fs::read(p).ok())
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default();
            if stored.server_url.is_empty() {
                stored.server_url = DEFAULT_SERVER.to_string();
            }
            *guard = Some(stored);
        }
        guard.clone().unwrap_or_default()
    }

    async fn save<R: tauri::Runtime>(&self, app: &tauri::AppHandle<R>, stored: Stored) -> Result<(), String> {
        let path = store_path(app)?;
        let temp = path.with_extension("json.tmp");
        fs::write(&temp, serde_json::to_vec_pretty(&stored).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        fs::rename(temp, path).map_err(|e| e.to_string())?;
        *self.stored.lock().await = Some(stored);
        Ok(())
    }

    async fn request<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        p2p_state: &P2pState,
        method: Method,
        path_and_query: &str,
        body: Option<Value>,
    ) -> Result<Value, Failure> {
        let stored = self.load(app).await;
        let core = p2p::core(app, p2p_state).await?;
        let text = body.map(|b| b.to_string()).unwrap_or_default();
        let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        let signature = core.sign_message(request_message(method.as_str(), path_and_query, ts, &text).as_bytes());
        let mut request = self
            .client
            .request(method, format!("{}{path_and_query}", stored.server_url))
            .header("Authorization", format!("Strata {}.{ts}.{signature}", core.info().id))
            .timeout(REQUEST_TIMEOUT);
        if !text.is_empty() {
            request = request.header("Content-Type", "application/json").body(text);
        }
        let response = request.send().await.map_err(|_| {
            Failure::plain(format!("No se pudo conectar con el servidor de cuentas de Strata ({})", stored.server_url))
        })?;
        let status = response.status();
        let value: Value = response.json().await.unwrap_or(Value::Null);
        if status.is_success() {
            return Ok(value);
        }
        Err(Failure {
            message: value["error"].as_str().unwrap_or("No se pudo completar la acción.").to_string(),
            code: value["code"].as_str().map(str::to_owned),
        })
    }
}

/// Con «solo amigos de amigos» (`mutual`) o «nadie» el P2P no acepta solicitudes de desconocidos: el servidor no puede
/// comprobar «amigos de amigos» en una conexión directa, así que solo valen las de amigos de la cuenta (que llegan por
/// el servidor, con sus reglas aplicadas).
fn closes_requests(friend_privacy: Option<&str>) -> bool {
    matches!(friend_privacy, Some("none" | "mutual"))
}

/// Lleva a este launcher los datos de la cuenta (nombre P2P, privacidad y amigos) y devuelve el
/// perfil y las listas sociales. Es idempotente y se repite cada pocos minutos.
async fn sync_account<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    accounts: &AccountsState,
    p2p_state: &P2pState,
) -> Result<Value, Failure> {
    let data = accounts.request(app, p2p_state, Method::GET, "/sync", None).await?;
    let core = p2p::core(app, p2p_state).await?;
    let username = data["profile"]["username"].as_str().unwrap_or_default();
    if !username.is_empty() && core.info().name != username {
        core.set_name(username)?;
    }
    let settings = &data["profile"]["settings"];
    core.set_privacy(closes_requests(settings["friend_privacy"].as_str()), settings["invite_privacy"].as_str() == Some("none"));
    // Los bloqueos de la cuenta también valen en el P2P (si no, un bloqueado seguiría mandando solicitudes directas).
    let blocked: Vec<String> = data["social"]["blocked"]
        .as_array()
        .map(|users| users.iter().filter_map(|u| u["id"].as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    core.sync_account_blocks(blocked).await?;
    // Cada amigo trae su clave pública, que es su código de partidas.
    let peers: Vec<(String, String)> = data["social"]["friends"]
        .as_array()
        .map(|friends| {
            friends
                .iter()
                .filter_map(|f| Some((f["id"].as_str()?.to_string(), f["username"].as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default();
    core.sync_account_friends(peers).await?;
    Ok(data)
}

/// Retira de este launcher lo que venía de la cuenta (amigos de la cuenta y ajustes de privacidad).
async fn clear_account_links<R: tauri::Runtime>(app: &tauri::AppHandle<R>, p2p_state: &P2pState) {
    if let Ok(core) = p2p::core(app, p2p_state).await {
        let _ = core.sync_account_friends(Vec::new()).await;
        let _ = core.sync_account_blocks(Vec::new()).await;
        core.set_privacy(false, false);
    }
}

/// Sincroniza cada pocos minutos mientras este launcher tenga cuenta.
pub fn start_in_background<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(SYNC_EVERY).await;
            let accounts = app.state::<AccountsState>();
            let p2p_state = app.state::<P2pState>();
            if let Err(error) = sync_account(&app, &accounts, &p2p_state).await
                && error.code.as_deref() != Some("no-account")
            {
                tracing::debug!("Sincronización de la cuenta: {}", error.message);
            }
        }
    });
}

#[derive(Serialize)]
pub struct AccountStatus {
    server_url: String,
    /// Este launcher tiene una cuenta registrada en el servidor.
    registered: bool,
    profile: Option<Value>,
    social: Option<Value>,
    /// No se pudo hablar con el servidor, así que no se sabe si hay cuenta.
    offline: bool,
    /// Mensaje del servidor si el fallo no es de red (p. ej. la hora del equipo está desajustada).
    problem: Option<String>,
    default_server: bool,
}

#[tauri::command]
pub async fn accounts_status<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, AccountsState>, p2p_state: tauri::State<'_, P2pState>) -> Result<AccountStatus, String> {
    let server_url = state.load(&app).await.server_url;
    let default_server = server_url == DEFAULT_SERVER;
    let base = |registered, offline, problem| AccountStatus { server_url: server_url.clone(), registered, profile: None, social: None, offline, problem, default_server };
    Ok(match sync_account(&app, &state, &p2p_state).await {
        Ok(data) => AccountStatus { profile: Some(data["profile"].clone()), social: Some(data["social"].clone()), ..base(true, false, None) },
        Err(error) if error.code.as_deref() == Some("no-account") => {
            clear_account_links(&app, &p2p_state).await;
            base(false, false, None)
        }
        Err(error) if error.code.is_some() => base(false, false, Some(error.message)),
        Err(_) => base(false, true, None),
    })
}

/// Crea la cuenta de este launcher con el nombre de usuario elegido.
#[tauri::command]
pub async fn accounts_register<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, AccountsState>, p2p_state: tauri::State<'_, P2pState>, username: String) -> Result<Value, String> {
    let profile = state.request(&app, &p2p_state, Method::POST, "/register", Some(json!({"username": username}))).await.map_err(|e| e.message)?;
    let _ = sync_account(&app, &state, &p2p_state).await;
    Ok(profile)
}

/// Actualiza la biografía y/o los ajustes de privacidad (`{"bio": …, "settings": {…}}`).
#[tauri::command]
pub async fn accounts_update<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, AccountsState>, p2p_state: tauri::State<'_, P2pState>, changes: Value) -> Result<Value, String> {
    let profile = state.request(&app, &p2p_state, Method::PATCH, "/me", Some(changes)).await.map_err(|e| e.message)?;
    let _ = sync_account(&app, &state, &p2p_state).await;
    Ok(profile)
}

/// Sube la foto de perfil: WebP de 128×128 en base64 (el servicio la valida y limita a 24 KB).
#[tauri::command]
pub async fn accounts_set_avatar<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, AccountsState>, p2p_state: tauri::State<'_, P2pState>, image: String) -> Result<Value, String> {
    if image.len() > 40_000 {
        return Err("La foto pesa demasiado".into());
    }
    let profile = state.request(&app, &p2p_state, Method::PUT, "/me/avatar", Some(json!({"image": image}))).await.map_err(|e| e.message)?;
    let _ = sync_account(&app, &state, &p2p_state).await;
    Ok(profile)
}

#[tauri::command]
pub async fn accounts_remove_avatar<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, AccountsState>, p2p_state: tauri::State<'_, P2pState>) -> Result<Value, String> {
    let profile = state.request(&app, &p2p_state, Method::POST, "/me/avatar/delete", Some(json!({}))).await.map_err(|e| e.message)?;
    let _ = sync_account(&app, &state, &p2p_state).await;
    Ok(profile)
}

/// Foto de una cuenta (`{id, v, image}`); solo la entrega el servicio si es la tuya o de un amigo o solicitud.
#[tauri::command]
pub async fn accounts_avatar<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, AccountsState>, p2p_state: tauri::State<'_, P2pState>, id: String) -> Result<Value, String> {
    // Solo un código válido: así la ruta firmada es idéntica a la que llega al servicio.
    if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err("Código no válido".into());
    }
    state.request(&app, &p2p_state, Method::GET, &format!("/avatar?user={id}"), None).await.map_err(|e| e.message)
}

#[tauri::command]
pub async fn accounts_rename<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, AccountsState>, p2p_state: tauri::State<'_, P2pState>, username: String) -> Result<Value, String> {
    let profile = state.request(&app, &p2p_state, Method::POST, "/me/rename", Some(json!({"username": username}))).await.map_err(|e| e.message)?;
    let _ = sync_account(&app, &state, &p2p_state).await;
    Ok(profile)
}

#[tauri::command]
pub async fn accounts_search<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, AccountsState>, p2p_state: tauri::State<'_, P2pState>, query: String) -> Result<Value, String> {
    // El servicio solo busca por letras, números y guiones bajos; se filtra aquí para que la ruta
    // firmada sea idéntica a la que llega al servidor.
    let query: String = query.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').take(24).collect();
    if query.len() < 2 {
        return Ok(json!({"users": []}));
    }
    state.request(&app, &p2p_state, Method::GET, &format!("/users/search?q={query}"), None).await.map_err(|e| e.message)
}

/// Solicitudes, aceptaciones, bajas y bloqueos: `action` es `request`, `accept`, `remove`, `block` o `unblock`.
#[tauri::command]
pub async fn accounts_friend_action<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, AccountsState>, p2p_state: tauri::State<'_, P2pState>, action: String, username: String) -> Result<(), String> {
    let path = match action.as_str() {
        "request" => "/friends/request",
        "accept" => "/friends/accept",
        "remove" => "/friends/remove",
        "block" => "/block",
        "unblock" => "/unblock",
        _ => return Err("Acción no válida".into()),
    };
    state.request(&app, &p2p_state, Method::POST, path, Some(json!({"username": username}))).await.map_err(|e| e.message)?;
    let _ = sync_account(&app, &state, &p2p_state).await;
    Ok(())
}

/// Elimina la cuenta del servidor (nombre, amistades y bloqueos). La clave del launcher no cambia.
#[tauri::command]
pub async fn accounts_delete<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, AccountsState>, p2p_state: tauri::State<'_, P2pState>) -> Result<(), String> {
    state.request(&app, &p2p_state, Method::POST, "/me/delete", Some(json!({}))).await.map_err(|e| e.message)?;
    clear_account_links(&app, &p2p_state).await;
    Ok(())
}

/// Cambia el servidor de cuentas. La cuenta pertenece al servidor anterior, así que se retiran sus amigos.
#[tauri::command]
pub async fn accounts_set_server<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, AccountsState>, p2p_state: tauri::State<'_, P2pState>, url: String) -> Result<(), String> {
    let normalized = if url.trim().is_empty() { DEFAULT_SERVER.to_string() } else { normalize_server_url(&url)? };
    state.save(&app, Stored { server_url: normalized }).await?;
    clear_account_links(&app, &p2p_state).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{closes_requests, normalize_server_url, request_message};

    #[test]
    fn mutual_and_none_close_p2p_requests_from_strangers() {
        assert!(closes_requests(Some("none")) && closes_requests(Some("mutual")));
        assert!(!closes_requests(Some("everyone")) && !closes_requests(None), "sin ajuste no se cierra nada");
    }

    #[test]
    fn server_url_must_be_https_or_local() {
        assert_eq!(normalize_server_url("https://cuentas.example.com/").unwrap(), "https://cuentas.example.com");
        assert_eq!(normalize_server_url(" https://cuentas.example.com:8443 ").unwrap(), "https://cuentas.example.com:8443");
        assert_eq!(normalize_server_url("http://127.0.0.1:8787").unwrap(), "http://127.0.0.1:8787");
        assert_eq!(normalize_server_url("http://localhost:3000").unwrap(), "http://localhost:3000");
        assert!(normalize_server_url("http://cuentas.example.com").is_err(), "http solo en local");
        assert!(normalize_server_url("ftp://cuentas.example.com").is_err());
        assert!(normalize_server_url("https://usuario:clave@cuentas.example.com").is_err());
        assert!(normalize_server_url("https://cuentas.example.com/?x=1").is_err());
        assert!(normalize_server_url("no es una url").is_err());
    }

    #[test]
    fn signed_message_matches_the_service_format() {
        // El hash de un cuerpo vacío es el SHA-256 de "" (el mismo que calcula el servicio para los GET).
        assert_eq!(
            request_message("GET", "/sync", 1700000000000, ""),
            "strata-req\nGET\n/sync\n1700000000000\ne3b0c44298fc1c149afbf4c8996fb924\
27ae41e4649b934ca495991b7852b855"
        );
    }
}
