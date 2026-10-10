import { invoke } from '@tauri-apps/api/core'

import { get_user_many } from '@/helpers/cache'
import { friends } from '@/helpers/friends'
import { get as getCredentials } from '@/helpers/mr_auth'
import { codeFromBio } from '@/helpers/p2p-link'

type PublicUser = { id: string; username: string; bio?: string | null }

/**
 * Pasa al P2P los amigos de Modrinth que han publicado su código de partidas en su biografía,
 * para que puedan chatear y jugar sin intercambiar códigos a mano.
 *
 * - Sin cuenta de Modrinth conectada, se envía una lista vacía (retira los amigos que venían de Modrinth).
 * - Si falla la red o la API, no se toca nada: un fallo temporal no debe borrar amigos.
 */
export async function syncModrinthFriendsToP2p(): Promise<void> {
	let userId: string | undefined
	try {
		userId = (await getCredentials())?.user_id
	} catch {
		return
	}
	if (!userId) {
		await invoke('p2p_sync_modrinth_friends', { peers: [] }).catch(() => {})
		return
	}

	let rows
	try {
		rows = await friends()
	} catch {
		return
	}
	const ids = rows
		.filter((row) => row.accepted)
		.map((row) => (row.id === userId ? row.friend_id : row.id))
	if (ids.length === 0) {
		await invoke('p2p_sync_modrinth_friends', { peers: [] }).catch(() => {})
		return
	}

	let users: PublicUser[]
	try {
		users = (await get_user_many(ids, 'must_revalidate')) as PublicUser[]
	} catch {
		return
	}
	const peers = users.flatMap((user) => {
		const code = codeFromBio(user.bio)
		return code ? [{ code, name: user.username }] : []
	})
	await invoke('p2p_sync_modrinth_friends', { peers }).catch(() => {})
}
