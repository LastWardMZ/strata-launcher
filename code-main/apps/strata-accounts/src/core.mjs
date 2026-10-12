// Servicio de cuentas de Strata (Cloudflare Workers + D1): nombres únicos, búsqueda, amistades,
// bloqueos y privacidad. No hay contraseñas ni sesiones: la cuenta ES la clave pública Ed25519 del
// launcher y cada petición va firmada con ella. El chat y las partidas no pasan por aquí (P2P).
//
// Esta lógica solo usa APIs estándar (fetch/Response, WebCrypto) y una base de datos con la forma de D1,
// así que corre igual en Workers y en los tests de Node.

const SKEW_MS = 5 * 60_000
const BODY_LIMIT = 16_384
const USERNAME = /^[a-zA-Z0-9_]{3,24}$/
const CODE = /^[0-9a-f]{64}$/
const SIGNATURE = /^[0-9a-f]{128}$/
const FRIEND_PRIVACY = ['everyone', 'mutual', 'none']
const INVITE_PRIVACY = ['friends', 'none']
const RENAME_COOLDOWN_MS = 24 * 3_600_000
const RESERVED = new Set([
	'admin', 'administrator', 'root', 'system', 'support', 'soporte', 'staff', 'moderator', 'mod',
	'strata', 'modrinth', 'mojang', 'minecraft', 'microsoft', 'null', 'undefined', 'anonymous', 'everyone',
])
const FRIEND_ACTIONS = new Set(['/friends/request', '/friends/accept', '/friends/remove', '/block', '/unblock'])

class HttpError extends Error {
	constructor(status, message, code) {
		super(message)
		this.status = status
		this.code = code
	}
}
const fail = (status, message, code) => {
	throw new HttpError(status, message, code)
}

const encoder = new TextEncoder()
const toHex = (bytes) => [...new Uint8Array(bytes)].map((b) => b.toString(16).padStart(2, '0')).join('')
const fromHex = (hex) => Uint8Array.from(hex.match(/../g) ?? [], (h) => parseInt(h, 16))

/** Mensaje que firma el cliente: método, ruta con consulta, hora y hash del cuerpo. */
export const requestMessage = (method, pathAndQuery, ts, bodyHashHex) =>
	`strata-req\n${method}\n${pathAndQuery}\n${ts}\n${bodyHashHex}`

export async function sha256Hex(text) {
	return toHex(await crypto.subtle.digest('SHA-256', encoder.encode(text)))
}

/** Verifica `Authorization: Strata <código>.<ts>.<firma>` y devuelve el código (id de la cuenta). */
async function authenticate(request, url, bodyText) {
	const header = request.headers.get('Authorization') || ''
	const match = /^Strata ([0-9a-f]{64})\.(\d{10,16})\.([0-9a-f]{128})$/.exec(header)
	if (!match || !CODE.test(match[1]) || !SIGNATURE.test(match[3])) fail(401, 'Falta la firma de tu launcher.', 'unauthorized')
	const [, code, ts, signature] = match
	if (Math.abs(Date.now() - Number(ts)) > SKEW_MS) {
		fail(401, 'La hora de tu equipo no coincide con la del servidor. Revísala e inténtalo de nuevo.', 'clock')
	}
	const message = requestMessage(request.method, url.pathname + url.search, ts, await sha256Hex(bodyText))
	let valid = false
	try {
		const key = await crypto.subtle.importKey('raw', fromHex(code), { name: 'Ed25519' }, false, ['verify'])
		valid = await crypto.subtle.verify({ name: 'Ed25519' }, key, fromHex(signature), encoder.encode(message))
	} catch {
		valid = false
	}
	if (!valid) fail(401, 'La firma de tu launcher no es válida.', 'unauthorized')
	return code
}

