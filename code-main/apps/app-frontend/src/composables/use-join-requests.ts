import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { onBeforeUnmount, onMounted, ref } from 'vue'

/** Un amigo que quiere unirse a tu partida y aún no has respondido (se pierde al cerrar el launcher). */
export type JoinRequest = { friend: string; id: string; ts: number }
/** Mundo o servidor que compartes ahora mismo. */
export type OpenShare = { id: string; kind: string; title: string }

type Handlers = {
	onRequest?: (friend: string) => void
	onDeclined?: (friend: string) => void
}

export function useJoinRequests(handlers: Handlers = {}) {
	const requests = ref<JoinRequest[]>([])
	const shares = ref<OpenShare[]>([])
	let unlisten: (() => void) | undefined

	async function load() {
		try {
			requests.value = await invoke<JoinRequest[]>('p2p_join_requests')
			if (requests.value.length) {
				shares.value = (await invoke<OpenShare[]>('p2p_shares')).filter((share) => share.kind !== 'instance')
			}
		} catch {
			// Sin P2P todavía: se vuelve a intentar con el próximo aviso.
		}
	}

	/** Acepta (envía la invitación al mundo o servidor elegido) o rechaza; devuelve el error si lo hay. */
	async function respond(friend: string, accept: boolean, shareId?: string): Promise<string | null> {
		try {
			await invoke('p2p_respond_join', { friend, accept, shareId: shareId ?? null })
			return null
		} catch (cause) {
			return typeof cause === 'string' ? cause : 'No se pudo responder.'
		} finally {
			await load()
		}
	}

	onMounted(async () => {
		void load()
		unlisten = await listen<{ type?: string; friend?: string }>('p2p-event', (event) => {
			const { type, friend } = event.payload ?? {}
			if (type === 'join_request' && friend) {
				void load()
				handlers.onRequest?.(friend)
			} else if (type === 'join_declined' && friend) {
				handlers.onDeclined?.(friend)
			}
		})
	})
	onBeforeUnmount(() => unlisten?.())
	return { requests, shares, respond, reload: load }
}
