<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { open } from '@tauri-apps/plugin-dialog'
import { SearchIcon, FolderOpenIcon, PlusIcon, BoxesIcon, CompassIcon, UploadIcon, BoxImportIcon, XIcon } from '@modrinth/assets'
import { Admonition, Button, Card, Chips, EmptyState, IconButton, Tabs } from '@modrinth/ui'
import { onMounted, onUnmounted, ref } from 'vue'
type Server = {
	id: string
	name: string
	directory: string
	jar: string
	kind: string
	java: string
	memory_mb: number
	game_version: string
	launch_mode: string
	args: string[]
	min_memory_mb: number
	extra_jvm_args: string
	port: number | null
	auto_restart: boolean
}
const servers = ref<Server[]>([])
const selected = ref<Server | null>(null)
type PanelTab = 'overview' | 'content' | 'files' | 'backups' | 'access' | 'settings'
const activeTab = ref<PanelTab>('overview')
const showCreate = ref(false)
const createMode = ref<'new' | 'import'>('new')
const createStep = ref(0)
const sourceMode = ref<'custom' | 'pack' | 'existing'>('custom')
const browsePacks = ref(false)
const packQuery = ref('')
const packOffset = ref(0)
const packTotal = ref(0)
const packLoading = ref(false)
type PackHit = { project_id: string; title: string; description: string; icon_url: string | null; downloads: number; author: string; categories: string[] }
type PackVersion = { id: string; name: string; version_number: string; game_versions: string[]; loaders: string[]; files: { filename: string }[] }
type PackPreview = { name: string; minecraft: string; loader: string; loader_version: string; server_files: number; client_files_skipped: number }
const packHits = ref<PackHit[]>([])
const packVersions = ref<PackVersion[]>([])
const packVersionId = ref('')
const packPath = ref('')
const packPreview = ref<PackPreview | null>(null)
const chosenPack = ref<PackHit | null>(null)
const buildPolicy = ref('stable')
const softwareTypes = ['vanilla', 'Paper', 'Purpur', 'Folia', 'Fabric', 'Forge', 'NeoForge', 'Velocity', 'BungeeCord']
async function searchPacks(reset = true) {
	if (reset) packOffset.value = 0
	packLoading.value = true
	try {
		const result = await call<{ hits: PackHit[]; total_hits: number }>('local_servers_pack_search', { query: packQuery.value, offset: packOffset.value })
		packHits.value = result.hits; packTotal.value = result.total_hits
	} catch (e) { error.value = String(e) }
	finally { packLoading.value = false }
}
async function choosePack(hit: PackHit) {
	chosenPack.value = hit; packPath.value = ''; packLoading.value = true
	try {
		packVersions.value = (await call<PackVersion[]>('local_servers_pack_versions', { projectId: hit.project_id })).filter(v => v.files.some(f => f.filename.endsWith('.mrpack')) && v.loaders.some(l => ['fabric', 'forge', 'neoforge', 'minecraft'].includes(l)))
		packVersionId.value = packVersions.value[0]?.id ?? ''
		if (!packVersionId.value) throw new Error('Este modpack no tiene una versión compatible para instalar.')
		await preparePack()
	} catch (e) { error.value = String(e) }
	finally { packLoading.value = false }
}
async function preparePack() {
	packLoading.value = true
	try {
		const preview = await call<PackPreview>('local_servers_pack_preview', { path: packPath.value || null, versionId: packVersionId.value || null })
		packPreview.value = preview; name.value = preview.name; kind.value = preview.loader
		downloadOnCreate.value = true; sourceMode.value = 'pack'
		await loadCatalogVersions()
		createVersion.value = preview.loader === 'NeoForge' ? preview.loader_version.split('.').slice(0, 2).join('.') : preview.minecraft
		await loadCatalogBuilds()
		const wanted = preview.loader === 'Forge' ? `${preview.minecraft}-${preview.loader_version}` : preview.loader === 'vanilla' ? preview.minecraft : preview.loader_version
		if (!catalogBuilds.value.some(b => b.id === wanted)) throw new Error(`El loader ${wanted} del modpack no está disponible.`)
		createBuild.value = wanted; buildPolicy.value = 'other'
		browsePacks.value = false; createStep.value = 0; error.value = ''
	} catch (e) { error.value = String(e); packPreview.value = null }
	finally { packLoading.value = false }
}
async function uploadPack() {
	const path = await open({ multiple: false, filters: [{ name: 'Modpack de Modrinth', extensions: ['mrpack'] }] })
	if (typeof path !== 'string') return
	packPath.value = path; packVersionId.value = ''; chosenPack.value = null
	await preparePack()
}
async function chooseCustom() {
	sourceMode.value = 'custom'; browsePacks.value = false; packPreview.value = null
	downloadOnCreate.value = true; createStep.value = 0
	await loadCatalogVersions()
}
function selectBuildPolicy(policy: string) {
	buildPolicy.value = policy
	if (policy === 'stable') createBuild.value = catalogBuilds.value.find(b => b.group === 'STABLE')?.id ?? catalogBuilds.value[0]?.id ?? ''
	if (policy === 'latest') createBuild.value = catalogBuilds.value[0]?.id ?? ''
}
const launchMode = ref<'jar' | 'command'>('jar')
const launchArgs = ref('')
const minMemory = ref(1024)
const extraJvmArgs = ref('')
const port = ref(25565)
const autoRestart = ref(false)
const downloadOnCreate = ref(false)
const createVersion = ref('')
const createBuild = ref('')
type CatalogOption = { id: string; group: string }
const catalogVersions = ref<CatalogOption[]>([])
const catalogBuilds = ref<CatalogOption[]>([])
const catalogLoading = ref(false)
const recommendedJava = ref<number | null>(null)
const installingCreateJava = ref(false)
const creating = ref(false)
const downloadId = ref('')
const downloadProgress = ref('')
const downloadCancelable = ref(false)
async function cancelCreateDownload() {
	if (downloadId.value) await call('local_servers_cancel_download', { id: downloadId.value })
}
async function loadCatalogVersions() {
	catalogVersions.value = []
	catalogBuilds.value = []
	createVersion.value = ''
	createBuild.value = ''
	recommendedJava.value = null
	if (!downloadOnCreate.value) return
	if (['forge', 'neoforge'].includes(kind.value.toLowerCase())) { launchMode.value = 'command'; jar.value = 'run.bat'; launchArgs.value = 'nogui' }
	else { launchMode.value = 'jar'; jar.value = 'server.jar' }
	catalogLoading.value = true
	try {
		catalogVersions.value = await call('local_servers_versions', { kind: kind.value })
		if (catalogVersions.value.length) { createVersion.value = catalogVersions.value[0].id; await loadCatalogBuilds() }
		error.value = ''
	} catch (e) { error.value = String(e) }
	finally { catalogLoading.value = false }
}
async function loadCatalogBuilds() {
	catalogBuilds.value = []
	createBuild.value = ''
	buildPolicy.value = 'stable'
	if (!createVersion.value) return
	const requestedVersion = createVersion.value
	try {
		catalogBuilds.value = await call('local_servers_builds', { kind: kind.value, version: createVersion.value })
		createBuild.value = catalogBuilds.value.find((option) => option.group === 'STABLE')?.id ?? catalogBuilds.value[0]?.id ?? ''
		error.value = ''
	} catch (e) { error.value = String(e) }
	try {
		const major = await call<number | null>('local_servers_java_requirement', { version: requestedVersion })
		if (createVersion.value === requestedVersion) recommendedJava.value = major
	} catch { recommendedJava.value = null }
}
async function installCreateJava() {
	if (!recommendedJava.value) return
	installingCreateJava.value = true
	try {
		java.value = await invoke<string>('plugin:jre|jre_auto_install_java', { javaVersion: recommendedJava.value })
		await checkJava()
		error.value = ''
	} catch (e) { error.value = String(e) }
	finally { installingCreateJava.value = false }
}
const javaCheck = ref<{ available: boolean; major_version: number | null; raw: string | null } | null>(null)
const detectedJar = ref(false)
async function pickServerDirectory() {
	const picked = await open({ directory: true, multiple: false })
	if (typeof picked !== 'string') return
	directory.value = picked
	const found = await call<string | null>('local_servers_detect_jar', { directory: picked })
	if (found) { jar.value = found; launchMode.value = 'jar'; detectedJar.value = true }
}
async function pickExecutable() {
	const picked = await open({ multiple: false, filters: launchMode.value === 'jar' ? [{ name: 'JAR', extensions: ['jar'] }] : undefined })
	if (typeof picked === 'string') jar.value = picked
}
async function importServerZip() {
	try {
		const zipPath = await open({ multiple: false, filters: [{ name: 'ZIP', extensions: ['zip'] }] })
		if (typeof zipPath !== 'string') return
		const destination = await open({ directory: true, multiple: false })
		if (typeof destination !== 'string') return
		const found = await call<string | null>('local_servers_import_zip', { zipPath, directory: destination })
		directory.value = destination
		if (found) { jar.value = found; launchMode.value = 'jar'; detectedJar.value = true }
		error.value = ''
	} catch (e) { error.value = String(e) }
}
async function checkJava() {
	javaCheck.value = await call('local_servers_java_check', { java: java.value })
}
async function nextCreateStep() {
	if (createStep.value === 0 && (!name.value.trim() || !directory.value.trim())) {
		error.value = 'Introduce el nombre y la carpeta del servidor'; return
	}
	if (createStep.value === 0 && downloadOnCreate.value && (!createVersion.value || !createBuild.value)) { error.value = 'Elige una versión del juego y del software'; return }
	if (createStep.value === 1 && (!jar.value.trim() || minMemory.value > memory.value)) {
		error.value = 'Revisa el ejecutable y la memoria'; return
	}
	if (createStep.value === 1 && downloadOnCreate.value && (!createVersion.value.trim() || !createBuild.value.trim())) {
		error.value = 'Elige una versión y una build para la descarga'; return
	}
	if (createStep.value === 1 && launchMode.value === 'jar') await checkJava()
	error.value = ''
	createStep.value++
}
function openCreate(mode: 'new' | 'import') {
	createMode.value = mode
	createStep.value = -1
	browsePacks.value = false; packPreview.value = null; chosenPack.value = null; packVersions.value = []; packPath.value = ''; packVersionId.value = ''; name.value = ''; error.value = ''
	if (mode === 'new') void call<string>('local_servers_default_directory').then(value => { directory.value = value })
	detectedJar.value = false
	javaCheck.value = null
	if (mode === 'new') void call<number>('local_servers_next_port').then((value) => { port.value = value }).catch(() => {})
	showCreate.value = true
}
function showTab(tab: PanelTab) {
	activeTab.value = tab
	error.value = ''
	if (tab === 'settings') {
		settingsArgs.value = selected.value?.args.join(' ') ?? ''
		void loadProperties()
	}
}
const settingsSection = ref('general')
const settingsArgs = ref('')
const propertiesText = ref('')
async function loadProperties() {
	if (!selected.value) return
	try { propertiesText.value = await call<string>('local_servers_properties', { id: selected.value.id, contents: null }) }
	catch (e) { error.value = String(e) }
}
async function saveProperties() {
	if (!selected.value) return
	try {
		await call('local_servers_properties', { id: selected.value.id, contents: propertiesText.value })
		await refresh()
		selected.value = servers.value.find(server => server.id === selected.value?.id) ?? selected.value
		statusMessage.value = 'server.properties guardado'; error.value = ''
	} catch (e) { error.value = String(e) }
}
function showServerList() {
	selected.value = null
	error.value = ''
}
const serverStatuses = ref<Record<string, boolean>>({})
const memoryPlans = [
	{ label: 'Pequeño', memory: 2048, hint: 'Vanilla y unos pocos amigos', tone: 'blue' },
	{ label: 'Mediano', memory: 4096, hint: 'Plugins y grupos medianos', tone: 'green' },
	{ label: 'Grande', memory: 8192, hint: 'Modpacks o grupos grandes', tone: 'purple' },
] as const
const tabs: { id: PanelTab; label: string }[] = [
	{ id: 'overview', label: 'Resumen' },
	{ id: 'content', label: 'Contenido' },
	{ id: 'files', label: 'Archivos' },
	{ id: 'backups', label: 'Copias' },
	{ id: 'access', label: 'Acceso' },
	{ id: 'settings', label: 'Ajustes' },
]
const tabItems = tabs.map((tab) => ({ value: tab.id, label: tab.label }))
async function selectServer(server: Server) {
	selected.value = server
	activeTab.value = 'overview'
	output.value = []
	call<string[]>('local_servers_console_tail', { id: server.id, lines: 500 })
		.then((lines) => {
			if (selected.value?.id === server.id) output.value = [...lines, ...output.value].slice(-500)
		})
		.catch(() => {})
	openedFile.value = ''
	diskBytes.value = null
	await Promise.all([openFolder(''), refreshContent(), loadBackupSchedule(), refresh()])
}
const name = ref('')
const directory = ref('')
const nyxConfigPath = ref('')
const importReport = ref('')
async function importNyx() {
	try {
		const result = await call<{ imported: Server[]; skipped: string[] }>(
			'local_servers_import_nyx',
			{ configPath: nyxConfigPath.value },
		)
		importReport.value =
			result.imported.length + ' servidores importados. ' + result.skipped.join('; ')
		await refresh()
		error.value = ''
	} catch (e) {
		error.value = String(e)
	}
}
const jar = ref('server.jar')
const kind = ref('vanilla')
const java = ref('java')
const memory = ref(2048)
const command = ref('')
function sendCommand() {
	const value = command.value
	command.value = ''
	void action('local_servers_command', { command: value })
}
type LocalFile = { name: string; is_dir: boolean; size: number }
const files = ref<LocalFile[]>([])
const folder = ref('')
const openedFile = ref('')
const contents = ref('')
const backupLocation = ref('')
const backupInterval = ref(0)
async function loadBackupSchedule() {
	if (!selected.value) return
	try {
		const schedules = await call<{ server_id: string; interval_minutes: number }[]>(
			'local_servers_backup_schedules',
		)
		backupInterval.value =
			schedules.find((item) => item.server_id === selected.value?.id)?.interval_minutes ?? 0
	} catch (e) {
		error.value = String(e)
	}
}
async function saveBackupSchedule() {
	if (!selected.value) return
	try {
		await call('local_servers_schedule_backup', {
			id: selected.value.id,
			intervalMinutes: Number(backupInterval.value),
		})
		statusMessage.value = 'Programa de copias guardado'
		error.value = ''
	} catch (e) {
		error.value = String(e)
	}
}
const cloneName = ref('')
const cloneDirectory = ref('')
const version = ref('')
const javaMajor = ref(21)
const busy = ref(false)
const statusMessage = ref('')
type SearchHit = { project_id: string; title: string; description: string }
type ContentItem = {
	project_id: string
	version_id: string
	title: string
	file_name: string
	folder: string
}
const contentQuery = ref('')
const searchHits = ref<SearchHit[]>([])
const installed = ref<ContentItem[]>([])
async function refreshContent() {
	if (!selected.value) return
	try {
		installed.value = await call<ContentItem[]>('local_servers_content_list', {
			id: selected.value.id,
		})
		error.value = ''
	} catch (e) {
		error.value = String(e)
	}
}
async function searchContent() {
	if (!selected.value) return
	busy.value = true
	try {
		const result = await call<{ hits: SearchHit[] }>('local_servers_content_search', {
			id: selected.value.id,
			query: contentQuery.value,
		})
		searchHits.value = result.hits
		error.value = ''
	} catch (e) {
		error.value = String(e)
	} finally {
		busy.value = false
	}
}
async function installContent(projectId: string) {
	if (!selected.value) return
	busy.value = true
	try {
		await call('local_servers_content_install', { id: selected.value.id, projectId })
		await refreshContent()
		await openFolder('')
		error.value = ''
	} catch (e) {
		error.value = String(e)
	} finally {
		busy.value = false
	}
}
async function removeContent(projectId: string) {
	if (!selected.value) return
	try {
		await call('local_servers_content_remove', { id: selected.value.id, projectId })
		await refreshContent()
		error.value = ''
	} catch (e) {
		error.value = String(e)
	}
}
async function downloadServer() {
	if (!selected.value) return
	busy.value = true
	try {
		await call('local_servers_download', { id: selected.value.id, version: version.value })
		selected.value.game_version = version.value
		await saveSettings()
		statusMessage.value = 'Descarga completada'
		error.value = ''
		await openFolder('')
	} catch (e) {
		error.value = String(e)
	} finally {
		busy.value = false
	}
}
async function installJava() {
	if (!selected.value) return
	busy.value = true
	try {
		const path = await invoke<string>('plugin:jre|jre_auto_install_java', {
			javaVersion: Number(javaMajor.value),
		})
		selected.value = await call<Server>('local_servers_update', {
			id: selected.value.id,
			name: selected.value.name,
			java: path,
			memoryMb: selected.value.memory_mb,
			gameVersion: selected.value.game_version,
		})
		statusMessage.value = 'Java instalado'
		error.value = ''
		await refresh()
	} catch (e) {
		error.value = String(e)
	} finally {
		busy.value = false
	}
}
async function saveSettings() {
	if (!selected.value) return
	try {
		selected.value = await call<Server>('local_servers_update', {
			id: selected.value.id,
			name: selected.value.name,
			java: selected.value.java,
			memoryMb: Number(selected.value.memory_mb),
			gameVersion: selected.value.game_version,
			executable: selected.value.jar,
			kind: selected.value.kind,
			launchMode: selected.value.launch_mode,
			args: settingsArgs.value.trim() ? settingsArgs.value.trim().split(/\s+/) : [],
			minMemoryMb: Number(selected.value.min_memory_mb),
			extraJvmArgs: selected.value.extra_jvm_args,
			port: selected.value.port,
			autoRestart: selected.value.auto_restart,

		})
		statusMessage.value = 'Configuración guardada'
		error.value = ''
		await refresh()
	} catch (e) {
		error.value = String(e)
	}
}
async function openFolder(relative: string) {
	if (!selected.value) return
	try {
		files.value = await call<LocalFile[]>('local_servers_files', {
			id: selected.value.id,
			relative,
		})
		folder.value = relative
		error.value = ''
	} catch (e) {
		error.value = String(e)
	}
}
async function openFile(name: string) {
	if (!selected.value) return
	const relative = [folder.value, name].filter(Boolean).join('/')
	try {
		contents.value = await call<string>('local_servers_read_file', {
			id: selected.value.id,
			relative,
		})
		openedFile.value = relative
		error.value = ''
	} catch (e) {
		error.value = String(e).includes('valid UTF-8')
			? 'Este archivo no es texto y no se puede abrir en el editor.'
			: String(e)
	}
}
async function saveFile() {
	if (!selected.value) return
	try {
		await call('local_servers_write_file', {
			id: selected.value.id,
			relative: openedFile.value,
			contents: contents.value,
		})
		error.value = ''
	} catch (e) {
		error.value = String(e)
	}
}
async function restartServer() {
	if (!selected.value) return
	const id = selected.value.id
	busy.value = true
	try {
		await call('local_servers_stop', { id })
		let stopped = false
		for (let attempt = 0; attempt < 60; attempt++) {
			await new Promise((resolve) => setTimeout(resolve, 500))
			if (!(await call<boolean>('local_servers_status', { id }))) {
				stopped = true
				break
			}
		}
		if (!stopped) throw new Error('El servidor no se detuvo dentro de 30 segundos')
		await call('local_servers_start', { id })
		error.value = ''
		await refresh()
	} catch (e) {
		error.value = String(e)
	} finally {
		busy.value = false
	}
}
async function backup() {
	if (!selected.value) return
	try {
		backupLocation.value = await call<string>('local_servers_backup', { id: selected.value.id })
		error.value = ''
	} catch (e) {
		error.value = String(e)
	}
}
async function cloneServer() {
	if (!selected.value) return
	try {
		const server = await call<Server>('local_servers_clone', {
			id: selected.value.id,
			name: cloneName.value,
			directory: cloneDirectory.value,
		})
		selected.value = server
		await openFolder('')
		await refreshContent()
		await loadBackupSchedule()
		await refresh()
		error.value = ''
	} catch (e) {
		error.value = String(e)
	}
}
const output = ref<string[]>([])
const running = ref(false)
const metrics = ref<{ cpu_percent: number; ram_bytes: number } | null>(null)
const diskBytes = ref<number | null>(null)
async function refreshDisk() {
	if (!selected.value) return
	try {
		diskBytes.value = await call<number>('local_servers_disk_usage', { id: selected.value.id })
		error.value = ''
	} catch (e) {
		error.value = String(e)
	}
}
const error = ref('')
let unlisten: (() => void) | undefined
let unlistenProgress: (() => void) | undefined
let timer: ReturnType<typeof setInterval> | undefined
const call = <T,>(cmd: string, args: Record<string, unknown> = {}) => invoke<T>(cmd, args)
async function refresh() {
	try {
		servers.value = await call<Server[]>('local_servers_list')
		const states = await Promise.all(
			servers.value.map((server) => call<boolean>('local_servers_status', { id: server.id })),
		)
		serverStatuses.value = Object.fromEntries(
			servers.value.map((server, index) => [server.id, states[index] ?? false]),
		)
		if (selected.value) {
			running.value = serverStatuses.value[selected.value.id] ?? false
			metrics.value = await call('local_servers_metrics', { id: selected.value.id })
		}
	} catch (e) {
		error.value = String(e)
	}
}
async function add() {
	if (creating.value) return
	creating.value = true
	try {
		const server = await call<Server>('local_servers_create', {
			name: name.value,
			directory: directory.value,
			executable: jar.value,
			kind: kind.value,
			launchMode: launchMode.value,
			args: launchArgs.value.trim() ? launchArgs.value.trim().split(/\s+/) : [],
			java: java.value,
			minMemoryMb: Number(minMemory.value),
			memoryMb: Number(memory.value),
			extraJvmArgs: extraJvmArgs.value,
			port: Number(port.value),
			autoRestart: autoRestart.value,
		})
		if (downloadOnCreate.value && createVersion.value.trim()) {
			downloadId.value = server.id
			downloadCancelable.value = true
			try {
				await call('local_servers_download', { id: server.id, version: createVersion.value.trim(), buildId: createBuild.value.trim() })
				const updated = await call<Server>('local_servers_update', { id: server.id, name: server.name, java: server.java, memoryMb: server.memory_mb, gameVersion: packPreview.value?.minecraft ?? createVersion.value.trim() })
				Object.assign(server, updated)
				if (sourceMode.value === 'pack' && packPreview.value) {
					downloadCancelable.value = false; downloadProgress.value = 'Instalando archivos del modpack…'
					await call('local_servers_pack_install', { id: server.id, path: packPath.value || null, versionId: packVersionId.value || null })
				}
			} catch (downloadError) {
				await call('local_servers_remove', { id: server.id })
				throw new Error('La descarga no terminó. Los archivos existentes permanecen en la carpeta. ' + String(downloadError))
			}
		}
		selected.value = server
		showCreate.value = false
		activeTab.value = 'overview'
		await openFolder('')
		error.value = ''
		await refresh()
	} catch (e) {
		error.value = String(e)
	} finally {
		creating.value = false
		downloadId.value = ''
		downloadProgress.value = ''
		downloadCancelable.value = false
	}
}
async function action(cmd: string, args: Record<string, unknown> = {}) {
	if (!selected.value) return
	try {
		await call(cmd, { id: selected.value.id, ...args })
		error.value = ''
		await refresh()
	} catch (e) {
		error.value = String(e)
	}
}
async function remove() {
	if (
		!selected.value ||
		!confirm('¿Quitar este servidor de la lista? Sus archivos permanecerán en disco.')
	)
		return
	try {
		await call('local_servers_remove', { id: selected.value.id })
		selected.value = null
		files.value = []
		installed.value = []
		error.value = ''
		await refresh()
	} catch (e) {
		error.value = String(e)
	}
}
onMounted(async () => {
	await refresh()
	unlisten = await listen<{ id: string; line: string }>('local-server-output', (event) => {
		if (event.payload.id === selected.value?.id)
			output.value = [...output.value.slice(-499), event.payload.line]
	})
	unlistenProgress = await listen<{ id: string; downloadedBytes?: number; totalBytes?: number | null; phase?: string }>('local-server-download-progress', (event) => {
		if (event.payload.id === downloadId.value) {
			if (event.payload.phase === 'install') { downloadProgress.value = 'Instalando servidor…'; downloadCancelable.value = false; return }
			const done = ((event.payload.downloadedBytes ?? 0) / 1048576).toFixed(1)
			const total = event.payload.totalBytes ? ` / ${(event.payload.totalBytes / 1048576).toFixed(1)} MB` : ' MB'
			downloadProgress.value = `${done}${total}`
		}
	})
	timer = setInterval(refresh, 2000)
})
onUnmounted(() => {
	unlisten?.()
	unlistenProgress?.()
	if (timer) clearInterval(timer)
})
</script>
<template>
	<main class="local-shell">
		<header class="page-head">
			<div>
				<p class="eyebrow">STRATA / SERVIDORES</p>
				<h1>Servidores personales</h1>
				<p class="subtle">Crea y administra tus servidores de Minecraft en este equipo.</p>
			</div>
			<div class="head-actions">
				<Button @click="openCreate('import')">
					Importar
				</Button>
				<Button type="colored" color="brand" @click="openCreate('new')">
					+ Crear servidor
				</Button>
			</div>
		</header>

		<Admonition v-if="error" type="critical" :body="error" class="mb-4" role="alert" />
		<Admonition v-if="statusMessage" type="success" :body="statusMessage" class="mb-4" role="status" />

		<section v-if="!selected" class="library">
			<div class="section-heading">
				<h2>Mis servidores</h2>
				<span>{{ servers.length }} en este equipo</span>
			</div>
			<div v-if="servers.length" class="server-grid">
				<button
					v-for="server in servers"
					:key="server.id"
					class="server-card"
					@click="selectServer(server)"
				>
					<span class="server-avatar">▦</span>
					<span class="server-card-body"
						><strong>{{ server.name }}</strong
						><small
							>{{ server.kind
							}}<template v-if="server.game_version"> · {{ server.game_version }}</template></small
						><small class="path">{{ server.directory }}</small></span
					>
					<span class="status-pill" :class="serverStatuses[server.id] ? 'online' : ''"
						><i />{{ serverStatuses[server.id] ? 'En marcha' : 'Detenido' }}</span
					>
					<span class="arrow">›</span>
				</button>
			</div>
			<EmptyState v-else type="empty" heading="Aún no hay servidores locales" description="Crea uno nuevo o importa los que ya tienes. Tus carpetas originales permanecerán donde están.">
					<template #actions>
						<Button type="colored" color="brand" @click="openCreate('new')">Crear mi primer servidor</Button>
					</template>
				</EmptyState>
			</section>

		<template v-else>
			<Button class="mb-4" type="quiet" size="sm" @click="showServerList">‹ Todos los servidores</Button>
			<section class="server-hero">
				<div class="server-identity">
					<span class="server-avatar large">▦</span>
					<div>
						<p class="eyebrow">SERVIDOR LOCAL</p>
						<h2>{{ selected.name }}</h2>
						<p class="subtle">
							{{ selected.kind
							}}<template v-if="selected.game_version">
								· Minecraft {{ selected.game_version }}</template
							>
						</p>
					</div>
				</div>
				<div class="hero-actions">
					<span class="status-pill" :class="running ? 'online' : ''"
						><i />{{ running ? 'En marcha' : 'Detenido' }}</span
					><Button type="colored" color="brand" :disabled="running || busy" @click="action('local_servers_start')">
						Iniciar</Button><Button :disabled="!running || busy" @click="restartServer">
						Reiniciar</Button><Button :disabled="!running || busy" @click="action('local_servers_stop')">
						Detener
					</Button>
				</div>
			</section>

			<Tabs
					class="my-5"
					color="gray"
					aria-label="Gestión del servidor"
					:value="activeTab"
					:tabs="tabItems"
					@update:value="showTab($event as PanelTab)"
				/>

			<div v-if="activeTab === 'overview'" class="panel-layout">
				<div class="panel-main">
					<Card class="panel-card">
						<div class="card-title">
							<div>
								<h3>Consola</h3>
								<p>Salida en tiempo real del servidor</p>
							</div>
							<span class="live-dot" :class="{ on: running }" />
						</div>
						<pre aria-label="Consola del servidor">{{
							output.length
								? output.join('\n')
								: 'La salida del servidor aparecerá aquí al iniciarlo.'
						}}</pre>
						<form
							class="command-bar"
							@submit.prevent="sendCommand"
						>
							<span>›</span
							><input
								v-model="command"
								:disabled="!running"
								aria-label="Comando"
								placeholder="Escribe un comando del servidor…"
							/><Button type="colored" color="brand" native-type="submit" :disabled="!running || !command">Enviar</Button>
						</form>
					</Card>
				</div>
				<aside class="panel-side">
					<Card class="panel-card">
						<h3>Recursos</h3>
						<div class="metric">
							<span>CPU</span
							><strong>{{ metrics ? metrics.cpu_percent.toFixed(1) + '%' : '—' }}</strong>
						</div>
						<div class="metric">
							<span>Memoria en uso</span
							><strong>{{
								metrics ? (metrics.ram_bytes / 1048576).toFixed(0) + ' MiB' : '—'
							}}</strong>
						</div>
						<div class="metric">
							<span>RAM asignada</span><strong>{{ selected.memory_mb }} MiB</strong>
						</div>
						<div class="metric">
							<span>Disco</span
							><strong>{{
								diskBytes === null ? '—' : (diskBytes / 1048576).toFixed(0) + ' MiB'
							}}</strong>
						</div>
						<Button type="quiet" size="sm" @click="refreshDisk">Calcular uso de disco</Button>
					</Card>
					<Card class="panel-card">
						<h3>Ubicación</h3>
						<p class="path-block">{{ selected.directory }}</p>
						<p class="subtle">Archivo de inicio: {{ selected.jar }}</p>
					</Card>
				</aside>
			</div>

			<Card v-if="activeTab === 'content'" class="panel-card wide">
				<div class="card-title">
					<div>
						<h3>Mods y plugins</h3>
						<p>
							Busca contenido compatible con {{ selected.kind }} y Minecraft
							{{ selected.game_version || 'la versión configurada' }}.
						</p>
					</div>
				</div>
				<form class="inline-form" @submit.prevent="searchContent">
					<input
						v-model="contentQuery"
						aria-label="Buscar en Modrinth"
						placeholder="Buscar en Modrinth…"
					/><Button type="colored" color="brand" native-type="submit" :disabled="busy || !selected.game_version">
						Buscar
					</Button>
				</form>
				<p v-if="!selected.game_version" class="hint">
					Indica la versión de Minecraft en Ajustes antes de buscar contenido.
				</p>
				<div v-for="hit in searchHits" :key="hit.project_id" class="list-row">
					<div>
						<strong>{{ hit.title }}</strong
						><small>{{ hit.description }}</small>
					</div>
					<Button :disabled="busy" @click="installContent(hit.project_id)">
						Instalar
					</Button>
				</div>
				<h4>Instalado por Strata</h4>
				<p v-if="!installed.length" class="subtle">
					Todavía no has instalado contenido desde esta sección.
				</p>
				<div v-for="item in installed" :key="item.project_id" class="list-row">
					<div>
						<strong>{{ item.title }}</strong
						><small>{{ item.file_name }}</small>
					</div>
					<Button @click="removeContent(item.project_id)">
						Desinstalar
					</Button>
				</div>
			</Card>

			<Card v-if="activeTab === 'files'" class="panel-card wide">
				<div class="card-title">
					<div>
						<h3>Archivos</h3>
						<p>Explora y edita los archivos de este servidor.</p>
					</div>
				</div>
				<div class="file-path">
					<Button type="quiet" size="sm" v-if="folder" @click="openFolder(folder.split('/').slice(0, -1).join('/'))">
						‹ Subir</Button><span>/{{ folder }}</span>
				</div>
				<div v-for="file in files" :key="file.name" class="file-row">
					<button
						@click="
							file.is_dir
								? openFolder([folder, file.name].filter(Boolean).join('/'))
								: openFile(file.name)
						"
					>
						<span>{{ file.is_dir ? '▣' : '▤' }}</span
						>{{ file.name }}</button
					><small>{{ file.is_dir ? 'Carpeta' : (file.size / 1024).toFixed(1) + ' KiB' }}</small>
				</div>
				<p v-if="!files.length" class="subtle">Esta carpeta está vacía.</p>
				<div v-if="openedFile" class="editor">
					<div class="card-title">
						<h4>{{ openedFile }}</h4>
						<Button type="colored" color="brand" @click="saveFile">Guardar archivo</Button>
					</div>
					<textarea v-model="contents" aria-label="Contenido del archivo" />
				</div>
			</Card>

			<div v-if="activeTab === 'backups'" class="panel-layout">
				<Card class="panel-card">
					<h3>Copia manual</h3>
					<p>Guarda una copia de los archivos del servidor. Detén el servidor antes de crearla.</p>
					<Button type="colored" color="brand" :disabled="running" @click="backup">
						Crear copia ahora
					</Button>
					<p v-if="backupLocation" class="hint">Copia creada en {{ backupLocation }}</p>
				</Card>
				<Card class="panel-card">
					<h3>Copias programadas</h3>
					<p>Intervalo en minutos. Usa 0 para desactivar la programación.</p>
					<label class="field"
						>Cada cuántos minutos<input
							v-model.number="backupInterval"
							type="number"
							min="0"
							max="43200" /></label
					><Button @click="saveBackupSchedule">Guardar programa</Button>
				</Card>
			</div>

			<Card v-if="activeTab === 'access'" class="panel-card wide">
				<h3>Acceso al servidor personal</h3>
				<p>Este servidor se administra desde Strata en este equipo.</p>
				<div class="metric"><span>Panel remoto</span><strong>Desactivado</strong></div>
				<div class="metric"><span>Usuarios remotos</span><strong>Sin configurar</strong></div>
				<p class="hint">El acceso remoto no se habilita ni abre puertos desde esta sección.</p>
			</Card>

			<div v-if="activeTab === 'settings'" class="settings-stack">
				<Chips
				:capitalize="false"
						:items="['general', 'properties']"
						:format-label="(section: string) => section === 'general' ? 'General y lanzamiento' : 'server.properties'"
						:model-value="settingsSection"
						aria-label="Sección de ajustes"
						@update:model-value="settingsSection = $event as string; if ($event === 'properties') loadProperties()"
					/>
