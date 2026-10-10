import test from 'node:test'
import assert from 'node:assert/strict'
import { createHandler, requestMessage, sha256Hex } from '../src/core.mjs'
import { createD1 } from './d1-shim.mjs'

const toHex = (bytes) => [...new Uint8Array(bytes)].map((b) => b.toString(16).padStart(2, '0')).join('')

/** Un launcher de prueba: una clave Ed25519 que firma cada petición, como hace la app. */
async function identity() {
	const pair = await crypto.subtle.generateKey('Ed25519', true, ['sign', 'verify'])
	const code = toHex(await crypto.subtle.exportKey('raw', pair.publicKey))
	return {
		code,
		async sign(message) {
			return toHex(await crypto.subtle.sign('Ed25519', pair.privateKey, new TextEncoder().encode(message)))
		},
	}
}

function setup() {
	const db = createD1()
	const handle = createHandler(db)
	let ip = 1
	let calls = 0
	/** Llama al servicio como `who` (o sin firma si `who` es null). `ts` y `tamper` permiten falsificar. */
	async function call(who, method, path, body, { expected = 200, ts = Date.now(), tamper, ipAddress = `10.2.${++calls % 250}.${Math.floor(calls / 250) + 1}`, headers = {} } = {}) {
		const bodyText = body === undefined ? '' : JSON.stringify(body)
		const init = { method, headers: { ...headers } }
		if (bodyText) init.body = tamper?.body ?? bodyText
		if (who) {
			const signed = requestMessage(method, tamper?.path ?? path, ts, await sha256Hex(bodyText))
			init.headers.Authorization = `Strata ${who.code}.${ts}.${await who.sign(signed)}`
		}
		const response = await handle(new Request(`https://cuentas.test${path}`, init), { ip: ipAddress })
		const json = await response.json()
		assert.equal(response.status, expected, `${method} ${path} -> ${response.status} ${JSON.stringify(json)}`)
		return json
	}
	const register = async (name) => {
		const who = await identity()
		await call(who, 'POST', '/register', { username: name }, { ipAddress: `10.1.${ip++}.1` })
		return who
	}
	return { db, call, register, identity }
}

test('registro: nombres únicos sin distinguir mayúsculas, reservados y una cuenta por launcher', async () => {
	const { call, identity, register } = setup()
	const alice = await identity()
	const created = await call(alice, 'POST', '/register', { username: 'Alice_QA' })
	assert.equal(created.username, 'Alice_QA')
	assert.equal(created.id, alice.code, 'el id de la cuenta es la clave pública')
	assert.deepEqual(created.settings, { friend_privacy: 'everyone', invite_privacy: 'friends', searchable: true })

	await call(alice, 'POST', '/register', { username: 'OtroNombre' }, { expected: 409 })
	const bob = await identity()
	await call(bob, 'POST', '/register', { username: 'alice_qa' }, { expected: 409 })
	await call(bob, 'POST', '/register', { username: 'ALICE_QA' }, { expected: 409 })
	for (const reserved of ['admin', 'Strata', 'MODRINTH', 'Minecraft']) await call(bob, 'POST', '/register', { username: reserved }, { expected: 400 })
	await call(bob, 'POST', '/register', { username: 'ab' }, { expected: 400 })
	await call(bob, 'POST', '/register', { username: 'con espacios' }, { expected: 400 })
	await call(bob, 'POST', '/register', {}, { expected: 400 })
	await call(bob, 'GET', '/me', undefined, { expected: 404 })
	await register('Bob_QA')
})

test('autenticación: sin firma, firma falsa, petición manipulada, hora incorrecta y origen ajeno', async () => {
	const { call, register, identity } = setup()
	const alice = await register('Alice_QA')
	await call(null, 'GET', '/me', undefined, { expected: 401 })
	await call(alice, 'GET', '/health', undefined)
	await call(null, 'GET', '/me', undefined, { headers: { Authorization: 'Strata nope' }, expected: 401 })

	// Firma de otra clave con el código de Alice.
	const mallory = await identity()
	const forged = { code: alice.code, sign: (m) => mallory.sign(m) }
	await call(forged, 'GET', '/me', undefined, { expected: 401 })

	// El mensaje firmado cubre la ruta y el cuerpo: cambiarlos invalida la firma.
	await call(alice, 'PATCH', '/me', { bio: 'hola' }, { tamper: { body: JSON.stringify({ bio: 'otra' }) }, expected: 401 })
	await call(alice, 'GET', '/me', undefined, { tamper: { path: '/sync' }, expected: 401 })

	// Hora fuera de ventana (pasada o futura): también protege de repeticiones antiguas.
	await call(alice, 'GET', '/me', undefined, { ts: Date.now() - 10 * 60_000, expected: 401 })
	const clock = await call(alice, 'GET', '/me', undefined, { ts: Date.now() + 10 * 60_000, expected: 401 })
	assert.equal(clock.code, 'clock')

	// Un navegador de otro sitio (con Origin) no puede usar el servicio.
	await call(alice, 'GET', '/me', undefined, { headers: { Origin: 'https://malo.example' }, expected: 403 })
})

