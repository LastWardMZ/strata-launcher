// Servicio de cuentas de Strata: usuarios únicos, búsqueda, amigos, bloqueos y privacidad.
// Sin dependencias externas: Node 24+ (node:sqlite). Las contraseñas se guardan con scrypt y sal propia,
// las sesiones como hash SHA-256 del token. El chat y las partidas NO pasan por aquí: van por P2P.
import { createServer } from 'node:http'
import { DatabaseSync } from 'node:sqlite'
import {
	createHash, createPublicKey, randomBytes, randomUUID, scrypt, timingSafeEqual, verify,
} from 'node:crypto'
import { promisify } from 'node:util'
import { mkdirSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { pathToFileURL } from 'node:url'

const derive = promisify(scrypt)
const digest = (value) => createHash('sha256').update(value).digest('hex')

const SESSION_MS = 30 * 24 * 60 * 60 * 1000
const USERNAME = /^[a-zA-Z0-9_]{3,24}$/
const P2P_CODE = /^[0-9a-f]{64}$/
const FRIEND_PRIVACY = ['everyone', 'mutual', 'none']
const INVITE_PRIVACY = ['friends', 'none']
const RESERVED = new Set([
	'admin', 'administrator', 'root', 'system', 'support', 'soporte', 'staff', 'moderator', 'mod',
	'strata', 'modrinth', 'mojang', 'minecraft', 'microsoft', 'null', 'undefined', 'anonymous', 'everyone',
])
const BROWSER_ORIGINS = new Set([
	'tauri://localhost', 'http://tauri.localhost', 'https://tauri.localhost', 'http://localhost:5173',
])

class HttpError extends Error {
	constructor(status, message) {
		super(message)
		this.status = status
	}
}
const fail = (status, message) => {
	throw new HttpError(status, message)
}

/** Límite de uso en memoria: `max` acciones por ventana y clave. */
function createLimiter() {
	const buckets = new Map()
	return (key, max, windowMs) => {
		const now = Date.now()
		if (buckets.size > 5000) for (const [k, v] of buckets) if (v.until < now) buckets.delete(k)
		let bucket = buckets.get(key)
		if (!bucket || bucket.until < now) {
			bucket = { count: 0, until: now + windowMs }
			buckets.set(key, bucket)
		}
		return ++bucket.count <= max
	}
}

/** Comprueba la firma Ed25519 (clave pública de 32 bytes en hexadecimal) de un mensaje. */
function verifyP2pSignature(codeHex, message, signatureHex) {
	try {
		const der = Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), Buffer.from(codeHex, 'hex')])
		const key = createPublicKey({ key: der, format: 'der', type: 'spki' })
		return verify(null, Buffer.from(message), key, Buffer.from(signatureHex, 'hex'))
	} catch {
		return false
	}
}