<Card v-if="settingsSection === 'properties'" class="panel-card editor">
	<div class="card-title"><div><h3>Propiedades del servidor</h3><p>Conserva todas las opciones y comentarios del archivo. Detén el servidor para guardar.</p></div><Button type="colored" color="brand" :disabled="running" @click="saveProperties">Guardar</Button></div>
	<textarea v-model="propertiesText" aria-label="server.properties" placeholder="El servidor generará este archivo al iniciarse. También puedes escribir sus propiedades aquí." />
</Card>
<Card v-if="settingsSection === 'general'" class="panel-card settings-general">
	<h3>General</h3>
	<label class="field">Nombre<input v-model="selected.name" /></label>
	<label class="field">Directorio de trabajo<input :value="selected.directory" readonly /><small>Carpeta vinculada al servidor. Sus archivos permanecen en esta ubicación.</small></label>
	<div class="form-grid"><label class="field">Puerto<input v-model.number="selected.port" type="number" min="1" max="65535" /></label><label class="field">Software<select v-model="selected.kind"><option v-for="type in [...softwareTypes, 'other']" :key="type">{{ type }}</option></select></label></div>
	<label class="toggle-field"><span>Reiniciar automáticamente si el proceso se cierra</span><input v-model="selected.auto_restart" type="checkbox" /></label>
	<h3>Lanzamiento</h3>
	<label class="field">Modo de lanzamiento<select v-model="selected.launch_mode"><option value="jar">Archivo .jar (vía Java)</option><option value="command">Comando o ejecutable</option></select></label>
	<label class="field">{{ selected.launch_mode === 'jar' ? 'Archivo .jar' : 'Ejecutable' }}<input v-model="selected.jar" /></label>
	<label class="field">Argumentos de lanzamiento<input v-model="settingsArgs" placeholder="--world mundo" /></label>
	<label class="field">Java<input v-model="selected.java" /></label>
	<div class="form-grid"><label class="field">RAM mínima (MB)<input v-model.number="selected.min_memory_mb" type="number" min="0" max="65536" /></label><label class="field">RAM máxima (MB)<input v-model.number="selected.memory_mb" type="number" min="512" max="65536" /></label></div>
	<label class="field">Argumentos JVM adicionales<input v-model="selected.extra_jvm_args" placeholder="-XX:+UseG1GC" /></label>
	<label class="field">Versión instalada<input v-model="selected.game_version" /></label>
	<Button type="colored" color="brand" :disabled="running" @click="saveSettings">Guardar cambios</Button><p v-if="running" class="hint">Detén el servidor antes de cambiar el lanzamiento.</p>