test('perfil, ajustes, cambio de nombre y borrado de la cuenta', async () => {
	const { call, register, identity } = setup()
	const alice = await register('Alice_QA')
	await call(alice, 'PATCH', '/me', { bio: 'Mi perfil' })
	await call(alice, 'PATCH', '/me', { bio: 'x'.repeat(501) }, { expected: 400 })
	await call(alice, 'PATCH', '/me', { settings: { friend_privacy: 'mutual', searchable: false } })
	const me = await call(alice, 'GET', '/me')
	assert.deepEqual(me.settings, { friend_privacy: 'mutual', invite_privacy: 'friends', searchable: false })
	assert.equal(me.bio, 'Mi perfil')
	await call(alice, 'PATCH', '/me', { settings: { friend_privacy: 'cualquiera' } }, { expected: 400 })
	await call(alice, 'PATCH', '/me', { settings: { searchable: 'si' } }, { expected: 400 })

	const bob = await register('Bob_QA')
	await call(alice, 'POST', '/me/rename', { username: 'bob_qa' }, { expected: 409 })
	await call(alice, 'POST', '/me/rename', { username: 'admin' }, { expected: 400 })
	assert.equal((await call(alice, 'POST', '/me/rename', { username: 'Alicia_QA' })).username, 'Alicia_QA')
	await call(alice, 'POST', '/me/rename', { username: 'Alicia_2' }, { expected: 429 })
	assert.equal((await call(alice, 'POST', '/me/rename', { username: 'alicia_qa' })).username, 'alicia_qa', 'cambiar solo las mayúsculas no cuenta')
	await register('Alice_QA') // el nombre antiguo quedó libre

	await call(bob, 'POST', '/me/delete', {})
	await call(bob, 'GET', '/me', undefined, { expected: 404 })
	const again = await identity()
	await call(again, 'POST', '/register', { username: 'Bob_QA' }) // el nombre quedó libre tras borrar
})

test('búsqueda por nombre: prefijo, privacidad, bloqueos y relación', async () => {
	const { call, register } = setup()
	const alice = await register('Alice_QA')
	const bob = await register('Bob_QA')
	const bobby = await register('Bobby')
	const hidden = await register('Bobo_oculto')
	await call(hidden, 'PATCH', '/me', { settings: { searchable: false } })

	const names = async (who, q) => (await call(who, 'GET', `/users/search?q=${q}`)).users.map((u) => u.username)
	assert.deepEqual(await names(alice, 'bo'), ['Bobby', 'Bob_QA'], 'por prefijo (primero los más cortos), sin ocultos ni uno mismo')
	assert.deepEqual(await names(alice, 'BOB'), ['Bobby', 'Bob_QA'], 'sin distinguir mayúsculas')
	assert.deepEqual(await names(alice, 'Alice'), [], 'no se encuentra a uno mismo')
	assert.deepEqual(await names(alice, 'b'), [], 'mínimo dos caracteres')
	assert.deepEqual(await names(alice, '%25_'), [], 'los comodines no se interpretan')
	assert.deepEqual(await names(alice, 'zzz'), [])

	await call(alice, 'POST', '/friends/request', { username: 'Bob_QA' })
	assert.equal((await call(alice, 'GET', '/users/search?q=bob')).users.find((u) => u.username === 'Bob_QA').relation, 'outgoing')
	assert.equal((await call(bob, 'GET', '/users/search?q=ali')).users[0].relation, 'incoming')
	await call(alice, 'POST', '/block', { username: 'Bobby' })
	assert.deepEqual(await names(alice, 'bob'), ['Bob_QA'], 'los bloqueados no aparecen')
	assert.deepEqual(await names(bobby, 'ali'), [], 'ni aparece quien te ha bloqueado')
})

