use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{BufRead, BufReader, Write},
    path::{Component, Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
};
use tauri::{Emitter, Manager};

#[derive(Clone, Serialize, Deserialize)]
pub struct LocalServer {
    id: String,
    name: String,
    pub(super) directory: PathBuf,
    jar: String,
    java: String,
    memory_mb: u32,
    pub(super) kind: String,
    #[serde(default)]
    pub(super) game_version: String,
    #[serde(default)]
    launch_mode: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    min_memory_mb: u32,
    #[serde(default)]
    extra_jvm_args: String,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    auto_restart: bool,
}

pub struct LocalServerState {
    children: Mutex<HashMap<String, Child>>,
    intentional_stops: Mutex<HashSet<String>>,
    monitors: Mutex<HashSet<String>>,
    cancelled_downloads: Mutex<HashSet<String>>,
    system: Mutex<sysinfo::System>,
}

impl Default for LocalServerState {
    fn default() -> Self {
        Self {
            children: Mutex::new(HashMap::new()),
            intentional_stops: Mutex::new(HashSet::new()),
            monitors: Mutex::new(HashSet::new()),
            cancelled_downloads: Mutex::new(HashSet::new()),
            system: Mutex::new(sysinfo::System::new()),
        }
    }
}

fn data_file<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<PathBuf, String> {
    let root = app.path().app_data_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    Ok(root.join("local-servers.json"))
}

fn read_servers<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<Vec<LocalServer>, String> {
    let file = data_file(app)?;
    if !file.exists() {
        return Ok(Vec::new());
    }
    serde_json::from_slice(&fs::read(file).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

fn save_servers<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    servers: &[LocalServer],
) -> Result<(), String> {
    {
        let file = data_file(app)?;
        let temp = file.with_extension("json.tmp");
        fs::write(
            &temp,
            serde_json::to_vec_pretty(servers).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        fs::rename(temp, file).map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub fn local_servers_list<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<Vec<LocalServer>, String> {
    read_servers(&app)
}

#[tauri::command]
pub fn local_servers_add<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    name: String,
    directory: PathBuf,
    jar: String,
    kind: String,
    java: String,
    memory_mb: u32,
) -> Result<LocalServer, String> {
    if name.trim().is_empty()
        || jar.is_empty()
        || PathBuf::from(&jar).components().count() != 1
    {
        return Err("Nombre o archivo JAR no válido".into());
    }
    if !(512..=65536).contains(&memory_mb) {
        return Err("Memoria fuera de rango".into());
    }
    if !directory.is_absolute() {
        return Err("La ruta debe ser absoluta".into());
    }
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let directory = directory.canonicalize().map_err(|e| e.to_string())?;
    if read_servers(&app)?.iter().any(|s| s.directory == directory) {
        return Err("Esta carpeta ya está registrada".into());
    }
    let server = LocalServer {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        directory,
        jar,
        java,
        memory_mb,
        kind,
        game_version: String::new(),
        launch_mode: "jar".into(),
        args: Vec::new(),
        min_memory_mb: 0,
        extra_jvm_args: String::new(),
        port: None,
        auto_restart: false,
    };
    let mut servers = read_servers(&app)?;
    servers.push(server.clone());
    save_servers(&app, &servers)?;
    Ok(server)
}

#[tauri::command]
pub fn local_servers_create<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    name: String,
    directory: PathBuf,
    executable: String,
    kind: String,
    launch_mode: String,
    args: Vec<String>,
    java: String,
    min_memory_mb: u32,
    memory_mb: u32,
    extra_jvm_args: String,
    port: u16,
    auto_restart: bool,
) -> Result<LocalServer, String> {
    if name.trim().is_empty() || !directory.is_absolute() || port == 0 {
        return Err("Nombre, carpeta o puerto no válido".into());
    }
    if launch_mode != "jar" && launch_mode != "command" {
        return Err("Modo de arranque no válido".into());
    }
    if !(512..=65536).contains(&memory_mb) || min_memory_mb > memory_mb {
        return Err("La memoria mínima debe ser menor o igual que la máxima".into());
    }
    if java.trim().is_empty() || args.len() > 64 || extra_jvm_args.len() > 2048 {
        return Err("Argumentos de arranque no válidos".into());
    }
    let executable_path = PathBuf::from(&executable);
    let relative = if executable_path.is_absolute() {
        executable_path.strip_prefix(&directory).map_err(|_| "El ejecutable debe estar dentro de la carpeta del servidor")?.to_path_buf()
    } else {
        executable_path
    };
    if relative.as_os_str().is_empty() || relative.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err("Ejecutable no válido".into());
    }
    if launch_mode == "jar" && relative.extension().and_then(|e| e.to_str()).is_none_or(|e| !e.eq_ignore_ascii_case("jar")) {
        return Err("El archivo de arranque debe ser un JAR".into());
    }
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let directory = directory.canonicalize().map_err(|e| e.to_string())?;
    if read_servers(&app)?.iter().any(|s| s.directory == directory) {
        return Err("Esta carpeta ya está registrada".into());
    }
    let server = LocalServer {
        id: uuid::Uuid::new_v4().to_string(), name, directory,
        jar: relative.to_string_lossy().into_owned(), java, memory_mb, kind,
        game_version: String::new(), launch_mode, args, min_memory_mb,
        extra_jvm_args, port: Some(port), auto_restart,
    };
    let mut servers = read_servers(&app)?;
    servers.push(server.clone());
    save_servers(&app, &servers)?;
    Ok(server)
}

#[tauri::command]
pub fn local_servers_detect_jar(directory: PathBuf) -> Result<Option<String>, String> {
    let entries = match fs::read_dir(directory) { Ok(entries) => entries, Err(_) => return Ok(None) };
    let mut jars = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_type().map_err(|e| e.to_string())?.is_file()
            && entry.path().extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("jar")) {
            jars.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    Ok(if jars.len() == 1 { jars.pop() } else { None })
}

#[tauri::command]
pub fn local_servers_next_port<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> Result<u16, String> {
    let used: std::collections::HashSet<u16> = read_servers(&app)?.into_iter().filter_map(|s| s.port).collect();
    (25565..=65535).find(|p| !used.contains(p) && std::net::TcpListener::bind(("127.0.0.1", *p)).is_ok())
        .ok_or_else(|| "No hay puertos libres".into())
}

#[derive(Serialize)]
pub struct JavaCheck { available: bool, major_version: Option<u32>, raw: Option<String> }

#[tauri::command]
pub fn local_servers_java_check(java: String) -> JavaCheck {
    let result = Command::new(if java.trim().is_empty() { "java" } else { &java }).arg("-version").output();
    match result {
        Ok(output) if output.status.success() => {
            let raw = String::from_utf8_lossy(&output.stderr).lines().next().map(str::to_owned);
            let major_version = raw.as_deref().and_then(|line| line.split('"').nth(1))
                .and_then(|version| version.split('.').next()).and_then(|v| v.parse().ok());
            JavaCheck { available: true, major_version, raw }
        }
        _ => JavaCheck { available: false, major_version: None, raw: None },
    }
}

#[tauri::command]
pub fn local_servers_import_zip(zip_path: PathBuf, directory: PathBuf) -> Result<Option<String>, String> {
    if !zip_path.is_file() || !directory.is_absolute() { return Err("ZIP o carpeta no válido".into()); }
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    if fs::read_dir(&directory).map_err(|e| e.to_string())?.next().is_some() {
        return Err("Elige una carpeta vacía para importar el ZIP".into());
    }
    let mut archive = zip::ZipArchive::new(fs::File::open(zip_path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    if archive.len() > 100_000 { return Err("ZIP demasiado grande".into()); }
    let mut total = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
        total = total.saturating_add(entry.size());
        if total > 20_000_000_000 { return Err("ZIP demasiado grande".into()); }
        let path = Path::new(entry.name());
        if path.components().any(|c| !matches!(c, Component::Normal(_))) { return Err("Ruta insegura dentro del ZIP".into()); }
        let target = directory.join(path);
        if entry.is_dir() { fs::create_dir_all(&target).map_err(|e| e.to_string())?; continue; }
        if let Some(parent) = target.parent() { fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
        let mut output = fs::OpenOptions::new().write(true).create_new(true).open(target).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut output).map_err(|e| e.to_string())?;
    }
    let entries = fs::read_dir(&directory).map_err(|e| e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())?;
    if entries.len() == 1 && entries[0].file_type().map_err(|e| e.to_string())?.is_dir() {
        let wrapper = entries[0].path();
        for child in fs::read_dir(&wrapper).map_err(|e| e.to_string())? {
            let child = child.map_err(|e| e.to_string())?;
            fs::rename(child.path(), directory.join(child.file_name())).map_err(|e| e.to_string())?;
        }
        fs::remove_dir(wrapper).map_err(|e| e.to_string())?;
    }
    local_servers_detect_jar(directory)
}

const CONSOLE_LOG: &str = "strata-console.log";
const CONSOLE_LOG_MAX_BYTES: u64 = 5_000_000;

/// Registro persistente de la consola. Rota a `.old` al superar el límite y
/// nunca interrumpe la lectura del proceso si el disco falla.
struct ConsoleLog {
    file: Option<fs::File>,
    written: u64,
}

impl ConsoleLog {
    fn open(directory: &Path) -> Self {
        let path = directory.join(CONSOLE_LOG);
        let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if size > CONSOLE_LOG_MAX_BYTES {
            let _ = fs::rename(&path, directory.join(format!("{CONSOLE_LOG}.old")));
        }
        let file = fs::OpenOptions::new().create(true).append(true).open(&path).ok();
        let written = file.as_ref().map(|_| fs::metadata(&path).map(|m| m.len()).unwrap_or(0)).unwrap_or(0);
        Self { file, written }
    }

    fn write(&mut self, line: &str) {
        if self.written > CONSOLE_LOG_MAX_BYTES * 2 { self.file = None; }
        if let Some(file) = self.file.as_mut() {
            if writeln!(file, "{line}").is_ok() { self.written += line.len() as u64 + 1; }
        }
    }
}

/// Últimas líneas de la consola guardada, para recuperar la salida tras reabrir Strata.
#[tauri::command]
pub fn local_servers_console_tail<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    id: String,
    lines: Option<usize>,
) -> Result<Vec<String>, String> {
    let server = read_servers(&app)?.into_iter().find(|s| s.id == id).ok_or("Servidor desconocido")?;
    tail_lines(&server.directory.join(CONSOLE_LOG), lines.unwrap_or(500))
}

fn tail_lines(path: &Path, lines: usize) -> Result<Vec<String>, String> {
    if !path.is_file() { return Ok(Vec::new()); }
    let wanted = lines.clamp(1, 5000);
    let size = fs::metadata(path).map_err(|e| e.to_string())?.len();
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let start = size.saturating_sub(1_000_000);
    std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(start)).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut file, &mut bytes).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&bytes);
    let mut all: Vec<&str> = text.lines().collect();
    if start > 0 && !all.is_empty() { all.remove(0); }
    let from = all.len().saturating_sub(wanted);
    Ok(all[from..].iter().map(|l| (*l).to_owned()).collect())
}

