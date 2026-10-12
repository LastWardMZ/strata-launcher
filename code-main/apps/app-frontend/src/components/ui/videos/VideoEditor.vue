<script setup lang="ts">
import { ArrowLeftIcon, PauseIcon, PlayIcon, XIcon } from '@modrinth/assets'
import { Admonition, Button, injectNotificationManager, ProgressBar } from '@modrinth/ui'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'

import {
	formatClock,
	onRecordingEvent,
	parseClock,
	playbackUrl,
	recording_reveal,
	recording_trim_cancel,
	recording_trim_start,
	recordingError,
	type VideoItem,
} from '@/helpers/recording'

import VideoPlayer from './VideoPlayer.vue'

const MIN_CLIP = 0.5

const props = defineProps<{ video: VideoItem }>()
const emit = defineEmits<{
	(e: 'close'): void
	(e: 'exported', output: string): void
}>()

const { addNotification, handleError } = injectNotificationManager()

const src = ref('')
const loadError = ref('')
const player = ref<InstanceType<typeof VideoPlayer>>()
const timeline = ref<HTMLElement>()
const duration = ref(props.video.duration ?? 0)
const start = ref(0)
const end = ref(props.video.duration ?? 0)
const now = ref(0)
const previewing = ref(false)
const precise = ref(true)
const startText = ref('0:00.0')
const endText = ref(formatClock(props.video.duration ?? 0, 1))
const job = ref<string | null>(null)
const progress = ref(0)
const exportError = ref('')
const dragging = ref<'start' | 'end' | null>(null)

const clipLength = computed(() => Math.max(0, end.value - start.value))
const valid = computed(() => duration.value > 0 && clipLength.value >= MIN_CLIP)
const exporting = computed(() => job.value !== null)
const startPercent = computed(() => (duration.value > 0 ? (start.value / duration.value) * 100 : 0))
const endPercent = computed(() => (duration.value > 0 ? (end.value / duration.value) * 100 : 100))
const nowPercent = computed(() => (duration.value > 0 ? Math.min(100, (now.value / duration.value) * 100) : 0))

function round(value: number) {
	return Math.round(value * 10) / 10
}

watch(start, (value) => {
	startText.value = formatClock(value, 1)
})
watch(end, (value) => {
	endText.value = formatClock(value, 1)
})

function setStart(value: number) {
	start.value = round(Math.max(0, Math.min(value, end.value - MIN_CLIP)))
}
function setEnd(value: number) {
	end.value = round(Math.min(duration.value, Math.max(value, start.value + MIN_CLIP)))
}

function commitText(which: 'start' | 'end') {
	const parsed = parseClock(which === 'start' ? startText.value : endText.value)
	if (parsed === null) {
		// Texto no válido: se restaura el valor actual.
		startText.value = formatClock(start.value, 1)
		endText.value = formatClock(end.value, 1)
		return
	}
	if (which === 'start') setStart(parsed)
	else setEnd(parsed)
	startText.value = formatClock(start.value, 1)
	endText.value = formatClock(end.value, 1)
	player.value?.seek(which === 'start' ? start.value : end.value)
}

function onLoaded(loaded: number) {
	// Si ffprobe no dio la duración, se usa la del vídeo y el recorte abarca todo.
	if (!props.video.duration || Math.abs(loaded - duration.value) > 0.5) {
		const wasWhole = end.value >= duration.value - 0.05 || end.value === 0
		duration.value = loaded
		if (wasWhole) end.value = round(loaded)
	}
}

function onTime(seconds: number) {
	now.value = seconds
	if (previewing.value && seconds >= end.value - 0.03) {
		player.value?.pause()
		previewing.value = false
		player.value?.seek(end.value)
	}
}

function preview() {
	if (!valid.value) return
	if (previewing.value) {
		player.value?.pause()
		previewing.value = false
		return
	}
	previewing.value = true
	player.value?.seek(start.value)
	void player.value?.play()
}

function markStart() {
	setStart(now.value)
}
function markEnd() {
	setEnd(now.value)
}

function timeFromPointer(event: PointerEvent) {
	const rect = timeline.value?.getBoundingClientRect()
	if (!rect || rect.width === 0) return 0
	return Math.max(0, Math.min(1, (event.clientX - rect.left) / rect.width)) * duration.value
}

function startDrag(which: 'start' | 'end', event: PointerEvent) {
	if (exporting.value) return
	dragging.value = which
	;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
	event.stopPropagation()
}
function moveDrag(event: PointerEvent) {
	if (!dragging.value) return
	const value = timeFromPointer(event)
	if (dragging.value === 'start') setStart(value)
	else setEnd(value)
}
function endDrag() {
	if (!dragging.value) return
	player.value?.seek(dragging.value === 'start' ? start.value : end.value)
	dragging.value = null
}
function onHandleKey(which: 'start' | 'end', event: KeyboardEvent) {
	const step = event.shiftKey ? 1 : 0.1
	const delta = event.key === 'ArrowRight' ? step : event.key === 'ArrowLeft' ? -step : 0
	if (!delta) return
	event.preventDefault()
	if (which === 'start') setStart(start.value + delta)
	else setEnd(end.value + delta)
	player.value?.seek(which === 'start' ? start.value : end.value)
}
function clickTimeline(event: PointerEvent) {
	if (exporting.value || dragging.value) return
	player.value?.seek(timeFromPointer(event))
}

