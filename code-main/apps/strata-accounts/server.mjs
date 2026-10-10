import { createServer } from 'node:http'
import { DatabaseSync } from 'node:sqlite'
import { randomBytes, randomUUID, scrypt, timingSafeEqual, createHash } from 'node:crypto'
import { promisify } from 'node:util'
import { mkdirSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { pathToFileURL } from 'node:url'

const derive = promisify(scrypt)
const digest = value => createHash('sha256').update(value).digest('hex')
const origins = new Set(['tauri://localhost', 'http://tauri.localhost', 'https://tauri.localhost', 'http://localhost:5173'])
export function createAccountService(path) {
	if (path !== ':memory:') mkdirSync(dirname(resolve(path)), { recursive: true })
	const db = new DatabaseSync(path)
	db.exec(`PRAGMA foreign_keys=ON;
		CREATE TABLE IF NOT EXISTS users(id TEXT PRIMARY KEY, username TEXT UNIQUE COLLATE NOCASE, bio TEXT NOT NULL DEFAULT '', salt TEXT NOT NULL, password TEXT NOT NULL);
		CREATE TABLE IF NOT EXISTS sessions(token TEXT PRIMARY KEY, user_id TEXT REFERENCES users(id) ON DELETE CASCADE, expires INTEGER NOT NULL);
		CREATE TABLE IF NOT EXISTS relations(sender TEXT REFERENCES users(id), receiver TEXT REFERENCES users(id), status TEXT NOT NULL, PRIMARY KEY(sender,receiver));
		CREATE TABLE IF NOT EXISTS blocks(owner TEXT REFERENCES users(id), target TEXT REFERENCES users(id), PRIMARY KEY(owner,target));`)
	const limits = new Map()
	const publicUser = u => ({ id:u.id, username:u.username, bio:u.bio })
	const fail = (status,message) => { throw Object.assign(new Error(message),{status}) }
	const server = createServer(async (req,res) => {
		res.setHeader('Content-Type','application/json; charset=utf-8')
		res.setHeader('Cache-Control','no-store')
		res.setHeader('X-Content-Type-Options','nosniff')
		try {
			if(req.headers.origin && !origins.has(req.headers.origin)) fail(403,'Origen no autorizado.')
			if(req.headers.origin) res.setHeader('Access-Control-Allow-Origin',req.headers.origin)
			res.setHeader('Vary','Origin')
			res.setHeader('Access-Control-Allow-Headers','Authorization, Content-Type')
			res.setHeader('Access-Control-Allow-Methods','GET, POST, PATCH, OPTIONS')
			if(req.method==='OPTIONS') { res.writeHead(204).end(); return }
			const route = `${req.method} ${new URL(req.url,'http://127.0.0.1').pathname}`
			let body = {}
			if(['POST','PATCH'].includes(req.method)) {
				let data=''
				for await(const chunk of req) { data+=chunk; if(Buffer.byteLength(data)>16384) fail(413,'Solicitud demasiado grande.') }
				try { body=JSON.parse(data||'{}') } catch { fail(400,'Solicitud no válida.') }
				if(!body || typeof body!=='object' || Array.isArray(body)) fail(400,'Solicitud no válida.')
			}
			let result
			if(['POST /register','POST /login'].includes(route)) {
				const now=Date.now(), address=req.socket.remoteAddress
				for(const [key,value] of limits) if(value.until<now) limits.delete(key)
				const limit=limits.get(address)||{count:0,until:now+60000}; limits.set(address,limit)
				if(++limit.count>20) fail(429,'Espera un minuto antes de intentarlo de nuevo.')
				if(typeof body.username!=='string'||!/^[a-zA-Z0-9_]{3,24}$/.test(body.username)) fail(400,'El usuario debe tener entre 3 y 24 letras, números o guiones bajos.')
				if(typeof body.password!=='string'||body.password.length<12||body.password.length>128) fail(400,'La contraseña debe tener entre 12 y 128 caracteres.')
				let user=db.prepare('SELECT * FROM users WHERE username=?').get(body.username)
				if(route==='POST /register') {
					if(user) fail(409,'Ese nombre de usuario ya existe.')
					const salt=randomBytes(16).toString('hex'), password=Buffer.from(await derive(body.password,salt,64)).toString('hex')
					user={id:randomUUID(),username:body.username,bio:'',salt,password}
					try { db.prepare('INSERT INTO users(id,username,salt,password) VALUES(?,?,?,?)').run(user.id,user.username,salt,password) } catch { fail(409,'Ese nombre de usuario ya existe.') }
				} else {
					const hash=Buffer.from(await derive(body.password,user?.salt||'unknown-account-salt',64))
					if(!user||!timingSafeEqual(hash,Buffer.from(user.password,'hex'))) fail(401,'Usuario o contraseña incorrectos.')
				}
				const token=randomBytes(32).toString('base64url')
				db.prepare('DELETE FROM sessions WHERE expires<?').run(Date.now())
				db.prepare('INSERT INTO sessions VALUES(?,?,?)').run(digest(token),user.id,Date.now()+86400000)
				result={token,user:publicUser(user)}
			} else {
				const token=req.headers.authorization?.replace(/^Bearer /,'')||''
				const session=db.prepare('SELECT * FROM sessions WHERE token=? AND expires>?').get(digest(token),Date.now())
				if(!session) fail(401,'Inicia sesión con tu cuenta de Strata.')
				const id=session.user_id
				if(route==='GET /me') result=publicUser(db.prepare('SELECT * FROM users WHERE id=?').get(id))
				else if(route==='PATCH /me') {
					if(typeof body.bio!=='string'||body.bio.length>1000) fail(400,'La biografía admite hasta 1000 caracteres.')
					db.prepare('UPDATE users SET bio=? WHERE id=?').run(body.bio,id)
					result=publicUser(db.prepare('SELECT * FROM users WHERE id=?').get(id))
				} else if(route==='POST /logout') { db.prepare('DELETE FROM sessions WHERE token=?').run(digest(token)); result={ok:true} }
				else if(['POST /me/password','POST /me/delete'].includes(route)) {
						const me=db.prepare('SELECT * FROM users WHERE id=?').get(id)
						if(typeof body.password!=='string'||body.password.length>128) fail(400,'Contraseña no válida.')
						const hash=Buffer.from(await derive(body.password,me.salt,64))
						if(!timingSafeEqual(hash,Buffer.from(me.password,'hex'))) fail(401,'Contraseña incorrecta.')
						if(route==='POST /me/password') {
							if(typeof body.newPassword!=='string'||body.newPassword.length<12||body.newPassword.length>128) fail(400,'La contraseña nueva debe tener entre 12 y 128 caracteres.')
							const salt=randomBytes(16).toString('hex'), password=Buffer.from(await derive(body.newPassword,salt,64)).toString('hex')
							db.prepare('UPDATE users SET salt=?,password=? WHERE id=?').run(salt,password,id)
							db.prepare('DELETE FROM sessions WHERE user_id=? AND token<>?').run(id,digest(token))
						} else {
							db.prepare('DELETE FROM relations WHERE sender=? OR receiver=?').run(id,id)
							db.prepare('DELETE FROM blocks WHERE owner=? OR target=?').run(id,id)
							db.prepare('DELETE FROM users WHERE id=?').run(id)
						}
						result={ok:true}
					}
					else if(route==='GET /social') result={
					friends:db.prepare("SELECT u.id,u.username,u.bio FROM users u JOIN relations r ON (r.sender=u.id OR r.receiver=u.id) WHERE r.status='accepted' AND (r.sender=? OR r.receiver=?) AND u.id<>?").all(id,id,id),
					incoming:db.prepare("SELECT u.id,u.username,u.bio FROM users u JOIN relations r ON r.sender=u.id WHERE r.receiver=? AND r.status='pending'").all(id),
					outgoing:db.prepare("SELECT u.id,u.username,u.bio FROM users u JOIN relations r ON r.receiver=u.id WHERE r.sender=? AND r.status='pending'").all(id),
					blocked:db.prepare('SELECT u.id,u.username,u.bio FROM users u JOIN blocks b ON b.target=u.id WHERE b.owner=?').all(id),
				}
				else if(['POST /friends/request','POST /friends/accept','POST /friends/remove','POST /block','POST /unblock'].includes(route)) {
					if(typeof body.username!=='string') fail(400,'Indica un nombre de usuario.')
					const target=db.prepare('SELECT id FROM users WHERE username=?').get(body.username)?.id
					if(!target||target===id) fail(400,'No se puede realizar esa acción con ese usuario.')
					if(route==='POST /block') {
						db.prepare('INSERT OR IGNORE INTO blocks VALUES(?,?)').run(id,target)
						db.prepare('DELETE FROM relations WHERE (sender=? AND receiver=?) OR (sender=? AND receiver=?)').run(id,target,target,id)
					} else if(route==='POST /unblock') db.prepare('DELETE FROM blocks WHERE owner=? AND target=?').run(id,target)
					else if(route==='POST /friends/remove') db.prepare('DELETE FROM relations WHERE (sender=? AND receiver=?) OR (sender=? AND receiver=?)').run(id,target,target,id)
					else {
						if(db.prepare('SELECT 1 FROM blocks WHERE (owner=? AND target=?) OR (owner=? AND target=?)').get(id,target,target,id)) fail(403,'No se puede contactar con ese usuario.')
						if(route==='POST /friends/accept') {
							if(!db.prepare("UPDATE relations SET status='accepted' WHERE sender=? AND receiver=? AND status='pending'").run(target,id).changes) fail(404,'La solicitud ya no está disponible.')
						} else {
							if(db.prepare('SELECT 1 FROM relations WHERE (sender=? AND receiver=?) OR (sender=? AND receiver=?)').get(id,target,target,id)) fail(409,'Ya existe una amistad o solicitud con ese usuario.')
							db.prepare("INSERT INTO relations VALUES(?,?,'pending')").run(id,target)
						}
					}
					result={ok:true}
				} else fail(404,'Ruta no encontrada.')
			}
			res.end(JSON.stringify(result))
		} catch(error) { res.statusCode=error.status||500; res.end(JSON.stringify({error:error.status?error.message:'Error del servicio de cuentas.'})) }
	})
	server.on('close',()=>db.close())
	return server
}
if(process.argv[1] && import.meta.url===pathToFileURL(resolve(process.argv[1])).href) {
	createAccountService(process.env.STRATA_ACCOUNTS_DB||'./strata-accounts.sqlite').listen(32145,'127.0.0.1',()=>console.log('Servicio de cuentas Strata: http://127.0.0.1:32145'))
}
