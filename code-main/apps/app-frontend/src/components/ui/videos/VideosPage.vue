<script setup lang="ts">
import { ArrowLeftIcon, FolderOpenIcon, SearchIcon, SettingsIcon, TrashIcon, VideoIcon } from '@modrinth/assets'
import {
	Admonition,
	Button,
	Combobox,
	type ComboboxOption,
	ConfirmModal,
	EmptyState,
	injectNotificationManager,
	Input,
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
	recording_reveal,
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
