<script setup lang="ts">
/**
 * Instalar un mapa de CurseForge en una instancia. Pasos: elegir instancia (o crear una nueva) y archivo → descargar y
 * comprobar → revisar (mundos, avisos de versión, ejecutables que se dejan fuera) → instalar → jugar.
 */
import { Admonition, Avatar, Button, Checkbox, Combobox, LoadingIndicator, useVIntl } from '@modrinth/ui'
import { openUrl } from '@tauri-apps/plugin-opener'
import { computed, ref } from 'vue'

import ModalWrapper from '@/components/ui/modal/ModalWrapper.vue'
import { installRecipe } from '@/composables/use-instance-sharing'
import { getInstanceIconUrl, list as listInstances, remove as removeInstance, run as runInstance } from '@/helpers/instance'
import {
	bestFile,
	compareVersions,
	discardPreparedMap,
	formatBytes,
	getMapFiles,
	installPreparedMap,
	type MapFile,
	type MapSummary,
	mapErrorMessage,
	newestVersion,
	type Prepared,
	prepareMapInstall,
	supportsQuickPlayWorld,
} from '@/helpers/maps'
import { get_game_versions } from '@/helpers/tags'
import { start_join_singleplayer_world } from '@/helpers/worlds'

import { messages } from './messages'

type Phase = 'loading' | 'choose' | 'external' | 'working' | 'review' | 'installing' | 'done' | 'error'
type Instance = { id: string; name: string; game_version: string; loader?: string; icon_path?: string }

const NEW = '__new__'
const MAX_BLOCKED_SHOWN = 8

const { formatMessage } = useVIntl()
const modal = ref<InstanceType<typeof ModalWrapper> | null>(null)

const map = ref<Pick<MapSummary, 'id' | 'name' | 'url'> | null>(null)
const phase = ref<Phase>('loading')
const error = ref('')
const instances = ref<Instance[]>([])
const orderedVersions = ref<string[]>([])
const files = ref<MapFile[]>([])
const selectedInstanceId = ref('')
const selectedFileId = ref<number | null>(null)
const prepared = ref<Prepared | null>(null)
const selectedWorlds = ref<string[]>([])
const confirmedNewer = ref(false)
const workingText = ref('')
const targetInstanceId = ref('')
/** Instancia creada solo para este mapa: si el usuario cancela antes de instalar, se borra. */
let createdInstanceId: string | null = null
const installedFolder = ref('')
const blockedFiles = ref<string[]>([])

const downloadable = computed(() =>
	files.value.filter((file) => file.available && file.downloadUrl).sort((a, b) => (b.date ?? '').localeCompare(a.date ?? '')),
)
const selectedFile = computed(() => downloadable.value.find((file) => file.id === selectedFileId.value) ?? null)
const fileVersion = computed(() => (selectedFile.value ? newestVersion(selectedFile.value.gameVersions, orderedVersions.value) : null))
const newInstanceVersion = computed(() => fileVersion.value ?? orderedVersions.value.find((version) => /^\d+\.\d+(\.\d+)?$/.test(version)) ?? '')

const instanceChoices = computed(() => {
	const wanted = selectedFile.value?.gameVersions ?? []
	return instances.value
		.map((instance) => ({ instance, recommended: wanted.includes(instance.game_version) }))
		.sort((a, b) => Number(b.recommended) - Number(a.recommended) || a.instance.name.localeCompare(b.instance.name))
})
const fileOptions = computed(() =>
	downloadable.value.map((file) => ({
		value: file.id,
		label: `${file.name} · ${file.gameVersions.join(', ') || '?'} · ${formatBytes(file.size)}`,
	})),
)

const targetInstance = computed(() => instances.value.find((instance) => instance.id === targetInstanceId.value) ?? null)
const worlds = computed(() => prepared.value?.inspection.worlds ?? [])
/** La versión real del mapa: la que dice su level.dat y, si no, la del archivo. */
const mapVersion = computed(() => {
	const chosen = worlds.value.filter((world) => selectedWorlds.value.includes(world.path))
	return chosen.find((world) => world.game_version)?.game_version ?? fileVersion.value
})
const comparison = computed(() =>
	mapVersion.value && targetInstance.value ? compareVersions(mapVersion.value, targetInstance.value.game_version, orderedVersions.value) : 'unknown',
)
const canInstall = computed(() => selectedWorlds.value.length > 0 && (comparison.value !== 'newer' || confirmedNewer.value))
const canQuickPlay = computed(() => !!targetInstance.value && supportsQuickPlayWorld(targetInstance.value.game_version, orderedVersions.value))
const blockedText = computed(() => {
	const shown = blockedFiles.value.slice(0, MAX_BLOCKED_SHOWN).join(', ')
	return blockedFiles.value.length > MAX_BLOCKED_SHOWN ? `${shown}…` : shown
})
const header = computed(() => formatMessage(messages.installTitle, { name: map.value?.name ?? '' }))

