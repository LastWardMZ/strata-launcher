import test from 'node:test'
import assert from 'node:assert/strict'
import { once } from 'node:events'
import { generateKeyPairSync, sign } from 'node:crypto'
import { mkdtempSync, rmSync } from 'node:fs'
import { DatabaseSync } from 'node:sqlite'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createAccountService } from './server.mjs'

const password = 'Test-only-password-2026'

async function withService(run, { database = ':memory:', directory } = {}) {
	const server = createAccountService(database)
	server.listen(0, '127.0.0.1')
	await once(server, 'listening')
	const base = `http://127.0.0.1:${server.address().port}`
	async function call(path, body, token, expected = 200, method = body ? 'POST' : 'GET', origin = 'http://tauri.localhost') {
		const response = await fetch(base + path, {
			method,
			headers: { Origin: origin, 'Content-Type': 'application/json', ...(token ? { Authorization: `Bearer ${token}` } : {}) },
			body: body ? JSON.stringify(body) : undefined,
		})
		assert.equal(response.status, expected, `${method} ${path} -> ${response.status}`)
		return response.json()
	}
	try {
		await run(call, base)
	} finally {
		await new Promise((resolve) => server.close(resolve))
		if (directory) rmSync(directory, { recursive: true, force: true })
	}
}

/** Clave P2P de prueba: devuelve el código (32 bytes en hex) y una función para firmar. */
function p2pIdentity() {
	const { publicKey, privateKey } = generateKeyPairSync('ed25519')
	const raw = publicKey.export({ format: 'der', type: 'spki' }).subarray(-32)
	return { code: raw.toString('hex'), sign: (message) => sign(null, Buffer.from(message), privateKey).toString('hex') }
}

test('registro: nombres únicos sin distinguir mayúsculas, reservados y validaciones', async () => {
	await withService(async (call) => {
		const alice = await call('/register', { username: 'Alice_QA', password })
		assert.equal(alice.user.username, 'Alice_QA')
		assert.equal('password' in alice.user, false)
		assert.equal('salt' in alice.user, false)
		await call('/register', { username: 'alice_qa', password }, null, 409)
		await call('/register', { username: 'ALICE_QA', password }, null, 409)
		for (const reserved of ['admin', 'Strata', 'MODRINTH', 'Minecraft']) {
			await call('/register', { username: reserved, password }, null, 400)
		}
		await call('/register', { username: 'ab', password }, null, 400)
		await call('/register', { username: 'con espacios', password }, null, 400)
		await call('/register', { username: 'Bob_QA', password: 'corta' }, null, 400)
		await call('/login', { username: 'Alice_QA', password: 'invalid-password' }, null, 401)
		await call('/me', undefined, undefined, 401)
		await call('/me', undefined, alice.token, 403, 'GET', 'https://untrusted.example')
		await call('/health', undefined, undefined, 200)
	})
})

test('perfil, ajustes de privacidad y cambio de contraseña', async () => {
	await withService(async (call) => {
		const alice = await call('/register', { username: 'Alice_QA', password })
		const me = await call('/me', undefined, alice.token)
		assert.deepEqual(me.settings, { friend_privacy: 'everyone', invite_privacy: 'friends', searchable: true })
		await call('/me', { bio: 'Mi perfil' }, alice.token, 200, 'PATCH')
		await call('/me', { bio: 'x'.repeat(501) }, alice.token, 400, 'PATCH')
		await call('/me', { settings: { friend_privacy: 'mutual', searchable: false } }, alice.token, 200, 'PATCH')
		const updated = await call('/me', undefined, alice.token)
		assert.deepEqual(updated.settings, { friend_privacy: 'mutual', invite_privacy: 'friends', searchable: false })
		assert.equal(updated.bio, 'Mi perfil')
		await call('/me', { settings: { friend_privacy: 'cualquiera' } }, alice.token, 400, 'PATCH')
		await call('/me', { settings: { searchable: 'si' } }, alice.token, 400, 'PATCH')

		const newPassword = 'Another-test-password-2026'
		await call('/me/password', { password: 'incorrecta-123456', newPassword }, alice.token, 401)
		await call('/me/password', { password, newPassword: 'corta' }, alice.token, 400)
		await call('/me/password', { password, newPassword }, alice.token)
		await call('/login', { username: 'Alice_QA', password }, null, 401)
		const renewed = await call('/login', { username: 'Alice_QA', password: newPassword })
		await call('/me/delete', { password }, renewed.token, 401)
		await call('/me/delete', { password: newPassword }, renewed.token)
		await call('/me', undefined, renewed.token, 401)
		await call('/login', { username: 'Alice_QA', password: newPassword }, null, 401)
		await call('/register', { username: 'Alice_QA', password }, null, 200) // el nombre queda libre tras borrar la cuenta
	})
})

