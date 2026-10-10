// Punto de entrada de Cloudflare Workers. La base de datos D1 llega como `env.DB` (ver wrangler.toml).
import { createHandler } from './core.mjs'

let handler
export default {
	async fetch(request, env) {
		handler ??= createHandler(env.DB)
		return handler(request)
	},
}
