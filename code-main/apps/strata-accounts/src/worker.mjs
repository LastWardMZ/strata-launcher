// Punto de entrada de Cloudflare Workers. La base de datos D1 llega como `env.DB` (ver wrangler.toml).
import { createHandler } from './core.mjs'
import { createMapsHandler } from './maps.mjs'

let handler
let maps
export default {
	async fetch(request, env) {
		// Mapas de CurseForge (/maps/…): no usa la base de datos. La clave es el secreto CURSEFORGE_API_KEY.
		maps ??= createMapsHandler({
			apiKey: env.CURSEFORGE_API_KEY,
			// Caché desactivada salvo que se indique (ver README: las condiciones de CurseForge limitan guardar datos de su API).
			cache: typeof caches === 'undefined' ? undefined : caches.default,
			ttl: { search: Number(env.MAPS_CACHE_SEARCH_SECONDS) || 0, detail: Number(env.MAPS_CACHE_DETAIL_SECONDS) || 0 },
		})
		const answer = await maps(request, { ip: request.headers.get('cf-connecting-ip') ?? 'unknown' })
		if (answer) return answer
		handler ??= createHandler(env.DB)
		return handler(request)
	},
}
