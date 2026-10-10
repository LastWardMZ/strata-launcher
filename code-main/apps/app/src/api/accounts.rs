//! Cliente del servicio de cuentas de Strata (usuarios únicos, búsqueda, amigos y privacidad).
//!
//! Las peticiones se hacen desde Rust, no desde la interfaz web: así no dependen de CORS ni de la CSP
//! y el servidor se puede cambiar sin recompilar. La sesión se guarda en el equipo; el chat y las
//! partidas NO pasan por el servidor, solo la lista de amigos y el código de partidas de cada uno.

use crate::api::p2p::{self, P2pState};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{fs, path::PathBuf, time::Duration};
use tauri::Manager;
use tokio::sync::Mutex;

/// Servidor por defecto. Se puede fijar al compilar con la variable `STRATA_ACCOUNTS_URL`.
pub const DEFAULT_SERVER: &str = match option_env!("STRATA_ACCOUNTS_URL") {
    Some(url) => url,
    None => "http://127.0.0.1:32145",
};
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const SYNC_EVERY: Duration = Duration::from_secs(60);

#[derive(Default, Serialize, Deserialize, Clone)]
struct Stored {
    #[serde(default)]
    server_url: String,
    /// Token de sesión (en este equipo, como las credenciales de cualquier launcher).
    #[serde(default)]
    token: String,
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

    async fn set_token<R: tauri::Runtime>(&self, app: &tauri::AppHandle<R>, token: &str) -> Result<(), String> {
        let mut stored = self.load(app).await;
        stored.token = token.to_string();
        self.save(app, stored).await
    }

    async fn request<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        method: Method,
        path: &str,
        body: Option<Value>,
        authenticated: bool,
    ) -> Result<Value, String> {
        let stored = self.load(app).await;
        if authenticated && stored.token.is_empty() {
            return Err("Inicia sesión con tu cuenta de Strata.".into());
        }
        let mut request = self
            .client
            .request(method, format!("{}{path}", stored.server_url))
            .timeout(REQUEST_TIMEOUT);
        if authenticated {
            request = request.bearer_auth(&stored.token);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .map_err(|_| format!("No se pudo conectar con el servidor de cuentas de Strata ({})", stored.server_url))?;
        let status = response.status();
        let value: Value = response.json().await.unwrap_or(Value::Null);
        if status.is_success() {
            return Ok(value);
        }
        if status.as_u16() == 401 && authenticated {
            // La sesión caducó o se cerró en otro sitio.
            let _ = self.set_token(app, "").await;
        }
        Err(value["error"].as_str().unwrap_or("No se pudo completar la acción.").to_string())
    }
}

/// Lleva a este launcher los datos de la cuenta: nombre P2P, enlace del código (firmado),
/// ajustes de privacidad y amigos. Es idempotente y se repite cada minuto.
async fn sync_account<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    accounts: &AccountsState,
    p2p_state: &P2pState,
) -> Result<(), String> {
    let me = accounts.request(app, Method::GET, "/me", None, true).await?;
    let core = p2p::core(app, p2p_state).await?;
    let (id, username) = (me["id"].as_str().unwrap_or_default(), me["username"].as_str().unwrap_or_default());
    if !username.is_empty() && core.info().name != username {
        core.set_name(username)?;
    }
    let (code, ts, sig) = core.sign_link(id);
    if me["p2p_code"].as_str() != Some(code.as_str()) {
        accounts
            .request(app, Method::POST, "/me/p2p", Some(json!({"code": code, "ts": ts, "sig": sig})), true)
            .await?;
    }
    core.set_privacy(
        me["settings"]["friend_privacy"].as_str() == Some("none"),
        me["settings"]["invite_privacy"].as_str() == Some("none"),
    );
    let social = accounts.request(app, Method::GET, "/social", None, true).await?;
    let peers: Vec<(String, String)> = social["friends"]
        .as_array()
        .map(|friends| {
            friends
                .iter()
                .filter_map(|f| Some((f["p2p_code"].as_str()?.to_string(), f["username"].as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default();
    core.sync_account_friends(peers).await
}

/// Cierra la parte P2P de la sesión: retira los amigos que venían de la cuenta.
async fn clear_account_links<R: tauri::Runtime>(app: &tauri::AppHandle<R>, p2p_state: &P2pState) {
    if let Ok(core) = p2p::core(app, p2p_state).await {
        let _ = core.sync_account_friends(Vec::new()).await;
        core.set_privacy(false, false);
    }
}

/// Sincroniza cada minuto mientras haya sesión iniciada.
pub fn start_in_background<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(SYNC_EVERY).await;
            let accounts = app.state::<AccountsState>();
            if accounts.load(&app).await.token.is_empty() {
                continue;
            }
            let p2p_state = app.state::<P2pState>();
            if let Err(error) = sync_account(&app, &accounts, &p2p_state).await {
                tracing::debug!("Sincronización de la cuenta: {error}");
            }
        }
    });
}

#[derive(Serialize)]
pub struct AccountStatus {
    server_url: String,
    logged_in: bool,
    /// Perfil de la cuenta, o `None` si no hay sesión o no se pudo contactar con el servidor.
    profile: Option<Value>,
    /// `true` si hay sesión guardada pero el servidor no respondió.
    offline: bool,
    default_server: bool,
}

#[tauri::command]
pub async fn accounts_status<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, AccountsState>) -> Result<AccountStatus, String> {
    let stored = state.load(&app).await;
    if stored.token.is_empty() {
        return Ok(AccountStatus { default_server: stored.server_url == DEFAULT_SERVER, server_url: stored.server_url, logged_in: false, profile: None, offline: false });
    }
    let result = state.request(&app, Method::GET, "/me", None, true).await;
    let now = state.load(&app).await;
    Ok(match result {
        Ok(profile) => AccountStatus { default_server: now.server_url == DEFAULT_SERVER, server_url: now.server_url, logged_in: true, profile: Some(profile), offline: false },
        // Con un 401 `request` ya borró la sesión; si no, es un fallo de red.
        Err(_) => AccountStatus { default_server: now.server_url == DEFAULT_SERVER, server_url: now.server_url, logged_in: !now.token.is_empty(), profile: None, offline: !now.token.is_empty() },
    })
}

