import test from 'node:test'
import assert from 'node:assert/strict'
import { once } from 'node:events'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createAccountService } from './server.mjs'

test('cuentas propias: autenticación, persistencia, amistades, bloqueos y separación de usuarios', async () => {
	const directory = mkdtempSync(join(tmpdir(), 'strata-accounts-'))
	const database = join(directory, 'accounts.sqlite')
	let server = createAccountService(database)
	let base
	async function start() {
		server.listen(0, '127.0.0.1')
		await once(server, 'listening')
		base = `http://127.0.0.1:${server.address().port}`
	}
	async function call(path, body, token, expected = 200, method = body ? 'POST' : 'GET', origin = 'http://tauri.localhost') {
		const response = await fetch(base + path, {
			method, headers: { Origin: origin, 'Content-Type': 'application/json', ...(token ? { Authorization: `Bearer ${token}` } : {}) },
			body: body ? JSON.stringify(body) : undefined,
		})
		assert.equal(response.status, expected)
		return response.json()
	}
	try {
		await start()
		const password = 'Test-only-password-2026'
		const alice = await call('/register', { username: 'Alice_QA', password })
		const bob = await call('/register', { username: 'Bob_QA', password })
		const eve = await call('/register', { username: 'Eve_QA', password })
		await call('/register', { username: 'alice_qa', password }, null, 409)
		await call('/login', { username: 'Alice_QA', password: 'invalid-password' }, null, 401)
		await call('/me', undefined, undefined, 401)
		await call('/me', undefined, alice.token, 403, 'GET', 'https://untrusted.example')
		await call('/me', { bio: 'Mi perfil de Strata' }, alice.token, 200, 'PATCH')
		assert.equal((await call('/me', undefined, bob.token)).bio, '')
		await call('/friends/request', { username: 'Bob_QA' }, alice.token)
		assert.equal((await call('/social', undefined, bob.token)).incoming[0].username, 'Alice_QA')
		assert.equal((await call('/social', undefined, eve.token)).incoming.length, 0)
		await call('/friends/accept', { username: 'Alice_QA' }, eve.token, 404)
		await call('/friends/accept', { username: 'Alice_QA' }, bob.token)
		assert.equal((await call('/social', undefined, alice.token)).friends[0].username, 'Bob_QA')
		await call('/block', { username: 'Alice_QA' }, bob.token)
		assert.equal((await call('/social', undefined, alice.token)).friends.length, 0)
		await call('/friends/request', { username: 'Bob_QA' }, alice.token, 403)
		await call('/unblock', { username: 'Alice_QA' }, bob.token)
		await call('/friends/request', { username: 'Bob_QA' }, alice.token)
		await call('/friends/remove', { username: 'Alice_QA' }, bob.token)
		assert.equal((await call('/social', undefined, alice.token)).outgoing.length, 0)
		await call('/logout', {}, alice.token)
		await call('/me', undefined, alice.token, 401)
		await new Promise(resolve => server.close(resolve))
		server = createAccountService(database)
		await start()
		const login = await call('/login', { username: 'Alice_QA', password })
		assert.equal(login.user.bio, 'Mi perfil de Strata')
		assert.equal('password' in login.user, false)
		assert.equal('salt' in login.user, false)
		const newPassword = 'Another-test-password-2026'
		await call('/me/password', { password: 'incorrecta-123456', newPassword }, login.token, 401)
		await call('/me/password', { password, newPassword: 'corta' }, login.token, 400)
		await call('/me/password', { password, newPassword }, login.token)
		await call('/login', { username: 'Alice_QA', password }, null, 401)
		const renewed = await call('/login', { username: 'Alice_QA', password: newPassword })
		await call('/me/delete', { password }, renewed.token, 401)
		await call('/me/delete', { password: newPassword }, renewed.token)
		await call('/me', undefined, renewed.token, 401)
		await call('/login', { username: 'Alice_QA', password: newPassword }, null, 401)
		assert.equal((await call('/social', undefined, bob.token)).incoming.length, 0)
	} finally {
		await new Promise(resolve => server.close(resolve))
		rmSync(directory, { recursive: true, force: true })
	}
})
