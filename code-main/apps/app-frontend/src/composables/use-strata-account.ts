import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { ref } from 'vue'

/**
 * La cuenta de Strata es la identidad P2P del launcher: no hay registro, contraseña ni
 * servicio. `id` es el código de amigo (la clave pública) y el perfil vive en este equipo.
 */
export interface StrataUser {
	id: string
	username: string
	bio: string
}
interface Social {
	friends: StrataUser[]
	incoming: StrataUser[]
	outgoing: StrataUser[]
	blocked: StrataUser[]
}
type P2pFriend = {
	id: string
	name: string
	status: 'outgoing' | 'incoming' | 'accepted' | 'blocked'
	synced: boolean
}
type P2pInfo = { id: string; name: string; bio: string }

const emptySocial = (): Social => ({ friends: [], incoming: [], outgoing: [], blocked: [] })
const user = ref<StrataUser | null>(null)
const social = ref<Social>(emptySocial())
const error = ref('')
const busy = ref(false)
let listening = false

const message = (cause: unknown) =>
	typeof cause === 'string'
		? cause
		: cause instanceof Error
			? cause.message
			: 'No se pudo completar la acción.'

async function load() {
	const [info, friends] = await Promise.all([
		invoke<P2pInfo>('p2p_info'),
		invoke<P2pFriend[]>('p2p_friends'),
	])
	user.value = { id: info.id, username: info.name, bio: info.bio }
	const next = emptySocial()
	for (const friend of friends) {
		const member = { id: friend.id, username: friend.name, bio: '' }
		if (friend.status === 'accepted') next.friends.push(member)
		else if (friend.status === 'incoming') next.incoming.push(member)
		else if (friend.status === 'outgoing') next.outgoing.push(member)
		else next.blocked.push(member)
	}
	social.value = next
}

async function ensureListening() {
	if (listening) return
	listening = true
	try {
		await listen('p2p-event', () => void load().catch(() => {}))
	} catch {
		listening = false
	}
}

export function useStrataAccount() {
	async function run(action: () => Promise<void>) {
		if (busy.value) return
		busy.value = true
		error.value = ''
		try {
			await action()
		} catch (cause) {
			error.value = message(cause)
		} finally {
			busy.value = false
		}
	}

	void ensureListening()
	void load().catch((cause) => {
		error.value = message(cause)
	})

	return {
		user,
		social,
		error,
		busy,
		saveProfile: (name: string, bio: string) =>
			run(async () => {
				if (name.trim() !== user.value?.username) await invoke('p2p_set_name', { name })
				if (bio !== user.value?.bio) await invoke('p2p_set_bio', { bio })
				await load()
			}),
		refresh: () => run(load),
		/** `friends/request` recibe un código de amigo; el resto, el id de un amigo existente. */
		socialAction: (action: string, value: string) =>
			run(async () => {
				if (action === 'friends/request') await invoke('p2p_add_friend', { code: value })
				else if (action === 'friends/accept') await invoke('p2p_accept_friend', { id: value })
				else if (action === 'friends/remove') await invoke('p2p_remove_friend', { id: value })
				else if (action === 'block') await invoke('p2p_block_friend', { id: value, blocked: true })
				else if (action === 'unblock')
					await invoke('p2p_block_friend', { id: value, blocked: false })
				await load()
			}),
	}
}