/// Apaga todos los servidores en marcha: `stop`, espera y, si no terminan, `kill`.
pub fn shutdown_all(state: &LocalServerState, grace: std::time::Duration) {
    let Ok(mut children) = state.children.lock() else { return };
    for child in children.values_mut() {
        if let Some(stdin) = child.stdin.as_mut() { let _ = writeln!(stdin, "stop"); }
    }
    let deadline = std::time::Instant::now() + grace;
    while std::time::Instant::now() < deadline {
        children.retain(|_, child| matches!(child.try_wait(), Ok(None)));
        if children.is_empty() { return; }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    for child in children.values_mut() { let _ = child.kill(); let _ = child.wait(); }
    children.clear();
}

/// Resuelve el ejecutable guardado y garantiza que sigue dentro de la carpeta
/// del servidor. No basta con validar al escribir: el JSON puede editarse a mano
/// y un enlace simbólico o una junction puede apuntar fuera tras el registro.
fn resolve_executable(server: &LocalServer) -> Result<PathBuf, String> {
    let relative = Path::new(&server.jar);
    if relative.as_os_str().is_empty()
        || relative.components().any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("Ejecutable no válido: debe ser una ruta relativa dentro de la carpeta del servidor".into());
    }
    let root = server.directory.canonicalize().map_err(|_| "La carpeta del servidor no está disponible")?;
    let resolved = root
        .join(relative)
        .canonicalize()
        .map_err(|_| "No existe el archivo de arranque del servidor")?;
    if !resolved.starts_with(&root) || !resolved.is_file() {
        return Err("El archivo de arranque queda fuera de la carpeta del servidor".into());
    }
    Ok(server.directory.join(relative))
}

#[tauri::command]
pub fn local_servers_start<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: tauri::State<'_, LocalServerState>,
    id: String,
) -> Result<(), String> {
    let server = read_servers(&app)?
        .into_iter()
        .find(|s| s.id == id)
        .ok_or("Servidor desconocido")?;
    state.intentional_stops.lock().map_err(|e| e.to_string())?.remove(&id);
    let executable_path = resolve_executable(&server)?;
    let mut children = state.children.lock().map_err(|e| e.to_string())?;
    if let Some(child) = children.get_mut(&id) {
        if child.try_wait().map_err(|e| e.to_string())?.is_none() {
            return Err("Servidor ya iniciado".into());
        }
        children.remove(&id);
    }
    let is_command = server.launch_mode == "command";
	if let Some(port) = server.port {
		if !["velocity", "bungeecord"].contains(&server.kind.to_ascii_lowercase().as_str()) {
			let path = server.directory.join("server.properties");
			if path.exists() && (!path.is_file() || !path.canonicalize().map_err(|e| e.to_string())?.starts_with(&server.directory)) { return Err("Archivo de propiedades no válido".into()); }
			let current = if path.exists() { fs::read_to_string(&path).map_err(|e| e.to_string())? } else { String::new() };
			let mut lines: Vec<String> = current.lines().filter(|line| !line.trim_start().starts_with("server-port=")).map(str::to_owned).collect();
			lines.push(format!("server-port={port}"));
			fs::write(path, lines.join("\n") + "\n").map_err(|e| e.to_string())?;
		}
	}
    let mut command = if is_command {
        let executable = executable_path.clone();
        if cfg!(windows) && executable.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("bat") || e.eq_ignore_ascii_case("cmd")) {
            let mut cmd = Command::new("cmd.exe");
            cmd.arg("/C").arg(executable);
            cmd
        } else { Command::new(executable) }
    } else { Command::new(&server.java) };
    command.current_dir(&server.directory);
    if !is_command {
        if server.min_memory_mb > 0 { command.arg(format!("-Xms{}M", server.min_memory_mb)); }
        for arg in server.extra_jvm_args.split_whitespace() { command.arg(arg); }
        command.arg(format!("-Xmx{}M", server.memory_mb)).arg("-jar").arg(&executable_path);
    }
    command.args(&server.args);
    if !is_command && !server.args.iter().any(|a| a == "nogui") { command.arg("nogui"); }
    command.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    if let Some(stdout) = child.stdout.take() {
        let app = app.clone();
        let id = id.clone();
        let log_dir = server.directory.clone();
        std::thread::spawn(move || {
            let mut log = ConsoleLog::open(&log_dir);
            for line in BufReader::new(stdout).lines().flatten() {
                log.write(&line);
                let _ = app.emit(
                    "local-server-output",
                    serde_json::json!({"id":id,"line":line}),
                );
            }
        });
    }
    if let Some(stderr) = child.stderr.take() {
        let app = app.clone();
        let id = id.clone();
        let log_dir = server.directory.clone();
        std::thread::spawn(move || {
            let mut log = ConsoleLog::open(&log_dir);
            for line in BufReader::new(stderr).lines().flatten() {
                log.write(&line);
                let _ = app.emit(
                    "local-server-output",
                    serde_json::json!({"id":id,"line":line}),
                );
            }
        });
    }
    children.insert(id.clone(), child);
    if server.auto_restart && state.monitors.lock().map_err(|e| e.to_string())?.insert(id.clone()) {
        let monitor_app = app.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(5));
                match local_servers_status(monitor_app.clone(), monitor_app.state::<LocalServerState>(), id.clone()) {
                    Ok(true) => {},
                    _ => break,
                }
            }
            if let Ok(mut monitors) = monitor_app.state::<LocalServerState>().monitors.lock() { monitors.remove(&id); }
        });
    }
    Ok(())
}

