import { invoke } from '@tauri-apps/api/core'

/**
 * Mapas de CurseForge. Los datos salen del Worker de Strata (que guarda la clave de la API); los archivos se descargan
 * directamente del CDN de CurseForge desde Rust. La interfaz nunca maneja rutas del disco, solo identificadores.
 */
export type MapCategory = { id: number; name: string; slug: string }

export type MapSummary = {
	id: number
	name: string
	slug: string
	summary: string
	logo: string | null
	authors: string[]
	downloads: number
	updated: string | null
	categories: { id: number; name: string }[]
	versions: string[]
	/** Página del mapa en CurseForge. */
	url: string | null
}

export type MapDetail = MapSummary & {
	created: string | null
	logoFull: string | null
	screenshots: { title: string; thumbnail: string | null; url: string }[]
	/** `false` si el autor no permite descargas desde aplicaciones de terceros. */
	distributable: boolean
	/** Descripción en HTML tal y como la manda CurseForge: hay que sanearla antes de mostrarla. */
	html: string
}

export type MapFileHash = { algo: 'sha1' | 'md5'; value: string }

export type MapFile = {
	id: number
	name: string
	fileName: string
	size: number
	date: string | null
	releaseType: number
	gameVersions: string[]
	available: boolean
	/** `null` si el autor no permite la descarga desde aplicaciones de terceros (o la URL no es del CDN de CurseForge). */
	downloadUrl: string | null
	hashes: MapFileHash[]
}

export type SearchMapsParams = {
	q?: string
	version?: string
	category?: number | null
	sort?: MapSort
	page?: number
}
export type MapSort = 'popular' | 'updated' | 'downloads'
export type MapSearchResult = { page: number; pageSize: number; total: number; maps: MapSummary[] }

/** Un mundo que trae el ZIP (ver `WorldInZip` en `world_import.rs`). */
export type WorldInZip = {
	path: string
	folder: string
	level_name: string | null
	game_version: string | null
	data_version: number | null
	size: number
	files: number
}
export type ZipInspection = { worlds: WorldInZip[]; blocked_files: string[]; total_size: number; entries: number }
export type Prepared = { token: string; inspection: ZipInspection }
export type InstallReport = {
	installed: { folder: string; level_name: string | null; game_version: string | null }[]
	blocked_files: string[]
}

export const searchMaps = (params: SearchMapsParams) =>
	invoke<MapSearchResult>('maps_search', {
		q: params.q || null,
		version: params.version || null,
		category: params.category ?? null,
		sort: params.sort ?? null,
		page: params.page ?? null,
	})
export const getMapCategories = () => invoke<{ categories: MapCategory[] }>('maps_categories')
export const getMap = (id: number) => invoke<MapDetail>('maps_get', { id })
export const getMapFiles = (id: number, version?: string, page?: number) =>
	invoke<{ total: number; files: MapFile[] }>('maps_files', { id, version: version || null, page: page ?? null })

/** Descarga (solo del CDN de CurseForge), verifica y analiza el archivo. No instala nada. */
export const prepareMapInstall = (instanceId: string, file: MapFile) =>
	invoke<Prepared>('maps_prepare', {
		instanceId,
		file: { url: file.downloadUrl, size: file.size, hashes: file.hashes },
	})
export const installPreparedMap = (instanceId: string, token: string, selected: string[], mapId: number) =>
	invoke<InstallReport>('maps_install', { instanceId, token, selected, mapId })
export const discardPreparedMap = (token: string) => invoke<void>('maps_discard', { token })

/** Mensaje legible de un error de Tauri (texto u objeto `{ message }`). */
export function mapErrorMessage(cause: unknown): string {
	if (typeof cause === 'string') return cause
	if (cause instanceof Error) return cause.message
	if (cause && typeof cause === 'object' && 'message' in cause) return String((cause as { message: unknown }).message)
	return ''
}

// ───────────────────────── Versiones ─────────────────────────

/**
 * Compara la versión de un mapa con la de una instancia usando el orden de las versiones de Minecraft (la lista viene de la
 * más nueva a la más antigua). `unknown` si alguna no está en la lista.
 */
export function compareVersions(mapVersion: string, instanceVersion: string, ordered: string[]): 'newer' | 'older' | 'same' | 'unknown' {
	if (mapVersion === instanceVersion) return 'same'
	const mapIndex = ordered.indexOf(mapVersion)
	const instanceIndex = ordered.indexOf(instanceVersion)
	if (mapIndex < 0 || instanceIndex < 0) return 'unknown'
	return mapIndex < instanceIndex ? 'newer' : 'older'
}

/** La versión más nueva de un archivo (un archivo puede valer para varias). */
export function newestVersion(versions: string[], ordered: string[]): string | null {
	const known = versions.filter((version) => ordered.includes(version)).sort((a, b) => ordered.indexOf(a) - ordered.indexOf(b))
	return known[0] ?? versions[0] ?? null
}

/** El archivo más adecuado para una versión de Minecraft: el más reciente que la admita; si no, el más reciente. */
export function bestFile(files: MapFile[], version?: string): MapFile | null {
	const usable = files.filter((file) => file.available)
	const byDate = (a: MapFile, b: MapFile) => (b.date ?? '').localeCompare(a.date ?? '')
	const matching = version ? usable.filter((file) => file.gameVersions.includes(version)).sort(byDate) : []
	return matching[0] ?? [...usable].sort(byDate)[0] ?? null
}

/** Quick Play de un mundo individual existe desde la 1.20. */
export function supportsQuickPlayWorld(version: string, ordered: string[]): boolean {
	const index = ordered.indexOf(version)
	const first = ordered.indexOf('1.20')
	return index >= 0 && first >= 0 && index <= first
}

export function formatBytes(bytes: number): string {
	if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`
	if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
	return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`
}
