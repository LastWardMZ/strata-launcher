<script setup lang="ts">
import { CheckIcon, EditIcon, FolderOpenIcon, MoreVerticalIcon, PlayIcon, TrashIcon, VideoIcon } from '@modrinth/assets'
import { TeleportOverflowMenu, useFormatDateTime } from '@modrinth/ui'
import { onBeforeUnmount, onMounted, ref } from 'vue'

import {
	formatClock,
	formatFileSize,
	formatResolution,
	recording_thumbnail,
	thumbnailUrl,
	type VideoItem,
} from '@/helpers/recording'

const props = defineProps<{
	video: VideoItem
	showInstance: boolean
	/** Modo «Unir»: pulsar la tarjeta la marca en vez de reproducirla. */
	selectable?: boolean
	/** Posición (1, 2…) en la unión, o 0 si no está marcada. */
	order?: number
}>()
const emit = defineEmits<{
	(e: 'play' | 'trim' | 'reveal' | 'delete' | 'toggle'): void
}>()

const card = ref<HTMLElement>()
const poster = ref<string | null>(null)
const posterFailed = ref(false)
const formatDate = useFormatDateTime({ dateStyle: 'medium' })
let observer: IntersectionObserver | undefined
let requested = false

async function loadPoster() {
	if (requested) return
	requested = true
	try {
		const path = await recording_thumbnail(props.video.path)
		if (path) poster.value = thumbnailUrl(path)
		else posterFailed.value = true
	} catch {
		posterFailed.value = true
	}
}

// La miniatura se genera solo cuando la tarjeta se acerca a la pantalla.
onMounted(() => {
	if (!card.value || !('IntersectionObserver' in window)) {
		void loadPoster()
		return
	}
	observer = new IntersectionObserver(
		(entries) => {
			if (entries.some((entry) => entry.isIntersecting)) {
				observer?.disconnect()
				void loadPoster()
			}
		},
		{ rootMargin: '300px' },
	)
	observer.observe(card.value)
})
onBeforeUnmount(() => observer?.disconnect())
</script>

<template>
	<article ref="card" class="group flex min-w-0 flex-col gap-2" data-video-card>
		<button
			type="button"
			class="relative isolate m-0 aspect-video w-full min-w-0 cursor-pointer overflow-hidden rounded-xl border border-solid border-surface-5 bg-surface-2 p-0 text-left shadow-sm transition-[filter] hover:brightness-110 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-brand"
			:aria-label="selectable ? `${order ? 'Quitar' : 'Marcar'} ${video.file_name}` : `Reproducir ${video.file_name}`"
			:aria-pressed="selectable ? !!order : undefined"
			:class="{ '!border-brand ring-2 ring-brand': selectable && order }"
			@click="selectable ? emit('toggle') : emit('play')"
		>
			<img
				v-if="poster"
				:src="poster"
				:alt="''"
				loading="lazy"
				decoding="async"
				draggable="false"
				class="absolute inset-0 z-[1] h-full w-full object-cover"
				@error="((poster = null), (posterFailed = true))"
			/>
			<span
				class="absolute inset-0 flex items-center justify-center bg-button-bg"
				:class="{ 'animate-pulse': !poster && !posterFailed }"
				aria-hidden="true"
			>
				<VideoIcon v-if="!poster" class="size-10 text-secondary opacity-60" />
			</span>
			<span
				class="pointer-events-none absolute inset-0 z-[2] flex items-center justify-center opacity-0 transition-opacity duration-200 group-hover:opacity-100 group-focus-within:opacity-100 motion-reduce:transition-none"
				aria-hidden="true"
			>
				<span class="flex size-14 items-center justify-center rounded-full bg-surface-2 text-contrast shadow">
					<PlayIcon class="size-7" />
				</span>
			</span>
			<span
				v-if="selectable"
				class="absolute left-2 top-2 z-[3] flex size-7 items-center justify-center rounded-full border-2 border-solid text-sm font-bold"
				:class="order ? 'border-brand bg-brand text-[var(--color-accent-contrast)]' : 'border-contrast bg-surface-2 text-transparent'"
				aria-hidden="true"
			>
				<template v-if="order">{{ order }}</template>
				<CheckIcon v-else class="size-4" />
			</span>
			<span
				v-if="video.duration !== null"
				class="absolute right-2 top-2 z-[3] rounded-md bg-surface-2 px-1.5 py-0.5 text-xs font-semibold tabular-nums text-contrast"
			>
				{{ formatClock(video.duration) }}
			</span>
		</button>

		<div class="flex min-w-0 items-start gap-2 px-0.5">
			<div class="min-w-0 flex-1">
				<div v-tooltip="video.file_name" class="truncate text-sm font-semibold text-contrast">
					{{ video.file_name }}
				</div>
				<div class="truncate text-xs text-secondary">
					{{ showInstance ? `${video.instance_name} · ` : '' }}{{ formatDate(video.created_at) }}
				</div>
				<div class="truncate text-xs text-secondary">
					{{ [formatResolution(video), formatFileSize(video.size)].filter(Boolean).join(' · ') }}
				</div>
			</div>
			<TeleportOverflowMenu
				type="quiet"
				label="Más acciones"
				class="shrink-0"
				:options="[
					{ id: 'trim', label: 'Recortar', action: () => emit('trim') },
					{ id: 'reveal', label: 'Abrir ubicación', action: () => emit('reveal') },
					{ id: 'delete', label: 'Eliminar', action: () => emit('delete'), tone: 'red' },
				]"
			>
				<MoreVerticalIcon />
				<template #trim><EditIcon /> Recortar</template>
				<template #reveal><FolderOpenIcon /> Abrir ubicación</template>
				<template #delete><TrashIcon /> Eliminar</template>
			</TeleportOverflowMenu>
		</div>
	</article>
</template>
