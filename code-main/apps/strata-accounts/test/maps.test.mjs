import test from 'node:test'
import assert from 'node:assert/strict'
import { CDN_SUFFIX, CLASS_ID, GAME_ID, SORT_FIELDS, createLimiter, createMapsHandler, isCdnUrl } from '../src/maps.mjs'

const KEY = 'clave-secreta-de-prueba'

/** CurseForge de mentira: apunta las peticiones y responde con lo que se le configure. */
function fakeCurseForge(routes = {}) {
	const calls = []
	const fetchImpl = async (url, init) => {
		calls.push({ url: String(url), headers: init?.headers ?? {} })
		const { pathname } = new URL(url)
		const answer = routes[pathname]
		if (!answer) return new Response('{}', { status: 404 })
		return new Response(JSON.stringify(typeof answer === 'function' ? answer(new URL(url)) : answer), { status: 200 })
	}
	return { fetchImpl, calls }
}

const world = (extra = {}) => ({ id: 10, gameId: GAME_ID, classId: CLASS_ID, name: 'Mapa', slug: 'mapa', summary: 'Un mapa', downloadCount: 5, ...extra })
const get = (handle, path, init = {}, options) => handle(new Request(`https://maps.test${path}`, init), options)

function setup(routes, options = {}) {
	const cf = fakeCurseForge(routes)
	const handle = createMapsHandler({ apiKey: KEY, fetchImpl: cf.fetchImpl, ...options })
	return { cf, handle }
}

test('solo atiende rutas /maps/ y deja pasar el resto', async () => {
	const { handle } = setup({})
	assert.equal(await get(handle, '/accounts/me'), null)
	assert.equal((await get(handle, '/maps/nada')).status, 404)
})

test('la búsqueda va siempre limitada a Minecraft y a la sección de mundos, con la clave en la cabecera', async () => {
	const { cf, handle } = setup({ '/v1/mods/search': { data: [world({ logo: { thumbnailUrl: 'https://media.forgecdn.net/a.png' } })], pagination: { totalCount: 1 } } })
	const response = await get(handle, '/maps/search?q=castillo&version=1.20.1&category=5&sort=downloads&page=2')
	assert.equal(response.status, 200)
	const sent = new URL(cf.calls[0].url)
	assert.equal(sent.searchParams.get('gameId'), String(GAME_ID))
	assert.equal(sent.searchParams.get('classId'), String(CLASS_ID))
	assert.equal(sent.searchParams.get('searchFilter'), 'castillo')
	assert.equal(sent.searchParams.get('gameVersion'), '1.20.1')
	assert.equal(sent.searchParams.get('categoryId'), '5')
	assert.equal(sent.searchParams.get('sortField'), String(SORT_FIELDS.downloads))
	assert.equal(sent.searchParams.get('index'), '40')
	assert.equal(cf.calls[0].headers['x-api-key'], KEY)
	const body = await response.json()
	assert.equal(body.maps[0].name, 'Mapa')
	assert.equal(JSON.stringify(body).includes(KEY), false, 'la clave nunca sale en una respuesta')
})

test('rechaza otros gameId o classId aunque se los pidan', async () => {
	const { cf, handle } = setup({ '/v1/mods/search': { data: [], pagination: {} } })
	for (const bad of ['gameId=1', 'classId=6', 'gameid=70667', 'CLASSID=6']) {
		const response = await get(handle, `/maps/search?${bad}`)
		assert.equal(response.status, 400, bad)
	}
	assert.equal(cf.calls.length, 0, 'no se llegó a llamar a CurseForge')
	assert.equal((await get(handle, `/maps/search?gameId=${GAME_ID}&classId=${CLASS_ID}`)).status, 200, 'los valores permitidos sí valen')
})

test('no deja leer proyectos de otros juegos o secciones por id', async () => {
	const { cf, handle } = setup({
		'/v1/mods/1': { data: world({ id: 1, gameId: 70667 }) }, // otro juego
		'/v1/mods/2': { data: world({ id: 2, classId: 6 }) }, // mods, no mundos
		'/v1/mods/2/description': { data: '<p>secreto</p>' },
		'/v1/mods/2/files': { data: [], pagination: {} },
	})
	for (const path of ['/maps/mod/1', '/maps/mod/2', '/maps/mod/2/files']) {
		const response = await get(handle, path)
		assert.equal(response.status, 404, path)
	}
	assert.equal(cf.calls.some((call) => call.url.includes('/description') || call.url.includes('/files')), false)
})

test('valida y limita los parámetros', async () => {
	const { cf, handle } = setup({ '/v1/mods/search': { data: [], pagination: {} } })
	const bad = [
		`/maps/search?q=${'a'.repeat(61)}`,
		'/maps/search?version=1.20.1%26gameId=1',
		'/maps/search?version=../../x',
		'/maps/search?category=abc',
		'/maps/search?category=-3',
		'/maps/search?sort=rating',
		'/maps/search?page=-1',
		'/maps/search?page=500',
		'/maps/mod/abc',
		'/maps/mod/99999999999',
	]
	for (const path of bad) assert.ok([400, 404].includes((await get(handle, path)).status), path)
	assert.equal(cf.calls.length, 0)
	assert.equal((await get(handle, '/maps/search', { method: 'POST' })).status, 405)
})

test('la ficha trae la descripción HTML y la distribución permitida', async () => {
	const { handle } = setup({
		'/v1/mods/10': { data: world({ allowModDistribution: false, screenshots: [{ title: 'a', thumbnailUrl: 'https://media.forgecdn.net/t.png', url: 'https://media.forgecdn.net/u.png' }, { title: 'sin url' }] }) },
		'/v1/mods/10/description': { data: '<p>Hola</p>' },
	})
	const body = await (await get(handle, '/maps/mod/10')).json()
	assert.equal(body.html, '<p>Hola</p>')
	assert.equal(body.distributable, false)
	assert.deepEqual(body.screenshots.map((s) => s.url), ['https://media.forgecdn.net/u.png'])
})

