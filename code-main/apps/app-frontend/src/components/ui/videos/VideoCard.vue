<script setup lang="ts">
import { EditIcon, FolderOpenIcon, PlayIcon, TrashIcon, VideoIcon } from '@modrinth/assets'
import { IconButton, useFormatDateTime } from '@modrinth/ui'
import { onBeforeUnmount, onMounted, ref } from 'vue'

import {
	formatClock,
	formatFileSize,
	formatResolution,
	recording_thumbnail,
	thumbnailUrl,
	type VideoItem,
} from '@/helpers/recording'

const props = defineProps<{ video: VideoItem; showInstance: boolean }>()
const emit = defineEmits<{
	(e: 'play' | 'trim' | 'reveal' | 'delete'): void
}>()

const card = ref<HTMLElement>()
const poster = ref<string | null>(null)
const posterFailed = ref(false)
const formatDate = useFormatDateTime({ dateStyle: 'medium', timeStyle: 'short' })
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

function activate(event: KeyboardEvent) {
	if (event.target !== event.currentTarget || (event.key !== 'Enter' && event.key !== ' ')) return
	event.preventDefault()
	emit('play')
}
</script>

<template>
	<article
		ref="card"
		role="button"
		tabindex="0"
		class="group relative isolate aspect-video min-w-0 cursor-pointer overflow-hidden rounded-xl border border-solid border-surface-5 bg-surface-2 p-0 text-left shadow-sm transition-[filter] hover:brightness-110 focus-visible:outline focus-visible:outline-2 focus-visible:outline-brand"
		:aria-label="`Reproducir ${video.file_name}`"
		data-video-card
		@click="emit('play')"
		@keydown="activate"
	>
		<img
			v-if="poster"
			:src="poster"
			:alt="video.file_name"
			loading="lazy"
			decoding="async"
			draggable="false"
			class="absolute inset-0 z-[1] h-full w-full object-cover"
			@error="((poster = null), (posterFailed = true))"
		/>
		<div
			class="absolute inset-0 flex items-center justify-center bg-button-bg"
			:class="{ 'animate-pulse': !poster && !posterFailed }"
			aria-hidden="true"
		>
			<VideoIcon v-if="!poster" class="size-10 text-secondary opacity-60" />
		</div>

		<div
			class="pointer-events-none absolute inset-0 z-[2] flex items-center justify-center opacity-0 transition-opacity duration-200 group-hover:opacity-100 group-focus-within:opacity-100"
		>
			<span class="flex size-14 items-center justify-center rounded-full bg-surface-2 text-contrast shadow">
				<PlayIcon class="size-7" />
			</span>
		</div>

		<span
			v-if="video.duration !== null"
			class="absolute right-2 top-2 z-[3] rounded-md bg-surface-2 px-1.5 py-0.5 text-xs font-semibold tabular-nums text-contrast"
		>
			{{ formatClock(video.duration) }}
		</span>

		<div
			class="absolute inset-x-0 bottom-0 z-[3] flex items-end justify-between gap-2 bg-gradient-to-t from-surface-1 to-transparent p-3 pt-[120px] text-contrast opacity-0 transition-opacity duration-200 group-hover:opacity-100 group-focus-within:opacity-100"
		>
			<div class="min-w-0">
				<div v-tooltip="video.file_name" class="truncate text-sm font-semibold">{{ video.file_name }}</div>
				<div class="truncate text-xs text-secondary">
					{{ showInstance ? `${video.instance_name} · ` : '' }}{{ formatDate(video.created_at) }}
				</div>
				<div class="truncate text-xs text-secondary">
					{{ [formatResolution(video), formatFileSize(video.size)].filter(Boolean).join(' · ') }}
				</div>
			</div>
			<div
				class="flex shrink-0 translate-y-1 gap-1 opacity-0 transition group-hover:translate-y-0 group-hover:opacity-100 group-focus-within:translate-y-0 group-focus-within:opacity-100"
				@click.stop
			>
				<IconButton
					v-tooltip="'Recortar'"
					label="Recortar"
					type="quiet"
					class="bg-surface-2 text-contrast hover:bg-surface-3"
					@click="emit('trim')"
				>
					<EditIcon />
				</IconButton>
				<IconButton
					v-tooltip="'Abrir ubicación'"
					label="Abrir ubicación"
					type="quiet"
					class="bg-surface-2 text-contrast hover:bg-surface-3"
					@click="emit('reveal')"
				>
					<FolderOpenIcon />
				</IconButton>
				<IconButton
					v-tooltip="'Eliminar'"
					label="Eliminar"
					type="quiet"
					class="bg-surface-2 text-contrast hover:!bg-red hover:!text-[var(--color-accent-contrast)]"
					@click="emit('delete')"
				>
					<TrashIcon />
				</IconButton>
			</div>
		</div>
	</article>
</template>
