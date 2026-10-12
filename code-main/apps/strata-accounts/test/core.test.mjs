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

test('sondeo de nombres [SEC-06]: las acciones de amistad tienen límite y no delatan qué usuarios existen', async () => {
	const { call, register } = setup()
	const alice = await register('Alice_QA')
	const bob = await register('Bob_QA')
	await call(bob, 'PATCH', '/me', { settings: { searchable: false } })

	// Bloquear, desbloquear, retirar y aceptar dan la misma respuesta si el usuario existe (sin relación) y si no existe.
	for (const path of ['/unblock', '/friends/remove', '/block']) {
		const existing = await call(alice, 'POST', path, { username: 'Bob_QA' })
		const missing = await call(alice, 'POST', path, { username: 'Nadie_QA' })
		assert.deepEqual(missing, existing, `${path}: misma respuesta exista o no el usuario`)
	}
	await call(alice, 'POST', '/unblock', { username: 'Bob_QA' })
	const noRequest = await call(alice, 'POST', '/friends/accept', { username: 'Bob_QA' }, { expected: 404 })
	const missingAccept = await call(alice, 'POST', '/friends/accept', { username: 'Nadie_QA' }, { expected: 404 })
	assert.deepEqual(missingAccept, noRequest, '/friends/accept: misma respuesta')

	// Sondear a ritmo ilimitado ya no es posible: 60 acciones por hora y cuenta (las 8 de arriba cuentan).
	const carol = await register('Carol_QA')
	for (let i = 0; i < 60; i++) await call(carol, 'POST', '/unblock', { username: `Prueba_${i}` })
	const blocked = await call(carol, 'POST', '/unblock', { username: 'Prueba_x' }, { expected: 429 })
	assert.equal(blocked.code, 'rate-limit')

	// Las solicitudes a nombres inexistentes también cuentan para el límite de 20 por hora.
	const dave = await register('Dave_QA')
	for (let i = 0; i < 20; i++) await call(dave, 'POST', '/friends/request', { username: `Nadie_${i}` }, { expected: 400 })
	assert.equal((await call(dave, 'POST', '/friends/request', { username: 'Nadie_x' }, { expected: 429 })).code, 'rate-limit')
})

/** Un WebP mínimo bien formado (cabecera RIFF/WEBP con el tamaño correcto) de `size` bytes. */
function fakeWebp(size = 40) {
	const bytes = new Uint8Array(size)
	bytes.set([0x52, 0x49, 0x46, 0x46], 0)
	new DataView(bytes.buffer).setUint32(4, size - 8, true)
	bytes.set([0x57, 0x45, 0x42, 0x50], 8)
	return Buffer.from(bytes).toString('base64')
}

test('foto de perfil: solo WebP pequeño, visible para ti, tus amigos y solicitudes; no para extraños ni bloqueados', async () => {
	const { call, register } = setup()
	const alice = await register('Alice_QA')
	const bob = await register('Bob_QA')
	const carol = await register('Carol_QA')

	// Validación: formato, tamaño y base64.
	await call(alice, 'PUT', '/me/avatar', { image: 'no es base64!' }, { expected: 400 })
	await call(alice, 'PUT', '/me/avatar', { image: Buffer.from('<svg onload=alert(1)>............').toString('base64') }, { expected: 400 })
	await call(alice, 'PUT', '/me/avatar', { image: fakeWebp(24 * 1024 + 4) }, { expected: 413 })
	const wrongSize = Buffer.from(fakeWebp(40), 'base64')
	wrongSize.writeUInt32LE(9999, 4)
	await call(alice, 'PUT', '/me/avatar', { image: wrongSize.toString('base64') }, { expected: 400 })
	await call(alice, 'PUT', '/me/avatar', {}, { expected: 400 })

	// Subida correcta: cambia `avatar_v` y se ve en el perfil propio.
	const image = fakeWebp(200)
	const before = (await call(alice, 'GET', '/me')).avatar_v
	assert.equal(before, 0)
	const updated = await call(alice, 'PUT', '/me/avatar', { image })
	assert.ok(updated.avatar_v > 0)
	assert.deepEqual(await call(alice, 'GET', `/avatar?user=${alice.code}`), { id: alice.code, v: updated.avatar_v, image })

	// Un extraño no la ve (404 igual que si no existiera la cuenta).
	const stranger = await call(bob, 'GET', `/avatar?user=${alice.code}`, undefined, { expected: 404 })
	const missing = await call(bob, 'GET', `/avatar?user=${'a'.repeat(64)}`, undefined, { expected: 404 })
	assert.deepEqual(stranger, missing)
	await call(bob, 'GET', '/avatar?user=zzz', undefined, { expected: 404 })

	// Con una solicitud pendiente y luego siendo amigos, sí.
	await call(bob, 'POST', '/friends/request', { username: 'Alice_QA' })
	assert.equal((await call(bob, 'GET', `/avatar?user=${alice.code}`)).image, image)
	await call(alice, 'POST', '/friends/accept', { username: 'Bob_QA' })
	assert.equal((await call(bob, 'GET', `/avatar?user=${alice.code}`)).v, updated.avatar_v)
	// La lista de amigos trae solo la versión, no la imagen.
	const sync = await call(bob, 'GET', '/sync')
	assert.deepEqual(sync.social.friends.map((f) => [f.username, f.avatar_v, f.avatar]), [['Alice_QA', updated.avatar_v, undefined]])

	// Quien no es amigo sigue sin verla, y un bloqueo la oculta.
	await call(carol, 'GET', `/avatar?user=${alice.code}`, undefined, { expected: 404 })
	await call(alice, 'POST', '/block', { username: 'Bob_QA' })
	await call(bob, 'GET', `/avatar?user=${alice.code}`, undefined, { expected: 404 })

	// Borrar la foto cambia la versión y deja la imagen vacía.
	const removed = await call(alice, 'POST', '/me/avatar/delete', {})
	assert.notEqual(removed.avatar_v, updated.avatar_v)
	assert.equal((await call(alice, 'GET', `/avatar?user=${alice.code}`)).image, '')
})