test('los archivos solo exponen URLs del CDN, con sus hashes y versiones', async () => {
	const files = [
		{ id: 1, displayName: 'Mapa v1', fileName: 'mapa.zip', fileLength: 100, fileDate: '2026-01-01T00:00:00Z', gameVersions: ['1.20.1', 'Java 17', 'Forge'], hashes: [{ algo: 1, value: 'aa' }, { algo: 2, value: 'bb' }, { algo: 9, value: 'cc' }], downloadUrl: 'https://edge.forgecdn.net/files/1/2/mapa.zip', isAvailable: true },
		{ id: 2, fileName: 'otro.zip', gameVersions: ['1.19'], hashes: [], downloadUrl: 'https://evil.example.com/otro.zip' },
		{ id: 3, fileName: 'sin-url.zip', gameVersions: ['1.18'], hashes: [], downloadUrl: null },
	]
	const { cf, handle } = setup({ '/v1/mods/10': { data: world() }, '/v1/mods/10/files': { data: files, pagination: { totalCount: 3 } } })
	const body = await (await get(handle, '/maps/mod/10/files?version=1.20.1')).json()
	assert.equal(new URL(cf.calls[1].url).searchParams.get('gameVersion'), '1.20.1')
	assert.deepEqual(body.files[0].hashes, [{ algo: 'sha1', value: 'aa' }, { algo: 'md5', value: 'bb' }])
	assert.deepEqual(body.files[0].gameVersions, ['1.20.1'])
	assert.equal(body.files[0].downloadUrl, 'https://edge.forgecdn.net/files/1/2/mapa.zip')
	assert.equal(body.files[1].downloadUrl, null, 'una URL de otro dominio no se entrega')
	assert.equal(body.files[2].downloadUrl, null)
})

test('isCdnUrl solo acepta https del dominio del CDN', () => {
	assert.equal(isCdnUrl(`https://edge.${CDN_SUFFIX}/a.zip`), true)
	assert.equal(isCdnUrl(`https://${CDN_SUFFIX}/a.zip`), true)
	assert.equal(isCdnUrl(`http://edge.${CDN_SUFFIX}/a.zip`), false)
	assert.equal(isCdnUrl(`https://evil-${CDN_SUFFIX}/a.zip`), false)
	assert.equal(isCdnUrl(`https://${CDN_SUFFIX}.evil.com/a.zip`), false)
	assert.equal(isCdnUrl('no es una url'), false)
	assert.equal(isCdnUrl(null), false)
})

test('sin clave configurada responde 503 sin llamar a nadie', async () => {
	const cf = fakeCurseForge({})
	const handle = createMapsHandler({ fetchImpl: cf.fetchImpl })
	const response = await get(handle, '/maps/search')
	assert.equal(response.status, 503)
	assert.equal((await response.json()).code, 'not-configured')
	assert.equal(cf.calls.length, 0)
})

test('los fallos de CurseForge no filtran detalles', async () => {
	const handle = createMapsHandler({ apiKey: KEY, fetchImpl: async () => new Response('clave inválida ' + KEY, { status: 403 }) })
	const response = await get(handle, '/maps/search')
	assert.equal(response.status, 502)
	assert.equal((await response.text()).includes(KEY), false)
})

/** Caché de mentira con la forma de la Cache API. */
function memoryCache() {
	const store = new Map()
	return {
		store,
		async match(request) {
			const hit = store.get(request.url)
			return hit?.clone()
		},
		async put(request, response) {
			store.set(request.url, response)
		},
	}
}

test('con caché activada la segunda búsqueda igual no vuelve a llamar a CurseForge; por defecto está desactivada', async () => {
	const routes = { '/v1/mods/search': { data: [world()], pagination: { totalCount: 1 } } }
	const on = setup(routes, { cache: memoryCache(), ttl: { search: 600 } })
	await get(on.handle, '/maps/search?q=a')
	await get(on.handle, '/maps/search?q=a')
	await get(on.handle, '/maps/search?q=b')
	assert.equal(on.cf.calls.length, 2, 'q=a una sola vez y q=b otra')

	const off = setup(routes, { cache: memoryCache() })
	await get(off.handle, '/maps/search?q=a')
	await get(off.handle, '/maps/search?q=a')
	assert.equal(off.cf.calls.length, 2, 'sin ttl no se guarda nada')
})

test('el límite por IP corta a la que se pasa y no afecta a las demás', async () => {
	let clock = 0
	const { handle } = setup({ '/v1/mods/search': { data: [], pagination: {} } }, { perMinute: 3, now: () => clock })
	const ask = (ip) => get(handle, '/maps/search', {}, { ip }).then((r) => r.status)
	assert.deepEqual([await ask('1.1.1.1'), await ask('1.1.1.1'), await ask('1.1.1.1'), await ask('1.1.1.1')], [200, 200, 200, 429])
	assert.equal(await ask('2.2.2.2'), 200, 'otra IP no se ve afectada')
	clock += 61_000
	assert.equal(await ask('1.1.1.1'), 200, 'pasado el minuto vuelve a poder')
})

test('createLimiter limpia entradas viejas', () => {
	let clock = 0
	const allow = createLimiter(1, 1000, () => clock)
	assert.equal(allow('a'), true)
	assert.equal(allow('a'), false)
	clock = 1001
	assert.equal(allow('a'), true)
})
