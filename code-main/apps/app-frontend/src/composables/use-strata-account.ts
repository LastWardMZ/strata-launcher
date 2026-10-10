import { invoke } from '@tauri-apps/api/core'
import { computed, ref } from 'vue'

/**
 * Cuenta de Strata: usuarios únicos, búsqueda, amigos y privacidad. Las peticiones al servicio de
 * cuentas las hace Rust (`accounts_*`); aquí solo vive el estado compartido de la interfaz.
 */
export type FriendPrivacy = 'everyone' | 'mutual' | 'none'
export type InvitePrivacy = 'friends' | 'none'
export type AccountSettings = {
	friend_privacy: FriendPrivacy
	invite_privacy: InvitePrivacy
	searchable: boolean
}
export type Profile = {
	id: string
	username: string
	bio: string
	p2p_code: string | null
	settings: AccountSettings
}
export type Member = { id: string; username: string; bio?: string; p2p_code?: string | null }
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
	logged_in: boolean
	profile: Profile | null
	offline: boolean
	default_server: boolean
}

const emptySocial = (): Social => ({ friends: [], incoming: [], outgoing: [], blocked: [] })
const status = ref<Status | null>(null)
const social = ref<Social>(emptySocial())
const error = ref('')
const busy = ref(false)
let started = false
let timer: ReturnType<typeof setInterval> | undefined

const messageOf = (cause: unknown) =>
	typeof cause === 'string'
		? cause
		: cause instanceof Error
			? cause.message
			: 'No se pudo completar la acción.'

async function refreshStatus() {
	status.value = await invoke<Status>('accounts_status')
	if (!status.value.logged_in) social.value = emptySocial()
}
async function refreshSocial() {
	if (!status.value?.logged_in) return
	try {
		social.value = await invoke<Social>('accounts_social')
	} catch {
		// Sin conexión con el servidor: se conserva la última lista.
	}
}

export function useStrataAccount() {
	if (!started) {
		started = true
		void refreshStatus()
			.then(refreshSocial)
			.catch((cause) => (error.value = messageOf(cause)))
		timer = setInterval(() => {
			void refreshStatus()
				.then(refreshSocial)
				.catch(() => {})
		}, 30_000)
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
	const reload = async () => {
		await refreshStatus()
		await refreshSocial()
	}

	return {
		status,
		social,
		error,
		busy,
		profile: computed(() => status.value?.profile ?? null),
		loggedIn: computed(() => !!status.value?.logged_in),
		pendingRequests: computed(() => social.value.incoming.length),
		clearError: () => (error.value = ''),
		reload,
		login: (username: string, password: string) =>
			run(async () => {
				await invoke('accounts_login', { username, password })
				await reload()
			}),
		register: (username: string, password: string) =>
			run(async () => {
				await invoke('accounts_register', { username, password })
				await reload()
			}),
		logout: () =>
			run(async () => {
				await invoke('accounts_logout')
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
		changePassword: (password: string, newPassword: string) =>
			run(() => invoke('accounts_change_password', { password, newPassword })),
		deleteAccount: (password: string) =>
			run(async () => {
				await invoke('accounts_delete', { password })
				await reload()
			}),
		setServer: (url: string) =>
			run(async () => {
				await invoke('accounts_set_server', { url })
				await reload()
			}),
		/** Búsqueda por nombre; los errores (p. ej. demasiadas búsquedas) se devuelven al llamador. */
		search: (query: string) => invoke<{ users: SearchHit[] }>('accounts_search', { query }),
		stop: () => {
			if (timer) clearInterval(timer)
			started = false
		},
	}
}