test('búsqueda por nombre: prefijo, privacidad, bloqueos y relación', async () => {
	await withService(async (call) => {
		const alice = await call('/register', { username: 'Alice_QA', password })
		const bob = await call('/register', { username: 'Bob_QA', password })
		const bobby = await call('/register', { username: 'Bobby', password })
		const hidden = await call('/register', { username: 'Bobo_oculto', password })
		await call('/me', { settings: { searchable: false } }, hidden.token, 200, 'PATCH')

		const names = async (token, q) => (await call(`/users/search?q=${encodeURIComponent(q)}`, undefined, token)).users.map((u) => u.username)
		assert.deepEqual(await names(alice.token, 'bo'), ['Bobby', 'Bob_QA'], 'por prefijo (primero los nombres más cortos), sin ocultos ni uno mismo')
		assert.deepEqual(await names(alice.token, 'BOB'), ['Bobby', 'Bob_QA'], 'sin distinguir mayúsculas')
		assert.deepEqual(await names(alice.token, 'Alice'), [], 'no se encuentra a uno mismo')
		assert.deepEqual(await names(alice.token, 'b'), [], 'mínimo dos caracteres')
		assert.deepEqual(await names(alice.token, '%_'), [], 'los comodines no se interpretan')
		assert.deepEqual(await names(alice.token, 'zzz'), [])
		await call('/users/search?q=bo', undefined, undefined, 401)

		await call('/friends/request', { username: 'Bob_QA' }, alice.token)
		const found = (await call('/users/search?q=bob', undefined, alice.token)).users
		assert.equal(found.find((u) => u.username === 'Bob_QA').relation, 'outgoing')
		assert.equal((await call('/users/search?q=ali', undefined, bob.token)).users[0].relation, 'incoming')
		await call('/block', { username: 'Bobby' }, alice.token)
		assert.deepEqual(await names(alice.token, 'bob'), ['Bob_QA'], 'los bloqueados no aparecen')
		assert.deepEqual(await names(bobby.token, 'ali'), [], 'ni aparece quien te ha bloqueado')
	})
})

test('amistades, bloqueos y separación de usuarios', async () => {
	await withService(async (call) => {
		const alice = await call('/register', { username: 'Alice_QA', password })
		const bob = await call('/register', { username: 'Bob_QA', password })
		const eve = await call('/register', { username: 'Eve_QA', password })
		await call('/friends/request', { username: 'Bob_QA' }, alice.token)
		assert.equal((await call('/social', undefined, bob.token)).incoming[0].username, 'Alice_QA')
		assert.equal((await call('/social', undefined, eve.token)).incoming.length, 0)
		await call('/friends/accept', { username: 'Alice_QA' }, eve.token, 404)
		await call('/friends/accept', { username: 'Alice_QA' }, bob.token)
		assert.equal((await call('/social', undefined, alice.token)).friends[0].username, 'Bob_QA')
		await call('/friends/request', { username: 'Bob_QA' }, alice.token, 409)
		await call('/block', { username: 'Alice_QA' }, bob.token)
		assert.equal((await call('/social', undefined, alice.token)).friends.length, 0)
		await call('/friends/request', { username: 'Bob_QA' }, alice.token, 403)
		await call('/unblock', { username: 'Alice_QA' }, bob.token)
		await call('/friends/request', { username: 'Bob_QA' }, alice.token)
		await call('/friends/remove', { username: 'Alice_QA' }, bob.token)
		assert.equal((await call('/social', undefined, alice.token)).outgoing.length, 0)
		await call('/logout', {}, alice.token)
		await call('/me', undefined, alice.token, 401)
	})
})

test('privacidad de solicitudes: todos, amigos de amigos y nadie', async () => {
	await withService(async (call) => {
		const ana = await call('/register', { username: 'Ana_QA', password })
		const bea = await call('/register', { username: 'Bea_QA', password })
		const cris = await call('/register', { username: 'Cris_QA', password })
		const dani = await call('/register', { username: 'Dani_QA', password })

		await call('/me', { settings: { friend_privacy: 'none' } }, cris.token, 200, 'PATCH')
		await call('/friends/request', { username: 'Cris_QA' }, ana.token, 403)

		// Bea y Dani son amigos (Dani aún acepta a todos).
		await call('/friends/request', { username: 'Dani_QA' }, bea.token)
		await call('/friends/accept', { username: 'Bea_QA' }, dani.token)
		// Ahora Dani solo acepta amigos de sus amigos: Ana no tiene ninguno en común con él.
		await call('/me', { settings: { friend_privacy: 'mutual' } }, dani.token, 200, 'PATCH')
		await call('/friends/request', { username: 'Dani_QA' }, ana.token, 403)
		// Ana se hace amiga de Bea: ya comparten un amigo (Bea) y la solicitud pasa.
		await call('/friends/request', { username: 'Bea_QA' }, ana.token)
		await call('/friends/accept', { username: 'Ana_QA' }, bea.token)
		await call('/friends/request', { username: 'Dani_QA' }, ana.token)
		assert.equal((await call('/social', undefined, dani.token)).incoming[0].username, 'Ana_QA')
	})
})

