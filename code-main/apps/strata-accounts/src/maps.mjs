// Intermediario de CurseForge para la pestaña «Mapas» de Strata.
//
// La clave de la API (CURSEFORGE_API_KEY) vive solo aquí, como secreto del Worker: la app es de código abierto y no puede
// llevarla. El Worker solo ofrece lo mínimo para mapas de Minecraft (gameId 432, classId 17) y rechaza cualquier otro
// juego o sección, para que nadie pueda usar la clave con otro fin. No guarda nada en D1 ni registra datos de usuarios, y
// los archivos de los mapas NO pasan por aquí: la app los descarga directamente del CDN de CurseForge.
//
// Antes de activar la caché lee el README: las condiciones de la API de terceros de CurseForge (sección 3.1) prohíben
// «guardar o cachear datos obtenidos a través de la API», así que por defecto está desactivada.

export const GAME_ID = 432 // Minecraft
export const CLASS_ID = 17 // Worlds (mapas)
const API = 'https://api.curseforge.com'
const PAGE_SIZE = 20
const FILES_PAGE_SIZE = 50 // máximo que admite la API
const MAX_RESULTS = 10_000 // «index + pageSize <= 10.000», según la documentación

/**
 * Valores de `sortField` de CurseForge. SIN CONFIRMAR: la documentación oficial solo dice «enumeración de 1 a 12» sin dar
 * los nombres; estos números salen de clientes de terceros (2 = popularidad, 3 = última actualización, 6 = descargas
 * totales). Comprueba con una petición real que el orden es el esperado antes de publicar.
 */
export const SORT_FIELDS = { popular: 2, updated: 3, downloads: 6 }

// `hashes[].algo`: 1 = Sha1, 2 = Md5 (documentación oficial).
const HASH_ALGOS = { 1: 'sha1', 2: 'md5' }

/** El CDN del que salen las descargas (los dominios que usa CurseForge para sus archivos). */
export const CDN_SUFFIX = 'forgecdn.net'
export function isCdnUrl(value) {
	try {
		const url = new URL(value)
		return url.protocol === 'https:' && (url.hostname === CDN_SUFFIX || url.hostname.endsWith(`.${CDN_SUFFIX}`))
	} catch {
		return false
	}
}

class Failure extends Error {
	constructor(status, message, code) {
		super(message)
		this.status = status
		this.code = code
	}
}
const fail = (status, message, code) => {
	throw new Failure(status, message, code)
}

const json = (body, status = 200, headers = {}) =>
	new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json; charset=utf-8', 'cache-control': 'no-store', ...headers } })

/** Limitador en memoria, por aislado del Worker: frena abusos sencillos sin guardar nada. */
export function createLimiter(max, windowMs, now = Date.now) {
	const hits = new Map()
	return (key) => {
		const t = now()
		if (hits.size > 5000) for (const [k, v] of hits) if (v.until <= t) hits.delete(k)
		const entry = hits.get(key)
		if (!entry || entry.until <= t) {
			hits.set(key, { count: 1, until: t + windowMs })
			return true
		}
		entry.count += 1
		return entry.count <= max
	}
}

const integer = (value, name, { min = 0, max = 2_147_483_647 } = {}) => {
	if (!/^\d{1,10}$/.test(value)) fail(400, `El parámetro «${name}» no es válido.`, 'bad-request')
	const number = Number(value)
	if (number < min || number > max) fail(400, `El parámetro «${name}» no es válido.`, 'bad-request')
	return number
}

/** Rechaza cualquier intento de apuntar a otro juego o sección distintos de Minecraft / Mundos. */
function checkScope(params) {
	for (const [name, value] of params) {
		const key = name.toLowerCase()
		if (key === 'gameid' && value !== String(GAME_ID)) fail(400, 'Solo se admiten mapas de Minecraft.', 'bad-request')
		if (key === 'classid' && value !== String(CLASS_ID)) fail(400, 'Solo se admiten mapas de Minecraft.', 'bad-request')
	}
}