export function createAccountService(path, options = {}) {
	const trustProxy = options.trustProxy ?? process.env.STRATA_TRUST_PROXY === '1'
	const origins = new Set([...BROWSER_ORIGINS, ...(options.origins ?? [])])
	if (path !== ':memory:') mkdirSync(dirname(resolve(path)), { recursive: true })
	const db = new DatabaseSync(path)
	db.exec(`PRAGMA foreign_keys=ON;
		CREATE TABLE IF NOT EXISTS users(id TEXT PRIMARY KEY, username TEXT UNIQUE COLLATE NOCASE, bio TEXT NOT NULL DEFAULT '', salt TEXT NOT NULL, password TEXT NOT NULL);
		CREATE TABLE IF NOT EXISTS sessions(token TEXT PRIMARY KEY, user_id TEXT REFERENCES users(id) ON DELETE CASCADE, expires INTEGER NOT NULL);
		CREATE TABLE IF NOT EXISTS relations(sender TEXT REFERENCES users(id), receiver TEXT REFERENCES users(id), status TEXT NOT NULL, PRIMARY KEY(sender,receiver));
		CREATE TABLE IF NOT EXISTS blocks(owner TEXT REFERENCES users(id), target TEXT REFERENCES users(id), PRIMARY KEY(owner,target));`)
	// Migraciones: bases de datos creadas por versiones anteriores del servicio.
	const columns = new Set(db.prepare('PRAGMA table_info(users)').all().map((c) => c.name))
	for (const [name, ddl] of [
		['friend_privacy', "TEXT NOT NULL DEFAULT 'everyone'"],
		['invite_privacy', "TEXT NOT NULL DEFAULT 'friends'"],
		['searchable', 'INTEGER NOT NULL DEFAULT 1'],
		['p2p_code', 'TEXT'],
		['created', 'INTEGER NOT NULL DEFAULT 0'],
	]) if (!columns.has(name)) db.exec(`ALTER TABLE users ADD COLUMN ${name} ${ddl}`)

	const allow = createLimiter()
	const publicUser = (u) => ({ id: u.id, username: u.username, bio: u.bio })
	const settingsOf = (u) => ({
		friend_privacy: u.friend_privacy, invite_privacy: u.invite_privacy, searchable: !!u.searchable,
	})
	const profileOf = (u) => ({ ...publicUser(u), p2p_code: u.p2p_code, settings: settingsOf(u) })
	const userById = (id) => db.prepare('SELECT * FROM users WHERE id=?').get(id)
	const userByName = (name) => db.prepare('SELECT * FROM users WHERE username=?').get(name)
	const isBlocked = (a, b) =>
		!!db.prepare('SELECT 1 FROM blocks WHERE (owner=? AND target=?) OR (owner=? AND target=?)').get(a, b, b, a)
	const areFriends = (a, b) =>
		!!db.prepare("SELECT 1 FROM relations WHERE status='accepted' AND ((sender=? AND receiver=?) OR (sender=? AND receiver=?))").get(a, b, b, a)
	const friendIds = (id) =>
		db.prepare("SELECT CASE WHEN sender=? THEN receiver ELSE sender END AS friend FROM relations WHERE status='accepted' AND (sender=? OR receiver=?)")
			.all(id, id, id).map((r) => r.friend)

	function validateUsername(name) {
		if (typeof name !== 'string' || !USERNAME.test(name)) {
			fail(400, 'El usuario debe tener entre 3 y 24 letras, números o guiones bajos.')
		}
		if (RESERVED.has(name.toLowerCase())) fail(400, 'Ese nombre de usuario no está disponible.')
	}
	function validatePassword(password, field = 'La contraseña') {
		if (typeof password !== 'string' || password.length < 12 || password.length > 128) {
			fail(400, `${field} debe tener entre 12 y 128 caracteres.`)
		}
	}
	async function hash(password, salt) {
		return Buffer.from(await derive(password, salt, 64))
	}
	function openSession(userId) {
		const token = randomBytes(32).toString('base64url')
		db.prepare('DELETE FROM sessions WHERE expires<?').run(Date.now())
		db.prepare('INSERT INTO sessions VALUES(?,?,?)').run(digest(token), userId, Date.now() + SESSION_MS)
		return token
	}
	function relationTo(me, other) {
		const row = db.prepare('SELECT sender,status FROM relations WHERE (sender=? AND receiver=?) OR (sender=? AND receiver=?)').get(me, other, other, me)
		if (!row) return 'none'
		if (row.status === 'accepted') return 'friend'
		return row.sender === me ? 'outgoing' : 'incoming'
	}
	const social = (id) => ({
		friends: db.prepare("SELECT u.id,u.username,u.bio,u.p2p_code FROM users u JOIN relations r ON (r.sender=u.id OR r.receiver=u.id) WHERE r.status='accepted' AND (r.sender=? OR r.receiver=?) AND u.id<>? ORDER BY u.username").all(id, id, id),
		incoming: db.prepare("SELECT u.id,u.username,u.bio FROM users u JOIN relations r ON r.sender=u.id WHERE r.receiver=? AND r.status='pending' ORDER BY u.username").all(id),
		outgoing: db.prepare("SELECT u.id,u.username,u.bio FROM users u JOIN relations r ON r.receiver=u.id WHERE r.sender=? AND r.status='pending' ORDER BY u.username").all(id),
		blocked: db.prepare('SELECT u.id,u.username,u.bio FROM users u JOIN blocks b ON b.target=u.id WHERE b.owner=? ORDER BY u.username').all(id),
	})

	async function readBody(req) {
		let data = ''
		for await (const chunk of req) {
			data += chunk
			if (Buffer.byteLength(data) > 16384) fail(413, 'Solicitud demasiado grande.')
		}
		let body
		try { body = JSON.parse(data || '{}') } catch { fail(400, 'Solicitud no válida.') }
		if (!body || typeof body !== 'object' || Array.isArray(body)) fail(400, 'Solicitud no válida.')
		return body
	}

	async function handle(req, url, body, address) {
		const route = `${req.method} ${url.pathname}`
		if (route === 'GET /health') return { ok: true }
		if (!allow(`ip:${address}`, 300, 60_000)) fail(429, 'Demasiadas peticiones. Espera un momento.')

		if (route === 'POST /register' || route === 'POST /login') {
			if (!allow(`auth:${address}`, 20, 60_000)) fail(429, 'Espera un minuto antes de intentarlo de nuevo.')
			if (route === 'POST /register' && !allow(`register:${address}`, 10, 3_600_000)) fail(429, 'Has creado demasiadas cuentas. Inténtalo más tarde.')
			validateUsername(body.username)
			validatePassword(body.password)
			let user = userByName(body.username)
			if (route === 'POST /register') {
				if (user) fail(409, 'Ese nombre de usuario ya existe.')
				const salt = randomBytes(16).toString('hex')
				const password = (await hash(body.password, salt)).toString('hex')
				const id = randomUUID()
				try {
					db.prepare('INSERT INTO users(id,username,salt,password,created) VALUES(?,?,?,?,?)').run(id, body.username, salt, password, Date.now())
				} catch { fail(409, 'Ese nombre de usuario ya existe.') }
				user = userById(id)
			} else {
				const candidate = await hash(body.password, user?.salt || 'unknown-account-salt')
				if (!user || !timingSafeEqual(candidate, Buffer.from(user.password, 'hex'))) fail(401, 'Usuario o contraseña incorrectos.')
			}
			return { token: openSession(user.id), user: profileOf(user) }
		}

		const token = req.headers.authorization?.replace(/^Bearer /, '') || ''
		const session = db.prepare('SELECT * FROM sessions WHERE token=? AND expires>?').get(digest(token), Date.now())
		if (!session) fail(401, 'Inicia sesión con tu cuenta de Strata.')
		const id = session.user_id
		const me = userById(id)

		switch (route) {
			case 'GET /me':
				return profileOf(me)
			case 'PATCH /me': {
				if (body.bio !== undefined) {
					if (typeof body.bio !== 'string' || body.bio.length > 500) fail(400, 'La biografía admite hasta 500 caracteres.')
					db.prepare('UPDATE users SET bio=? WHERE id=?').run(body.bio.trim(), id)
				}
				if (body.settings !== undefined) {
					const s = body.settings
					if (!s || typeof s !== 'object') fail(400, 'Ajustes no válidos.')
					if (s.friend_privacy !== undefined && !FRIEND_PRIVACY.includes(s.friend_privacy)) fail(400, 'Ajuste de solicitudes no válido.')
					if (s.invite_privacy !== undefined && !INVITE_PRIVACY.includes(s.invite_privacy)) fail(400, 'Ajuste de invitaciones no válido.')
					if (s.searchable !== undefined && typeof s.searchable !== 'boolean') fail(400, 'Ajuste de búsqueda no válido.')
					db.prepare('UPDATE users SET friend_privacy=COALESCE(?,friend_privacy), invite_privacy=COALESCE(?,invite_privacy), searchable=COALESCE(?,searchable) WHERE id=?')
						.run(s.friend_privacy ?? null, s.invite_privacy ?? null, s.searchable === undefined ? null : Number(s.searchable), id)
				}
				return profileOf(userById(id))
			}
			case 'POST /me/p2p': {
				// Enlaza el código de partidas P2P con la cuenta, demostrando que se posee la clave.
				if (typeof body.code !== 'string' || !P2P_CODE.test(body.code)) fail(400, 'Código de partidas no válido.')
				if (!Number.isInteger(body.ts) || Math.abs(Date.now() - body.ts) > 5 * 60_000) fail(400, 'La firma ha caducado: revisa la hora de tu equipo.')
				if (typeof body.sig !== 'string' || !verifyP2pSignature(body.code, `strata-link:${id}:${body.code}:${body.ts}`, body.sig)) {
					fail(403, 'La firma del código de partidas no es válida.')
				}
				db.prepare('UPDATE users SET p2p_code=? WHERE id=?').run(body.code, id)
				return profileOf(userById(id))
			}
			case 'POST /logout':
				db.prepare('DELETE FROM sessions WHERE token=?').run(digest(token))
				return { ok: true }
			case 'POST /me/password':
			case 'POST /me/delete': {
				if (!allow(`pw:${id}`, 10, 60_000)) fail(429, 'Demasiados intentos. Espera un minuto.')
				if (typeof body.password !== 'string' || body.password.length > 128) fail(400, 'Contraseña no válida.')
				if (!timingSafeEqual(await hash(body.password, me.salt), Buffer.from(me.password, 'hex'))) fail(401, 'Contraseña incorrecta.')
				if (route === 'POST /me/password') {
					validatePassword(body.newPassword, 'La contraseña nueva')
					const salt = randomBytes(16).toString('hex')
					db.prepare('UPDATE users SET salt=?,password=? WHERE id=?').run(salt, (await hash(body.newPassword, salt)).toString('hex'), id)
					db.prepare('DELETE FROM sessions WHERE user_id=? AND token<>?').run(id, digest(token))
				} else {
					db.prepare('DELETE FROM relations WHERE sender=? OR receiver=?').run(id, id)
					db.prepare('DELETE FROM blocks WHERE owner=? OR target=?').run(id, id)
					db.prepare('DELETE FROM users WHERE id=?').run(id)
				}
				return { ok: true }
			}
			case 'GET /social':
				return social(id)
			case 'GET /users/search': {
				if (!allow(`search:${id}`, 30, 60_000)) fail(429, 'Estás buscando demasiado deprisa. Espera un momento.')
				const q = (url.searchParams.get('q') || '').trim()
				if (q.length < 2 || q.length > 24 || !/^[a-zA-Z0-9_]+$/.test(q)) return { users: [] }
				const rows = db.prepare(`SELECT id,username FROM users
					WHERE username LIKE ? ESCAPE '\\' AND searchable=1 AND id<>?
					AND id NOT IN (SELECT target FROM blocks WHERE owner=?) AND id NOT IN (SELECT owner FROM blocks WHERE target=?)
					ORDER BY LENGTH(username), username LIMIT 20`).all(`${q.replace(/[\\%_]/g, '\\$&')}%`, id, id, id)
				return { users: rows.map((u) => ({ ...u, relation: relationTo(id, u.id) })) }
			}
			default:
		}

		if (['POST /friends/request', 'POST /friends/accept', 'POST /friends/remove', 'POST /block', 'POST /unblock'].includes(route)) {
			if (typeof body.username !== 'string') fail(400, 'Indica un nombre de usuario.')
			const target = userByName(body.username)
			if (!target || target.id === id) fail(400, 'No se puede realizar esa acción con ese usuario.')
			const other = target.id
			if (route === 'POST /block') {
				db.prepare('INSERT OR IGNORE INTO blocks VALUES(?,?)').run(id, other)
				db.prepare('DELETE FROM relations WHERE (sender=? AND receiver=?) OR (sender=? AND receiver=?)').run(id, other, other, id)
			} else if (route === 'POST /unblock') {
				db.prepare('DELETE FROM blocks WHERE owner=? AND target=?').run(id, other)
			} else if (route === 'POST /friends/remove') {
				db.prepare('DELETE FROM relations WHERE (sender=? AND receiver=?) OR (sender=? AND receiver=?)').run(id, other, other, id)
			} else {
				if (isBlocked(id, other)) fail(403, 'No se puede contactar con ese usuario.')
				if (route === 'POST /friends/accept') {
					if (!db.prepare("UPDATE relations SET status='accepted' WHERE sender=? AND receiver=? AND status='pending'").run(other, id).changes) fail(404, 'La solicitud ya no está disponible.')
				} else {
					if (!allow(`request:${id}`, 20, 3_600_000)) fail(429, 'Has enviado demasiadas solicitudes. Inténtalo más tarde.')
					if (relationTo(id, other) !== 'none') fail(409, 'Ya existe una amistad o solicitud con ese usuario.')
					if (target.friend_privacy === 'none') fail(403, 'Este usuario no acepta solicitudes de amistad.')
					if (target.friend_privacy === 'mutual') {
						const mine = new Set(friendIds(id))
						if (!friendIds(other).some((f) => mine.has(f))) fail(403, 'Este usuario solo acepta solicitudes de amigos de sus amigos.')
					}
					db.prepare("INSERT INTO relations VALUES(?,?,'pending')").run(id, other)
				}
			}
			return { ok: true }
		}
		fail(404, 'Ruta no encontrada.')
	}

	const server = createServer(async (req, res) => {
		res.setHeader('Content-Type', 'application/json; charset=utf-8')
		res.setHeader('Cache-Control', 'no-store')
		res.setHeader('X-Content-Type-Options', 'nosniff')
		try {
			if (req.headers.origin && !origins.has(req.headers.origin)) fail(403, 'Origen no autorizado.')
			if (req.headers.origin) res.setHeader('Access-Control-Allow-Origin', req.headers.origin)
			res.setHeader('Vary', 'Origin')
			res.setHeader('Access-Control-Allow-Headers', 'Authorization, Content-Type')
			res.setHeader('Access-Control-Allow-Methods', 'GET, POST, PATCH, OPTIONS')
			if (req.method === 'OPTIONS') { res.writeHead(204).end(); return }
			const forwarded = trustProxy ? String(req.headers['x-forwarded-for'] || '').split(',')[0].trim() : ''
			const address = forwarded || req.socket.remoteAddress || 'desconocida'
			const url = new URL(req.url, 'http://127.0.0.1')
			const body = ['POST', 'PATCH'].includes(req.method) ? await readBody(req) : {}
			res.end(JSON.stringify(await handle(req, url, body, address)))
		} catch (error) {
			if (!(error instanceof HttpError)) console.error(error)
			res.statusCode = error instanceof HttpError ? error.status : 500
			res.end(JSON.stringify({ error: error instanceof HttpError ? error.message : 'Error del servicio de cuentas.' }))
		}
	})
	server.on('close', () => db.close())
	return server
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
	const port = Number(process.env.PORT || 32145)
	const host = process.env.HOST || '127.0.0.1'
	createAccountService(process.env.STRATA_ACCOUNTS_DB || './strata-accounts.sqlite').listen(port, host, () =>
		console.log(`Servicio de cuentas Strata: http://${host}:${port}`))
}
