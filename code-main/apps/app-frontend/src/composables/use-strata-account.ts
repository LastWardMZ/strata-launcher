import { invoke } from '@tauri-apps/api/core'
import { ref } from 'vue'

/**
 * Identidad P2P del launcher (nombre, código y biografía). Los amigos y la privacidad no viven
 * aquí: los gestiona la cuenta de Modrinth conectada. `id` es el código de partidas (la clave
 * pública) y el perfil se guarda solo en este equipo.
 */
export interface StrataUser {
	id: string
	username: string
	bio: string
}
type P2pInfo = { id: string; name: string; bio: string }

const user = ref<StrataUser | null>(null)
const error = ref('')
const busy = ref(false)

const message = (cause: unknown) =>
	typeof cause === 'string'
		? cause
		: cause instanceof Error
			? cause.message
			: 'No se pudo completar la acción.'

async function load() {
	const info = await invoke<P2pInfo>('p2p_info')
	user.value = { id: info.id, username: info.name, bio: info.bio }
}

export function useStrataAccount() {
	void load().catch((cause) => {
		error.value = message(cause)
	})

	return {
		user,
		error,
		busy,
		saveProfile: async (name: string, bio: string) => {
			if (busy.value) return
			busy.value = true
			error.value = ''
			try {
				if (name.trim() !== user.value?.username) await invoke('p2p_set_name', { name })
				if (bio !== user.value?.bio) await invoke('p2p_set_bio', { bio })
				await load()
			} catch (cause) {
				error.value = message(cause)
			} finally {
				busy.value = false
			}
		},
	}
}