async function startExport() {
	if (!valid.value || exporting.value) return
	exportError.value = ''
	progress.value = 0
	previewing.value = false
	player.value?.pause()
	try {
		job.value = await recording_trim_start(props.video.path, start.value, end.value, precise.value)
	} catch (error) {
		job.value = null
		exportError.value = recordingError(error)
	}
}

async function cancelExport() {
	if (job.value) await recording_trim_cancel(job.value).catch(() => {})
}

let unlisten: (() => void) | undefined
onMounted(async () => {
	try {
		src.value = await playbackUrl(props.video.path)
	} catch (error) {
		loadError.value = recordingError(error)
	}
	unlisten = await onRecordingEvent((event) => {
		if (event.type !== 'trim' || event.job !== job.value) return
		if (event.state === 'progress') {
			progress.value = event.progress ?? 0
		} else if (event.state === 'done') {
			job.value = null
			addNotification({
				type: 'success',
				title: 'Recorte exportado',
				text: `Se ha guardado un clip nuevo de ${formatClock(event.duration ?? clipLength.value)}. El vídeo original no se ha tocado.`,
			})
			emit('exported', event.output ?? '')
		} else if (event.state === 'error') {
			job.value = null
			exportError.value = event.message ?? 'No se pudo exportar el recorte.'
		} else if (event.state === 'canceled') {
			job.value = null
			addNotification({ type: 'info', title: 'Exportación cancelada', text: 'No se ha creado ningún archivo.' })
		}
	})
})

onBeforeUnmount(() => {
	unlisten?.()
	// Salir del editor durante una exportación la cancela: no queda ningún proceso ni archivo a medias.
	if (job.value) void recording_trim_cancel(job.value).catch(() => {})
})

function back() {
	if (exporting.value) return
	emit('close')
}

async function reveal() {
	try {
		await recording_reveal(props.video.path)
	} catch (error) {
		handleError(error as Error)
	}
}
</script>

