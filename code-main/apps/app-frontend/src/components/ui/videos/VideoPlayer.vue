<script setup lang="ts">
import { FolderOpenIcon, PauseIcon, PlayIcon } from '@modrinth/assets'
import { Button, IconButton } from '@modrinth/ui'
import { computed, onBeforeUnmount, ref, watch } from 'vue'

import { formatClock } from '@/helpers/recording'

const props = withDefaults(
	defineProps<{
		src: string
		/** Duración conocida (de ffprobe): se usa hasta que el vídeo informa de la suya. */
		duration?: number | null
		/** `false` si se sabe que el vídeo no tiene pista de audio (se oculta el volumen). */
		hasAudio?: boolean | null
		autoplay?: boolean
		/** Limita la altura del vídeo (editor), para que los controles de debajo quepan sin desplazarse. */
		compact?: boolean
	}>(),
	{ duration: null, hasAudio: null, autoplay: false, compact: false },
)

const emit = defineEmits<{
	(e: 'loaded', duration: number): void
	(e: 'time', seconds: number): void
	(e: 'ended'): void
	(e: 'reveal'): void
}>()

const video = ref<HTMLVideoElement>()
const bar = ref<HTMLElement>()
const currentTime = ref(0)
const videoDuration = ref<number | null>(null)
const paused = ref(true)
const volume = ref(1)
const muted = ref(false)
const failed = ref(false)
const scrubbing = ref(false)
const buffering = ref(true)

const total = computed(() => videoDuration.value ?? props.duration ?? 0)
const progress = computed(() => (total.value > 0 ? Math.min(1, currentTime.value / total.value) : 0))
const showVolume = computed(() => props.hasAudio !== false)

watch(
	() => props.src,
	() => {
		failed.value = false
		buffering.value = true
		currentTime.value = 0
		videoDuration.value = null
	},
)

function onLoaded() {
	const element = video.value
	if (!element) return
	buffering.value = false
	// Los MP4 fragmentados pueden no informar de la duración: se usa la de ffprobe.
	if (Number.isFinite(element.duration) && element.duration > 0) {
		videoDuration.value = element.duration
		emit('loaded', element.duration)
	} else if (props.duration) {
		emit('loaded', props.duration)
	}
	if (props.autoplay) void play()
}

function onTime() {
	if (!video.value || scrubbing.value) return
	currentTime.value = video.value.currentTime
	emit('time', currentTime.value)
}

async function play() {
	try {
		await video.value?.play()
	} catch {
		// La reproducción puede interrumpirse al buscar o cambiar de vídeo: no es un error.
	}
}

function pause() {
	video.value?.pause()
}

function toggle() {
	if (failed.value) return
	if (video.value?.paused) void play()
	else pause()
}

function seek(seconds: number) {
	const element = video.value
	if (!element) return
	const limit = total.value > 0 ? total.value : seconds
	const next = Math.max(0, Math.min(limit, seconds))
	element.currentTime = next
	currentTime.value = next
	emit('time', next)
}

function skip(delta: number) {
	seek((video.value?.currentTime ?? currentTime.value) + delta)
}

function timeFromPointer(event: PointerEvent) {
	const rect = bar.value?.getBoundingClientRect()
	if (!rect || rect.width === 0) return 0
	return Math.max(0, Math.min(1, (event.clientX - rect.left) / rect.width)) * total.value
}

function startScrub(event: PointerEvent) {
	if (total.value <= 0) return
	scrubbing.value = true
	;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
	currentTime.value = timeFromPointer(event)
}

function moveScrub(event: PointerEvent) {
	if (scrubbing.value) currentTime.value = timeFromPointer(event)
}

function endScrub(event: PointerEvent) {
	if (!scrubbing.value) return
	scrubbing.value = false
	seek(timeFromPointer(event))
}

function onBarKey(event: KeyboardEvent) {
	const step = event.shiftKey ? 10 : 5
	if (event.key === 'ArrowRight') seek(currentTime.value + step)
	else if (event.key === 'ArrowLeft') seek(currentTime.value - step)
	else if (event.key === 'Home') seek(0)
	else if (event.key === 'End') seek(total.value)
	else return
	event.preventDefault()
}

function setVolume(event: Event) {
	const value = Number((event.target as HTMLInputElement).value)
	volume.value = value
	muted.value = value === 0
	if (video.value) {
		video.value.volume = value
		video.value.muted = value === 0
	}
}

function toggleMute() {
	muted.value = !muted.value
	if (video.value) video.value.muted = muted.value
	if (!muted.value && volume.value === 0) {
		volume.value = 0.5
		if (video.value) video.value.volume = 0.5
	}
}

onBeforeUnmount(() => video.value?.pause())

defineExpose({
	play,
	pause,
	toggle,
	seek,
	currentTime,
	paused,
	duration: total,
})
</script>

