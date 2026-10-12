<script setup lang="ts">
import { ArrowLeftIcon, FolderOpenIcon, SearchIcon, SettingsIcon, TrashIcon, VideoIcon, XIcon } from '@modrinth/assets'
import {
	Admonition,
	Button,
	Combobox,
	type ComboboxOption,
	ConfirmModal,
	EmptyState,
	injectNotificationManager,
	Input,
	ProgressBar,
	ReadyTransition,
	useReadyState,
} from '@modrinth/ui'
import { useQuery, useQueryClient } from '@tanstack/vue-query'
import { useStorage } from '@vueuse/core'
import { computed, inject, onBeforeUnmount, onMounted, ref } from 'vue'

import {
	formatFileSize,
	onRecordingEvent,
	playbackUrl,
	recording_delete,
	recording_list,
	recording_merge_start,
	recording_reveal,
	recording_trim_cancel,
	recordingError,
	type VideoItem,
	videoKeys,
} from '@/helpers/recording'
import { appSettingsModalOpenRecordingKey } from '@/providers/app-settings-modal'

import VideoCard from './VideoCard.vue'
import VideoEditor from './VideoEditor.vue'
import VideoPlayer from './VideoPlayer.vue'

const props = defineProps<{ instanceId?: string }>()

type View = { mode: 'library' } | { mode: 'player'; video: VideoItem } | { mode: 'editor'; video: VideoItem }

const queryClient = useQueryClient()
const { addNotification, handleError } = injectNotificationManager()
const openRecordingSettings = inject(appSettingsModalOpenRecordingKey, () => {})

const view = ref<View>({ mode: 'library' })
const search = ref('')
const sort = useStorage<'newest' | 'oldest' | 'name' | 'size'>('videos-sort', 'newest')
const sortModel = computed<string>({
	get: () => sort.value,
	set: (value) => {
		sort.value = value as typeof sort.value
	},
})
const toDelete = ref<VideoItem | null>(null)
const deleteModal = ref<InstanceType<typeof ConfirmModal>>()
const playerUrl = ref('')

// Modo «Unir»: los vídeos se marcan en el orden en que se unirán.
const MAX_MERGE = 8
const merging = ref(false)
const mergeOrder = ref<string[]>([])
const mergeJob = ref<string | null>(null)
const mergeProgress = ref(0)
const mergeError = ref('')
const mergeBusy = computed(() => mergeJob.value !== null)
function startMerge() {
	merging.value = true
	mergeOrder.value = []
	mergeError.value = ''
}
function cancelSelection() {
	merging.value = false
	mergeOrder.value = []
}
function toggleMerge(video: VideoItem) {
	if (mergeOrder.value.includes(video.id)) mergeOrder.value = mergeOrder.value.filter((id) => id !== video.id)
	else if (mergeOrder.value.length < MAX_MERGE) mergeOrder.value = [...mergeOrder.value, video.id]
}
async function runMerge() {
	const chosen = mergeOrder.value
		.map((id) => videos.value.find((video) => video.id === id))
		.filter((video): video is VideoItem => !!video)
	if (chosen.length < 2) return
	mergeError.value = ''
	mergeProgress.value = 0
	try {
		mergeJob.value = await recording_merge_start(chosen.map((video) => video.path))
		cancelSelection()
	} catch (error) {
		mergeError.value = recordingError(error)
	}
}
async function cancelMergeJob() {
	if (mergeJob.value) await recording_trim_cancel(mergeJob.value).catch(() => {})
}
const playerError = ref('')

const videosQuery = useQuery({
	queryKey: videoKeys.list(),
	queryFn: recording_list,
	staleTime: 5_000,
})
const pending = useReadyState(videosQuery)

const sortOptions: ComboboxOption<string>[] = [
	{ value: 'newest', label: 'Más recientes' },
	{ value: 'oldest', label: 'Más antiguos' },
	{ value: 'name', label: 'Nombre' },
	{ value: 'size', label: 'Tamaño' },
]