function reset() {
	error.value = ''
	prepared.value = null
	selectedWorlds.value = []
	confirmedNewer.value = false
	targetInstanceId.value = ''
	installedFolder.value = ''
	blockedFiles.value = []
	createdInstanceId = null
}

function chooseInstance(id: string) {
	selectedInstanceId.value = id
	// El archivo que mejor encaja con esa instancia (el usuario aún puede cambiarlo).
	const instance = instances.value.find((candidate) => candidate.id === id)
	const best = bestFile(downloadable.value, instance?.game_version)
	if (best) selectedFileId.value = best.id
}

/** Abre el diálogo para instalar un mapa. `preferredInstanceId` preselecciona una instancia (p. ej. desde la ficha de una). */
async function show(target: Pick<MapSummary, 'id' | 'name' | 'url'>, preferredInstanceId?: string) {
	map.value = target
	reset()
	phase.value = 'loading'
	files.value = []
	modal.value?.show()
	try {
		const [instanceList, versionList, fileList] = await Promise.all([listInstances(), get_game_versions(), getMapFiles(target.id)])
		instances.value = instanceList as unknown as Instance[]
		orderedVersions.value = (versionList as { version: string }[]).map((entry) => entry.version)
		files.value = fileList.files
	} catch (cause) {
		error.value = mapErrorMessage(cause)
		phase.value = 'error'
		return
	}
	if (!downloadable.value.length) {
		phase.value = files.value.length ? 'external' : 'error'
		if (!files.value.length) error.value = formatMessage(messages.noFiles)
		return
	}
	const preferred = preferredInstanceId && instances.value.some((i) => i.id === preferredInstanceId) ? preferredInstanceId : ''
	selectedFileId.value = bestFile(downloadable.value)?.id ?? null
	const recommended = instanceChoices.value.find((choice) => choice.recommended)?.instance.id
	chooseInstance(preferred || recommended || (instances.value.length ? '' : NEW))
	phase.value = 'choose'
}

async function removeCreatedInstance() {
	if (!createdInstanceId) return
	const id = createdInstanceId
	createdInstanceId = null
	await removeInstance(id).catch(() => {})
}

async function start() {
	const file = selectedFile.value
	if (!file || !map.value || !selectedInstanceId.value) return
	error.value = ''
	phase.value = 'working'
	try {
		let instanceId = selectedInstanceId.value
		if (instanceId === NEW) {
			workingText.value = formatMessage(messages.creatingInstance)
			instanceId = await installRecipe({
				name: map.value.name.slice(0, 40),
				game_version: newInstanceVersion.value,
				loader: 'vanilla',
				loader_version: null,
				modpack: null,
			})
			createdInstanceId = instanceId
			instances.value = (await listInstances()) as unknown as Instance[]
		}
		workingText.value = formatMessage(messages.downloading)
		targetInstanceId.value = instanceId
		prepared.value = await prepareMapInstall(instanceId, file)
		selectedWorlds.value = prepared.value.inspection.worlds.length === 1 ? [prepared.value.inspection.worlds[0].path] : []
		blockedFiles.value = prepared.value.inspection.blocked_files
		phase.value = 'review'
	} catch (cause) {
		await removeCreatedInstance()
		error.value = mapErrorMessage(cause)
		phase.value = 'error'
	}
}

async function install() {
	if (!prepared.value || !map.value || !canInstall.value) return
	phase.value = 'installing'
	try {
		const report = await installPreparedMap(targetInstanceId.value, prepared.value.token, selectedWorlds.value, map.value.id)
		prepared.value = null
		createdInstanceId = null
		installedFolder.value = report.installed[0]?.folder ?? ''
		blockedFiles.value = report.blocked_files
		phase.value = 'done'
	} catch (cause) {
		error.value = mapErrorMessage(cause)
		phase.value = 'error'
	}
}

/** Se cierra o se cancela antes de instalar: se borra el ZIP descargado y la instancia creada para esto. */
async function abandon() {
	if (prepared.value) {
		await discardPreparedMap(prepared.value.token).catch(() => {})
		prepared.value = null
	}
	await removeCreatedInstance()
}

