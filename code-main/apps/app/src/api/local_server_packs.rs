use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, io::Read, path::{Component, PathBuf}};
use tauri::Manager;

#[derive(Deserialize)]
struct PackIndex {
	name: String,
	game: String,
	#[serde(rename = "formatVersion")]
	format_version: u32,
	dependencies: HashMap<String, String>,
	files: Vec<PackFile>,
}

#[derive(Deserialize)]
struct PackFile {
	path: String,
	hashes: HashMap<String, String>,
	downloads: Vec<String>,
	#[serde(default)]
	env: HashMap<String, String>,
}

#[derive(Serialize)]
pub struct PackPreview {
	name: String,
	minecraft: String,
	loader: String,
	loader_version: String,
	server_files: usize,
	client_files_skipped: usize,
}

fn relative_path(value: &str) -> Result<PathBuf, String> {
	let path = PathBuf::from(value);
	if value.is_empty() || value.contains(':') || path.components().any(|c| !matches!(c, Component::Normal(_))) {
		return Err(format!("Ruta no válida en el modpack: {value}"));
	}
	Ok(path)
}

fn download_url(value: &str) -> Result<url::Url, String> {
	let url = url::Url::parse(value).map_err(|e| e.to_string())?;
	let host = url.host_str().unwrap_or("");
	if url.scheme() != "https" || !["cdn.modrinth.com", "edge.forgecdn.net", "media.forgecdn.net", "github.com", "raw.githubusercontent.com", "objects.githubusercontent.com"].contains(&host) {
		return Err(format!("Dominio de descarga no permitido: {host}"));
	}
	Ok(url)
}

async fn fetch_file(client: &reqwest::Client, url: &str, limit: usize) -> Result<Vec<u8>, String> {
	let mut response = client.get(download_url(url)?).send().await.map_err(|e| e.to_string())?.error_for_status().map_err(|e| e.to_string())?;
	if response.content_length().is_some_and(|n| n > limit as u64) { return Err("Archivo del modpack demasiado grande".into()); }
	let mut bytes = Vec::new();
	while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
		if bytes.len().saturating_add(chunk.len()) > limit { return Err("Archivo del modpack demasiado grande".into()); }
		bytes.extend_from_slice(&chunk);
	}
	Ok(bytes)
}

fn verify(bytes: &[u8], hashes: &HashMap<String, String>) -> Result<(), String> {
	use sha2::Digest;
	if let Some(expected) = hashes.get("sha512") {
		if format!("{:x}", sha2::Sha512::digest(bytes)) != expected.to_ascii_lowercase() { return Err("Hash SHA-512 incorrecto en el modpack".into()); }
	} else if let Some(expected) = hashes.get("sha1") {
		if format!("{:x}", sha1::Sha1::digest(bytes)) != expected.to_ascii_lowercase() { return Err("Hash SHA-1 incorrecto en el modpack".into()); }
	} else { return Err("El modpack contiene un archivo sin hash verificable".into()); }
	Ok(())
}

async fn archive_bytes(path: Option<PathBuf>, version_id: Option<String>) -> Result<Vec<u8>, String> {
	if let Some(path) = path {
		if path.metadata().map_err(|e| e.to_string())?.len() > 200_000_000 { return Err("Modpack demasiado grande".into()); }
		return fs::read(path).map_err(|e| e.to_string());
	}
	let id = version_id.ok_or("Elige una versión de modpack")?;
	if !id.chars().all(|c| c.is_ascii_alphanumeric()) { return Err("Versión de modpack no válida".into()); }
	let client = reqwest::Client::builder().user_agent("StrataLauncher/0.1").redirect(reqwest::redirect::Policy::none()).build().map_err(|e| e.to_string())?;
	let version: serde_json::Value = client.get(format!("https://api.modrinth.com/v2/version/{id}")).send().await.map_err(|e| e.to_string())?.error_for_status().map_err(|e| e.to_string())?.json().await.map_err(|e| e.to_string())?;
	let file = version["files"].as_array().ok_or("Versión sin archivos")?.iter().find(|f| f["filename"].as_str().is_some_and(|s| s.ends_with(".mrpack"))).ok_or("Esta versión no contiene un archivo .mrpack")?;
	let bytes = fetch_file(&client, file["url"].as_str().ok_or("Descarga no disponible")?, 200_000_000).await?;
	let hashes = serde_json::from_value(file["hashes"].clone()).map_err(|e| e.to_string())?;
	verify(&bytes, &hashes)?;
	Ok(bytes)
}