export function createHandler(db) {
	const first = async (sql, ...args) => (await db.prepare(sql).bind(...args).first()) ?? null
	const all = async (sql, ...args) => (await db.prepare(sql).bind(...args).all()).results
	const run = async (sql, ...args) => (await db.prepare(sql).bind(...args).run()).meta?.changes ?? 0

	/** Límite de uso persistente (cada isolate de Workers tiene su propia memoria, así que va en D1). */
	async function allow(key, max, windowMs) {
		const now = Date.now()
		const row = await first(
			`INSERT INTO limits(key,count,until) VALUES(?,1,?)
			 ON CONFLICT(key) DO UPDATE SET
			   count = CASE WHEN until < ? THEN 1 ELSE count + 1 END,
			   until = CASE WHEN until < ? THEN ? ELSE until END
			 RETURNING count`,
			key, now + windowMs, now, now, now + windowMs,
		)
		return row.count <= max
	}
	const limit = async (key, max, windowMs, message) => {
		if (!(await allow(key, max, windowMs))) fail(429, message, 'rate-limit')
	}

	const settingsOf = (u) => ({ friend_privacy: u.friend_privacy, invite_privacy: u.invite_privacy, searchable: !!u.searchable })
	const profileOf = (u) => ({ id: u.id, username: u.username, bio: u.bio, settings: settingsOf(u) })
	const userById = (id) => first('SELECT * FROM users WHERE id=?', id)
	const userByName = (name) => first('SELECT * FROM users WHERE username=?', name)

	function validateUsername(name) {
		if (typeof name !== 'string' || !USERNAME.test(name)) fail(400, 'El usuario debe tener entre 3 y 24 letras, números o guiones bajos.')
		if (RESERVED.has(name.toLowerCase())) fail(400, 'Ese nombre de usuario no está disponible.', 'name-taken')
	}
	async function relationTo(me, other) {
		const row = await first(
			'SELECT sender,status FROM relations WHERE (sender=? AND receiver=?) OR (sender=? AND receiver=?)', me, other, other, me,
		)
		if (!row) return 'none'
		if (row.status === 'accepted') return 'friend'
		return row.sender === me ? 'outgoing' : 'incoming'
	}
	const friendIds = async (id) =>
		(await all(
			"SELECT CASE WHEN sender=? THEN receiver ELSE sender END AS friend FROM relations WHERE status='accepted' AND (sender=? OR receiver=?)", id, id, id,
		)).map((r) => r.friend)

	async function social(id) {
		const [friends, incoming, outgoing, blocked] = await Promise.all([
			all("SELECT u.id,u.username,u.bio FROM users u JOIN relations r ON (r.sender=u.id OR r.receiver=u.id) WHERE r.status='accepted' AND (r.sender=? OR r.receiver=?) AND u.id<>? ORDER BY u.username", id, id, id),
			all("SELECT u.id,u.username,u.bio FROM users u JOIN relations r ON r.sender=u.id WHERE r.receiver=? AND r.status='pending' ORDER BY u.username", id),
			all("SELECT u.id,u.username,u.bio FROM users u JOIN relations r ON r.receiver=u.id WHERE r.sender=? AND r.status='pending' ORDER BY u.username", id),
			all('SELECT u.id,u.username,u.bio FROM users u JOIN blocks b ON b.target=u.id WHERE b.owner=? ORDER BY u.username', id),
		])
		return { friends, incoming, outgoing, blocked }
	}

	async function route(request, url, bodyText, ip) {
		const path = url.pathname
		const key = `${request.method} ${path}`
		if (key === 'GET /health') return { ok: true }

		const id = await authenticate(request, url, bodyText)
		let body = {}
		if (request.method !== 'GET') {
			try { body = bodyText ? JSON.parse(bodyText) : {} } catch { fail(400, 'Solicitud no válida.') }
			if (!body || typeof body !== 'object' || Array.isArray(body)) fail(400, 'Solicitud no válida.')
		}

		if (key === 'POST /register') {
			await limit(`reg:${ip}`, 10, 3_600_000, 'Has creado demasiadas cuentas desde esta conexión. Inténtalo más tarde.')
			if (await userById(id)) fail(409, 'Este launcher ya tiene una cuenta.', 'has-account')
			validateUsername(body.username)
			if (await userByName(body.username)) fail(409, 'Ese nombre de usuario ya existe.', 'name-taken')
			try {
				await run('INSERT INTO users(id,username,created) VALUES(?,?,?)', id, body.username, Date.now())
			} catch {
				fail(409, 'Ese nombre de usuario ya existe.', 'name-taken')
			}
			await run('DELETE FROM limits WHERE until < ?', Date.now())
			return profileOf(await userById(id))
		}

		const me = await userById(id)
		if (!me) fail(404, 'Este launcher aún no tiene una cuenta de Strata.', 'no-account')

		switch (key) {
			case 'GET /me':
				return profileOf(me)
			case 'GET /sync':
				return { profile: profileOf(me), social: await social(id) }
			case 'PATCH /me': {
				if (body.bio !== undefined) {
					if (typeof body.bio !== 'string' || body.bio.length > 500) fail(400, 'La biografía admite hasta 500 caracteres.')
					await run('UPDATE users SET bio=? WHERE id=?', body.bio.trim(), id)
				}
				if (body.settings !== undefined) {
					const s = body.settings
					if (!s || typeof s !== 'object') fail(400, 'Ajustes no válidos.')
					if (s.friend_privacy !== undefined && !FRIEND_PRIVACY.includes(s.friend_privacy)) fail(400, 'Ajuste de solicitudes no válido.')
					if (s.invite_privacy !== undefined && !INVITE_PRIVACY.includes(s.invite_privacy)) fail(400, 'Ajuste de invitaciones no válido.')
					if (s.searchable !== undefined && typeof s.searchable !== 'boolean') fail(400, 'Ajuste de búsqueda no válido.')
					await run(
						'UPDATE users SET friend_privacy=COALESCE(?,friend_privacy), invite_privacy=COALESCE(?,invite_privacy), searchable=COALESCE(?,searchable) WHERE id=?',
						s.friend_privacy ?? null, s.invite_privacy ?? null, s.searchable === undefined ? null : Number(s.searchable), id,
					)
				}
				return profileOf(await userById(id))
			}
			case 'POST /me/rename': {
				validateUsername(body.username)
				if (body.username.toLowerCase() !== me.username.toLowerCase()) {
					const wait = me.renamed_at + RENAME_COOLDOWN_MS - Date.now()
					if (wait > 0) fail(429, `Solo puedes cambiar de nombre una vez al día. Inténtalo en ${Math.ceil(wait / 3_600_000)} h.`, 'rate-limit')
					const other = await userByName(body.username)
					if (other && other.id !== id) fail(409, 'Ese nombre de usuario ya existe.', 'name-taken')
				}
				try {
					await run('UPDATE users SET username=?, renamed_at=? WHERE id=?', body.username, Date.now(), id)
				} catch {
					fail(409, 'Ese nombre de usuario ya existe.', 'name-taken')
				}
				return profileOf(await userById(id))
			}
			case 'POST /me/delete':
				await db.batch([
					db.prepare('DELETE FROM relations WHERE sender=? OR receiver=?').bind(id, id),
					db.prepare('DELETE FROM blocks WHERE owner=? OR target=?').bind(id, id),
					db.prepare('DELETE FROM users WHERE id=?').bind(id),
				])
				return { ok: true }
			case 'GET /users/search': {
				await limit(`search:${id}`, 30, 60_000, 'Estás buscando demasiado deprisa. Espera un momento.')
				const q = (url.searchParams.get('q') || '').trim()
				if (q.length < 2 || q.length > 24 || !/^[a-zA-Z0-9_]+$/.test(q)) return { users: [] }
				const rows = await all(
					`SELECT id,username FROM users
					 WHERE username LIKE ? ESCAPE '\\' AND searchable=1 AND id<>?
					 AND id NOT IN (SELECT target FROM blocks WHERE owner=?) AND id NOT IN (SELECT owner FROM blocks WHERE target=?)
					 ORDER BY LENGTH(username), username LIMIT 20`,
					`${q.replace(/[\\%_]/g, '\\$&')}%`, id, id, id,
				)
				return { users: await Promise.all(rows.map(async (u) => ({ ...u, relation: await relationTo(id, u.id) }))) }
			}
			default:
		}

		if (request.method === 'POST' && FRIEND_ACTIONS.has(path)) {
			if (typeof body.username !== 'string') fail(400, 'Indica un nombre de usuario.')
			// Los límites van ANTES de buscar al destino: sin ellos estas acciones permitían sondear nombres (también los de
			// usuarios que no quieren aparecer en la búsqueda) a ritmo ilimitado.
			await limit(`act:${id}`, 60, 3_600_000, 'Has hecho demasiadas acciones de amistad. Inténtalo más tarde.')
			if (path === '/friends/request') await limit(`req:${id}`, 20, 3_600_000, 'Has enviado demasiadas solicitudes. Inténtalo más tarde.')
			const target = await userByName(body.username)
			if (!target || target.id === id) {
				// Misma respuesta que si el usuario existiera pero no hubiera nada que hacer (sin relación o sin solicitud).
				if (path === '/friends/accept') fail(404, 'La solicitud ya no está disponible.')
				if (path === '/friends/request') fail(400, 'No se puede realizar esa acción con ese usuario.')
				return { ok: true }
			}
			const other = target.id
			if (path === '/block') {
				await db.batch([
					db.prepare('INSERT OR IGNORE INTO blocks VALUES(?,?)').bind(id, other),
					db.prepare('DELETE FROM relations WHERE (sender=? AND receiver=?) OR (sender=? AND receiver=?)').bind(id, other, other, id),
				])
			} else if (path === '/unblock') {
				await run('DELETE FROM blocks WHERE owner=? AND target=?', id, other)
			} else if (path === '/friends/remove') {
				await run('DELETE FROM relations WHERE (sender=? AND receiver=?) OR (sender=? AND receiver=?)', id, other, other, id)
			} else {
				if (await first('SELECT 1 AS x FROM blocks WHERE (owner=? AND target=?) OR (owner=? AND target=?)', id, other, other, id)) {
					fail(403, 'No se puede contactar con ese usuario.')
				}
				if (path === '/friends/accept') {
					if (!(await run("UPDATE relations SET status='accepted' WHERE sender=? AND receiver=? AND status='pending'", other, id))) {
						fail(404, 'La solicitud ya no está disponible.')
					}
				} else {
					if ((await relationTo(id, other)) !== 'none') fail(409, 'Ya existe una amistad o solicitud con ese usuario.')
					if (target.friend_privacy === 'none') fail(403, 'Este usuario no acepta solicitudes de amistad.')
					if (target.friend_privacy === 'mutual') {
						const mine = new Set(await friendIds(id))
						if (!(await friendIds(other)).some((f) => mine.has(f))) {
							fail(403, 'Este usuario solo acepta solicitudes de amigos de sus amigos.')
						}
					}
					await run("INSERT INTO relations VALUES(?,?,'pending')", id, other)
				}
			}
			return { ok: true }
		}
		fail(404, 'Ruta no encontrada.')
	}

	return async function handle(request, options = {}) {
		const headers = {
			'Content-Type': 'application/json; charset=utf-8',
			'Cache-Control': 'no-store',
			'X-Content-Type-Options': 'nosniff',
		}
		const reply = (status, payload) => new Response(JSON.stringify(payload), { status, headers })
		try {
			// El launcher llama desde Rust y no envía Origin. Un navegador de otro sitio, sí: se rechaza.
			if (request.headers.get('Origin')) fail(403, 'Origen no autorizado.')
			const url = new URL(request.url)
			const declared = Number(request.headers.get('Content-Length') || 0)
			if (declared > BODY_LIMIT) fail(413, 'Solicitud demasiado grande.')
			const bodyText = request.method === 'GET' || request.method === 'HEAD' ? '' : await request.text()
			if (bodyText.length > BODY_LIMIT) fail(413, 'Solicitud demasiado grande.')
			const ip = options.ip || request.headers.get('CF-Connecting-IP') || 'desconocida'
			return reply(200, await route(request, url, bodyText, ip))
		} catch (error) {
			if (error instanceof HttpError) return reply(error.status, { error: error.message, code: error.code })
			console.error(error)
			return reply(500, { error: 'Error del servicio de cuentas.' })
		}
	}
}