</Card>
<Card class="panel-card">
					<h3>Software y Java</h3>
					<p>
						Descarga el software del servidor en su carpeta o instala una versión de Java con el
						gestor de Strata.
					</p>
					<div class="inline-form">
						<label class="field"
							>Versión Minecraft<input v-model="version" placeholder="Ej.: 1.21.1" /></label
						><Button :disabled="busy || !version" @click="downloadServer">
							Descargar software
						</Button>
					</div>
					<div class="inline-form">
						<label class="field"
							>Versión Java<input
								v-model.number="javaMajor"
								type="number"
								min="8"
								max="30" /></label
						><Button :disabled="busy" @click="installJava">
							Instalar Java
						</Button>
					</div>
				</Card>
				<Card class="panel-card">
					<h3>Clonar servidor</h3>
					<form class="form-grid" @submit.prevent="cloneServer">
						<label class="field">Nombre del clon<input v-model="cloneName" required /></label
						><label class="field">Carpeta nueva<input v-model="cloneDirectory" required /></label
						><Button native-type="submit">Clonar servidor</Button>
					</form>
				</Card>
				<Card class="panel-card danger">
					<h3>Quitar de la lista</h3>
					<p>Los archivos del servidor permanecerán en disco.</p>
					<Button type="colored" color="red" @click="remove">Quitar servidor</Button>
				</Card>
			</div>
		</template>

		<div v-if="showCreate" class="modal-mask" @click.self="!creating && (showCreate = false)">
			<div
				:class="['create-modal', { 'pack-modal': browsePacks }]"
				role="dialog"
				aria-modal="true"
				:aria-label="createMode === 'new' ? 'Crear servidor local' : 'Importar servidores'"
			>
				<div class="modal-head">
					<div>
						<p class="steps">
							{{
								createMode === 'new'
									? (createStep === -1 ? 'Elige tu servidor' : 'Configuración › Lanzamiento › Revisión')
									: 'Importar › Revisar › Añadir'
							}}
						</p>
						<h2>{{ createMode === 'new' ? 'Crear servidor local' : 'Importar servidores' }}</h2>
					</div>
					<IconButton label="Cerrar" :disabled="creating" @click="showCreate = false"><XIcon aria-hidden="true" /></IconButton>
				</div>
				<form v-if="createMode === 'new'" class="modal-body" @submit.prevent="createStep === 2 && add()">
