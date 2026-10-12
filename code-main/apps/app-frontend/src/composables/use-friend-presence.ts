import { invoke } from '@tauri-apps/api/core'
import { onBeforeUnmount, onMounted, ref, type Ref } from 'vue'

const INTERVAL_MS = 30_000

/** A qué juega un amigo ahora (si lo comparte): nombre de su instancia y segundos desde 1970 en que empezó. */
export type Playing = { title: string; since: number }
export type Presence = { online: boolean; playing: Playing | null }

/**
 * Quién tiene Strata abierto ahora y a qué juega. Se pregunta directamente a cada amigo (P2P, sin pasar por
 * el servidor de cuentas), solo mientras el componente está en pantalla.
 */
export function useFriendPresence(ids: Ref<string[]>) {
	const presence = ref<Record<string, Presence>>({})
	let timer: ReturnType<typeof setInterval> | undefined
	let running = false

	async function check() {
		if (running || document.hidden) return
		running = true
		try {
			const entries = await Promise.all(
				ids.value.map(async (id) => {
					try {
						return [id, await invoke<Presence>('p2p_presence', { id })] as const
					} catch {
						return [id, { online: false, playing: null }] as const
					}
				}),
			)
			presence.value = Object.fromEntries(entries)
		} finally {
			running = false
		}
	}

	onMounted(() => {
		void check()
		timer = setInterval(() => void check(), INTERVAL_MS)
	})
	onBeforeUnmount(() => clearInterval(timer))
	return { presence, refresh: check }
}