async fn authenticate<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    state: &AccountsState,
    p2p_state: &P2pState,
    path: &str,
    username: String,
    password: String,
) -> Result<Value, String> {
    let result = state
        .request(app, Method::POST, path, Some(json!({"username": username, "password": password})), false)
        .await?;
    let token = result["token"].as_str().ok_or("Respuesta del servidor no válida")?;
    state.set_token(app, token).await?;
    if let Err(error) = sync_account(app, state, p2p_state).await {
        tracing::warn!("No se pudo sincronizar tras iniciar sesión: {error}");
    }
    Ok(result["user"].clone())
}

#[tauri::command]
pub async fn accounts_register<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, AccountsState>, p2p_state: tauri::State<'_, P2pState>, username: String, password: String) -> Result<Value, String> {
    authenticate(&app, &state, &p2p_state, "/register", username, password).await
}

#[tauri::command]
pub async fn accounts_login<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, AccountsState>, p2p_state: tauri::State<'_, P2pState>, username: String, password: String) -> Result<Value, String> {
    authenticate(&app, &state, &p2p_state, "/login", username, password).await
}

#[tauri::command]
pub async fn accounts_logout<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, AccountsState>, p2p_state: tauri::State<'_, P2pState>) -> Result<(), String> {
    // Si el servidor no responde, la sesión se cierra igualmente en este equipo.
    let _ = state.request(&app, Method::POST, "/logout", Some(json!({})), true).await;
    state.set_token(&app, "").await?;
    clear_account_links(&app, &p2p_state).await;
    Ok(())
}

/// Actualiza la biografía y/o los ajustes de privacidad (`{"bio": …, "settings": {…}}`).
#[tauri::command]
pub async fn accounts_update<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, AccountsState>, p2p_state: tauri::State<'_, P2pState>, changes: Value) -> Result<Value, String> {
    let profile = state.request(&app, Method::PATCH, "/me", Some(changes), true).await?;
    let _ = sync_account(&app, &state, &p2p_state).await;
    Ok(profile)
}

#[tauri::command]
pub async fn accounts_search<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, AccountsState>, query: String) -> Result<Value, String> {
    let path = format!("/users/search?q={}", urlencoding::encode(query.trim()));
    state.request(&app, Method::GET, &path, None, true).await
}

#[tauri::command]
pub async fn accounts_social<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, AccountsState>) -> Result<Value, String> {
    state.request(&app, Method::GET, "/social", None, true).await
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
    state.request(&app, Method::POST, path, Some(json!({"username": username})), true).await?;
    let _ = sync_account(&app, &state, &p2p_state).await;
    Ok(())
}

#[tauri::command]
pub async fn accounts_sync<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, AccountsState>, p2p_state: tauri::State<'_, P2pState>) -> Result<(), String> {
    if state.load(&app).await.token.is_empty() {
        return Ok(());
    }
    sync_account(&app, &state, &p2p_state).await
}

#[tauri::command]
pub async fn accounts_change_password<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, AccountsState>, password: String, new_password: String) -> Result<(), String> {
    state
        .request(&app, Method::POST, "/me/password", Some(json!({"password": password, "newPassword": new_password})), true)
        .await
        .map(|_| ())
}

#[tauri::command]
pub async fn accounts_delete<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, AccountsState>, p2p_state: tauri::State<'_, P2pState>, password: String) -> Result<(), String> {
    state.request(&app, Method::POST, "/me/delete", Some(json!({"password": password})), true).await?;
    state.set_token(&app, "").await?;
    clear_account_links(&app, &p2p_state).await;
    Ok(())
}

/// Cambia el servidor de cuentas. Cierra la sesión, porque pertenece al servidor anterior.
#[tauri::command]
pub async fn accounts_set_server<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, AccountsState>, p2p_state: tauri::State<'_, P2pState>, url: String) -> Result<(), String> {
    let normalized = if url.trim().is_empty() { DEFAULT_SERVER.to_string() } else { normalize_server_url(&url)? };
    state.save(&app, Stored { server_url: normalized, token: String::new() }).await?;
    clear_account_links(&app, &p2p_state).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::normalize_server_url;

    #[test]
    fn server_url_must_be_https_or_local() {
        assert_eq!(normalize_server_url("https://cuentas.example.com/").unwrap(), "https://cuentas.example.com");
        assert_eq!(normalize_server_url(" https://cuentas.example.com:8443 ").unwrap(), "https://cuentas.example.com:8443");
        assert_eq!(normalize_server_url("http://127.0.0.1:32145").unwrap(), "http://127.0.0.1:32145");
        assert_eq!(normalize_server_url("http://localhost:3000").unwrap(), "http://localhost:3000");
        assert!(normalize_server_url("http://cuentas.example.com").is_err(), "http solo en local");
        assert!(normalize_server_url("ftp://cuentas.example.com").is_err());
        assert!(normalize_server_url("https://usuario:clave@cuentas.example.com").is_err());
        assert!(normalize_server_url("https://cuentas.example.com/?x=1").is_err());
        assert!(normalize_server_url("no es una url").is_err());
    }
}