<Admonition v-if="error" type="critical" :body="error" class="mb-4" role="alert" />
					<template v-if="createStep === -1">
	<div v-if="!browsePacks" class="creation-choices">
		<h3>¿Qué quieres preparar?</h3>
		<div class="pack-search" @keydown.enter.prevent="browsePacks = true; searchPacks()"><SearchIcon /><input v-model="packQuery" placeholder="Buscar modpacks para tu servidor…" aria-label="Buscar modpacks" /><Button type="quiet" size="sm" @click="browsePacks = true; searchPacks()">Buscar</Button></div>
		<div class="choice-divider">o elige cómo empezar</div>
		<button type="button" class="creation-choice" @click="chooseCustom"><span class="choice-icon" aria-hidden="true"><BoxesIcon /></span><span><strong>Configuración personalizada</strong><small>Elige el software y la versión de Minecraft.</small></span><span>›</span></button>
		<button type="button" class="creation-choice" @click="browsePacks = true; searchPacks()"><span class="choice-icon" aria-hidden="true"><CompassIcon /></span><span><strong>Empezar desde un modpack</strong><small>Explora packs de Modrinth compatibles con servidores.</small></span><span>›</span></button>
		<button type="button" class="creation-choice" @click="uploadPack"><span class="choice-icon" aria-hidden="true"><UploadIcon /></span><span><strong>Subir un modpack</strong><small>Instala un archivo .mrpack de tu equipo.</small></span><span>›</span></button>
		<button type="button" class="creation-choice" @click="sourceMode = 'existing'; downloadOnCreate = false; createStep = 0; pickServerDirectory()"><span class="choice-icon" aria-hidden="true"><BoxImportIcon /></span><span><strong>Importar un servidor</strong><small>Usa una carpeta existente sin mover sus archivos.</small></span><span>›</span></button>
	</div>
	<div v-else class="pack-browser">
		<div class="pack-search" @keydown.enter.prevent="searchPacks()"><SearchIcon /><input v-model="packQuery" placeholder="Buscar modpacks…" aria-label="Buscar modpacks" /><Button :disabled="packLoading" @click="searchPacks()">Buscar</Button></div>
		<p class="subtle">Solo packs que declaran compatibilidad con servidores. Los archivos exclusivos del cliente se omiten.</p>
		<p v-if="packLoading" role="status">Cargando modpacks…</p>
		<div v-for="hit in packHits" :key="hit.project_id" class="pack-result">
			<img v-if="hit.icon_url" :src="hit.icon_url" alt="" /><span v-else class="pack-placeholder">▦</span>
			<div><h3>{{ hit.title }} <small>por {{ hit.author }}</small></h3><p>{{ hit.description }}</p><div class="pack-tags"><span v-for="tag in hit.categories.slice(0, 4)" :key="tag">{{ tag }}</span><span>↓ {{ hit.downloads.toLocaleString() }}</span></div></div>
			<Button type="outlined" color="brand" :disabled="packLoading" @click="choosePack(hit)"><PlusIcon /> Elegir</Button>
		</div>
		<p v-if="!packLoading && !packHits.length" class="subtle">No hay resultados para esta búsqueda.</p>
		<div class="modal-actions"><Button @click="browsePacks = false">Volver</Button><div><Button type="quiet" size="sm" :disabled="!packOffset || packLoading" @click="packOffset -= 12; searchPacks(false)">Anterior</Button><span>{{ Math.floor(packOffset / 12) + 1 }}</span><Button type="quiet" size="sm" :disabled="packOffset + 12 >= packTotal || packLoading" @click="packOffset += 12; searchPacks(false)">Siguiente</Button></div></div>
	</div>
