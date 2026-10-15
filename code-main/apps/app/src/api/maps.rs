//! Pestaña «Mapas»: mapas de Minecraft de CurseForge.
//!
//! Los datos (búsqueda, ficha, archivos) se piden al Worker de Strata (`/maps/…`), que guarda la clave de la API de
//! CurseForge. Los archivos NO pasan por el Worker: aquí se descargan directamente del CDN de CurseForge, solo si la URL
//! es de su dominio, y se comprueba la suma de comprobación que da la API antes de tocar nada. La instalación en `saves`
//! la hace `theseus::world_import`, que no sabe de dónde viene el ZIP.

use crate::api::accounts::AccountsState;
use sha1::{Digest, Sha1};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
use theseus::world_import::{self, InstallReport, WorldOrigin, ZipInspection};
use theseus::{LoadingBarType, emit_loading, init_loading};
use tokio::{io::AsyncWriteExt, sync::Mutex};

/// Dominio del que salen los archivos de CurseForge (lo que devuelve su API en la práctica; la documentación oficial no
/// lista los dominios).
const CDN_SUFFIX: &str = "forgecdn.net";
/// Una descarga no puede pasar de esto (el límite del contenido descomprimido lo pone `world_import`).
const MAX_DOWNLOAD: u64 = 2 * 1024 * 1024 * 1024;
const STAGING_DIR: &str = "strata-maps";
/// Los ZIP descargados que nadie instaló ni descartó se borran pasado este tiempo.
const STAGING_MAX_AGE: Duration = Duration::from_secs(24 * 3600);

/// ZIP descargados y comprobados que esperan a que el usuario elija mundos o cancele, por identificador. La interfaz solo
/// maneja identificadores: nunca una ruta.
#[derive(Default)]
pub struct MapsState {
    staged: Mutex<HashMap<String, PathBuf>>,
}

// ───────────────────────── Datos del Worker ─────────────────────────

fn query(pairs: &[(&str, Option<String>)]) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in pairs {
        if let Some(value) = value.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
            serializer.append_pair(key, value);
        }
    }
    serializer.finish()
}

fn with_query(path: &str, query: String) -> String {
    if query.is_empty() { path.to_string() } else { format!("{path}?{query}") }
}

#[tauri::command]
pub async fn maps_search<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    accounts: tauri::State<'_, AccountsState>,
    q: Option<String>,
    version: Option<String>,
    category: Option<u32>,
    sort: Option<String>,
    page: Option<u32>,
) -> Result<serde_json::Value, String> {
    let query = query(&[
        ("q", q),
        ("version", version),
        ("category", category.map(|c| c.to_string())),
        ("sort", sort),
        ("page", page.map(|p| p.to_string())),
    ]);
    accounts.get_public(&app, &with_query("/maps/search", query)).await
}

#[tauri::command]
pub async fn maps_categories<R: tauri::Runtime>(app: tauri::AppHandle<R>, accounts: tauri::State<'_, AccountsState>) -> Result<serde_json::Value, String> {
    accounts.get_public(&app, "/maps/categories").await
}

#[tauri::command]
pub async fn maps_get<R: tauri::Runtime>(app: tauri::AppHandle<R>, accounts: tauri::State<'_, AccountsState>, id: u32) -> Result<serde_json::Value, String> {
    accounts.get_public(&app, &format!("/maps/mod/{id}")).await
}

#[tauri::command]
pub async fn maps_files<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    accounts: tauri::State<'_, AccountsState>,
    id: u32,
    version: Option<String>,
    page: Option<u32>,
) -> Result<serde_json::Value, String> {
    let query = query(&[("version", version), ("page", page.map(|p| p.to_string()))]);
    accounts.get_public(&app, &with_query(&format!("/maps/mod/{id}/files"), query)).await
}

// ───────────────────────── Descarga segura ─────────────────────────

/// La descarga solo puede salir del CDN de CurseForge, por https.
fn check_download_url(value: &str) -> Result<(), String> {
    match url::Url::parse(value) {
        Ok(url) if allowed_host(&url) => Ok(()),
        _ => Err("La descarga no viene del servidor de CurseForge, así que no se ha realizado.".to_string()),
    }
}