#[tauri::command]
pub fn local_servers_command(
    state: tauri::State<'_, LocalServerState>,
    id: String,
    command: String,
) -> Result<(), String> {
    if command.chars().any(|c| c == '\r' || c == '\n') || command.len() > 4096 {
        return Err("Comando no válido".into());
    }
    let mut children = state.children.lock().map_err(|e| e.to_string())?;
    let child = children.get_mut(&id).ok_or("Servidor detenido")?;
    let stdin = child.stdin.as_mut().ok_or("Consola no disponible")?;
    writeln!(stdin, "{command}").map_err(|e| e.to_string())
}

#[tauri::command]
pub fn local_servers_stop<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: tauri::State<'_, LocalServerState>,
    id: String,
) -> Result<(), String> {
    local_servers_command(state, id.clone(), "stop".into())?;
    app.state::<LocalServerState>().intentional_stops.lock().map_err(|e| e.to_string())?.insert(id);
    Ok(())
}

#[tauri::command]
pub fn local_servers_status<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: tauri::State<'_, LocalServerState>,
    id: String,
) -> Result<bool, String> {
    let mut children = state.children.lock().map_err(|e| e.to_string())?;
    if let Some(child) = children.get_mut(&id) {
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            children.remove(&id);
            drop(children);
            let stopped = state.intentional_stops.lock().map_err(|e| e.to_string())?.remove(&id);
            let restart = !stopped && read_servers(&app)?.iter().any(|s| s.id == id && s.auto_restart);
            if restart {
                let _ = app.emit("local-server-output", serde_json::json!({"id":id,"line":"[Strata] El servidor terminó; reinicio automático."}));
                local_servers_start(app.clone(), app.state::<LocalServerState>(), id)?;
                return Ok(true);
            }
            return Ok(false);
        }
        return Ok(true);
    }
    Ok(false)
}

#[tauri::command]
pub fn local_servers_remove<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: tauri::State<'_, LocalServerState>,
    id: String,
) -> Result<(), String> {
    if local_servers_status(app.clone(), state, id.clone())? {
        return Err("Detén el servidor primero".into());
    }
    let mut servers = read_servers(&app)?;
    servers.retain(|s| s.id != id);
    save_servers(&app, &servers)
}

#[derive(Serialize)]
pub struct LocalFile {
    name: String,
    is_dir: bool,
    size: u64,
}

pub(super) fn find_server<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    id: &str,
) -> Result<LocalServer, String> {
    read_servers(app)?
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| "Servidor desconocido".into())
}

fn resolve_file(
    server: &LocalServer,
    relative: &str,
) -> Result<PathBuf, String> {
    use std::path::Component;
    let path = std::path::Path::new(relative);
    if path.is_absolute()
        || path.components().any(|part| {
            !matches!(part, Component::Normal(_) | Component::CurDir)
        })
    {
        return Err("Ruta no válida".into());
    }
    let root = server.directory.canonicalize().map_err(|e| e.to_string())?;
    let target = root.join(path);
    let resolved = target.canonicalize().map_err(|e| e.to_string())?;
    if !resolved.starts_with(&root) {
        return Err("Ruta fuera del servidor".into());
    }
    Ok(resolved)
}

#[tauri::command]
pub fn local_servers_files<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    id: String,
    relative: String,
) -> Result<Vec<LocalFile>, String> {
    let server = find_server(&app, &id)?;
    let dir = resolve_file(&server, &relative)?;
    if !dir.is_dir() {
        return Err("No es una carpeta".into());
    }
    let mut result = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_symlink() {
            continue;
        }
        let metadata = entry.metadata().map_err(|e| e.to_string())?;
        result.push(LocalFile {
            name: entry.file_name().to_string_lossy().into_owned(),
            is_dir: kind.is_dir(),
            size: metadata.len(),
        });
    }
    result.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name))
    });
    Ok(result)
}

#[tauri::command]
pub fn local_servers_read_file<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    id: String,
    relative: String,
) -> Result<String, String> {
    let server = find_server(&app, &id)?;
    let path = resolve_file(&server, &relative)?;
    if !path.is_file() {
        return Err("No es un archivo".into());
    }
    if path.metadata().map_err(|e| e.to_string())?.len() > 2_000_000 {
        return Err("Archivo demasiado grande para el editor".into());
    }
    fs::read_to_string(path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn local_servers_write_file<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    id: String,
    relative: String,
    contents: String,
) -> Result<(), String> {
    if contents.len() > 2_000_000 {
        return Err("Archivo demasiado grande".into());
    }
    let server = find_server(&app, &id)?;
    let path = resolve_file(&server, &relative)?;
    if !path.is_file() {
        return Err("No es un archivo".into());
    }
    fs::write(path, contents).map_err(|e| e.to_string())
}

fn copy_tree(
    source: &std::path::Path,
    target: &std::path::Path,
) -> Result<(), String> {
    fs::create_dir_all(target).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_symlink() {
            continue;
        }
        let destination = target.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &destination)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), destination).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn local_servers_clone<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    id: String,
    name: String,
    directory: PathBuf,
) -> Result<LocalServer, String> {
    if name.trim().is_empty() || !directory.is_absolute() {
        return Err("Nombre o ruta no válidos".into());
    }
    let source = find_server(&app, &id)?;
    if directory.exists() {
        return Err("La carpeta de destino debe ser nueva".into());
    }
    let parent = directory
        .parent()
        .ok_or("Ruta de destino no válida")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if parent.starts_with(&source.directory) {
        return Err("El destino está dentro del servidor original".into());
    }
    copy_tree(&source.directory, &directory)?;
    let mut server = source.clone();
    server.id = uuid::Uuid::new_v4().to_string();
    server.name = name;
    server.directory = directory.canonicalize().map_err(|e| e.to_string())?;
    let mut servers = read_servers(&app)?;
    servers.push(server.clone());
    save_servers(&app, &servers)?;
    Ok(server)
}

#[tauri::command]
pub fn local_servers_backup<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: tauri::State<'_, LocalServerState>,
    id: String,
) -> Result<PathBuf, String> {
    if local_servers_status(app.clone(), state, id.clone())? {
        return Err("Detén el servidor antes de hacer la copia".into());
    }
    let server = find_server(&app, &id)?;
    let destination = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("local-server-backups")
        .join(&id)
        .join(uuid::Uuid::new_v4().to_string());
    copy_tree(&server.directory, &destination)?;
    Ok(destination)
}

#[derive(Serialize)]
pub struct ServerCatalogOption {
    id: String,
    group: String,
}

fn maven_versions(xml: &str) -> Vec<String> {
    let mut versions: Vec<String> = xml.split("<version>")
        .skip(1)
        .filter_map(|part| part.split("</version>").next())
        .filter(|value| !value.is_empty() && value.len() < 100)
        .map(str::to_owned)
        .collect();
    versions.reverse();
    versions
}

/// Hash en hexadecimal (minúsculas) de `bytes` con `sha512`, `sha256` o `sha1`.
fn digest_hex(algorithm: &str, bytes: &[u8]) -> String {
    use sha1::Digest as _;
    match algorithm {
        "sha512" => format!("{:x}", sha2::Sha512::digest(bytes)),
        "sha256" => format!("{:x}", sha2::Sha256::digest(bytes)),
        _ => format!("{:x}", sha1::Sha1::digest(bytes)),
    }
}

/// Extrae el hash de un archivo `.sha1`/`.sha256`/`.sha512` de Maven (a veces seguido del nombre del archivo).
fn parse_published_checksum(text: &str, hex_length: usize) -> Option<String> {
    let token = text.split_whitespace().next()?.to_ascii_lowercase();
    (token.len() == hex_length && token.chars().all(|c| c.is_ascii_hexdigit())).then_some(token)
}

/// El hash que el repositorio Maven publica junto a un archivo (`<url>.sha512`, `.sha256` o `.sha1`, en ese orden).
async fn maven_checksum(client: &reqwest::Client, url: &str) -> Result<(&'static str, String), String> {
    for (algorithm, hex_length) in [("sha512", 128), ("sha256", 64), ("sha1", 40)] {
        let Ok(response) = client.get(format!("{url}.{algorithm}")).send().await else { continue };
        if !response.status().is_success() || response.content_length().is_some_and(|n| n > 1024) {
            continue;
        }
        let Ok(text) = response.text().await else { continue };
        if let Some(hash) = parse_published_checksum(&text[..text.len().min(1024)], hex_length) {
            return Ok((algorithm, hash));
        }
    }
    Err("No se pudo obtener el hash que publica el repositorio para este instalador, así que no se descarga".into())
}