const videos = computed(() =>
	(videosQuery.data.value ?? []).filter((video) => !props.instanceId || video.instance_id === props.instanceId),
)
const filtered = computed(() => {
	const query = search.value.trim().toLocaleLowerCase()
	const list = videos.value.filter(
		(video) =>
			!query ||
			video.file_name.toLocaleLowerCase().includes(query) ||
			video.instance_name.toLocaleLowerCase().includes(query),
	)
	return [...list].sort((a, b) => {
		if (sort.value === 'name') return a.file_name.localeCompare(b.file_name)
		if (sort.value === 'size') return b.size - a.size
		const difference = new Date(a.created_at).getTime() - new Date(b.created_at).getTime()
		return sort.value === 'oldest' ? difference : -difference
	})
})
const totalSize = computed(() => videos.value.reduce((sum, video) => sum + video.size, 0))
const error = computed(() => {
	const value: unknown = videosQuery.error.value
	return value ? recordingError(value) : ''
})
const missingMetadata = computed(() => videos.value.some((video) => video.duration === null))

async function openPlayer(video: VideoItem) {
	playerError.value = ''
	playerUrl.value = ''
	view.value = { mode: 'player', video }
	try {
		playerUrl.value = await playbackUrl(video.path)
	} catch (error) {
		playerError.value = recordingError(error)
	}
}

function openEditor(video: VideoItem) {
	view.value = { mode: 'editor', video }
}

async function reveal(video: VideoItem) {
	try {
		await recording_reveal(video.path)
	} catch (error) {
		handleError(error as Error)
	}
}

function askDelete(video: VideoItem) {
	toDelete.value = video
	deleteModal.value?.show()
}

async function confirmDelete() {
	const video = toDelete.value
	if (!video) return
	try {
		await recording_delete(video.path)
		if (view.value.mode !== 'library' && view.value.video.id === video.id) view.value = { mode: 'library' }
		await queryClient.invalidateQueries({ queryKey: videoKeys.all })
		addNotification({ type: 'success', title: 'Vídeo eliminado', text: video.file_name })
	} catch (error) {
		handleError(error as Error)
	} finally {
		toDelete.value = null
	}
}

async function backToLibrary() {
	view.value = { mode: 'library' }
	await queryClient.invalidateQueries({ queryKey: videoKeys.all })
}

async function onExported() {
	await backToLibrary()
}

let unlisten: (() => void) | undefined
onMounted(async () => {
	// La biblioteca se actualiza sola cuando termina una grabación o un recorte.
	unlisten = await onRecordingEvent((event) => {
		if (event.type === 'saved' || (event.type === 'trim' && event.state === 'done')) {
			void queryClient.invalidateQueries({ queryKey: videoKeys.all })
		}
		if (event.type === 'trim' && event.job === mergeJob.value) {
			if (event.state === 'progress') {
				mergeProgress.value = event.progress ?? 0
				return
			}
			mergeJob.value = null
			if (event.state === 'done') {
				addNotification({ type: 'success', title: 'Vídeos unidos', text: 'Se ha guardado un vídeo nuevo. Los originales no se han tocado.' })
			} else if (event.state === 'error') {
				mergeError.value = event.message ?? 'No se pudieron unir los vídeos.'
			} else if (event.state === 'canceled') {
				addNotification({ type: 'info', title: 'Unión cancelada', text: 'No se ha creado ningún archivo.' })
			}
		}
	})
})
onBeforeUnmount(() => unlisten?.())
</script>