test('foto de perfil: límite de cambios por hora y el cuerpo grande solo se admite en su ruta', async () => {
	const { call, register } = setup()
	const alice = await register('Alice_QA')
	for (let i = 0; i < 10; i++) await call(alice, 'PUT', '/me/avatar', { image: fakeWebp(100 + i) })
	assert.equal((await call(alice, 'PUT', '/me/avatar', { image: fakeWebp(300) }, { expected: 429 })).code, 'rate-limit')
	await call(alice, 'PATCH', '/me', { bio: 'x'.repeat(20_000) }, { expected: 413 })
})

test('denuncias: solo a amigos o solicitudes, con motivo y límite, y sin revelar si el usuario existe', async () => {
	const { call, register, db } = setup()
	const alice = await register('Alice_QA')
	const bob = await register('Bob_QA')
	await register('Carol_QA')
	const count = async () => (await db.prepare('SELECT COUNT(*) AS n FROM reports').first()).n

	// Sin motivo, con motivo demasiado largo o sin usuario: error (también cuentan para el límite, por eso otra cuenta).
	const eve = await register('Eve_QA')
	await call(eve, 'POST', '/report', { username: 'Bob_QA' }, { expected: 400 })
	await call(eve, 'POST', '/report', { username: 'Bob_QA', reason: '   ' }, { expected: 400 })
	await call(eve, 'POST', '/report', { username: 'Bob_QA', reason: 'x'.repeat(301) }, { expected: 400 })
	await call(eve, 'POST', '/report', { reason: 'spam' }, { expected: 400 })

	// Un desconocido, un nombre que no existe y uno mismo dan la misma respuesta y no guardan nada.
	const stranger = await call(alice, 'POST', '/report', { username: 'Carol_QA', reason: 'spam' })
	const missing = await call(alice, 'POST', '/report', { username: 'Nadie_QA', reason: 'spam' })
	const self = await call(alice, 'POST', '/report', { username: 'Alice_QA', reason: 'spam' })
	assert.deepEqual([stranger, missing, self], [{ ok: true }, { ok: true }, { ok: true }])
	assert.equal(await count(), 0)

	// Con una solicitud pendiente, sí se guarda (con quién denuncia a quién y el nombre de ese momento).
	await call(bob, 'POST', '/friends/request', { username: 'Alice_QA' })
	await call(alice, 'POST', '/report', { username: 'Bob_QA', reason: '  Me envía spam  ' })
	const row = await db.prepare('SELECT * FROM reports').first()
	assert.deepEqual([row.reporter, row.target, row.target_name, row.reason], [alice.code, bob.code, 'Bob_QA', 'Me envía spam'])

	// La denuncia sobrevive a que la cuenta denunciada se borre, y hay un máximo por hora.
	await call(bob, 'POST', '/me/delete', {})
	assert.equal(await count(), 1)
	const dave = await register('Dave_QA')
	for (let i = 0; i < 5; i++) await call(dave, 'POST', '/report', { username: `Nadie_${i}`, reason: 'x' })
	assert.equal((await call(dave, 'POST', '/report', { username: 'Nadie_x', reason: 'x' }, { expected: 429 })).code, 'rate-limit')
})
