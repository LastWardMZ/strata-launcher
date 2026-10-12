import { invoke } from '@tauri-apps/api/core'
import { onBeforeUnmount, onMounted, ref, type Ref } from 'vue'

const INTERVAL_MS = 30_000

/**
 * Quién tiene Strata abierto ahora. Se pregunta directamente a cada amigo (P2P, sin pasar por el
 * servidor de cuentas), solo mientras el componente está en pantalla.
 */
export function useFriendPresence(ids: Ref<string[]>) {
	const online = ref<Record<string, boolean>>({})
	let timer: ReturnType<typeof setInterval> | undefined
	let running = false

	async function check() {
		if (running || document.hidden) return
		running = true
		try {
			const entries = await Promise.all(
				ids.value.map(async (id) => {
					try {
						return [id, await invoke<boolean>('p2p_is_online', { id })] as const
					} catch {
						return [id, false] as const
					}
				}),
			)
			online.value = Object.fromEntries(entries)
		} finally {
			running = false
		}
	}

	onMounted(() => {
		void check()
		timer = setInterval(() => void check(), INTERVAL_MS)
	})
	onBeforeUnmount(() => clearInterval(timer))
	return { online, refresh: check }
}