async fn catalog_json(client: &reqwest::Client, url: &str) -> Result<serde_json::Value, String> {
    client.get(url).send().await.map_err(|e| e.to_string())?
        .error_for_status().map_err(|e| e.to_string())?
        .json().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn local_servers_java_requirement(version: String) -> Result<Option<u32>, String> {
    let client = reqwest::Client::builder().user_agent("StrataLauncher/0.1").build().map_err(|e| e.to_string())?;
    let manifest = catalog_json(&client, "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json").await?;
    let Some(detail_url) = manifest["versions"].as_array().and_then(|versions| versions.iter().find(|v| v["id"].as_str() == Some(version.as_str()))).and_then(|v| v["url"].as_str()) else { return Ok(None) };
    let url = url::Url::parse(detail_url).map_err(|e| e.to_string())?;
    if url.scheme() != "https" || url.host_str() != Some("piston-meta.mojang.com") { return Err("Dominio de manifest inesperado".into()); }
    let detail = catalog_json(&client, detail_url).await?;
    Ok(detail["javaVersion"]["majorVersion"].as_u64().and_then(|v| u32::try_from(v).ok()))
}

#[tauri::command]
pub async fn local_servers_versions(kind: String) -> Result<Vec<ServerCatalogOption>, String> {
    let client = reqwest::Client::builder().user_agent("StrataLauncher/0.1").build().map_err(|e| e.to_string())?;
    let kind = kind.to_ascii_lowercase();
    let mut options = Vec::new();
    match kind.as_str() {
        "paper" | "folia" | "velocity" => {
            let data = catalog_json(&client, &format!("https://fill.papermc.io/v3/projects/{kind}")).await?;
            let groups = data["versions"].as_object().ok_or("Catálogo PaperMC no válido")?;
            for (group, versions) in groups.iter().rev() {
                if let Some(versions) = versions.as_array() {
                    for id in versions.iter().filter_map(|v| v.as_str()) {
                        options.push(ServerCatalogOption { id: id.into(), group: group.clone() });
                    }
                }
            }
        }
        "purpur" => {
            let data = catalog_json(&client, "https://api.purpurmc.org/v2/purpur").await?;
            for id in data["versions"].as_array().ok_or("Catálogo Purpur no válido")?.iter().rev().filter_map(|v| v.as_str()) {
                options.push(ServerCatalogOption { id: id.into(), group: id.split('.').take(2).collect::<Vec<_>>().join(".") });
            }
        }
        "vanilla" => {
            let data = catalog_json(&client, "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json").await?;
            for item in data["versions"].as_array().ok_or("Catálogo Mojang no válido")? {
                if let Some(id) = item["id"].as_str() {
                    options.push(ServerCatalogOption { id: id.into(), group: item["type"].as_str().unwrap_or("other").into() });
                }
            }
        }
        "fabric" => {
            let data = catalog_json(&client, "https://meta.fabricmc.net/v2/versions/game").await?;
            for item in data.as_array().ok_or("Catálogo Fabric no válido")? {
                if let Some(id) = item["version"].as_str() {
                    options.push(ServerCatalogOption { id: id.into(), group: if item["stable"] == true { "release" } else { "snapshot" }.into() });
                }
            }
        }
        "forge" => {
            let xml = client.get("https://maven.minecraftforge.net/net/minecraftforge/forge/maven-metadata.xml").send().await.map_err(|e| e.to_string())?.error_for_status().map_err(|e| e.to_string())?.text().await.map_err(|e| e.to_string())?;
            let mut seen = HashSet::new();
            for full in maven_versions(&xml) {
                let id = full.split('-').next().unwrap_or("");
                if seen.insert(id.to_owned()) {
                    options.push(ServerCatalogOption { id: id.into(), group: id.split('.').take(2).collect::<Vec<_>>().join(".") });
                }
            }
        }
        "neoforge" => {
            let data = catalog_json(&client, "https://maven.neoforged.net/api/maven/versions/releases/net%2Fneoforged%2Fneoforge").await?;
            let mut seen = HashSet::new();
            for full in data["versions"].as_array().ok_or("Catálogo NeoForge no válido")?.iter().rev().filter_map(|v| v.as_str()) {
                let id = full.split('.').take(2).collect::<Vec<_>>().join(".");
                if seen.insert(id.clone()) { options.push(ServerCatalogOption { id: id.clone(), group: id }); }
            }
        }
        "bungeecord" => options.push(ServerCatalogOption { id: "latest".into(), group: "BungeeCord".into() }),
        _ => return Err("Este software no ofrece un catálogo de descargas".into()),
    }
    Ok(options)
}

#[tauri::command]
pub async fn local_servers_builds(kind: String, version: String) -> Result<Vec<ServerCatalogOption>, String> {
    if version.is_empty() || !version.chars().all(|c| c.is_ascii_alphanumeric() || ".-_+".contains(c)) {
        return Err("Versión no válida".into());
    }
    let client = reqwest::Client::builder().user_agent("StrataLauncher/0.1").build().map_err(|e| e.to_string())?;
    let kind = kind.to_ascii_lowercase();
    let mut options = Vec::new();
    match kind.as_str() {
        "paper" | "folia" | "velocity" => {
            let data = catalog_json(&client, &format!("https://fill.papermc.io/v3/projects/{kind}/versions/{version}/builds")).await?;
            for item in data.as_array().ok_or("Builds PaperMC no válidas")? {
                if let Some(id) = item["id"].as_str().map(str::to_owned).or_else(|| item["id"].as_u64().map(|v| v.to_string())) {
                    options.push(ServerCatalogOption { id, group: item["channel"].as_str().unwrap_or("UNKNOWN").into() });
                }
            }
        }
        "purpur" => {
            let data = catalog_json(&client, &format!("https://api.purpurmc.org/v2/purpur/{version}")).await?;
            let latest = data["builds"]["latest"].as_str().map(str::to_owned).or_else(|| data["builds"]["latest"].as_u64().map(|v| v.to_string()));
            for item in data["builds"]["all"].as_array().ok_or("Builds Purpur no válidas")?.iter().rev() {
                if let Some(id) = item.as_str().map(str::to_owned).or_else(|| item.as_u64().map(|v| v.to_string())) {
                    options.push(ServerCatalogOption { group: if latest.as_deref() == Some(id.as_str()) { "STABLE" } else { "UNKNOWN" }.into(), id });
                }
            }
        }
        "vanilla" => options.push(ServerCatalogOption { id: version, group: "STABLE".into() }),
        "fabric" => {
            let data = catalog_json(&client, &format!("https://meta.fabricmc.net/v2/versions/loader/{version}")).await?;
            for item in data.as_array().ok_or("Builds Fabric no válidas")? {
                if let Some(id) = item["loader"]["version"].as_str() {
                    options.push(ServerCatalogOption { id: id.into(), group: if item["loader"]["stable"] == true { "STABLE" } else { "UNKNOWN" }.into() });
                }
            }
        }
        "forge" => {
            let xml = client.get("https://maven.minecraftforge.net/net/minecraftforge/forge/maven-metadata.xml").send().await.map_err(|e| e.to_string())?.error_for_status().map_err(|e| e.to_string())?.text().await.map_err(|e| e.to_string())?;
            for full in maven_versions(&xml).into_iter().filter(|v| v.split('-').next() == Some(version.as_str())) {
                options.push(ServerCatalogOption { id: full, group: if options.is_empty() { "STABLE" } else { "UNKNOWN" }.into() });
            }
        }
        "neoforge" => {
            let data = catalog_json(&client, "https://maven.neoforged.net/api/maven/versions/releases/net%2Fneoforged%2Fneoforge").await?;
            for full in data["versions"].as_array().ok_or("Builds NeoForge no válidas")?.iter().rev().filter_map(|v| v.as_str()).filter(|v| v.split('.').take(2).collect::<Vec<_>>().join(".") == version) {
                options.push(ServerCatalogOption { id: full.into(), group: if options.is_empty() { "STABLE" } else { "UNKNOWN" }.into() });
            }
        }
        "bungeecord" => {
            let data = catalog_json(&client, "https://ci.md-5.net/job/BungeeCord/api/json?tree=builds[number]").await?;
            for item in data["builds"].as_array().ok_or("Builds BungeeCord no válidas")?.iter().take(15) {
                if let Some(id) = item["number"].as_u64() { options.push(ServerCatalogOption { id: id.to_string(), group: if options.is_empty() { "STABLE" } else { "UNKNOWN" }.into() }); }
            }
        }
        _ => return Err("Este software no ofrece builds".into()),
    }
    Ok(options)
}

#[tauri::command]
pub async fn local_servers_download<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: tauri::State<'_, LocalServerState>,
    id: String,
    version: String,
    build_id: Option<String>,
) -> Result<(), String> {
    use sha1::Digest as _;
    if version.is_empty()
        || !version.chars().all(|c| {
            c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_'
        })
    {
        return Err("Versión no válida".into());
    }
    if build_id.as_deref().is_some_and(|id| id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || ".-_+".contains(c))) {
        return Err("Build no válida".into());
    }
    let server = find_server(&app, &id)?;
    state.cancelled_downloads.lock().map_err(|e| e.to_string())?.remove(&id);
    let installer_kind = matches!(server.kind.to_ascii_lowercase().as_str(), "forge" | "neoforge");
    let target = if installer_kind { server.directory.join(if cfg!(windows) { "run.bat" } else { "run.sh" }) } else { server.directory.join(&server.jar) };
    if target.exists() {
        return Err("El JAR ya existe; no se sobrescribirá".into());
    }
    let client = reqwest::Client::builder()
        .user_agent("StrataLauncher/0.1")
        .build()
        .map_err(|e| e.to_string())?;
    let mut expected_sha1 = None;
    let mut expected_sha256 = None;
    let url = match server.kind.to_ascii_lowercase().as_str() {
        "paper" | "folia" | "velocity" => {
            let project = server.kind.to_ascii_lowercase();
            let endpoint = format!(
                "https://fill.papermc.io/v3/projects/{project}/versions/{version}/builds"
            );
            let builds: serde_json::Value = client
                .get(endpoint)
                .send()
                .await
                .map_err(|e| e.to_string())?
                .error_for_status()
                .map_err(|e| e.to_string())?
                .json()
                .await
                .map_err(|e| e.to_string())?;
            let build = builds
                .as_array()
                .ok_or("Respuesta de builds no válida")?
                .iter()
                .find(|b| {
                    let id_matches = build_id.as_deref().map_or(b["channel"] == "STABLE", |wanted| b["id"].as_str() == Some(wanted) || b["id"].as_u64().is_some_and(|n| n.to_string() == wanted));
                    id_matches && b["downloads"]["server:default"]["url"].as_str().is_some()
                })
                .ok_or("No hay build estable para esta versión")?;
            expected_sha256 = build["downloads"]["server:default"]["checksums"]
                ["sha256"]
                .as_str()
                .map(str::to_owned);
            let url = build["downloads"]["server:default"]["url"]
                .as_str()
                .ok_or("Descarga no disponible")?;
            if !url.starts_with("https://")
                || !url::Url::parse(url)
                    .map_err(|e| e.to_string())?
                    .host_str()
                    .is_some_and(|h| {
                        h == "papermc.io" || h.ends_with(".papermc.io")
                    })
            {
                return Err("Dominio de descarga inesperado".into());
            }
            url.to_owned()
        }
        "purpur" => format!("https://api.purpurmc.org/v2/purpur/{version}/{}/download", build_id.as_deref().unwrap_or("latest")),
        "vanilla" => {
            let manifest: serde_json::Value = client.get("https://piston-meta.mojang.com/mc/game/version_manifest_v2.json")
                .send().await.map_err(|e| e.to_string())?.error_for_status().map_err(|e| e.to_string())?
                .json().await.map_err(|e| e.to_string())?;
            let detail_url = manifest["versions"]
                .as_array()
                .ok_or("Manifest no válido")?
                .iter()
                .find(|v| v["id"].as_str() == Some(version.as_str()))
                .and_then(|v| v["url"].as_str())
                .ok_or("Versión no encontrada")?;
            let detail: serde_json::Value = client
                .get(detail_url)
                .send()
                .await
                .map_err(|e| e.to_string())?
                .error_for_status()
                .map_err(|e| e.to_string())?
                .json()
                .await
                .map_err(|e| e.to_string())?;
            expected_sha1 = detail["downloads"]["server"]["sha1"]
                .as_str()
                .map(str::to_owned);
            detail["downloads"]["server"]["url"]
                .as_str()
                .ok_or("Sin descarga de servidor")?
                .to_owned()
        }
        "fabric" => {
            let loader = build_id.as_deref().ok_or("Elige una versión del loader Fabric")?;
            let installers = catalog_json(&client, "https://meta.fabricmc.net/v2/versions/installer").await?;
            let installer = installers.as_array().ok_or("Instaladores Fabric no válidos")?
                .iter().find(|v| v["stable"] == true).or_else(|| installers.as_array().and_then(|a| a.first()))
                .and_then(|v| v["version"].as_str()).ok_or("No hay instalador Fabric")?;
            format!("https://meta.fabricmc.net/v2/versions/loader/{version}/{loader}/{installer}/server/jar")
        }
        "bungeecord" => {
            let build = build_id.as_deref().ok_or("Elige una build de BungeeCord")?;
            format!("https://ci.md-5.net/job/BungeeCord/{build}/artifact/bootstrap/target/BungeeCord.jar")
        }
        "forge" => {
            let build = build_id.as_deref().ok_or("Elige una build de Forge")?;
            if !build.starts_with(&format!("{version}-")) { return Err("La build Forge no corresponde a esta versión".into()); }
            format!("https://maven.minecraftforge.net/net/minecraftforge/forge/{build}/forge-{build}-installer.jar")
        }
        "neoforge" => {
            let build = build_id.as_deref().ok_or("Elige una build de NeoForge")?;
            if !build.starts_with(&format!("{version}.")) { return Err("La build NeoForge no corresponde a esta versión".into()); }
            format!("https://maven.neoforged.net/releases/net/neoforged/neoforge/{build}/neoforge-{build}-installer.jar")
        }
        _ => {
            return Err(
                "Descarga automática aún no implementada para este software"
                    .into(),
            );
        }
    };
    // Los instaladores de Forge y NeoForge se ejecutan con Java: el repositorio Maven publica su hash junto al archivo.
    let published = if matches!(server.kind.to_ascii_lowercase().as_str(), "forge" | "neoforge") {
        Some(maven_checksum(&client, &url).await?)
    } else {
        None
    };
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    let total_bytes = response.content_length();
    if total_bytes.is_some_and(|n| n > 512_000_000) {
        return Err("Descarga demasiado grande".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if state.cancelled_downloads.lock().map_err(|e| e.to_string())?.remove(&id) {
            return Err("Descarga cancelada".into());
        }
        if bytes.len().saturating_add(chunk.len()) > 512_000_000 {
            return Err("Descarga demasiado grande".into());
        }
        bytes.extend_from_slice(&chunk);
        let _ = app.emit("local-server-download-progress", serde_json::json!({"id":id,"downloadedBytes":bytes.len(),"totalBytes":total_bytes}));
    }
    if let Some((algorithm, expected)) = published {
        if digest_hex(algorithm, &bytes) != expected {
            return Err(format!("El {} del instalador no coincide con el que publica el repositorio", algorithm.to_ascii_uppercase()));
        }
    }
    if let Some(expected) = expected_sha256 {
        use sha2::Digest as _;
        let actual = format!("{:x}", sha2::Sha256::digest(&bytes));
        if actual != expected {
            return Err("El SHA-256 del servidor no coincide".into());
        }
    }
    if let Some(expected) = expected_sha1 {
        let actual = format!("{:x}", sha1::Sha1::digest(&bytes));
        if actual != expected {
            return Err("El SHA-1 del servidor no coincide".into());
        }
    }
    let temp = server
        .directory
        .join(format!(".strata-download-{}", uuid::Uuid::new_v4()));
    fs::write(&temp, bytes).map_err(|e| e.to_string())?;
    let download_target = if installer_kind {
        server.directory.join(format!("{}-{}-installer.jar", server.kind.to_ascii_lowercase(), build_id.as_deref().unwrap_or("unknown")))
    } else { target.clone() };
    if target.exists() || download_target.exists() {
        let _ = fs::remove_file(&temp);
        return Err("El archivo de servidor ya existe".into());
    }
    fs::rename(&temp, &download_target).map_err(|e| {
        let _ = fs::remove_file(&temp);
        e.to_string()
    })?;
    if installer_kind {
        let _ = app.emit("local-server-download-progress", serde_json::json!({"id":id,"phase":"install"}));
        let java = server.java.clone();
        let directory = server.directory.clone();
        let installer = download_target.file_name().ok_or("Instalador no válido")?.to_owned();
        let result = tokio::task::spawn_blocking(move || {
            let mut command = Command::new(java);
            command.current_dir(directory).arg("-jar").arg(installer).arg("--installServer");
            #[cfg(windows)] {
                use std::os::windows::process::CommandExt;
                command.creation_flags(0x0800_0000);
            }
            command.output()
        }).await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
        if !result.status.success() {
            let output = String::from_utf8_lossy(&result.stderr);
            return Err(format!("El instalador terminó con {}: {}", result.status, output.chars().rev().take(400).collect::<String>().chars().rev().collect::<String>()));
        }
        if !target.is_file() { return Err("El instalador no creó el script de arranque".into()); }
        let mut servers = read_servers(&app)?;
        let current = servers.iter_mut().find(|s| s.id == id).ok_or("Servidor desconocido")?;
        current.launch_mode = "command".into();
        current.jar = target.file_name().ok_or("Script no válido")?.to_string_lossy().into_owned();
        current.args = vec!["nogui".into()];
        current.game_version = version;
        save_servers(&app, &servers)?;
    }
    Ok(())
}