<template>
	<ConfirmModal
		ref="deleteModal"
		title="Eliminar vídeo"
		:description="`¿Eliminar definitivamente «${toDelete?.file_name ?? ''}»? Esta acción no se puede deshacer.`"
		proceed-label="Eliminar"
		:markdown="false"
		@proceed="confirmDelete"
	/>

	<!-- Reproductor -->
	<div v-if="view.mode === 'player'" class="flex w-full flex-col gap-4">
		<div class="flex flex-wrap items-center justify-between gap-3">
			<div class="flex min-w-0 items-center gap-3">
				<Button @click="backToLibrary"><ArrowLeftIcon /> Biblioteca</Button>
				<h2 class="m-0 truncate text-xl font-bold text-contrast">{{ view.video.file_name }}</h2>
			</div>
			<div class="flex gap-2">
				<Button @click="openEditor(view.video)">Recortar</Button>
				<Button @click="reveal(view.video)"><FolderOpenIcon /> Abrir ubicación</Button>
				<Button color="red" type="outlined" @click="askDelete(view.video)"><TrashIcon /> Eliminar</Button>
			</div>
		</div>
		<Admonition v-if="playerError" type="critical" :body="playerError" />
		<VideoPlayer
			v-else-if="playerUrl"
			:src="playerUrl"
			:duration="view.video.duration"
			:has-audio="view.video.has_audio"
			autoplay
			@reveal="reveal(view.video)"
		/>
	</div>

	<!-- Editor de recorte -->
	<VideoEditor
		v-else-if="view.mode === 'editor'"
		:key="view.video.id"
		:video="view.video"
		@close="backToLibrary"
		@exported="onExported"
	/>

	<!-- Biblioteca -->
	<div
		v-else
		class="flex h-full w-full flex-col gap-3"
		:class="{ 'justify-center': videos.length === 0 && !error && !pending }"
	>
		<div v-if="videos.length > 0" class="flex flex-col gap-2">
			<div class="flex flex-wrap gap-2">
				<Input
					v-model="search"
					:icon="SearchIcon"
					type="text"
					placeholder="Buscar"
					clearable
					wrapper-class="min-w-[16rem] flex-1"
				/>
				<Button :disabled="merging || mergeBusy || videos.length < 2" @click="startMerge">Unir vídeos</Button>
				<Button @click="openRecordingSettings()"><SettingsIcon /> Ajustes de grabación</Button>
			</div>
			<div class="flex flex-wrap items-center gap-3">
				<Combobox
					v-model="sortModel"
					class="w-max"
					:options="sortOptions"
					:show-icon-in-selected="false"
					dropdown-min-width="160px"
				/>
				<span class="text-sm text-secondary">
					{{ videos.length }} {{ videos.length === 1 ? 'vídeo' : 'vídeos' }} · {{ formatFileSize(totalSize) }}
				</span>
			</div>
		</div>

		<ReadyTransition :pending="pending">
			<EmptyState v-if="error" type="error" heading="No se pudieron cargar los vídeos" :description="error">
				<template #actions>
					<Button type="outlined" @click="videosQuery.refetch()">Reintentar</Button>
				</template>
			</EmptyState>

			<EmptyState
				v-else-if="videos.length === 0"
				type="no-images"
				heading="Aún no hay vídeos"
				description="Activa la grabación en los ajustes y pulsa la tecla de grabación mientras juegas: tus partidas aparecerán aquí."
			>
				<template #actions>
					<Button color="brand" type="colored" @click="openRecordingSettings()">
						<SettingsIcon /> Ajustes de grabación
					</Button>
				</template>
			</EmptyState>

			<EmptyState
				v-else-if="filtered.length === 0"
				type="no-images"
				heading="No hay resultados"
				description="Prueba con otra búsqueda."
			/>

			<template v-else>
				<Admonition v-if="mergeError" type="critical" :body="mergeError" dismissible @dismiss="mergeError = ''" />
				<div
					v-if="mergeBusy"
					class="flex flex-col gap-2 rounded-2xl bg-surface-3 p-4 ring-1 ring-inset ring-surface-5"
				>
					<div class="flex items-center justify-between gap-3">
						<span class="font-semibold text-contrast">Uniendo vídeos… {{ Math.round(mergeProgress * 100) }}%</span>
						<Button type="outlined" @click="cancelMergeJob"><XIcon /> Cancelar</Button>
					</div>
					<ProgressBar :progress="mergeProgress" full-width label="Progreso de la unión" />
				</div>
				<div
					v-if="merging"
					class="flex flex-wrap items-center gap-3 rounded-2xl bg-surface-3 p-3 ring-1 ring-inset ring-surface-5"
					role="status"
				>
					<span class="min-w-0 flex-1 text-sm text-contrast">
						<template v-if="!mergeOrder.length">Marca los vídeos en el orden en que quieres unirlos (de 2 a {{ MAX_MERGE }}).</template>
						<template v-else>{{ mergeOrder.length }} marcados: se unirán en el orden de los números.</template>
					</span>
					<Button type="colored" color="brand" :disabled="mergeOrder.length < 2" @click="runMerge">Unir</Button>
					<Button type="outlined" @click="cancelSelection">Cancelar</Button>
				</div>
				<Admonition
					v-if="missingMetadata"
					type="info"
					body="Algunos vídeos no muestran duración ni miniatura porque falta el componente de grabación (FFmpeg). Se instala en Ajustes → Grabación."
				/>
				<div class="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-4">
					<VideoCard
						v-for="video in filtered"
						:key="video.id"
						:video="video"
						:show-instance="!instanceId"
						:selectable="merging"
						:order="mergeOrder.indexOf(video.id) + 1"
						@toggle="toggleMerge(video)"
						@play="openPlayer(video)"
						@trim="openEditor(video)"
						@reveal="reveal(video)"
						@delete="askDelete(video)"
					/>
				</div>
			</template>
		</ReadyTransition>
	</div>
</template>
