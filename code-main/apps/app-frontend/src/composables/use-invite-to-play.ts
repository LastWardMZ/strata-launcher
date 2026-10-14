import { invoke } from '@tauri-apps/api/core'

import { type LocalInstance, recipeOf } from '@/composables/use-instance-sharing'

type Share = { id: string; port: number }

/** Mundos ya compartidos desde esta ventana, por puerto: invitar dos veces no crea dos copias. */
const sharedByPort = new Map<number, string>()

function describe(cause: unknown) {
	if (typeof cause === 'string') return cause
	if (cause && typeof cause === 'object' && 'message' in cause) return String((cause as { message: unknown }).message)
	return 'No se pudo invitar. Inténtalo de nuevo.'
}

/**
 * «Invitar a jugar», como en Steam: con un clic abre el mundo actual a LAN (si no lo estaba), lo comparte con tus amigos y le
 * manda la invitación a este amigo. Devuelve el texto para enseñar al usuario; lanza con el motivo si no se pudo.
 */
export async function inviteToPlay(friendId: string, friendName: string): Promise<string> {
	try {
		const world = await invoke<{ instanceId: string; port: number }>('p2p_open_world_for_invite')
		const shares = await invoke<Share[]>('p2p_shares')
		let shareId = sharedByPort.get(world.port)
		if (!shareId || !shares.some((share) => share.id === shareId)) {
			const instances = await invoke<LocalInstance[]>('plugin:instance|instance_list').catch(() => [])
			const instance = instances.find((item) => item.id === world.instanceId)
			const share = await invoke<Share>('p2p_share_start', {
				kind: 'world',
				title: instance?.name ?? 'Mi mundo',
				gameVersion: instance?.game_version ?? '',
				port: world.port,
				// Con qué instancia se juega: así el amigo usa (o instala) la misma.
				instance: instance ? recipeOf(instance) : null,
				instanceId: world.instanceId,
			})
			shareId = share.id
			sharedByPort.set(world.port, shareId)
		}
		await invoke('p2p_invite', { friend: friendId, shareId })
		return `Invitación enviada a ${friendName}. Cuando acepte, entrará a tu mundo.`
	} catch (cause) {
		throw new Error(describe(cause))
	}
}