function searchQuery(params) {
	const query = new URLSearchParams({ gameId: String(GAME_ID), classId: String(CLASS_ID), pageSize: String(PAGE_SIZE) })
	const q = (params.get('q') ?? '').trim()
	if (q.length > 60) fail(400, 'El texto de búsqueda es demasiado largo.', 'bad-request')
	if (q) query.set('searchFilter', q)
	const version = params.get('version')
	if (version) {
		if (!/^[0-9a-z.\-]{1,24}$/i.test(version)) fail(400, 'La versión de Minecraft no es válida.', 'bad-request')
		query.set('gameVersion', version)
	}
	const category = params.get('category')
	if (category) query.set('categoryId', String(integer(category, 'category', { min: 1 })))
	const sort = params.get('sort') ?? 'popular'
	if (!(sort in SORT_FIELDS)) fail(400, 'El orden no es válido.', 'bad-request')
	query.set('sortField', String(SORT_FIELDS[sort]))
	query.set('sortOrder', 'desc')
	const page = params.has('page') ? integer(params.get('page'), 'page', { min: 0, max: Math.floor(MAX_RESULTS / PAGE_SIZE) - 1 }) : 0
	query.set('index', String(page * PAGE_SIZE))
	return { query, page }
}

const names = (list) => (Array.isArray(list) ? list.map((item) => item?.name).filter((name) => typeof name === 'string') : [])

/** Versiones de Minecraft (solo números de versión) de una lista de archivos. */
function versionsOf(files) {
	const found = new Set()
	for (const file of files ?? []) for (const version of file?.gameVersions ?? []) if (/^\d+\.\d+/.test(version)) found.add(version)
	return [...found]
}

function mapSummary(mod) {
	return {
		id: mod.id,
		name: mod.name,
		slug: mod.slug,
		summary: mod.summary ?? '',
		logo: mod.logo?.thumbnailUrl ?? mod.logo?.url ?? null,
		authors: names(mod.authors),
		downloads: mod.downloadCount ?? 0,
		updated: mod.dateModified ?? null,
		categories: (mod.categories ?? []).map((c) => ({ id: c.id, name: c.name })),
		versions: versionsOf(mod.latestFiles),
		url: mod.links?.websiteUrl ?? null,
	}
}

function mapDetail(mod, description) {
	return {
		...mapSummary(mod),
		created: mod.dateCreated ?? null,
		logoFull: mod.logo?.url ?? null,
		screenshots: (mod.screenshots ?? []).map((s) => ({ title: s.title ?? '', thumbnail: s.thumbnailUrl ?? null, url: s.url ?? null })).filter((s) => s.url),
		// Si el autor no permite descargas desde apps de terceros, CurseForge no da URL de descarga.
		distributable: mod.allowModDistribution !== false,
		html: description,
	}
}

function mapFile(file) {
	return {
		id: file.id,
		name: file.displayName ?? file.fileName,
		fileName: file.fileName,
		size: file.fileLength ?? 0,
		date: file.fileDate ?? null,
		releaseType: file.releaseType ?? 1,
		gameVersions: (file.gameVersions ?? []).filter((v) => /^\d+\.\d+/.test(v)),
		available: file.isAvailable !== false,
		// Solo se entrega la URL si es del CDN de CurseForge; sin ella la app manda al usuario a la web.
		downloadUrl: isCdnUrl(file.downloadUrl) ? file.downloadUrl : null,
		hashes: (file.hashes ?? []).map((h) => ({ algo: HASH_ALGOS[h.algo], value: h.value })).filter((h) => h.algo && typeof h.value === 'string'),
	}
}

/**
 * @param {object} options
 * @param {string} [options.apiKey] clave de CurseForge (secreto del Worker)
 * @param {typeof fetch} [options.fetchImpl]
 * @param {{ match(request: Request): Promise<Response|undefined>, put(request: Request, response: Response): Promise<void> }} [options.cache]
 * @param {{ search?: number, detail?: number }} [options.ttl] segundos de caché; 0 (por defecto) la desactiva
 * @param {number} [options.perMinute] peticiones por minuto y dirección IP
 * @param {() => number} [options.now]
 */
