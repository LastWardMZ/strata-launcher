import { invoke } from '@tauri-apps/api/core'
import { ref } from 'vue'

/**
 * Fotos de perfil de tus amigos. El servicio de cuentas solo envía la versión (`avatar_v`) junto a cada
 * amigo; la imagen se pide una vez por versión y se guarda aquí, así que no se descarga en cada refresco.
 */
type Cached = { v: number; image: string }
const KEY = 'strata:avatar-cache'
const MAX_PARALLEL = 3

function load(): Record<string, Cached> {
	try {
		const parsed = JSON.parse(localStorage.getItem(KEY) ?? '{}')
		return parsed && typeof parsed === 'object' ? parsed : {}
	} catch {
		return {}
	}
}

const cache = ref<Record<string, Cached>>(load())
const pending = new Set<string>()
const queue: { id: string; v: number }[] = []
let running = 0

function persist() {
	try {
		localStorage.setItem(KEY, JSON.stringify(cache.value))
	} catch {
		// Sin almacenamiento, las fotos se piden de nuevo en el próximo arranque.
	}
}

async function drain() {
	while (running < MAX_PARALLEL && queue.length) {
		const { id, v } = queue.shift()!
		running++
		invoke<{ v: number; image: string }>('accounts_avatar', { id })
			.then((reply) => {
				cache.value = { ...cache.value, [id]: { v: reply.v, image: reply.image } }
				persist()
			})
			.catch(() => {
				// Sin foto o sin conexión: se reintenta en el próximo refresco, cuando la versión siga sin coincidir.
			})
			.finally(() => {
				running--
				pending.delete(`${id}:${v}`)
				void drain()
			})
	}
}

export function useFriendAvatars() {
	/** Dirección de la foto (data URL) o `null` si no tiene o aún no se ha descargado. */
	function avatarOf(member: { id: string; avatar_v?: number }): string | null {
		const entry = cache.value[member.id]
		const version = member.avatar_v ?? 0
		if (version > 0 && entry?.v !== version && !pending.has(`${member.id}:${version}`)) {
			pending.add(`${member.id}:${version}`)
			queue.push({ id: member.id, v: version })
			void drain()
		}
		if (version === 0 || !entry?.image) return null
		return `data:image/webp;base64,${entry.image}`
	}
	return { avatarOf }
}