#[tauri::command]
pub fn local_servers_cancel_download(state: tauri::State<'_, LocalServerState>, id: String) -> Result<(), String> {
    state.cancelled_downloads.lock().map_err(|e| e.to_string())?.insert(id);
    Ok(())
}

#[tauri::command]
pub fn local_servers_update<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    id: String,
    name: String,
    java: String,
    memory_mb: u32,
    game_version: String,
    executable: Option<String>,
    kind: Option<String>,
    launch_mode: Option<String>,
    args: Option<Vec<String>>,
    min_memory_mb: Option<u32>,
    extra_jvm_args: Option<String>,
    port: Option<u16>,
    auto_restart: Option<bool>,
) -> Result<LocalServer, String> {
    if local_servers_status(app.clone(), app.state::<LocalServerState>(), id.clone())? { return Err("Detén el servidor antes de cambiar su configuración".into()); }
    if name.trim().is_empty()
        || java.trim().is_empty()
        || !(512..=65536).contains(&memory_mb)
    {
        return Err("Configuración no válida".into());
    }
    let mut servers = read_servers(&app)?;
    let server = servers
        .iter_mut()
        .find(|s| s.id == id)
        .ok_or("Servidor desconocido")?;
    server.name = name;
    server.java = java;
    server.memory_mb = memory_mb;
    server.game_version = game_version;
    if let Some(value) = executable {
        let path = PathBuf::from(value);
        let relative = if path.is_absolute() { path.strip_prefix(&server.directory).map_err(|_| "El ejecutable debe estar dentro del servidor")?.to_path_buf() } else { path };
        if relative.as_os_str().is_empty() || relative.components().any(|c| !matches!(c, Component::Normal(_))) { return Err("Ejecutable no válido".into()); }
        server.jar = relative.to_string_lossy().into_owned();
    }
    if let Some(value) = kind { server.kind = value; }
    if let Some(value) = launch_mode {
        if value != "jar" && value != "command" { return Err("Modo de lanzamiento no válido".into()); }
        server.launch_mode = value;
    }
    if let Some(value) = args { if value.len() > 64 { return Err("Demasiados argumentos".into()); } server.args = value; }
    if let Some(value) = min_memory_mb { server.min_memory_mb = value; }
    if server.min_memory_mb > memory_mb { return Err("La memoria mínima supera la máxima".into()); }
    if let Some(value) = extra_jvm_args { if value.len() > 2048 { return Err("Argumentos JVM demasiado largos".into()); } server.extra_jvm_args = value; }
    if let Some(value) = port { if value == 0 { return Err("Puerto no válido".into()); } server.port = Some(value); }
    if let Some(value) = auto_restart { server.auto_restart = value; }
    let updated = server.clone();
    save_servers(&app, &servers)?;
    Ok(updated)
}