export function createMapsHandler({ apiKey, fetchImpl = fetch, cache, ttl = {}, perMinute = 60, now = Date.now } = {}) {
	const allow = createLimiter(perMinute, 60_000, now)

	async function curseforge(path, query) {
		if (!apiKey) fail(503, 'El servicio de mapas no está configurado todavía.', 'not-configured')
		const url = `${API}${path}${query ? `?${query}` : ''}`
		let response
		try {
			response = await fetchImpl(url, { headers: { 'x-api-key': apiKey, accept: 'application/json' } })
		} catch {
			fail(502, 'No se pudo contactar con CurseForge.', 'upstream')
		}
		if (response.status === 404) fail(404, 'No se ha encontrado ese mapa.', 'not-found')
		if (!response.ok) fail(502, 'CurseForge no responde ahora mismo. Inténtalo más tarde.', 'upstream')
		return response.json()
	}

	/** El mapa solo vale si es de Minecraft y de la sección Mundos: así no se puede leer nada más con la clave. */
	async function getMap(id) {
		const { data } = await curseforge(`/v1/mods/${id}`)
		if (!data || data.gameId !== GAME_ID || data.classId !== CLASS_ID) fail(404, 'No se ha encontrado ese mapa.', 'not-found')
		return data
	}

	async function cached(key, seconds, produce) {
		if (!cache || !(seconds > 0)) return produce()
		const request = new Request(`https://strata-maps.cache/${key}`)
		const hit = await cache.match(request)
		if (hit) return hit.json()
		const value = await produce()
		await cache.put(request, new Response(JSON.stringify(value), { headers: { 'content-type': 'application/json', 'cache-control': `public, max-age=${seconds}` } }))
		return value
	}

	async function route(url) {
		const params = url.searchParams
		checkScope(params)
		const path = url.pathname.replace(/\/+$/, '')
		let match
		if (path === '/maps/search') {
			const { query, page } = searchQuery(params)
			return cached(`search?${query}`, ttl.search, async () => {
				const result = await curseforge('/v1/mods/search', query)
				return { page, pageSize: PAGE_SIZE, total: Math.min(result.pagination?.totalCount ?? 0, MAX_RESULTS), maps: (result.data ?? []).map(mapSummary) }
			})
		}
		if (path === '/maps/categories') {
			return cached('categories', ttl.detail, async () => {
				const result = await curseforge('/v1/categories', new URLSearchParams({ gameId: String(GAME_ID), classId: String(CLASS_ID) }))
				return { categories: (result.data ?? []).filter((c) => c.classId === CLASS_ID && c.isClass !== true).map((c) => ({ id: c.id, name: c.name, slug: c.slug })) }
			})
		}
		if ((match = path.match(/^\/maps\/mod\/(\d{1,10})$/))) {
			const id = integer(match[1], 'id', { min: 1 })
			return cached(`mod/${id}`, ttl.detail, async () => {
				const mod = await getMap(id)
				const { data } = await curseforge(`/v1/mods/${id}/description`)
				return mapDetail(mod, typeof data === 'string' ? data : '')
			})
		}
		if ((match = path.match(/^\/maps\/mod\/(\d{1,10})\/files$/))) {
			const id = integer(match[1], 'id', { min: 1 })
			const query = new URLSearchParams({ pageSize: String(FILES_PAGE_SIZE) })
			const version = params.get('version')
			if (version) {
				if (!/^[0-9a-z.\-]{1,24}$/i.test(version)) fail(400, 'La versión de Minecraft no es válida.', 'bad-request')
				query.set('gameVersion', version)
			}
			query.set('index', String(params.has('page') ? integer(params.get('page'), 'page', { min: 0, max: 200 }) * FILES_PAGE_SIZE : 0))
			return cached(`mod/${id}/files?${query}`, ttl.detail, async () => {
				await getMap(id)
				const result = await curseforge(`/v1/mods/${id}/files`, query)
				return { total: result.pagination?.totalCount ?? 0, files: (result.data ?? []).map(mapFile) }
			})
		}
		return fail(404, 'No existe.', 'not-found')
	}

	/** Devuelve `null` si la ruta no es de mapas (para que otro módulo la atienda). */
	return async function handle(request, { ip = 'unknown' } = {}) {
		const url = new URL(request.url)
		if (!url.pathname.startsWith('/maps/')) return null
		try {
			if (request.method !== 'GET') fail(405, 'Método no permitido.', 'bad-request')
			if (!allow(ip)) fail(429, 'Estás pidiendo demasiado deprisa. Espera un momento.', 'rate-limit')
			return json(await route(url))
		} catch (error) {
			if (error instanceof Failure) return json({ error: error.message, code: error.code }, error.status)
			return json({ error: 'Error interno.', code: 'internal' }, 500)
		}
	}
}