fn read_index(bytes: &[u8]) -> Result<PackIndex, String> {
	let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
	let mut entry = archive.by_name("modrinth.index.json").map_err(|_| "El archivo no es un modpack de Modrinth válido")?;
	if entry.size() > 10_000_000 { return Err("Índice de modpack demasiado grande".into()); }
	let mut index = Vec::new();
	entry.read_to_end(&mut index).map_err(|e| e.to_string())?;
	let index: PackIndex = serde_json::from_slice(&index).map_err(|e| e.to_string())?;
	if index.game != "minecraft" || index.format_version != 1 || index.files.len() > 10000 { return Err("Formato de modpack no compatible".into()); }
	Ok(index)
}

fn preview(index: &PackIndex) -> Result<PackPreview, String> {
	let minecraft = index.dependencies.get("minecraft").ok_or("El modpack no indica su versión de Minecraft")?.clone();
	let (loader, loader_version) = ["fabric-loader", "forge", "neoforge", "quilt-loader"].iter().find_map(|key| index.dependencies.get(*key).map(|v| (*key, v.clone()))).unwrap_or(("vanilla", String::new()));
	if loader == "quilt-loader" { return Err("Los servidores Quilt todavía no son compatibles".into()); }
	let skipped = index.files.iter().filter(|file| file.env.get("server").is_some_and(|v| v == "unsupported")).count();
	if skipped == index.files.len() && skipped > 0 { return Err("Este modpack solo incluye archivos de cliente".into()); }
	Ok(PackPreview { name: index.name.clone(), minecraft, loader: if loader == "fabric-loader" { "Fabric" } else if loader == "forge" { "Forge" } else if loader == "neoforge" { "NeoForge" } else { "vanilla" }.into(), loader_version, server_files: index.files.len() - skipped, client_files_skipped: skipped })
}

#[tauri::command]
pub async fn local_servers_pack_preview(path: Option<PathBuf>, version_id: Option<String>) -> Result<PackPreview, String> {
	preview(&read_index(&archive_bytes(path, version_id).await?)?)
}