test('código P2P: solo se enlaza con una firma válida y solo lo ven los amigos', async () => {
	await withService(async (call) => {
		const alice = await call('/register', { username: 'Alice_QA', password })
		const bob = await call('/register', { username: 'Bob_QA', password })
		const eve = await call('/register', { username: 'Eve_QA', password })
		const aliceKey = p2pIdentity()
		const ts = Date.now()
		const aliceId = (await call('/me', undefined, alice.token)).id
		const message = `strata-link:${aliceId}:${aliceKey.code}:${ts}`

		await call('/me/p2p', { code: 'no-es-un-codigo', ts, sig: 'x' }, alice.token, 400)
		await call('/me/p2p', { code: aliceKey.code, ts, sig: 'ab'.repeat(64) }, alice.token, 403)
		await call('/me/p2p', { code: aliceKey.code, ts: ts - 3_600_000, sig: aliceKey.sign(message) }, alice.token, 400)
		// Firmado por otra clave: no demuestra que se posea el código.
		await call('/me/p2p', { code: aliceKey.code, ts, sig: p2pIdentity().sign(message) }, alice.token, 403)
		// Una firma hecha para otra cuenta no vale para esta.
		await call('/me/p2p', { code: aliceKey.code, ts, sig: aliceKey.sign(`strata-link:otra-cuenta:${aliceKey.code}:${ts}`) }, alice.token, 403)
		const linked = await call('/me/p2p', { code: aliceKey.code, ts, sig: aliceKey.sign(message) }, alice.token)
		assert.equal(linked.p2p_code, aliceKey.code)

		await call('/friends/request', { username: 'Bob_QA' }, alice.token)
		await call('/friends/accept', { username: 'Alice_QA' }, bob.token)
		assert.equal((await call('/social', undefined, bob.token)).friends[0].p2p_code, aliceKey.code, 'un amigo lo ve')
		await call('/friends/request', { username: 'Eve_QA' }, alice.token)
		assert.equal('p2p_code' in (await call('/social', undefined, eve.token)).incoming[0], false, 'una solicitud pendiente no lo ve')
		assert.equal((await call('/users/search?q=ali', undefined, bob.token)).users.some((u) => 'p2p_code' in u), false, 'la búsqueda no lo expone')
	})
})

test('migración: una base de datos de la versión anterior sigue funcionando', async () => {
	const directory = mkdtempSync(join(tmpdir(), 'strata-accounts-'))
	const database = join(directory, 'old.sqlite')
	const old = new DatabaseSync(database)
	old.exec(`CREATE TABLE users(id TEXT PRIMARY KEY, username TEXT UNIQUE COLLATE NOCASE, bio TEXT NOT NULL DEFAULT '', salt TEXT NOT NULL, password TEXT NOT NULL);
		CREATE TABLE sessions(token TEXT PRIMARY KEY, user_id TEXT REFERENCES users(id) ON DELETE CASCADE, expires INTEGER NOT NULL);
		CREATE TABLE relations(sender TEXT REFERENCES users(id), receiver TEXT REFERENCES users(id), status TEXT NOT NULL, PRIMARY KEY(sender,receiver));
		CREATE TABLE blocks(owner TEXT REFERENCES users(id), target TEXT REFERENCES users(id), PRIMARY KEY(owner,target));`)
	old.close()
	await withService(async (call) => {
		const alice = await call('/register', { username: 'Alice_QA', password })
		assert.equal((await call('/me', undefined, alice.token)).settings.friend_privacy, 'everyone')
	}, { database })
	// Reiniciar sobre la misma base no falla (las migraciones son idempotentes) y conserva los datos.
	const again = createAccountService(database)
	again.listen(0, '127.0.0.1')
	await once(again, 'listening')
	const response = await fetch(`http://127.0.0.1:${again.address().port}/login`, {
		method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ username: 'alice_qa', password }),
	})
	assert.equal(response.status, 200)
	await new Promise((resolve) => again.close(resolve))
	rmSync(directory, { recursive: true, force: true })
})