#[tauri::command]
pub fn local_servers_default_directory<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> Result<PathBuf, String> {
    Ok(app.path().app_data_dir().map_err(|e| e.to_string())?.join("servers").join(uuid::Uuid::new_v4().to_string()))
}

#[tauri::command]
pub fn local_servers_properties<R: tauri::Runtime>(app: tauri::AppHandle<R>, id: String, contents: Option<String>) -> Result<String, String> {
    let server = find_server(&app, &id)?;
    let path = server.directory.join("server.properties");
    if path.exists() && (!path.is_file() || !path.canonicalize().map_err(|e| e.to_string())?.starts_with(&server.directory)) { return Err("Archivo de configuración no válido".into()); }
    if let Some(contents) = contents {
        if local_servers_status(app.clone(), app.state::<LocalServerState>(), id.clone())? { return Err("Detén el servidor antes de cambiar server.properties".into()); }
        if contents.len() > 2_000_000 { return Err("Configuración demasiado grande".into()); }
		let mut servers = read_servers(&app)?;
		if let Some(value) = contents.lines().filter_map(|line| line.trim().strip_prefix("server-port=")).last() {
			let port: u16 = value.trim().parse().map_err(|_| "Puerto no válido en server.properties")?;
			if port == 0 { return Err("Puerto no válido en server.properties".into()); }
			servers.iter_mut().find(|s| s.id == id).ok_or("Servidor desconocido")?.port = Some(port);
		}
        fs::write(path, &contents).map_err(|e| e.to_string())?;
		save_servers(&app, &servers)?;
        return Ok(contents);
    }
    if !path.exists() { return Ok(String::new()); }
    if path.metadata().map_err(|e| e.to_string())?.len() > 2_000_000 { return Err("Configuración demasiado grande".into()); }
    fs::read_to_string(path).map_err(|e| e.to_string())
}

#[derive(Clone, Serialize, Deserialize)]
pub struct LocalContent {
    project_id: String,
    version_id: String,
    title: String,
    file_name: String,
    folder: String,
}

fn content_file<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    id: &str,
) -> Result<PathBuf, String> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("local-content");
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    Ok(root.join(format!("{id}.json")))
}