</template>
<template v-else-if="createStep === 0">
	<div class="creation-banner"><img v-if="chosenPack?.icon_url" :src="chosenPack.icon_url" alt="" /><span v-else class="choice-icon">▦</span><div><strong>{{ packPreview?.name ?? (sourceMode === 'existing' ? 'Servidor existente' : 'Tu próximo mundo') }}</strong><p class="subtle">{{ packPreview ? `${packPreview.server_files} archivos de servidor · ${packPreview.client_files_skipped} archivos de cliente omitidos` : 'Una configuración a tu medida.' }}</p></div></div>
	<label class="field">Nombre<input v-model="name" placeholder="Mi servidor de Minecraft" /></label>
	<template v-if="downloadOnCreate">
		<label v-if="packVersions.length" class="field">Versión del modpack<select v-model="packVersionId" :disabled="packLoading" @change="preparePack"><option v-for="release in packVersions" :key="release.id" :value="release.id">{{ release.name }} · {{ release.game_versions.join(', ') }}</option></select></label>
		<label class="field">Software del servidor</label><Chips
				:capitalize="false"
				:items="softwareTypes"
				:format-label="(type: string) => type === 'vanilla' ? 'Vanilla' : type"
				:model-value="kind"
				:disabled-items="packPreview || catalogLoading ? softwareTypes : []"
				aria-label="Software del servidor"
				@update:model-value="kind = $event as string; loadCatalogVersions()"
			/>
		<label class="field">{{ ['Velocity','BungeeCord'].includes(kind) ? 'Versión del proxy' : 'Versión del juego' }}<select v-model="createVersion" :disabled="catalogLoading || !!packPreview" @change="loadCatalogBuilds"><option v-for="option in catalogVersions" :key="option.id" :value="option.id">{{ option.id }} · {{ option.group }}</option></select></label>
		<label class="field">Versión del software</label><Chips
				:capitalize="false"
				v-if="!packPreview"
				:items="['stable', 'latest', 'other']"
				:format-label="(policy: string) => ({ stable: 'Estable', latest: 'Última', other: 'Otra' } as Record<string, string>)[policy]"
				:model-value="buildPolicy"
				aria-label="Versión del software"
				@update:model-value="selectBuildPolicy($event as string)"
			/>
		<label v-if="buildPolicy === 'other' || packPreview" class="field">Build<select v-model="createBuild" :disabled="catalogLoading || !!packPreview"><option v-for="option in catalogBuilds" :key="option.id" :value="option.id">{{ option.id }} · {{ option.group }}</option></select></label>
		<p v-if="catalogLoading || packLoading" class="subtle" role="status">Cargando versiones…</p>
	</template>
	<label class="field">Carpeta del servidor<div class="path-picker"><input v-model="directory" /><Button aria-label="Elegir carpeta" @click="pickServerDirectory"><FolderOpenIcon /></Button></div></label>
	<Button v-if="sourceMode === 'existing'" @click="importServerZip">Importar desde .zip</Button>
	<p v-if="detectedJar" class="subtle">Archivo detectado: {{ jar }}</p>