#[tauri::command]
pub async fn local_servers_pack_search(query: String, offset: u32) -> Result<serde_json::Value, String> {
	let client = reqwest::Client::builder().user_agent("StrataLauncher/0.1").build().map_err(|e| e.to_string())?;
	client.get("https://api.modrinth.com/v2/search").query(&[("query", query), ("facets", r#"[["project_type:modpack"],["server_side:required","server_side:optional"]]"#.into()), ("limit", "12".into()), ("offset", offset.to_string())])
		.send().await.map_err(|e| e.to_string())?.error_for_status().map_err(|e| e.to_string())?.json().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn local_servers_pack_versions(project_id: String) -> Result<serde_json::Value, String> {
	if !project_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') { return Err("Proyecto no válido".into()); }
	let client = reqwest::Client::builder().user_agent("StrataLauncher/0.1").build().map_err(|e| e.to_string())?;
	client.get(format!("https://api.modrinth.com/v2/project/{project_id}/version")).send().await.map_err(|e| e.to_string())?.error_for_status().map_err(|e| e.to_string())?.json().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn local_servers_pack_install<R: tauri::Runtime>(app: tauri::AppHandle<R>, id: String, path: Option<PathBuf>, version_id: Option<String>) -> Result<usize, String> {
	use super::local_servers::{LocalServerState, local_servers_status};
	if local_servers_status(app.clone(), app.state::<LocalServerState>(), id.clone())? { return Err("Detén el servidor antes de instalar un modpack".into()); }
	let server = super::local_servers::find_server(&app, &id)?;
	let bytes = archive_bytes(path, version_id).await?;
	let index = read_index(&bytes)?;
    let info = preview(&index)?;
    if !info.loader.eq_ignore_ascii_case(&server.kind) || info.minecraft != server.game_version { return Err("El modpack no coincide con la versión o loader del servidor".into()); }
	let staging = tempfile::tempdir().map_err(|e| e.to_string())?;
	let client = reqwest::Client::builder().user_agent("StrataLauncher/0.1").redirect(reqwest::redirect::Policy::none()).build().map_err(|e| e.to_string())?;
	let mut paths = std::collections::HashSet::new();
	let mut total = 0_u64;
	for file in &index.files {
		if file.env.get("server").is_some_and(|v| v == "unsupported") { continue; }
		let relative = relative_path(&file.path)?;
		if !paths.insert(relative.clone()) { return Err("El modpack contiene rutas duplicadas".into()); }
		let url = file.downloads.iter().find(|url| download_url(url).is_ok()).ok_or("Archivo sin una descarga compatible")?;
		let content = fetch_file(&client, url, 512_000_000).await?;
		verify(&content, &file.hashes)?;
		total = total.saturating_add(content.len() as u64);
		if total > 5_000_000_000 { return Err("Modpack demasiado grande".into()); }
		let target = staging.path().join(relative);
		fs::create_dir_all(target.parent().ok_or("Ruta no válida")?).map_err(|e| e.to_string())?;
		fs::write(target, content).map_err(|e| e.to_string())?;
	}
	let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
	for prefix in ["overrides/", "server-overrides/"] {
		for number in 0..archive.len() {
			let mut entry = archive.by_index(number).map_err(|e| e.to_string())?;
			let Some(name) = entry.name().strip_prefix(prefix).map(str::to_owned) else { continue };
			if entry.is_dir() { continue; }
			let relative = relative_path(&name)?;
			total = total.saturating_add(entry.size());
			if total > 5_000_000_000 { return Err("Modpack demasiado grande".into()); }
			let target = staging.path().join(&relative);
			fs::create_dir_all(target.parent().ok_or("Ruta no válida")?).map_err(|e| e.to_string())?;
			let mut output = fs::File::create(target).map_err(|e| e.to_string())?;
			std::io::copy(&mut entry, &mut output).map_err(|e| e.to_string())?;
			paths.insert(relative);
		}
	}
	for relative in &paths {
		let destination = server.directory.join(relative);
		if destination.exists() { return Err(format!("No se sobrescribirá el archivo existente: {}", relative.display())); }
		let mut parent = destination.parent();
		while let Some(path) = parent {
			if path == server.directory { break; }
			if path.exists() && (!path.is_dir() || !path.canonicalize().map_err(|e| e.to_string())?.starts_with(&server.directory)) { return Err("La carpeta contiene una ruta externa o no válida".into()); }
			parent = path.parent();
		}
	}
	for relative in &paths {
		let destination = server.directory.join(relative);
		fs::create_dir_all(destination.parent().ok_or("Ruta no válida")?).map_err(|e| e.to_string())?;
		let mut source = fs::File::open(staging.path().join(relative)).map_err(|e| e.to_string())?;
		let mut output = fs::OpenOptions::new().write(true).create_new(true).open(destination).map_err(|e| e.to_string())?;
		std::io::copy(&mut source, &mut output).map_err(|e| e.to_string())?;
	}
	Ok(paths.len())
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn rejects_paths_outside_server() {
		for path in ["../world/level.dat", "C:\\server.properties", "/etc/passwd", "mods/../../escape.jar"] { assert!(relative_path(path).is_err()); }
		assert!(relative_path("mods/example.jar").is_ok());
	}
	#[test]
	fn validates_server_environment_and_loader() {
		let index: PackIndex = serde_json::from_value(serde_json::json!({"formatVersion":1,"game":"minecraft","name":"Test","dependencies":{"minecraft":"1.21.1","fabric-loader":"0.16.0"},"files":[{"path":"mods/client.jar","hashes":{},"downloads":[],"env":{"server":"unsupported"}},{"path":"mods/server.jar","hashes":{},"downloads":[]}]})).unwrap();
		let info = preview(&index).unwrap();
		assert_eq!(info.loader, "Fabric");
		assert_eq!(info.server_files, 1);
		assert_eq!(info.client_files_skipped, 1);
	}
	#[test]
	fn detects_corrupt_downloads() {
		let hashes = HashMap::from([("sha1".into(), "a9993e364706816aba3e25717850c26c9cd0d89d".into())]);
		assert!(verify(b"abc", &hashes).is_ok());
		assert!(verify(b"abd", &hashes).is_err());
		assert!(verify(b"abc", &HashMap::new()).is_err());
	}
}