fn read_content<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    id: &str,
) -> Result<Vec<LocalContent>, String> {
    let file = content_file(app, id)?;
    if !file.exists() {
        return Ok(Vec::new());
    }
    serde_json::from_slice(&fs::read(file).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

fn save_content<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    id: &str,
    items: &[LocalContent],
) -> Result<(), String> {
    let file = content_file(app, id)?;
    let temp = file.with_extension("json.tmp");
    fs::write(
        &temp,
        serde_json::to_vec_pretty(items).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::rename(temp, file).map_err(|e| e.to_string())
}

fn content_loader(
    server: &LocalServer,
) -> Result<(&'static str, &'static str), String> {
    match server.kind.to_ascii_lowercase().as_str() {
        "fabric" => Ok(("fabric", "mod")),
        "forge" => Ok(("forge", "mod")),
        "neoforge" => Ok(("neoforge", "mod")),
        "paper" => Ok(("paper", "plugin")),
        "purpur" => Ok(("purpur", "plugin")),
        "folia" => Ok(("folia", "plugin")),
        "velocity" => Ok(("velocity", "plugin")),
        _ => Err("Este software no admite mods o plugins en el gestor".into()),
    }
}

#[tauri::command]
pub async fn local_servers_content_search<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    id: String,
    query: String,
) -> Result<serde_json::Value, String> {
    let server = find_server(&app, &id)?;
    if server.game_version.is_empty() {
        return Err("Configura antes la versión Minecraft del servidor".into());
    }
    let (loader, project_type) = content_loader(&server)?;
    let facets = serde_json::json!([
        [format!("versions:{}", server.game_version)],
        [format!("categories:{loader}")],
        [format!("project_type:{project_type}")]
    ])
    .to_string();
    let response = reqwest::Client::builder()
        .user_agent("StrataLauncher/0.1")
        .build()
        .map_err(|e| e.to_string())?
        .get("https://api.modrinth.com/v2/search")
        .query(&[("query", query), ("facets", facets)])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    response.json().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub fn local_servers_content_list<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    id: String,
) -> Result<Vec<LocalContent>, String> {
    find_server(&app, &id)?;
    read_content(&app, &id)
}

#[tauri::command]
pub async fn local_servers_content_install<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    id: String,
    project_id: String,
) -> Result<LocalContent, String> {
    use sha2::Digest as _;
    if project_id.is_empty()
        || !project_id.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return Err("ID de proyecto no válido".into());
    }
    let server = find_server(&app, &id)?;
    if server.game_version.is_empty() {
        return Err("Configura antes la versión Minecraft del servidor".into());
    }
    let (loader, project_type) = content_loader(&server)?;
    let client = reqwest::Client::builder()
        .user_agent("StrataLauncher/0.1")
        .build()
        .map_err(|e| e.to_string())?;
    let versions: serde_json::Value = client
        .get(format!(
            "https://api.modrinth.com/v2/project/{project_id}/version"
        ))
        .query(&[
            ("loaders", serde_json::json!([loader]).to_string()),
            (
                "game_versions",
                serde_json::json!([server.game_version]).to_string(),
            ),
            ("include_changelog", "false".to_owned()),
        ])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let selected = versions
        .as_array()
        .ok_or("Respuesta de versiones no válida")?
        .iter()
        .find(|v| v["version_type"] == "release")
        .ok_or("No hay versión estable compatible")?;
    let version_id = selected["id"].as_str().ok_or("ID de versión ausente")?;
    let file = selected["files"]
        .as_array()
        .ok_or("Versión sin archivos")?
        .iter()
        .find(|f| f["primary"] == true)
        .ok_or("Versión sin archivo principal")?;
    let file_name = file["filename"]
        .as_str()
        .ok_or("Nombre de archivo ausente")?;
    if std::path::Path::new(file_name).components().count() != 1
        || !file_name.ends_with(".jar")
    {
        return Err("Archivo de contenido no válido".into());
    }
    let url = file["url"].as_str().ok_or("URL de descarga ausente")?;
    let parsed = url::Url::parse(url).map_err(|e| e.to_string())?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("cdn.modrinth.com")
    {
        return Err("Origen de archivo no válido".into());
    }
    let expected = file["hashes"]["sha512"].as_str().ok_or("Falta SHA-512")?;
    let folder = if project_type == "plugin" {
        "plugins"
    } else {
        "mods"
    };
    let destination_dir = server.directory.join(folder);
    fs::create_dir_all(&destination_dir).map_err(|e| e.to_string())?;
    let resolved_dir =
        destination_dir.canonicalize().map_err(|e| e.to_string())?;
    if !resolved_dir.starts_with(
        server.directory.canonicalize().map_err(|e| e.to_string())?,
    ) {
        return Err("Carpeta de contenido fuera del servidor".into());
    }
    let destination = destination_dir.join(file_name);
    if destination.exists() {
        return Err("El archivo ya existe".into());
    }
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    if response.content_length().is_some_and(|n| n > 200_000_000) {
        return Err("Archivo demasiado grande".into());
    }
    let bytes = response.bytes().await.map_err(|e| e.to_string())?;
    if bytes.len() > 200_000_000 {
        return Err("Archivo demasiado grande".into());
    }
    if format!("{:x}", sha2::Sha512::digest(&bytes)) != expected {
        return Err("El SHA-512 no coincide".into());
    }
    let temp = destination_dir
        .join(format!(".strata-content-{}", uuid::Uuid::new_v4()));
    fs::write(&temp, bytes).map_err(|e| e.to_string())?;
    fs::rename(&temp, &destination).map_err(|e| {
        let _ = fs::remove_file(&temp);
        e.to_string()
    })?;
    let item = LocalContent {
        project_id: project_id.clone(),
        version_id: version_id.to_owned(),
        title: project_id,
        file_name: file_name.to_owned(),
        folder: folder.to_owned(),
    };
    let mut items = read_content(&app, &id)?;
    items.push(item.clone());
    save_content(&app, &id, &items)?;
    Ok(item)
}

#[tauri::command]
pub fn local_servers_content_remove<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    id: String,
    project_id: String,
) -> Result<(), String> {
    let server = find_server(&app, &id)?;
    let mut items = read_content(&app, &id)?;
    let item = items
        .iter()
        .find(|i| i.project_id == project_id)
        .ok_or("Contenido no registrado")?
        .clone();
    let relative = format!("{}/{}", item.folder, item.file_name);
    let path = resolve_file(&server, &relative)?;
    if !path.is_file() {
        return Err("Archivo no encontrado".into());
    }
    fs::remove_file(path).map_err(|e| e.to_string())?;
    items.retain(|i| i.project_id != project_id);
    save_content(&app, &id, &items)
}

#[derive(Clone, Serialize, Deserialize)]
pub struct BackupSchedule {
    server_id: String,
    interval_minutes: u64,
    last_run_unix: u64,
}

fn schedule_file<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("local-backup-schedules.json"))
}

fn read_schedules<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<Vec<BackupSchedule>, String> {
    let file = schedule_file(app)?;
    if !file.exists() {
        return Ok(Vec::new());
    }
    serde_json::from_slice(&fs::read(file).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

fn save_schedules<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    schedules: &[BackupSchedule],
) -> Result<(), String> {
    let file = schedule_file(app)?;
    let temp = file.with_extension("json.tmp");
    fs::write(
        &temp,
        serde_json::to_vec_pretty(schedules).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::rename(temp, file).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn local_servers_schedule_backup<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    id: String,
    interval_minutes: u64,
) -> Result<(), String> {
    find_server(&app, &id)?;
    if interval_minutes != 0 && !(15..=43200).contains(&interval_minutes) {
        return Err("Intervalo fuera de rango".into());
    }
    let mut schedules = read_schedules(&app)?;
    schedules.retain(|s| s.server_id != id);
    if interval_minutes != 0 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs();
        schedules.push(BackupSchedule {
            server_id: id,
            interval_minutes,
            last_run_unix: now,
        });
    }
    save_schedules(&app, &schedules)
}

#[tauri::command]
pub fn local_servers_backup_schedules<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<Vec<BackupSchedule>, String> {
    read_schedules(&app)
}

pub fn start_backup_scheduler<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(60));
            let now = match std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
            {
                Ok(value) => value.as_secs(),
                Err(_) => continue,
            };
            let Ok(mut schedules) = read_schedules(&app) else {
                continue;
            };
            let mut changed = false;
            for schedule in &mut schedules {
                if now.saturating_sub(schedule.last_run_unix)
                    < schedule.interval_minutes.saturating_mul(60)
                {
                    continue;
                }
                if local_servers_backup(
                    app.clone(),
                    app.state::<LocalServerState>(),
                    schedule.server_id.clone(),
                )
                .is_ok()
                {
                    schedule.last_run_unix = now;
                    changed = true;
                }
            }
            if changed {
                let _ = save_schedules(&app, &schedules);
            }
        }
    });
}

#[derive(Serialize)]
pub struct NyxImportResult {
    imported: Vec<LocalServer>,
    skipped: Vec<String>,
}