<template>
	<div class="flex w-full flex-col gap-3">
		<div
			class="relative mx-auto flex aspect-video w-full items-center justify-center overflow-hidden rounded-xl border border-solid border-surface-5 bg-black"
			:style="compact ? { width: 'min(100%, calc(42vh * 16 / 9))' } : undefined"
		>
			<video
				ref="video"
				:src="src"
				class="h-full w-full cursor-pointer object-contain"
				preload="metadata"
				playsinline
				@click="toggle"
				@loadedmetadata="onLoaded"
				@timeupdate="onTime"
				@play="paused = false"
				@pause="paused = true"
				@ended="emit('ended')"
				@waiting="buffering = true"
				@playing="buffering = false"
				@canplay="buffering = false"
				@error="failed = true"
			/>
			<div
				v-if="failed"
				class="absolute inset-0 flex flex-col items-center justify-center gap-3 bg-surface-2 p-6 text-center"
			>
				<p class="m-0 text-lg font-semibold text-contrast">No se puede reproducir este vídeo</p>
				<p class="m-0 max-w-md text-secondary">
					El archivo puede estar dañado o tener un formato que el reproductor no admite. Puedes abrirlo en su
					carpeta.
				</p>
				<Button @click="emit('reveal')"><FolderOpenIcon /> Abrir ubicación</Button>
			</div>
			<div
				v-else-if="buffering"
				class="pointer-events-none absolute inset-0 flex items-center justify-center"
				aria-hidden="true"
			>
				<div class="size-8 animate-spin rounded-full border-4 border-solid border-surface-5 border-t-brand" />
			</div>
			<button
				v-if="!failed && paused && !buffering"
				type="button"
				aria-label="Reproducir"
				class="absolute flex size-16 cursor-pointer items-center justify-center rounded-full border-0 bg-surface-2 text-contrast opacity-90 transition-all hover:scale-105 hover:opacity-100"
				@click="toggle"
			>
				<PlayIcon class="size-8" />
			</button>
		</div>

		<div class="flex flex-wrap items-center gap-2">
			<IconButton :label="paused ? 'Reproducir' : 'Pausar'" :disabled="failed" @click="toggle">
				<PlayIcon v-if="paused" />
				<PauseIcon v-else />
			</IconButton>
			<Button type="quiet" :disabled="failed" aria-label="Retroceder 10 segundos" @click="skip(-10)"
				>−10 s</Button
			>
			<Button type="quiet" :disabled="failed" aria-label="Avanzar 10 segundos" @click="skip(10)"
				>+10 s</Button
			>
			<span class="min-w-[7.5rem] text-sm tabular-nums text-secondary">
				{{ formatClock(currentTime) }} / {{ formatClock(total || null) }}
			</span>
			<div
				ref="bar"
				role="slider"
				tabindex="0"
				aria-label="Posición del vídeo"
				:aria-valuemin="0"
				:aria-valuemax="Math.round(total)"
				:aria-valuenow="Math.round(currentTime)"
				:aria-valuetext="`${formatClock(currentTime)} de ${formatClock(total || null)}`"
				class="group relative h-6 min-w-[8rem] flex-1 cursor-pointer touch-none"
				@pointerdown="startScrub"
				@pointermove="moveScrub"
				@pointerup="endScrub"
				@pointercancel="scrubbing = false"
				@keydown="onBarKey"
			>
				<div class="absolute inset-x-0 top-1/2 h-1.5 -translate-y-1/2 rounded-full bg-surface-5">
					<div class="h-full rounded-full bg-brand" :style="{ width: `${progress * 100}%` }" />
				</div>
				<div
					class="absolute top-1/2 size-3.5 -translate-x-1/2 -translate-y-1/2 rounded-full bg-contrast opacity-0 shadow transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100"
					:class="{ '!opacity-100': scrubbing }"
					:style="{ left: `${progress * 100}%` }"
				/>
			</div>
			<div v-if="showVolume" class="flex items-center gap-1">
				<IconButton :label="muted || volume === 0 ? 'Activar sonido' : 'Silenciar'" @click="toggleMute">
					<svg
						xmlns="http://www.w3.org/2000/svg"
						width="24"
						height="24"
						viewBox="0 0 24 24"
						fill="none"
						stroke="currentColor"
						stroke-width="2"
						stroke-linecap="round"
						stroke-linejoin="round"
						aria-hidden="true"
					>
						<polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5" />
						<template v-if="muted || volume === 0">
							<line x1="22" y1="9" x2="16" y2="15" />
							<line x1="16" y1="9" x2="22" y2="15" />
						</template>
						<template v-else>
							<path d="M15.54 8.46a5 5 0 0 1 0 7.07" />
							<path v-if="volume > 0.5" d="M19.07 4.93a10 10 0 0 1 0 14.14" />
						</template>
					</svg>
				</IconButton>
				<input
					type="range"
					min="0"
					max="1"
					step="0.05"
					:value="muted ? 0 : volume"
					aria-label="Volumen"
					class="volume-range w-24 cursor-pointer"
					:style="{ '--fill': `${(muted ? 0 : volume) * 100}%` }"
					@input="setVolume"
				/>
			</div>
		</div>
	</div>
</template>

<style scoped>
.volume-range {
	appearance: none;
	height: 1.5rem;
	background: transparent;
}
.volume-range::-webkit-slider-runnable-track {
	height: 6px;
	border-radius: 9999px;
	background: linear-gradient(to right, var(--color-brand) var(--fill), var(--color-divider) var(--fill));
}
.volume-range::-webkit-slider-thumb {
	appearance: none;
	width: 14px;
	height: 14px;
	margin-top: -4px;
	border: 0;
	border-radius: 9999px;
	background: var(--color-contrast);
	box-shadow: 0 1px 3px rgb(0 0 0 / 0.4);
}
.volume-range:focus-visible {
	outline: 2px solid var(--color-brand);
	outline-offset: 2px;
	border-radius: 9999px;
}
</style>
