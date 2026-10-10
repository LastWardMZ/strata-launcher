import { invoke } from '@tauri-apps/api/core'
import { computed, ref } from 'vue'

/**
 * Cuenta de Strata: nombre de usuario único, búsqueda, amigos y privacidad. No hay contraseña: la
 * cuenta es la clave de este launcher. Las peticiones al servicio las hace Rust (`accounts_*`);
 * aquí solo vive el estado compartido de la interfaz.
 */
export type FriendPrivacy = 'everyone' | 'mutual' | 'none'
export type InvitePrivacy = 'friends' | 'none'
export type AccountSettings = {
	friend_privacy: FriendPrivacy
	invite_privacy: InvitePrivacy
	searchable: boolean
}
/** `id` es la clave pública de la cuenta, que también es su código de partidas. */
export type Profile = {
	id: string
	username: string
	bio: string
	settings: AccountSettings
}
export type Member = { id: string; username: string; bio?: string }
export type Social = {
	friends: Member[]
	incoming: Member[]
	outgoing: Member[]
	blocked: Member[]
}
export type Relation = 'none' | 'friend' | 'outgoing' | 'incoming'
export type SearchHit = { id: string; username: string; relation: Relation }
export type FriendAction = 'request' | 'accept' | 'remove' | 'block' | 'unblock'
type Status = {
	server_url: string
	/** Este launcher tiene una cuenta registrada. */
	registered: boolean
	profile: Profile | null
	social: Social | null
	/** No se pudo hablar con el servidor. */
	offline: boolean
	/** Mensaje del servidor si el fallo no es de red (p. ej. la hora del equipo no coincide). */
	problem: string | null
	default_server: boolean
}

const emptySocial = (): Social => ({ friends: [], incoming: [], outgoing: [], blocked: [] })
const status = ref<Status | null>(null)
const error = ref('')
const busy = ref(false)
let started = false
let timer: ReturnType<typeof setInterval> | undefined
// El plan gratuito de Cloudflare admite 100.000 peticiones al día: se consulta poco.
const REFRESH_MS = 120_000

const messageOf = (cause: unknown) =>
	typeof cause === 'string'
		? cause
		: cause instanceof Error
			? cause.message
			: 'No se pudo completar la acción.'

/** Pide al servidor el estado (perfil y listas); además sincroniza el P2P. */
async function refresh() {
	const next = await invoke<Status>('accounts_status')
	// Con el servidor caído se conserva lo último que se sabía, en vez de vaciar las listas.
	status.value =
		next.offline && status.value?.registered
			? { ...status.value, offline: true, problem: null }
			: next
}

export function useStrataAccount() {
	if (!started) {
		started = true
		void refresh().catch((cause) => (error.value = messageOf(cause)))
		timer = setInterval(() => void refresh().catch(() => {}), REFRESH_MS)
	}

	/** Ejecuta una acción mostrando el error en `error`; devuelve `true` si salió bien. */
	async function run(action: () => Promise<unknown>): Promise<boolean> {
		if (busy.value) return false
		busy.value = true
		error.value = ''
		try {
			await action()
			return true
		} catch (cause) {
			error.value = messageOf(cause)
			return false
		} finally {
			busy.value = false
		}
	}
	const reload = () => refresh()

	return {
		status,
		error,
		busy,
		profile: computed(() => status.value?.profile ?? null),
		social: computed(() => status.value?.social ?? emptySocial()),
		/** Este launcher tiene cuenta (se mantiene el nombre antiguo `loggedIn` en los componentes). */
		registered: computed(() => !!status.value?.registered),
		loggedIn: computed(() => !!status.value?.registered),
		pendingRequests: computed(() => status.value?.social?.incoming.length ?? 0),
		clearError: () => (error.value = ''),
		reload,
		register: (username: string) =>
			run(async () => {
				await invoke('accounts_register', { username })
				await reload()
			}),
		rename: (username: string) =>
			run(async () => {
				await invoke('accounts_rename', { username })
				await reload()
			}),
		update: (changes: { bio?: string; settings?: Partial<AccountSettings> }) =>
			run(async () => {
				await invoke('accounts_update', { changes })
				await reload()
			}),
		friendAction: (action: FriendAction, username: string) =>
			run(async () => {
				await invoke('accounts_friend_action', { action, username })
				await reload()
			}),
		deleteAccount: () =>
			run(async () => {
				await invoke('accounts_delete')
				await reload()
			}),
		setServer: (url: string) =>
			run(async () => {
				await invoke('accounts_set_server', { url })
				await reload()
			}),
		exportIdentity: (path: string) => run(() => invoke('p2p_export_identity', { path })),
		importIdentity: (path: string) => run(() => invoke('p2p_import_identity', { path })),
		/** Búsqueda por nombre; los errores (p. ej. demasiadas búsquedas) se devuelven al llamador. */
		search: (query: string) => invoke<{ users: SearchHit[] }>('accounts_search', { query }),
		stop: () => {
			if (timer) clearInterval(timer)
			started = false
		},
	}
}