</template>
					<template v-else-if="createStep === 1">
					<p class="subtle">Configura el proceso que arrancará en esta carpeta.</p>
					<div class="form-grid">
						<label class="field">Modo de lanzamiento<select v-model="launchMode"><option value="jar">Archivo JAR vía Java</option><option value="command">Comando o ejecutable</option></select></label>
						<label class="field full">{{ launchMode === 'jar' ? 'JAR del servidor' : 'Ejecutable' }}<input v-model="jar" placeholder="server.jar o run.bat" /></label>
						<Button @click="pickExecutable">Elegir ejecutable</Button>
						<label class="field full">Argumentos de lanzamiento<input v-model="launchArgs" placeholder="--world mundo" /></label>
						<label v-if="launchMode === 'jar'" class="field full">Java<input v-model="java" placeholder="java o ruta a java.exe" @blur="checkJava" /></label>
						<p v-if="recommendedJava" class="subtle">Esta versión de Minecraft recomienda Java {{ recommendedJava }}.</p>
						<Button v-if="recommendedJava" :disabled="installingCreateJava" @click="installCreateJava">{{ installingCreateJava ? 'Instalando Java…' : `Instalar Java ${recommendedJava}` }}</Button>
						<p v-if="launchMode === 'jar' && javaCheck" class="subtle">{{ javaCheck.available ? (javaCheck.raw || 'Java disponible') : 'Java no disponible en esta ruta' }}</p>
					</div>
					<p class="subtle">Memoria máxima del servidor</p>
					<div v-if="launchMode === 'jar'" class="plan-grid">
						<button
							v-for="plan in memoryPlans"
							:key="plan.label"
							type="button"
							class="plan-card"
							:class="{ selected: memory === plan.memory }"
							@click="memory = plan.memory"
						>
							<strong>{{ plan.label }}</strong
							><b>{{ plan.memory / 1024 }} GB <small>RAM</small></b
							><span>{{ plan.hint }}</span
							><em>{{ memory === plan.memory ? 'Seleccionado' : 'Seleccionar' }}</em>
						</button>
					</div>
					<div v-if="launchMode === 'jar'" class="form-grid">
						<label class="field">RAM mínima (MB)<input v-model.number="minMemory" type="number" min="0" max="65536" /></label>
						<label class="field">RAM máxima (MB)<input v-model.number="memory" type="number" min="512" max="65536" /></label>
						<label class="field full">Argumentos JVM adicionales<input v-model="extraJvmArgs" placeholder="-XX:+UseG1GC" /></label>
					</div>
					</template>
					<template v-else>
					<div class="form-grid">
						<label class="field">Puerto<input v-model.number="port" type="number" min="1" max="65535" /></label>
						<label class="field">Reinicio automático<select v-model="autoRestart"><option :value="false">Desactivado</option><option :value="true">Activado</option></select></label>
					</div>
					<p class="subtle">{{ name }} · {{ kind }} · {{ launchMode === 'jar' ? 'Java' : 'Comando' }} · {{ directory }}</p>
					<p class="subtle">{{ jar }} · puerto {{ port }} · {{ memory }} MB máximo</p>
					<p v-if="creating" role="status" class="subtle">{{ downloadId ? `Descargando: ${downloadProgress || 'iniciando…'}` : 'Creando servidor…' }}</p>
					</template>
					<div class="modal-actions">
						<Button v-if="creating && downloadId && downloadCancelable" @click="cancelCreateDownload">Cancelar descarga</Button>
						<Button v-else :disabled="creating" @click="createStep >= 0 ? createStep-- : showCreate = false">{{ createStep >= 0 ? 'Volver' : 'Cancelar' }}</Button>
						<Button type="colored" color="brand" v-if="createStep >= 0 && createStep < 2" @click="nextCreateStep">Continuar</Button>
						<Button type="colored" color="brand" native-type="submit" v-else-if="createStep === 2" :disabled="creating">Crear servidor</Button>
					</div>
				</form>
				<form v-else class="modal-body" @submit.prevent="importNyx">
					<p class="subtle">
						Importa servidores JAR de NyxLauncher sin mover ni borrar sus carpetas.
					</p>
					<label class="field"
						>Archivo de configuración de NyxLauncher<input
							v-model="nyxConfigPath"
							placeholder="Ruta absoluta al JSON de NyxLauncher"
							required
					/></label>
					<p v-if="importReport" role="status">{{ importReport }}</p>
					<div class="modal-actions">
						<Button @click="showCreate = false">
							Cerrar</Button><Button type="colored" color="brand" native-type="submit">Importar servidores</Button>
					</div>
				</form>
			</div>
		</div>
	</main>