#[tauri::command]
pub fn local_servers_import_nyx<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    config_path: PathBuf,
) -> Result<NyxImportResult, String> {
    if !config_path.is_file() {
        return Err(
            "No existe el archivo de configuración de NyxLauncher".into()
        );
    }
    if config_path.metadata().map_err(|e| e.to_string())?.len() > 10_000_000 {
        return Err("Archivo de configuración demasiado grande".into());
    }
    let data: serde_json::Value = serde_json::from_slice(
        &fs::read(config_path).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let source = data["servers"]
        .as_array()
        .ok_or("La configuración no contiene servidores")?;
    let mut servers = read_servers(&app)?;
    let mut imported = Vec::new();
    let mut imported_schedules = Vec::new();
    let mut skipped = Vec::new();
    for item in source {
        let name = item["name"].as_str().unwrap_or("Servidor importado");
        let Some(directory) = item["workingDirectory"].as_str() else {
            skipped.push(format!("{name}: falta la carpeta"));
            continue;
        };
        let directory = match PathBuf::from(directory).canonicalize() {
            Ok(path) => path,
            Err(_) => {
                skipped.push(format!("{name}: carpeta inaccesible"));
                continue;
            }
        };
        if servers.iter().any(|s| s.directory == directory) {
            skipped.push(format!("{name}: ya registrado"));
            continue;
        }
        let launch_mode = item["launchMode"].as_str().unwrap_or("jar");
        if launch_mode != "jar" && launch_mode != "command" {
            skipped.push(format!("{name}: modo de arranque desconocido"));
            continue;
        }
        let Some(executable) = item["executable"].as_str() else {
            skipped.push(format!("{name}: falta el ejecutable"));
            continue;
        };
        let executable = PathBuf::from(executable);
        let jar = if executable.is_absolute() {
            match executable.strip_prefix(&directory) {
                Ok(value) => value.to_path_buf(),
                Err(_) => {
                    skipped.push(format!("{name}: ejecutable fuera de la carpeta"));
                    continue;
                }
            }
        } else {
            executable
        };
        if jar.as_os_str().is_empty() || jar.components().any(|c| !matches!(c, Component::Normal(_))) || !directory.join(&jar).is_file() {
            skipped.push(format!("{name}: ejecutable no disponible en la carpeta"));
            continue;
        }
        let memory_mb = item["java"]["maxMemoryMb"]
            .as_u64()
            .unwrap_or(2048)
            .clamp(512, 65536) as u32;
        let java = item["java"]["javaPath"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or("java")
            .to_owned();
        let kind = item["flavor"].as_str().unwrap_or("other").to_owned();
        let game_version = item["installedBuild"]["version"]
            .as_str()
            .unwrap_or("")
            .to_owned();
        let server = LocalServer {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_owned(),
            directory,
            jar: jar.to_string_lossy().into_owned(),
            java,
            memory_mb,
            kind,
            game_version,
            launch_mode: launch_mode.into(),
            args: item["args"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect()).unwrap_or_default(),
            min_memory_mb: item["java"]["minMemoryMb"].as_u64().unwrap_or(0).min(memory_mb as u64) as u32,
            extra_jvm_args: item["java"]["extraArgs"].as_str().unwrap_or("").to_owned(),
            port: item["port"].as_u64().and_then(|v| u16::try_from(v).ok()),
            auto_restart: item["autoRestart"].as_bool().unwrap_or(false),
        };
        if let Some(hours) = item["backup"]["scheduleHours"].as_u64() {
            if hours > 0 && hours <= 720 {
                imported_schedules.push((server.id.clone(), hours * 60));
            }
        }
        servers.push(server.clone());
        imported.push(server);
    }
    save_servers(&app, &servers)?;
    for (id, minutes) in imported_schedules {
        local_servers_schedule_backup(app.clone(), id, minutes)?;
    }
    Ok(NyxImportResult { imported, skipped })
}

#[derive(Serialize)]
pub struct LocalMetrics {
    cpu_percent: f32,
    ram_bytes: u64,
}

#[tauri::command]
pub fn local_servers_metrics(
    state: tauri::State<'_, LocalServerState>,
    id: String,
) -> Result<Option<LocalMetrics>, String> {
    let pid = {
        let mut children = state.children.lock().map_err(|e| e.to_string())?;
        let Some(child) = children.get_mut(&id) else {
            return Ok(None);
        };
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            children.remove(&id);
            return Ok(None);
        }
        sysinfo::Pid::from_u32(child.id())
    };
    let mut system = state.system.lock().map_err(|e| e.to_string())?;
    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    Ok(system.process(pid).map(|process| LocalMetrics {
        cpu_percent: process.cpu_usage(),
        ram_bytes: process.memory(),
    }))
}

#[tauri::command]
pub fn local_servers_disk_usage<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    id: String,
) -> Result<u64, String> {
    let server = find_server(&app, &id)?;
    let mut pending = vec![server.directory];
    let mut bytes = 0_u64;
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                bytes = bytes.saturating_add(
                    entry.metadata().map_err(|e| e.to_string())?.len(),
                );
            }
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod executable_tests {
    use super::*;

    fn server(directory: PathBuf, jar: &str) -> LocalServer {
        LocalServer {
            id: "t".into(), name: "t".into(), directory, jar: jar.into(),
            java: "java".into(), memory_mb: 1024, kind: "vanilla".into(),
            game_version: String::new(), launch_mode: "jar".into(), args: Vec::new(),
            min_memory_mb: 0, extra_jvm_args: String::new(), port: None, auto_restart: false,
        }
    }

    #[test]
    fn rejects_jars_outside_the_server_directory() {
        let base = std::env::temp_dir().join(format!("strata-exe-{}", uuid::Uuid::new_v4()));
        let root = base.join("server");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("server.jar"), b"x").unwrap();
        fs::write(base.join("evil.jar"), b"x").unwrap();
        let root = root.canonicalize().unwrap();

        assert!(resolve_executable(&server(root.clone(), "server.jar")).is_ok());
        assert!(resolve_executable(&server(root.clone(), r"..\evil.jar")).is_err());
        assert!(resolve_executable(&server(root.clone(), "../evil.jar")).is_err());
        assert!(resolve_executable(&server(root.clone(), &base.join("evil.jar").to_string_lossy())).is_err());
        assert!(resolve_executable(&server(root.clone(), "")).is_err());
        assert!(resolve_executable(&server(root.clone(), "missing.jar")).is_err());
        let _ = fs::remove_dir_all(base);
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn temp_dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("strata-life-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn console_log_persists_rotates_and_tails() {
        let dir = temp_dir();
        {
            let mut log = ConsoleLog::open(&dir);
            for i in 0..10 { log.write(&format!("linea {i}")); }
        }
        let tail = tail_lines(&dir.join(CONSOLE_LOG), 3).unwrap();
        assert_eq!(tail, vec!["linea 7", "linea 8", "linea 9"]);
        assert!(tail_lines(&dir.join("no-existe.log"), 5).unwrap().is_empty());

        fs::write(dir.join(CONSOLE_LOG), vec![b'x'; (CONSOLE_LOG_MAX_BYTES + 10) as usize]).unwrap();
        let mut log = ConsoleLog::open(&dir);
        log.write("nueva");
        assert!(dir.join(format!("{CONSOLE_LOG}.old")).is_file());
        assert_eq!(tail_lines(&dir.join(CONSOLE_LOG), 5).unwrap(), vec!["nueva"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn shutdown_all_stops_a_real_server_gracefully() {
        let jar = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../QA/Servidor/server.jar");
        if !jar.is_file() || Command::new("java").arg("-version").output().is_err() { return; }
        let state = LocalServerState::default();
        let child = Command::new("java").arg("-jar").arg(&jar)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        state.children.lock().unwrap().insert("fixture".into(), child);
        let started = Instant::now();
        shutdown_all(&state, Duration::from_secs(15));
        assert!(state.children.lock().unwrap().is_empty());
        assert!(started.elapsed() < Duration::from_secs(10), "debería cerrar sin esperar al kill");
    }

    #[cfg(windows)]
    #[test]
    fn shutdown_all_kills_a_process_that_ignores_stop() {
        let state = LocalServerState::default();
        let child = Command::new("ping").args(["-n", "120", "127.0.0.1"])
            .stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        state.children.lock().unwrap().insert("stubborn".into(), child);
        let started = Instant::now();
        shutdown_all(&state, Duration::from_secs(1));
        assert!(state.children.lock().unwrap().is_empty());
        assert!(started.elapsed() < Duration::from_secs(10));
    }
}

#[cfg(test)]
mod checksum_tests {
    use super::*;

    #[test]
    fn digests_match_known_vectors() {
        assert_eq!(digest_hex("sha1", b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(digest_hex("sha256", b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert!(digest_hex("sha512", b"abc").starts_with("ddaf35a193617aba"));
        assert_eq!(digest_hex("sha512", b"abc").len(), 128);
    }

    /// SEC-04: un archivo alterado no puede dar el hash publicado.
    #[test]
    fn a_tampered_installer_does_not_match_the_published_hash() {
        let published = parse_published_checksum("A9993E364706816ABA3E25717850C26C9CD0D89D  forge-installer.jar\n", 40).unwrap();
        assert_eq!(digest_hex("sha1", b"abc"), published, "el original coincide (se normaliza a minúsculas)");
        assert_ne!(digest_hex("sha1", b"abd"), published, "un byte distinto no coincide");
    }

    #[test]
    fn published_checksums_must_be_plain_hex_of_the_expected_length() {
        assert!(parse_published_checksum("", 40).is_none());
        assert!(parse_published_checksum("<html>404</html>", 40).is_none());
        assert!(parse_published_checksum("a9993e364706816aba3e25717850c26c9cd0d89", 40).is_none(), "39 caracteres");
        assert!(parse_published_checksum("zz993e364706816aba3e25717850c26c9cd0d89d", 40).is_none(), "no hexadecimal");
    }
}