test('amistades, bloqueos y separación de usuarios', async () => {
	const { call, register } = setup()
	const alice = await register('Alice_QA')
	const bob = await register('Bob_QA')
	const eve = await register('Eve_QA')
	await call(alice, 'POST', '/friends/request', { username: 'Bob_QA' })
	assert.equal((await call(bob, 'GET', '/sync')).social.incoming[0].username, 'Alice_QA')
	assert.equal((await call(eve, 'GET', '/sync')).social.incoming.length, 0)
	await call(eve, 'POST', '/friends/accept', { username: 'Alice_QA' }, { expected: 404 })
	await call(bob, 'POST', '/friends/accept', { username: 'Alice_QA' })
	const friends = (await call(alice, 'GET', '/sync')).social.friends
	assert.equal(friends[0].username, 'Bob_QA')
	assert.equal(friends[0].id, bob.code, 'cada amigo trae su clave pública, que es su código de partidas')
	await call(alice, 'POST', '/friends/request', { username: 'Bob_QA' }, { expected: 409 })
	await call(bob, 'POST', '/block', { username: 'Alice_QA' })
	assert.equal((await call(alice, 'GET', '/sync')).social.friends.length, 0)
	await call(alice, 'POST', '/friends/request', { username: 'Bob_QA' }, { expected: 403 })
	await call(bob, 'POST', '/unblock', { username: 'Alice_QA' })
	await call(alice, 'POST', '/friends/request', { username: 'Bob_QA' })
	await call(bob, 'POST', '/friends/remove', { username: 'Alice_QA' })
	assert.equal((await call(alice, 'GET', '/sync')).social.outgoing.length, 0)
	await call(alice, 'POST', '/friends/nope', { username: 'Bob_QA' }, { expected: 404 })
})

test('privacidad de solicitudes: todos, amigos de amigos y nadie', async () => {
	const { call, register } = setup()
	const ana = await register('Ana_QA')
	const bea = await register('Bea_QA')
	const cris = await register('Cris_QA')
	const dani = await register('Dani_QA')
	await call(cris, 'PATCH', '/me', { settings: { friend_privacy: 'none' } })
	await call(ana, 'POST', '/friends/request', { username: 'Cris_QA' }, { expected: 403 })

	await call(bea, 'POST', '/friends/request', { username: 'Dani_QA' })
	await call(dani, 'POST', '/friends/accept', { username: 'Bea_QA' })
	await call(dani, 'PATCH', '/me', { settings: { friend_privacy: 'mutual' } })
	await call(ana, 'POST', '/friends/request', { username: 'Dani_QA' }, { expected: 403 })
	await call(ana, 'POST', '/friends/request', { username: 'Bea_QA' })
	await call(bea, 'POST', '/friends/accept', { username: 'Ana_QA' })
	await call(ana, 'POST', '/friends/request', { username: 'Dani_QA' })
	assert.equal((await call(dani, 'GET', '/sync')).social.incoming[0].username, 'Ana_QA')
})

test('límites de uso: cuentas por conexión, búsquedas y solicitudes', async () => {
	const { call, identity, register } = setup()
	// Diez cuentas por hora y conexión; la undécima se rechaza, pero otra conexión sigue pudiendo.
	for (let i = 0; i < 10; i++) await call(await identity(), 'POST', '/register', { username: `Usuario_${i}` }, { ipAddress: '203.0.113.9' })
	await call(await identity(), 'POST', '/register', { username: 'Usuario_x' }, { ipAddress: '203.0.113.9', expected: 429 })
	await call(await identity(), 'POST', '/register', { username: 'Usuario_y' }, { ipAddress: '203.0.113.10' })

	const alice = await register('Alice_QA')
	for (let i = 0; i < 30; i++) await call(alice, 'GET', '/users/search?q=usu')
	assert.equal((await call(alice, 'GET', '/users/search?q=usu', undefined, { expected: 429 })).code, 'rate-limit')

	for (let i = 0; i < 20; i++) await call(alice, 'POST', '/friends/request', { username: `Usuario_${i % 10}` }).catch(() => {})
})

test('borrar la cuenta elimina también amistades y bloqueos (en cascada)', async () => {
	const { db, call, register } = setup()
	const alice = await register('Alice_QA')
	const bob = await register('Bob_QA')
	const cris = await register('Cris_QA')
	await call(alice, 'POST', '/friends/request', { username: 'Bob_QA' })
	await call(alice, 'POST', '/block', { username: 'Cris_QA' })
	await call(alice, 'POST', '/me/delete', {})
	assert.equal((await db.prepare('SELECT COUNT(*) AS n FROM relations').bind().first()).n, 0)
	assert.equal((await db.prepare('SELECT COUNT(*) AS n FROM blocks').bind().first()).n, 0)
	assert.equal((await call(bob, 'GET', '/sync')).social.incoming.length, 0)
	assert.equal((await call(cris, 'GET', '/sync')).social.friends.length, 0)
})

test('cuerpos demasiado grandes y rutas desconocidas', async () => {
	const { call, register } = setup()
	const alice = await register('Alice_QA')
	await call(alice, 'PATCH', '/me', { bio: 'x'.repeat(20_000) }, { expected: 413 })
	await call(alice, 'GET', '/no-existe', undefined, { expected: 404 })
	await call(null, 'GET', '/health', undefined)
})
