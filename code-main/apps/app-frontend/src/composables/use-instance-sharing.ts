import { invoke } from '@tauri-apps/api/core'

import {
	install_create_instance,
	install_create_modpack_instance,
	install_job_get,
	installJobInstanceId,
	isInstallJobFinished,
} from '@/helpers/install'
import type { InstanceLink, InstanceLoader } from '@/helpers/types'

/** Instancia local tal y como la devuelve `instance_list` (solo lo que aquí hace falta). */
export type LocalInstance = {
	id: string
	name: string
	game_version: string
	loader?: InstanceLoader
	loader_version?: string | null
	link?: InstanceLink | null
	install_stage?: string
}

export type ModpackRef = {
	project_id: string
	version_id: string
	title: string
	icon_url: string | null
}

/** Lo que viaja por el chat para reconocer o reconstruir una instancia (ver `InstanceRecipe` en p2p.rs). */
export type InstanceRecipe = {
	name: string
	game_version: string
	loader: InstanceLoader
	loader_version: string | null
	modpack: ModpackRef | null
}

/** Cómo de buena es la coincidencia entre una receta y una instancia local. */
export type MatchLevel = 'modpack' | 'loader' | 'version'

export type InstanceMatch = {
	instance: LocalInstance
	level: MatchLevel
	/** Misma modpack pero otra versión: puede que los mods no coincidan del todo. */
	versionDiffers?: boolean
}

/** Identificadores del modpack de Modrinth del que viene una instancia, si lo hay. */
function modpackIds(link: InstanceLink | null | undefined) {
	if (!link) return null
	if (link.type === 'modrinth_modpack' || link.type === 'imported_modpack') {
		return link.project_id && link.version_id
			? { project_id: link.project_id, version_id: link.version_id }
			: null
	}
	if (link.type === 'server_project_modpack') {
		return link.content_project_id && link.content_version_id
			? { project_id: link.content_project_id, version_id: link.content_version_id }
			: null
	}
	return null
}

export function recipeOf(instance: LocalInstance): InstanceRecipe {
	const ids = modpackIds(instance.link)
	return {
		name: instance.name,
		game_version: instance.game_version,
		loader: instance.loader ?? 'vanilla',
		loader_version: instance.loader_version ?? null,
		modpack: ids ? { ...ids, title: instance.name, icon_url: null } : null,
	}
}

/**
 * Busca entre las instancias propias la que sirve para jugar con la receta de un amigo.
 * Con modpack solo vale la misma modpack (si no, los mods no coincidirían); sin él, vale la que tenga
 * la misma versión y cargador y, como último recurso, la misma versión de Minecraft.
 */
export function findMatchingInstance(
	instances: LocalInstance[],
	recipe: InstanceRecipe | null | undefined,
	gameVersion = '',
): InstanceMatch | null {
	const usable = instances.filter((i) => i.install_stage !== 'not_installed')
	if (recipe?.modpack) {
		const found = usable.find((i) => modpackIds(i.link)?.project_id === recipe.modpack?.project_id)
		if (!found) return null
		return {
			instance: found,
			level: 'modpack',
			versionDiffers: modpackIds(found.link)?.version_id !== recipe.modpack.version_id,
		}
	}
	const version = recipe?.game_version || gameVersion
	if (!version) return null
	if (recipe) {
		const found = usable.find(
			(i) => i.game_version === version && (i.loader ?? 'vanilla') === recipe.loader && !modpackIds(i.link),
		)
		if (found) return { instance: found, level: 'loader' }
		return null
	}
	const found = usable.find((i) => i.game_version === version)
	return found ? { instance: found, level: 'version' } : null
}

export function loaderLabel(loader: string) {
	const names: Record<string, string> = {
		vanilla: 'Vanilla',
		forge: 'Forge',
		fabric: 'Fabric',
		quilt: 'Quilt',
		neoforge: 'NeoForge',
	}
	return names[loader] ?? loader
}

export function recipeSummary(recipe: InstanceRecipe) {
	const parts = [`${loaderLabel(recipe.loader)} ${recipe.game_version}`]
	if (recipe.modpack) parts.push(`modpack ${recipe.modpack.title || recipe.name}`)
	return parts.join(' · ')
}

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))

/**
 * Instala la instancia de una receta (el mismo modpack de Modrinth o, sin modpack, versión y cargador)
 * y espera a que termine. Devuelve el id de la instancia nueva. El progreso se ve en el panel de tareas.
 */
export async function installRecipe(recipe: InstanceRecipe, timeoutMs = 30 * 60 * 1000) {
	const job = recipe.modpack
		? await install_create_modpack_instance({
				type: 'fromVersionId',
				project_id: recipe.modpack.project_id,
				version_id: recipe.modpack.version_id,
				title: recipe.modpack.title || recipe.name,
				icon_url: recipe.modpack.icon_url,
			})
		: await install_create_instance({
				name: recipe.name,
				gameVersion: recipe.game_version,
				loader: recipe.loader,
				loaderVersion: recipe.loader_version,
				iconPath: null,
			})

	const deadline = Date.now() + timeoutMs
	while (Date.now() < deadline) {
		const current = await install_job_get(job.job_id)
		if (isInstallJobFinished(current.status)) {
			if (current.status !== 'succeeded') {
				throw new Error(current.error?.message ?? 'No se pudo instalar la instancia.')
			}
			const id = installJobInstanceId(current)
			if (!id) throw new Error('La instancia se instaló pero no se pudo localizar.')
			return id
		}
		await sleep(1500)
	}
	throw new Error('La instalación está tardando demasiado. Mira el progreso en el panel de tareas.')
}

/** Lo que hace falta de una invitación a partida para unirse a ella. */
export type JoinableInvite = {
	share_id: string
	game_version: string
	/** Instancia con la que juega quien invita. */
	instance?: InstanceRecipe | null
}

/**
 * Se une a la partida de un amigo: abre el túnel P2P y entra con la instancia que corresponda. Si esa instancia ya está
 * abierta, el propio juego se conecta (sin reiniciarlo). Devuelve el texto para enseñar; lanza si no se pudo.
 */
export async function joinInvite(
	friendId: string,
	info: JoinableInvite,
	instances: LocalInstance[],
	fallback: LocalInstance | null = null,
	onConnected: (port: number) => void = () => {},
): Promise<string> {
	// La instancia de la invitación manda; `fallback` solo se usa con invitaciones antiguas.
	const match = findMatchingInstance(instances, info.instance, info.game_version)
	if (info.instance && !match) {
		throw new Error(`No tienes la instancia «${info.instance.name}». Pulsa «Instalar y unirme».`)
	}
	const instance = match?.instance ?? fallback
	const port = await invoke<number>('p2p_join', { friend: friendId, shareId: info.share_id })
	onConnected(port)
	if (!instance) {
		return `Conectado. En Minecraft añade el servidor 127.0.0.1:${port}, o elige una instancia para abrirlo directo.`
	}
	if (!match && info.game_version && instance.game_version !== info.game_version) {
		return `Conectado, pero tu instancia es ${instance.game_version} y la partida es ${info.game_version}. Usa una instancia ${info.game_version} y entra en 127.0.0.1:${port}.`
	}
	await invoke('plugin:worlds|start_join_server', { instanceId: instance.id, address: `127.0.0.1:${port}` })
	return `Entrando a la partida con «${instance.name}»…`
}