<template>
	<div class="flex w-full flex-col gap-4">
		<div class="flex flex-wrap items-center justify-between gap-3">
			<div class="flex min-w-0 items-center gap-3">
				<Button :disabled="exporting" @click="back"><ArrowLeftIcon /> Biblioteca</Button>
				<h2 class="m-0 truncate text-xl font-bold text-contrast">Recortar: {{ video.file_name }}</h2>
			</div>
		</div>

		<Admonition v-if="loadError" type="critical" :body="loadError" />
		<Admonition
			v-if="exportError"
			type="critical"
			:body="exportError"
			dismissible
			@dismiss="exportError = ''"
		/>

		<VideoPlayer
			v-if="src"
			ref="player"
			:src="src"
			:duration="video.duration"
			:has-audio="video.has_audio"
			compact
			@loaded="onLoaded"
			@time="onTime"
			@ended="previewing = false"
			@reveal="reveal"
		/>

		<section
			class="flex flex-col gap-4 rounded-2xl bg-surface-3 p-4 ring-1 ring-inset ring-surface-5"
			:class="{ 'pointer-events-none opacity-70': exporting }"
		>
			<div class="flex flex-col gap-1.5">
				<span class="font-semibold text-contrast">Fragmento que se conserva</span>
				<div
					ref="timeline"
					class="relative h-12 cursor-pointer touch-none select-none rounded-lg bg-surface-4"
					@pointerdown="clickTimeline"
				>
					<div
						class="absolute inset-y-0 rounded-md border-y-2 border-solid border-brand bg-brand-highlight"
						:style="{ left: `${startPercent}%`, width: `${Math.max(0, endPercent - startPercent)}%` }"
					/>
					<div
						class="pointer-events-none absolute inset-y-0 w-0.5 bg-contrast"
						:style="{ left: `${nowPercent}%` }"
					/>
					<div
						role="slider"
						tabindex="0"
						aria-label="Inicio del recorte"
						:aria-valuemin="0"
						:aria-valuemax="Math.round(end)"
						:aria-valuenow="Math.round(start)"
						:aria-valuetext="formatClock(start, 1)"
						class="absolute inset-y-0 z-[2] flex w-3 -translate-x-1/2 cursor-ew-resize items-center justify-center rounded-md bg-brand outline-none focus-visible:ring-4 focus-visible:ring-brand-shadow"
						:style="{ left: `${startPercent}%` }"
						@pointerdown="startDrag('start', $event)"
						@pointermove="moveDrag"
						@pointerup="endDrag"
						@pointercancel="endDrag"
						@keydown="onHandleKey('start', $event)"
					>
						<span class="h-5 w-0.5 rounded-full bg-surface-1" />
					</div>
					<div
						role="slider"
						tabindex="0"
						aria-label="Final del recorte"
						:aria-valuemin="Math.round(start)"
						:aria-valuemax="Math.round(duration)"
						:aria-valuenow="Math.round(end)"
						:aria-valuetext="formatClock(end, 1)"
						class="absolute inset-y-0 z-[2] flex w-3 -translate-x-1/2 cursor-ew-resize items-center justify-center rounded-md bg-brand outline-none focus-visible:ring-4 focus-visible:ring-brand-shadow"
						:style="{ left: `${endPercent}%` }"
						@pointerdown="startDrag('end', $event)"
						@pointermove="moveDrag"
						@pointerup="endDrag"
						@pointercancel="endDrag"
						@keydown="onHandleKey('end', $event)"
					>
						<span class="h-5 w-0.5 rounded-full bg-surface-1" />
					</div>
				</div>
				<div class="flex justify-between text-xs tabular-nums text-secondary">
					<span>0:00</span>
					<span>{{ formatClock(duration || null) }}</span>
				</div>
			</div>

			<div class="grid gap-4 sm:grid-cols-[1fr_1fr_auto] sm:items-end">
				<div class="flex flex-col gap-1.5">
					<label for="trim-start" class="font-semibold text-contrast">Inicio</label>
					<div class="flex gap-2">
						<input
							id="trim-start"
							v-model="startText"
							class="w-full min-w-0 rounded-xl border border-solid border-surface-5 bg-surface-4 px-3 py-2 tabular-nums text-contrast"
							inputmode="decimal"
							autocomplete="off"
							@change="commitText('start')"
							@keyup.enter="commitText('start')"
						/>
						<Button type="outlined" @click="markStart">Aquí</Button>
					</div>
				</div>
				<div class="flex flex-col gap-1.5">
					<label for="trim-end" class="font-semibold text-contrast">Final</label>
					<div class="flex gap-2">
						<input
							id="trim-end"
							v-model="endText"
							class="w-full min-w-0 rounded-xl border border-solid border-surface-5 bg-surface-4 px-3 py-2 tabular-nums text-contrast"
							inputmode="decimal"
							autocomplete="off"
							@change="commitText('end')"
							@keyup.enter="commitText('end')"
						/>
						<Button type="outlined" @click="markEnd">Aquí</Button>
					</div>
				</div>
				<div class="flex flex-col gap-1.5">
					<span class="font-semibold text-contrast">Duración del clip</span>
					<span class="py-2 text-lg font-bold tabular-nums" :class="valid ? 'text-brand' : 'text-red'">
						{{ formatClock(clipLength, 1) }}
					</span>
				</div>
			</div>

			<div class="flex flex-col gap-2">
				<span class="font-semibold text-contrast">Tipo de corte</span>
				<div class="flex flex-wrap gap-2">
					<Button :type="precise ? 'colored' : 'outlined'" :color="precise ? 'brand' : undefined" @click="precise = true">
						Exacto
					</Button>
					<Button :type="!precise ? 'colored' : 'outlined'" :color="!precise ? 'brand' : undefined" @click="precise = false">
						Rápido
					</Button>
				</div>
				<p class="m-0 text-sm text-secondary">
					<template v-if="precise">
						Corta justo donde marcas (vuelve a codificar el fragmento, así que tarda un poco más).
					</template>
					<template v-else>
						Instantáneo y sin pérdida de calidad, pero el inicio puede adelantarse hasta 2 segundos hasta el
						fotograma clave más cercano.
					</template>
				</p>
			</div>
		</section>

		<div v-if="exporting" class="flex flex-col gap-2 rounded-2xl bg-surface-3 p-4 ring-1 ring-inset ring-surface-5">
			<div class="flex items-center justify-between gap-3">
				<span class="font-semibold text-contrast">Exportando… {{ Math.round(progress * 100) }}%</span>
				<Button type="outlined" @click="cancelExport"><XIcon /> Cancelar exportación</Button>
			</div>
			<ProgressBar :progress="progress" full-width label="Progreso de la exportación" />
		</div>

		<div
			class="sticky bottom-0 z-10 -mx-1 flex flex-wrap items-center gap-2 border-0 border-t border-solid border-surface-5 bg-surface-1 px-1 py-3"
		>
			<Button type="colored" color="brand" :disabled="!valid || exporting" @click="startExport">
				Exportar recorte
			</Button>
			<Button :disabled="!valid || exporting" @click="preview">
				<PauseIcon v-if="previewing" />
				<PlayIcon v-else />
				{{ previewing ? 'Detener la previsualización' : 'Previsualizar fragmento' }}
			</Button>
			<Button :disabled="exporting" @click="back">Cancelar</Button>
			<span v-if="!valid && duration > 0" class="text-sm text-red">
				El fragmento debe durar al menos medio segundo.
			</span>
		</div>
	</div>
</template>