fn allowed_host(url: &url::Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.host_str().is_some_and(|host| host == CDN_SUFFIX || host.ends_with(&format!(".{CDN_SUFFIX}")))
}

/// Cliente que solo sigue redirecciones que se quedan en el CDN de CurseForge.
fn download_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(format!("Strata/{}", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() < 5 && allowed_host(attempt.url()) { attempt.follow() } else { attempt.stop() }
        }))
        .build()
        .map_err(|e| e.to_string())
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct FileHash {
    /// `sha1` o `md5` (lo que traduce el Worker de la API de CurseForge).
    algo: String,
    value: String,
}

/// El archivo que el usuario ha elegido, tal y como lo devolvió `maps_files`.
#[derive(serde::Deserialize, Debug, Clone)]
pub struct MapFile {
    url: String,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    hashes: Vec<FileHash>,
}

#[derive(Debug)]
struct Digests {
    sha1: String,
    md5: String,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Compara lo descargado con las sumas de comprobación que da la API. Si no da ninguna no se puede verificar y no se instala.
fn verify_hashes(digests: &Digests, expected: &[FileHash]) -> Result<(), String> {
    let mut checked = 0;
    for hash in expected {
        let actual = match hash.algo.as_str() {
            "sha1" => &digests.sha1,
            "md5" => &digests.md5,
            _ => continue,
        };
        checked += 1;
        if !actual.eq_ignore_ascii_case(hash.value.trim()) {
            return Err("La descarga no coincide con la suma de comprobación que da CurseForge: se ha descartado.".to_string());
        }
    }
    if checked == 0 {
        return Err("CurseForge no ofrece una suma de comprobación para este archivo, así que no se puede verificar y no se ha instalado.".to_string());
    }
    Ok(())
}

/// Descarga a `dest` calculando sha1 y md5 sobre la marcha. Si algo falla borra el archivo a medias.
async fn download(client: &reqwest::Client, url: &str, dest: &Path, max: u64, progress: &mut (dyn FnMut(u64, Option<u64>) + Send)) -> Result<Digests, String> {
    let result = download_inner(client, url, dest, max, progress).await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(dest).await;
    }
    result
}

async fn download_inner(client: &reqwest::Client, url: &str, dest: &Path, max: u64, progress: &mut (dyn FnMut(u64, Option<u64>) + Send)) -> Result<Digests, String> {
    let mut response = client.get(url).send().await.map_err(|_| "No se pudo descargar el mapa. Comprueba tu conexión.".to_string())?;
    if !response.status().is_success() {
        return Err(format!("CurseForge no ha dado el archivo (error {}).", response.status().as_u16()));
    }
    let total = response.content_length();
    if total.is_some_and(|total| total > max) {
        return Err("El archivo es demasiado grande.".to_string());
    }
    let mut file = tokio::fs::File::create(dest).await.map_err(|e| e.to_string())?;
    let mut sha1 = Sha1::new();
    let mut md5 = md5::Context::new();
    let mut done = 0u64;
    while let Some(chunk) = response.chunk().await.map_err(|_| "Se interrumpió la descarga del mapa.".to_string())? {
        done += chunk.len() as u64;
        if done > max {
            return Err("El archivo es demasiado grande.".to_string());
        }
        sha1.update(&chunk);
        md5.consume(&chunk);
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        progress(done, total);
    }
    file.flush().await.map_err(|e| e.to_string())?;
    Ok(Digests { sha1: hex(&sha1.finalize()), md5: format!("{:x}", md5.finalize()) })
}