</template>
<style scoped>
/* Colores: solo tokens del tema (claro/oscuro) y del acento del usuario. Botones, avisos, pestañas, chips, tarjetas y
   estado vacío son componentes compartidos de @modrinth/ui; aquí queda solo la maquetación propia de la página. */
.local-shell {
	max-width: 1320px;
	margin: auto;
	padding: 2rem 2.25rem 5rem;
	color: var(--color-text-default);
}
.local-shell h1,
.local-shell h2,
.local-shell h3,
.local-shell h4 {
	color: var(--color-contrast);
}
.page-head,
.section-heading,
.card-title,
.hero-actions,
.head-actions,
.server-hero,
.server-identity,
.list-row,
.metric,
.file-row,
.modal-head,
.modal-actions {
	display: flex;
	align-items: center;
	justify-content: space-between;
	gap: 1rem;
}
.page-head {
	margin-bottom: 2rem;
}
.page-head h1 {
	font-size: 2rem;
	margin: 0.2rem 0;
}
.eyebrow,
.steps {
	font-size: 0.7rem;
	font-weight: 800;
	letter-spacing: 0.12em;
	color: var(--color-brand);
	margin: 0;
}
.subtle,
.section-heading span,
.card-title p,
.panel-card p,
.server-card small {
	color: var(--color-text-tertiary);
}
.subtle {
	margin: 0.25rem 0;
}
.head-actions {
	flex-shrink: 0;
}
.section-heading {
	margin-bottom: 1rem;
}
.section-heading h2 {
	font-size: 1.2rem;
	margin: 0;
}
.server-grid {
	display: grid;
	gap: 0.75rem;
}
.server-card,
.server-hero,
.creation-choice,
.plan-card,
.pack-result,
.toggle-field {
	border: 1px solid var(--surface-5);
	background: var(--color-raised-bg);
}
.server-card {
	display: flex;
	align-items: center;
	gap: 1rem;
	width: 100%;
	padding: 1.1rem 1.2rem;
	text-align: left;
	border-radius: 0.9rem;
	color: inherit;
	cursor: pointer;
}
.server-card:hover,
.creation-choice:hover {
	border-color: var(--color-brand);
}
.server-card:focus-visible,
.creation-choice:focus-visible,
.plan-card:focus-visible,
.file-row button:focus-visible {
	outline: 2px solid var(--color-brand);
	outline-offset: 2px;
}
.server-avatar {
	display: grid;
	place-items: center;
	width: 3.2rem;
	height: 3.2rem;
	flex-shrink: 0;
	border-radius: 0.7rem;
	background: var(--color-brand-highlight);
	color: var(--color-brand);
	font-size: 1.6rem;
}
.server-avatar.large {
	width: 3.8rem;
	height: 3.8rem;
}
.server-card-body {
	display: grid;
	gap: 0.2rem;
	min-width: 0;
	flex: 1;
}
.server-card-body strong {
	font-size: 1.1rem;
	color: var(--color-contrast);
}
.server-card-body .path {
	overflow: hidden;
	text-overflow: ellipsis;
	white-space: nowrap;
}
/* Estado: siempre con texto (no depende solo del color); «en marcha» es verde semántico. */
.status-pill {
	display: inline-flex;
	align-items: center;
	gap: 0.45rem;
	border-radius: 2rem;
	background: var(--surface-4);
	color: var(--color-text-tertiary);
	padding: 0.4rem 0.7rem;
	font-size: 0.78rem;
	font-weight: 700;
	white-space: nowrap;
}
.status-pill i,
.live-dot {
	width: 0.5rem;
	height: 0.5rem;
	border-radius: 50%;
	background: var(--color-text-tertiary);
}
.status-pill.online {
	background: var(--color-green-highlight);
	color: var(--color-green);
}
.status-pill.online i,
.live-dot.on {
	background: var(--color-green);
}
.arrow {
	font-size: 1.7rem;
	color: var(--color-text-tertiary);
}
.server-hero {
	border-radius: 1rem;
	padding: 1.35rem 1.5rem;
	flex-wrap: wrap;
}
.server-identity {
	justify-content: flex-start;
}
.server-identity h2 {
	font-size: 1.5rem;
	margin: 0.2rem 0;
}
.panel-layout {
	display: grid;
	grid-template-columns: minmax(0, 2fr) minmax(240px, 1fr);
	gap: 1rem;
}
.panel-main,
.panel-side,
.settings-stack {
	display: grid;
	gap: 1rem;
	align-content: start;
}
.panel-card {
	min-width: 0;
}
.panel-card.wide {
	max-width: 100%;
}
.panel-card h3 {
	margin: 0 0 0.8rem;
	font-size: 1.1rem;
}
.panel-card h4 {
	margin: 1.5rem 0 0.6rem;
}
.card-title {
	margin-bottom: 1rem;
}
.card-title h3,
.card-title p {
	margin: 0;
}
.card-title p {
	font-size: 0.85rem;
	margin-top: 0.25rem;
}
/* La consola es un terminal: colores fijos a propósito (la salida del servidor no depende del tema). */
.panel-card pre {
	height: 300px;
	overflow: auto;
	background: #181a1f;
	color: #d6f2df;
	padding: 1rem;
	border: 1px solid #363840;
	border-radius: 0.6rem;
	white-space: pre-wrap;
	word-break: break-word;
	font-family: ui-monospace, Consolas, monospace;
	font-size: 0.82rem;
}
.command-bar,
.inline-form {
	display: flex;
	align-items: end;
	gap: 0.65rem;
}
.command-bar {
	background: var(--color-button-bg);
	padding: 0.35rem 0.4rem 0.35rem 0.8rem;
	border: 1px solid var(--surface-5);
	border-radius: 0.65rem;
	margin-top: 0.75rem;
}
.command-bar span {
	color: var(--color-brand);
	font-size: 1.3rem;
}
.command-bar input {
	border: 0;
	background: none;
	flex: 1;
	min-width: 0;
}
.metric {
	padding: 0.65rem 0;
	border-bottom: 1px solid var(--surface-5);
}
.metric span {
	color: var(--color-text-tertiary);
}
.metric strong {
	font-size: 0.95rem;
	color: var(--color-contrast);
}
.path-block {
	overflow-wrap: anywhere;
}
.hint {
	font-size: 0.85rem;
	color: var(--color-text-tertiary);
}
.list-row {
	padding: 1rem 0;
	border-top: 1px solid var(--surface-5);
}
.list-row div {
	display: grid;
	gap: 0.25rem;
}
.list-row small {
	color: var(--color-text-tertiary);
}
.file-path {
	background: var(--color-button-bg);
	border-radius: 0.5rem;
	padding: 0.7rem;
	margin-bottom: 0.5rem;
}
.file-path span {
	margin-left: 0.7rem;
	color: var(--color-text-tertiary);
}
.file-row {
	border-bottom: 1px solid var(--surface-5);
	padding: 0.5rem;
}
.file-row button {
	background: none;
	border: 0;
	color: var(--color-contrast);
	cursor: pointer;
	text-align: left;
}
.file-row button span {
	color: var(--color-brand);
	margin-right: 0.7rem;
}
.file-row small {
	color: var(--color-text-tertiary);
}
.editor {
	margin-top: 1.5rem;
}
.editor textarea {
	width: 100%;
	min-height: 20rem;
	box-sizing: border-box;
	font-family: ui-monospace, Consolas, monospace;
}
.form-grid {
	display: grid;
	grid-template-columns: repeat(2, minmax(0, 1fr));
	gap: 1rem;
	margin: 1rem 0;
}
.field {
	display: grid;
	gap: 0.4rem;
	color: var(--color-text-default);
	font-size: 0.85rem;
	font-weight: 700;
}
.field.full {
	grid-column: 1/-1;
}
.local-shell input,
.local-shell select,
.local-shell textarea {
	background: var(--color-button-bg);
	border: 1px solid var(--surface-5);
	border-radius: 0.55rem;
	color: var(--color-text-primary);
	padding: 0.68rem 0.78rem;
	min-width: 0;
}
.local-shell input:focus-visible,
.local-shell select:focus-visible,
.local-shell textarea:focus-visible {
	outline: 2px solid var(--color-brand);
	outline-offset: 1px;
}
.settings-stack {
	max-width: 900px;
}
.settings-general,
.creation-choices,
.pack-browser {
	display: flex;
	flex-direction: column;
	gap: 1rem;
}
.danger {
	border-color: var(--color-red-highlight);
}
.modal-mask {
	position: fixed;
	inset: 3rem 0 0;
	z-index: 100;
	display: flex;
	align-items: flex-start;
	justify-content: center;
	overflow-y: auto;
	padding: 1rem;
	background: rgba(0, 0, 0, 0.6);
	backdrop-filter: blur(5px);
}
.create-modal {
	width: min(620px, calc(100vw - 3rem));
	max-height: calc(100vh - 8rem);
	overflow: auto;
	border: 1px solid var(--surface-5);
	border-radius: 1rem;
	background: var(--color-raised-bg);
	box-shadow: 0 25px 80px rgba(0, 0, 0, 0.45);
}
.create-modal.pack-modal {
	width: min(940px, calc(100vw - 3rem));
}
.modal-head {
	padding: 1.5rem;
	border-bottom: 1px solid var(--surface-5);
}
.modal-head h2 {
	margin: 0.5rem 0 0;
}
.steps {
	color: var(--color-text-tertiary);
	letter-spacing: 0;
	font-size: 0.8rem;
}
.modal-body {
	padding: 1.5rem;
}
.modal-body > .field,
.modal-body > .chips {
	margin-bottom: 1rem;
}
.modal-actions {
	justify-content: flex-end;
	margin-top: 1.5rem;
}
.creation-choice {
	display: flex;
	align-items: center;
	gap: 1rem;
	width: 100%;
	text-align: left;
	padding: 1rem;
	border-radius: 1.1rem;
	color: var(--color-contrast);
	cursor: pointer;
}
.creation-choice > span:nth-child(2) {
	flex: 1;
}
.creation-choice strong,
.creation-choice small {
	display: block;
}
.creation-choice small {
	color: var(--color-text-tertiary);
	margin-top: 0.3rem;
	line-height: 1.4;
}
.choice-icon {
	display: grid;
	place-items: center;
	width: 3.5rem;
	height: 3.5rem;
	flex-shrink: 0;
	border: 1px solid var(--surface-5);
	border-radius: 1rem;
	color: var(--color-text-tertiary);
	font-size: 1.5rem;
}
.choice-icon svg {
	width: 1.75rem;
	height: 1.75rem;
}
.choice-divider {
	display: flex;
	align-items: center;
	gap: 1rem;
	color: var(--color-text-tertiary);
	font-size: 0.8rem;
}
.choice-divider::before,
.choice-divider::after {
	content: '';
	height: 1px;
	background: var(--surface-5);
	flex: 1;
}
.pack-search {
	display: flex;
	align-items: center;
	gap: 0.7rem;
	border: 1px solid var(--surface-5);
	border-radius: 0.8rem;
	background: var(--color-button-bg);
	padding: 0.6rem 0.8rem;
}
.pack-search:focus-within {
	outline: 2px solid var(--color-brand);
}
.pack-search input {
	min-width: 0;
	flex: 1;
	border: 0;
	outline: 0;
	background: transparent;
	color: inherit;
}
.pack-search svg,
.path-picker svg {
	width: 1.2rem;
	height: 1.2rem;
}
.pack-result {
	display: flex;
	align-items: center;
	gap: 1rem;
	padding: 1rem;
	margin-top: 0.8rem;
	border-radius: 1rem;
}
.pack-result > img,
.pack-placeholder {
	width: 4.5rem;
	height: 4.5rem;
	border-radius: 0.9rem;
	object-fit: cover;
	flex-shrink: 0;
}
.pack-result > div {
	flex: 1;
	min-width: 0;
}
.pack-result h3 {
	margin: 0 0 0.4rem;
	font-size: 1.05rem;
}
.pack-result h3 small {
	font-weight: 400;
	color: var(--color-text-tertiary);
}
.pack-result p {
	margin: 0;
	color: var(--color-text-tertiary);
	display: -webkit-box;
	-webkit-line-clamp: 2;
	-webkit-box-orient: vertical;
	overflow: hidden;
}
.pack-tags {
	display: flex;
	flex-wrap: wrap;
	gap: 0.5rem;
	margin-top: 0.5rem;
	font-size: 0.75rem;
	color: var(--color-text-tertiary);
}
.pack-tags span {
	background: var(--surface-4);
	border-radius: 1rem;
	padding: 0.2rem 0.5rem;
}
.creation-banner {
	display: flex;
	align-items: center;
	gap: 1rem;
	margin-bottom: 1rem;
}
.creation-banner img {
	width: 4rem;
	height: 4rem;
	border-radius: 1rem;
}
.path-picker {
	display: flex;
	gap: 0.5rem;
}
.path-picker input {
	flex: 1;
	min-width: 0;
}
.toggle-field {
	display: flex;
	align-items: center;
	justify-content: space-between;
	padding: 0.8rem;
	border-radius: 0.7rem;
	gap: 1rem;
}
.toggle-field input {
	accent-color: var(--color-brand);
	width: 1.2rem;
	height: 1.2rem;
}
.plan-grid {
	display: grid;
	grid-template-columns: repeat(3, 1fr);
	gap: 1rem;
	margin: 1.5rem 0;
}
.plan-card {
	text-align: left;
	display: grid;
	gap: 0.65rem;
	align-content: start;
	min-height: 180px;
	color: var(--color-contrast);
	border-width: 2px;
	border-radius: 0.8rem;
	padding: 1.15rem;
	cursor: pointer;
}
.plan-card.selected {
	border-color: var(--color-brand);
	background: var(--color-brand-highlight);
}
.plan-card strong {
	font-size: 1.3rem;
}
.plan-card b {
	font-size: 1.7rem;
}
.plan-card b small,
.plan-card span {
	font-size: 0.85rem;
	font-weight: 400;
	color: var(--color-text-tertiary);
}
.plan-card em {
	align-self: end;
	font-style: normal;
	text-align: center;
	border-radius: 0.5rem;
	padding: 0.45rem;
	background: var(--surface-5);
	color: var(--color-contrast);
	font-size: 0.85rem;
	font-weight: 700;
}
.plan-card.selected em {
	background: var(--color-brand);
	color: var(--color-accent-contrast);
}
@media (max-width: 850px) {
	.panel-layout,
	.plan-grid,
	.form-grid {
		grid-template-columns: 1fr;
	}
	.plan-card {
		min-height: 0;
	}
	.page-head {
		align-items: flex-start;
		flex-direction: column;
	}
	.head-actions {
		width: 100%;
	}
	.server-hero {
		align-items: flex-start;
	}
	.hero-actions {
		flex-wrap: wrap;
		justify-content: flex-start;
	}
	.field.full {
		grid-column: auto;
	}
}
</style>