async function cancel() {
	await abandon()
	modal.value?.hide()
}

async function play() {
	const instanceId = targetInstanceId.value
	modal.value?.hide()
	try {
		if (canQuickPlay.value && installedFolder.value) await start_join_singleplayer_world(instanceId, installedFolder.value)
		else await runInstance(instanceId)
	} catch (cause) {
		// El arranque informa de sus propios errores; aquí solo se evita una excepción sin atender.
		console.error(cause)
	}
}

function toggleWorld(path: string, checked: boolean) {
	selectedWorlds.value = checked ? [...selectedWorlds.value, path] : selectedWorlds.value.filter((item) => item !== path)
}

defineExpose({ show })
</script>

<template>
	<ModalWrapper ref="modal" :header="header" :on-hide="abandon">
		<div class="flex w-[34rem] max-w-full flex-col gap-4">
			<div v-if="phase === 'loading'" class="flex items-center gap-3 text-secondary">
				<LoadingIndicator />
				{{ formatMessage(messages.loadingFiles) }}
			</div>

			<!-- Sin descarga directa: el autor no permite apps de terceros -->
			<template v-else-if="phase === 'external'">
				<Admonition type="info" :body="formatMessage(messages.noDirectDownload)" />
				<div class="flex justify-end gap-2">
					<Button @click="modal?.hide()">{{ formatMessage(messages.close) }}</Button>
					<Button v-if="map?.url" type="colored" color="brand" @click="openUrl(map.url)">{{ formatMessage(messages.downloadOnCurseForge) }}</Button>
				</div>
			</template>

			<!-- 1. Instancia y archivo -->
			<template v-else-if="phase === 'choose'">
				<div class="flex flex-col gap-2">
					<h3 class="m-0 text-base font-semibold text-contrast">{{ formatMessage(messages.chooseInstance) }}</h3>
					<div role="radiogroup" :aria-label="formatMessage(messages.chooseInstance)" class="flex max-h-72 flex-col gap-1.5 overflow-y-auto">
						<button
							v-for="choice in instanceChoices"
							:key="choice.instance.id"
							type="button"
							role="radio"
							:aria-checked="selectedInstanceId === choice.instance.id"
							class="flex w-full cursor-pointer items-center gap-3 rounded-xl border border-solid bg-surface-3 p-2.5 text-left focus-visible:outline focus-visible:outline-2 focus-visible:outline-brand"
							:class="selectedInstanceId === choice.instance.id ? 'border-brand' : 'border-surface-5'"
							@click="chooseInstance(choice.instance.id)"
						>
							<Avatar :src="getInstanceIconUrl(choice.instance.icon_path) ?? undefined" size="40px" />
							<span class="min-w-0 flex-1">
								<span class="block truncate font-semibold text-contrast">{{ choice.instance.name }}</span>
								<span class="block truncate text-xs text-secondary">{{ choice.instance.loader ?? 'vanilla' }} · Minecraft {{ choice.instance.game_version }}</span>
							</span>
							<span v-if="choice.recommended" class="shrink-0 rounded-full bg-brand-highlight px-2 py-0.5 text-xs font-semibold text-brand">
								{{ formatMessage(messages.recommended) }}
							</span>
						</button>
						<button
							type="button"
							role="radio"
							:aria-checked="selectedInstanceId === NEW"
							class="flex w-full cursor-pointer items-center gap-3 rounded-xl border border-solid bg-surface-3 p-2.5 text-left focus-visible:outline focus-visible:outline-2 focus-visible:outline-brand"
							:class="selectedInstanceId === NEW ? 'border-brand' : 'border-dashed border-surface-5'"
							@click="chooseInstance(NEW)"
						>
							<span class="flex size-10 shrink-0 items-center justify-center rounded-lg bg-surface-4 text-xl text-secondary" aria-hidden="true">+</span>
							<span class="min-w-0 flex-1">
								<span class="block font-semibold text-contrast">{{ formatMessage(messages.newInstance) }}</span>
								<span class="block text-xs text-secondary">{{ formatMessage(messages.newInstanceHint, { version: newInstanceVersion }) }}</span>
							</span>
						</button>
					</div>
					<p v-if="!instances.length" class="m-0 text-sm text-secondary">{{ formatMessage(messages.noInstances) }}</p>
				</div>
				<div v-if="fileOptions.length > 1" class="flex flex-col gap-1.5">
					<h3 class="m-0 text-base font-semibold text-contrast">{{ formatMessage(messages.chooseFile) }}</h3>
					<Combobox v-model="selectedFileId" :options="fileOptions" trigger-type="base" :aria-label="formatMessage(messages.chooseFile)" />
				</div>
				<div class="flex justify-end gap-2">
					<Button @click="modal?.hide()">{{ formatMessage(messages.cancel) }}</Button>
					<Button type="colored" color="brand" :disabled="!selectedInstanceId || !selectedFile" @click="start">
						{{ formatMessage(messages.downloadAndInstall) }}
					</Button>
				</div>
			</template>

			<!-- 2. Descargando -->
			<div v-else-if="phase === 'working' || phase === 'installing'" class="flex flex-col gap-2" role="status">
				<div class="flex items-center gap-3 text-contrast">
					<LoadingIndicator />
					{{ phase === 'installing' ? formatMessage(messages.installing) : workingText }}
				</div>
				<p class="m-0 text-sm text-secondary">{{ formatMessage(messages.progressHint) }}</p>
			</div>

			<!-- 3. Revisar -->
			<template v-else-if="phase === 'review'">
				<h3 class="m-0 text-base font-semibold text-contrast">{{ formatMessage(messages.reviewTitle) }}</h3>
				<p v-if="targetInstance" class="m-0 text-sm text-secondary">{{ formatMessage(messages.installsInto, { instance: targetInstance.name }) }}</p>

				<div v-if="worlds.length > 1" class="flex flex-col gap-1.5">
					<h4 class="m-0 text-sm font-semibold text-contrast">{{ formatMessage(messages.worldsTitle) }}</h4>
					<p class="m-0 text-sm text-secondary">{{ formatMessage(messages.worldsHint) }}</p>
					<Checkbox
						v-for="world in worlds"
						:key="world.path"
						:label="world.level_name || world.folder"
						:description="formatMessage(messages.worldDetails, { size: formatBytes(world.size), files: world.files })"
						:model-value="selectedWorlds.includes(world.path)"
						@update:model-value="(checked: boolean) => toggleWorld(world.path, checked)"
					/>
				</div>

				<Admonition
					v-if="comparison === 'newer' && mapVersion && targetInstance"
					type="warning"
					:body="formatMessage(messages.newerWarning, { mapVersion, instanceVersion: targetInstance.game_version })"
				>
					<template #actions>
						<Checkbox v-model="confirmedNewer" :label="formatMessage(messages.newerConfirm)" />
					</template>
				</Admonition>
				<Admonition
					v-else-if="comparison === 'older' && mapVersion && targetInstance"
					type="info"
					:body="formatMessage(messages.olderNotice, { mapVersion, instanceVersion: targetInstance.game_version })"
				/>
				<Admonition v-else-if="comparison === 'unknown'" type="info" :body="formatMessage(messages.unknownVersionNotice)" />

				<Admonition v-if="blockedFiles.length" type="warning" :header="formatMessage(messages.blockedTitle)" :body="formatMessage(messages.blockedText, { files: blockedText })" />

				<div class="flex justify-end gap-2">
					<Button @click="cancel">{{ formatMessage(messages.cancel) }}</Button>
					<Button type="colored" color="brand" :disabled="!canInstall" @click="install">{{ formatMessage(messages.install_) }}</Button>
				</div>
			</template>

			<!-- 4. Listo -->
			<template v-else-if="phase === 'done'">
				<Admonition type="success" :header="formatMessage(messages.doneTitle)" :body="formatMessage(messages.doneText, { instance: targetInstance?.name ?? '' })" />
				<Admonition v-if="blockedFiles.length" type="warning" :header="formatMessage(messages.blockedTitle)" :body="formatMessage(messages.blockedText, { files: blockedText })" />
				<div class="flex justify-end gap-2">
					<Button @click="modal?.hide()">{{ formatMessage(messages.close) }}</Button>
					<Button type="colored" color="brand" @click="play">{{ canQuickPlay ? formatMessage(messages.play) : formatMessage(messages.openInstance) }}</Button>
				</div>
			</template>

			<!-- Error -->
			<template v-else>
				<Admonition type="critical" :header="formatMessage(messages.failedTitle)" :body="error" />
				<div class="flex justify-end"><Button @click="modal?.hide()">{{ formatMessage(messages.close) }}</Button></div>
			</template>
		</div>
	</ModalWrapper>
</template>