/// Borra los ZIP de descargas anteriores que quedaron sin instalar ni descartar.
fn purge_old(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let old = entry.metadata().and_then(|m| m.modified()).is_ok_and(|modified| SystemTime::now().duration_since(modified).is_ok_and(|age| age > STAGING_MAX_AGE));
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[derive(serde::Serialize)]
pub struct Prepared {
    token: String,
    inspection: ZipInspection,
}

/// Descarga el archivo elegido (solo del CDN de CurseForge), comprueba su suma de comprobación y lo inspecciona. No instala
/// nada: devuelve qué mundos trae para que el usuario elija, y un identificador para `maps_install`.
#[tauri::command]
pub async fn maps_prepare(state: tauri::State<'_, MapsState>, instance_id: String, file: MapFile) -> Result<Prepared, String> {
    check_download_url(&file.url)?;
    // El destino tiene que ser una instancia real.
    theseus::instance::get_full_path(&instance_id).await.map_err(|e| e.to_string())?;
    let instance_name = theseus::instance::get(&instance_id).await.ok().flatten().map(|m| m.instance.name).unwrap_or_else(|| instance_id.clone());

    let dir = std::env::temp_dir().join(STAGING_DIR);
    tokio::fs::create_dir_all(&dir).await.map_err(|e| e.to_string())?;
    purge_old(&dir);
    let token = uuid::Uuid::new_v4().to_string();
    let path = dir.join(format!("{token}.zip"));

    let bar = init_loading(LoadingBarType::ZipExtract { instance_id: instance_id.clone(), instance_name }, 1.0, "Descargando mapa…").await.map_err(|e| e.to_string())?;
    let client = download_client()?;
    let mut emitted = 0.0f64;
    let known = file.size.filter(|size| *size > 0);
    let digests = download(&client, &file.url, &path, MAX_DOWNLOAD, &mut |done, total| {
        if let Some(total) = total.or(known) {
            let fraction = (done as f64 / total as f64).min(1.0);
            if fraction - emitted >= 0.01 {
                let _ = emit_loading(&bar, fraction - emitted, None);
                emitted = fraction;
            }
        }
    })
    .await;
    // La barra se completa siempre, también si falló.
    let _ = emit_loading(&bar, (1.0 - emitted).max(0.0), None);
    drop(bar);
    let digests = digests?;

    if let Err(message) = verify_hashes(&digests, &file.hashes) {
        let _ = tokio::fs::remove_file(&path).await;
        return Err(message);
    }
    let inspection = match world_import::inspect_zip(path.clone()).await {
        Ok(inspection) => inspection,
        Err(error) => {
            let _ = tokio::fs::remove_file(&path).await;
            return Err(error.to_string());
        }
    };
    state.staged.lock().await.insert(token.clone(), path);
    Ok(Prepared { token, inspection })
}

/// Instala en `saves` de la instancia los mundos elegidos del ZIP ya descargado y recuerda su origen (proyecto de CurseForge).
#[tauri::command]
pub async fn maps_install(state: tauri::State<'_, MapsState>, instance_id: String, token: String, selected: Vec<String>, map_id: u32) -> Result<InstallReport, String> {
    let path = state.staged.lock().await.get(&token).cloned().ok_or("La descarga ya no está disponible. Vuelve a empezar.".to_string())?;
    let origin = WorldOrigin { project_id: format!("curseforge:{map_id}"), content_kind: "map".to_string() };
    let report = world_import::install_into_instance(&instance_id, path.clone(), selected, Some(origin)).await.map_err(|e| e.to_string())?;
    state.staged.lock().await.remove(&token);
    let _ = tokio::fs::remove_file(path).await;
    Ok(report)
}

/// El usuario cancela: se borra el ZIP descargado.
#[tauri::command]
pub async fn maps_discard(state: tauri::State<'_, MapsState>, token: String) -> Result<(), String> {
    if let Some(path) = state.staged.lock().await.remove(&token) {
        let _ = tokio::fs::remove_file(path).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn only_https_urls_of_the_curseforge_cdn_are_downloaded() {
        for good in ["https://edge.forgecdn.net/files/1/2/mapa.zip", "https://mediafilez.forgecdn.net/files/9/9/x.zip", "https://forgecdn.net/a.zip"] {
            assert!(check_download_url(good).is_ok(), "{good}");
        }
        for bad in [
            "http://edge.forgecdn.net/a.zip",
            "https://evil.example.com/a.zip",
            "https://forgecdn.net.evil.com/a.zip",
            "https://evilforgecdn.net/a.zip",
            "https://edge.forgecdn.net@evil.com/a.zip",
            "https://user:pass@edge.forgecdn.net/a.zip",
            "ftp://edge.forgecdn.net/a.zip",
            "file:///C:/Windows/win.ini",
            "https://127.0.0.1/a.zip",
            "no es una url",
            "",
        ] {
            assert!(check_download_url(bad).is_err(), "{bad}");
        }
    }

    fn digests_of(data: &[u8]) -> Digests {
        let mut md5 = md5::Context::new();
        md5.consume(data);
        Digests { sha1: hex(&Sha1::digest(data)), md5: format!("{:x}", md5.finalize()) }
    }

    fn hash(algo: &str, value: &str) -> FileHash {
        FileHash { algo: algo.to_string(), value: value.to_string() }
    }

    #[test]
    fn hashes_must_match_and_at_least_one_must_exist() {
        let digests = digests_of(b"hola mundo");
        assert!(verify_hashes(&digests, &[hash("sha1", &digests.sha1)]).is_ok());
        assert!(verify_hashes(&digests, &[hash("md5", &digests.md5.to_uppercase())]).is_ok(), "las mayúsculas no importan");
        assert!(verify_hashes(&digests, &[hash("sha1", &digests.sha1), hash("md5", &digests.md5)]).is_ok());
        // Un hash incorrecto aborta aunque el otro coincida.
        assert!(verify_hashes(&digests, &[hash("sha1", &digests.sha1), hash("md5", "0000")]).is_err());
        assert!(verify_hashes(&digests, &[hash("sha1", &"0".repeat(40))]).is_err());
        // Sin ninguna suma de comprobación (o con algoritmos desconocidos) no se puede verificar.
        assert!(verify_hashes(&digests, &[]).is_err());
        assert!(verify_hashes(&digests, &[hash("crc32", "abcd")]).is_err());
    }

    /// Servidor HTTP mínimo que responde siempre con `body`.
    async fn serve(body: Vec<u8>, status: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let body = body.clone();
                tokio::spawn(async move {
                    let mut buffer = [0u8; 2048];
                    let _ = socket.read(&mut buffer).await;
                    let head = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(&body).await;
                });
            }
        });
        format!("http://{address}/mapa.zip")
    }

    #[tokio::test]
    async fn download_computes_the_hashes_and_reports_progress() {
        let data: Vec<u8> = (0..200_000u32).map(|n| (n % 251) as u8).collect();
        let url = serve(data.clone(), "200 OK").await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("mapa.zip");
        let mut last = (0, None);
        let digests = download(&download_client().unwrap(), &url, &dest, 10_000_000, &mut |done, total| last = (done, total)).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert_eq!(digests.sha1, digests_of(&data).sha1);
        assert_eq!(digests.md5, digests_of(&data).md5);
        assert_eq!(last, (200_000, Some(200_000)));
        assert!(verify_hashes(&digests, &[hash("sha1", &digests_of(&data).sha1)]).is_ok());
        // Un archivo distinto del que anunció la API no pasa la comprobación.
        let tampered = digests_of(b"otro contenido");
        assert!(verify_hashes(&digests, &[hash("sha1", &tampered.sha1)]).is_err());
    }

    #[tokio::test]
    async fn download_refuses_too_big_files_and_http_errors_and_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let client = download_client().unwrap();

        let url = serve(vec![1u8; 5000], "200 OK").await;
        let dest = dir.path().join("grande.zip");
        assert!(download(&client, &url, &dest, 1000, &mut |_, _| {}).await.is_err(), "pasa del límite");
        assert!(!dest.exists(), "no queda el archivo a medias");

        let url = serve(b"no existe".to_vec(), "404 Not Found").await;
        let dest = dir.path().join("404.zip");
        let error = download(&client, &url, &dest, 1000, &mut |_, _| {}).await.unwrap_err();
        assert!(error.contains("404"), "{error}");
        assert!(!dest.exists());
    }

    #[test]
    fn queries_skip_empty_values_and_escape_the_rest() {
        assert_eq!(query(&[("q", Some("  ".into())), ("version", None)]), "");
        assert_eq!(query(&[("q", Some("castillo medieval&x=1".into())), ("page", Some("2".into()))]), "q=castillo+medieval%26x%3D1&page=2");
        assert_eq!(with_query("/maps/search", String::new()), "/maps/search");
        assert_eq!(with_query("/maps/search", "q=a".into()), "/maps/search?q=a");
    }
}
