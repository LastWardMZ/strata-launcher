<script setup lang="ts">
import { DownloadIcon, FolderOpenIcon, KeyboardIcon, RefreshCwIcon, TrashIcon, UndoIcon } from '@modrinth/assets'
import {
	Admonition,
	Button,
	Combobox,
	type ComboboxOption,
	injectNotificationManager,
	ProgressBar,
	Toggle,
	useSavable,
} from '@modrinth/ui'
import { open } from '@tauri-apps/plugin-dialog'
import { computed, inject, onBeforeUnmount, onMounted, ref } from 'vue'

import {
	DEFAULT_HOTKEY,
	formatFileSize,
	type Hotkey,
	hotkeyLabel,
	isSafeAlone,
	isSupportedKey,
	onRecordingEvent,
	type RecordingConflict,
	recording_check_conflicts,
	recording_install_ffmpeg,
	recording_remove_ffmpeg,
	recording_reprobe_encoders,
	recording_set_settings,
	recording_status,
	recordingError,
	type RecordingSettings,
	type RecordingStatus,
} from '@/helpers/recording'
import { appSettingsModalContextKey } from '@/providers/app-settings-modal'

const { addNotification, handleError } = injectNotificationManager()
const settingsModal = inject(appSettingsModalContextKey, null)

const DEFAULTS: RecordingSettings = {
	enabled: false,
	hotkey: { ...DEFAULT_HOTKEY },
	fps: 30,
	quality: 'medium',
	resolution: 'original',
	format: 'mp4',
	game_audio: true,
	microphone: false,
	encoder: 'auto',
	folder: null,
	indicator: true,
}

const status = ref<RecordingStatus | null>(null)
const loadError = ref('')
const installing = ref(false)
const installProgress = ref<{ phase: string; downloaded: number; total: number } | null>(null)
const installError = ref('')
const capturing = ref(false)
const keyError = ref('')
const conflicts = ref<RecordingConflict[]>([])
const busyEncoders = ref(false)

const supported = computed(() => status.value?.supported !== false)
const ffmpegReady = computed(() => status.value?.ffmpeg.installed === true)

async function refresh() {
	try {
		status.value = await recording_status()
		loadError.value = ''
	} catch (error) {
		loadError.value = recordingError(error)
	}
}

const { saved, current, changes, saving, hasChanges, reset, save } = useSavable(
	() => ({ ...DEFAULTS, ...(status.value?.settings ?? {}) }) as RecordingSettings,
	async () => {
		try {
			status.value = await recording_set_settings({ ...current.value })
		} catch (error) {
			handleError(error as Error)
			throw error
		}
	},
)

async function saveSettings() {
	try {
		await save()
		addNotification({ type: 'success', title: 'Ajustes de grabación guardados', text: '' })
	} catch {
		// El error ya se ha mostrado.
	}
}

// ------------------------------------------------------------------ componente FFmpeg

const installPercent = computed(() => {
	const progress = installProgress.value
	return progress && progress.total > 0 ? Math.min(1, progress.downloaded / progress.total) : 0
})
const installLabel = computed(() => {
	const phase = installProgress.value?.phase
	if (phase === 'verifying') return 'Verificando la descarga…'
	if (phase === 'extracting') return 'Descomprimiendo…'
	if (phase === 'checking') return 'Comprobando…'
	if (installProgress.value)
		return `Descargando… ${formatFileSize(installProgress.value.downloaded)} de ${formatFileSize(installProgress.value.total)}`
	return 'Preparando la descarga…'
})

async function installFfmpeg() {
	installing.value = true
	installError.value = ''
	installProgress.value = null
	try {
		status.value = await recording_install_ffmpeg()
	} catch (error) {
		installError.value = recordingError(error)
	} finally {
		installing.value = false
		installProgress.value = null
	}
}

async function removeFfmpeg() {
	try {
		status.value = await recording_remove_ffmpeg()
		current.value.enabled = false
	} catch (error) {
		handleError(error as Error)
	}
}

async function reprobe() {
	busyEncoders.value = true
	try {
		status.value = await recording_reprobe_encoders()
	} catch (error) {
		handleError(error as Error)
	} finally {
		busyEncoders.value = false
	}
}

// ------------------------------------------------------------------ tecla

const hotkeyText = computed(() => hotkeyLabel(current.value.hotkey))
const hotkeyIsDefault = computed(() => hotkeyLabel(current.value.hotkey) === hotkeyLabel(DEFAULT_HOTKEY))

async function checkConflicts(hotkey: Hotkey) {
	try {
		conflicts.value = await recording_check_conflicts(hotkey)
	} catch {
		conflicts.value = []
	}
}

function stopCapture() {
	capturing.value = false
	window.removeEventListener('keydown', onCaptureKey, true)
}

function onCaptureKey(event: KeyboardEvent) {
	event.preventDefault()
	event.stopPropagation()
	if (event.code === 'Escape') {
		keyError.value = ''
		stopCapture()
		return
	}
	// Una pulsación solo de modificadores no es una tecla: se sigue esperando.
	if (['ControlLeft', 'ControlRight', 'AltLeft', 'AltRight', 'ShiftLeft', 'ShiftRight', 'MetaLeft', 'MetaRight'].includes(event.code)) return
	const hotkey: Hotkey = { ctrl: event.ctrlKey, alt: event.altKey, shift: event.shiftKey, key: event.code }
	stopCapture()
	if (event.metaKey) {
		keyError.value = 'La tecla de Windows no se puede usar en el atajo.'
		return
	}
	if (!isSupportedKey(hotkey.key)) {
		keyError.value = 'Esa tecla no está soportada. Prueba con una tecla de función (F1–F24) o una letra con Ctrl o Alt.'
		return
	}
	if (!hotkey.ctrl && !hotkey.alt && !isSafeAlone(hotkey.key)) {
		keyError.value =
			'Esa tecla sola se usa para escribir. Usa una tecla de función (F1–F24), Pausa, Impr Pant, o combínala con Ctrl o Alt.'
		return
	}
	keyError.value = ''
	current.value.hotkey = hotkey
	void checkConflicts(hotkey)
}

function startCapture() {
	keyError.value = ''
	capturing.value = true
	window.addEventListener('keydown', onCaptureKey, true)
}

function restoreHotkey() {
	keyError.value = ''
	current.value.hotkey = { ...DEFAULT_HOTKEY }
	void checkConflicts(current.value.hotkey)
}

// ------------------------------------------------------------------ carpeta

async function chooseFolder() {
	try {
		const chosen = await open({ directory: true, multiple: false, title: 'Carpeta de destino de las grabaciones' })
		if (typeof chosen === 'string' && chosen) current.value.folder = chosen
	} catch (error) {
		handleError(error as Error)
	}
}

// ------------------------------------------------------------------ opciones

const fpsOptions: ComboboxOption<string>[] = [
	{ value: '30', label: '30 FPS' },
	{ value: '60', label: '60 FPS' },
]
const qualityOptions: ComboboxOption<string>[] = [
	{ value: 'low', label: 'Baja (archivos pequeños)' },
	{ value: 'medium', label: 'Media (recomendada)' },
	{ value: 'high', label: 'Alta (archivos grandes)' },
]
const resolutionOptions: ComboboxOption<string>[] = [
	{ value: 'original', label: 'Original (la de la ventana)' },
	{ value: '1080p', label: 'Máximo 1080p' },
	{ value: '720p', label: 'Máximo 720p' },
]
const formatOptions: ComboboxOption<string>[] = [
	{ value: 'mp4', label: 'MP4 (recomendado)' },
	{ value: 'mkv', label: 'MKV' },
]
const encoderOptions: ComboboxOption<string>[] = [
	{ value: 'auto', label: 'Automático' },
	{ value: 'hardware', label: 'Hardware (tarjeta gráfica)' },
	{ value: 'software', label: 'Software (procesador)' },
]

const fpsModel = computed<string>({ get: () => String(current.value.fps), set: (v) => (current.value.fps = Number(v) as 30 | 60) })
const qualityModel = computed<string>({ get: () => current.value.quality, set: (v) => (current.value.quality = v as RecordingSettings['quality']) })
const resolutionModel = computed<string>({ get: () => current.value.resolution, set: (v) => (current.value.resolution = v as RecordingSettings['resolution']) })
const formatModel = computed<string>({ get: () => current.value.format, set: (v) => (current.value.format = v as RecordingSettings['format']) })
const encoderModel = computed<string>({ get: () => current.value.encoder, set: (v) => (current.value.encoder = v as RecordingSettings['encoder']) })

const encoderSummary = computed(() => {
	const found = status.value?.encoders
	if (status.value?.probing_encoders && !found) return 'Comprobando los codificadores de tu equipo…'
	if (!found) return ''
	const inUse = found.find((e) => e.name === status.value?.encoder_in_use)
	return inUse ? `Se usará: ${inUse.label}${inUse.hardware ? '' : ' (más carga para el procesador)'}` : 'No hay ningún codificador que funcione con este modo.'
})

// ------------------------------------------------------------------ ciclo de vida

let unlisten: (() => void) | undefined
onMounted(async () => {
	await refresh()
	void checkConflicts(current.value.hotkey)
	unlisten = await onRecordingEvent((event) => {
		if (event.type === 'ffmpeg') installProgress.value = { phase: event.phase, downloaded: event.downloaded, total: event.total }
		else if (['encoders', 'hotkey', 'game', 'started', 'saved'].includes(event.type)) void refresh()
	})
	settingsModal?.registerUnsavedChangesController({
		hasChanges: () => hasChanges.value,
		getOriginal: () => saved.value as unknown as Record<string, unknown>,
		getModified: () => changes.value as Record<string, unknown>,
		isSaving: () => saving.value,
		reset,
		save: saveSettings,
	})
})

onBeforeUnmount(() => {
	stopCapture()
	unlisten?.()
	settingsModal?.registerUnsavedChangesController(null)
})
</script>

<template>
	<section class="flex flex-col gap-2">
		<h2 class="m-0 text-xl font-semibold text-contrast">Grabación de partidas</h2>
		<p class="m-0 text-secondary">
			Graba tu partida de Minecraft con una tecla, sin salir del juego. Los vídeos aparecen en
			<strong class="text-contrast">Capturas → Vídeos</strong>, donde puedes verlos y recortarlos.
		</p>
	</section>

	<Admonition v-if="!supported" type="warning" class="mt-4" body="La grabación de partidas solo está disponible en Windows." />
	<Admonition v-if="loadError" type="critical" class="mt-4" :body="loadError" />

	<template v-if="supported && status">
		<!-- Componente -->
		<section class="mt-6 flex flex-col gap-3 border-0 border-t border-solid border-surface-4 pt-6">
			<h3 class="m-0 text-lg font-semibold text-contrast">Componente de grabación</h3>
			<template v-if="!ffmpegReady">
				<p class="m-0 text-secondary">
					Para grabar y recortar hace falta FFmpeg {{ status.ffmpeg.version }}. Se descarga una sola vez
					(unos {{ status.ffmpeg.download_mb }} MB) desde su origen oficial y se verifica con una huella
					SHA-256 antes de usarlo. No se instala nada más ni se toca el sistema.
				</p>
				<Admonition v-if="installError" type="critical" :body="installError" dismissible @dismiss="installError = ''" />
				<div v-if="installing" class="flex flex-col gap-2">
					<span class="text-sm text-secondary">{{ installLabel }}</span>
					<ProgressBar :progress="installPercent" full-width label="Descarga del componente de grabación" />
				</div>
				<div>
					<Button color="brand" type="colored" :disabled="installing" @click="installFfmpeg">
						<DownloadIcon /> {{ installing ? 'Instalando…' : 'Descargar e instalar' }}
					</Button>
				</div>
			</template>
			<template v-else>
				<p class="m-0 text-secondary">
					FFmpeg {{ status.ffmpeg.version }} instalado.
					<span v-if="encoderSummary">{{ encoderSummary }}</span>
				</p>
				<ul v-if="status.encoders" class="m-0 flex list-none flex-wrap gap-2 p-0">
					<li
						v-for="encoder in status.encoders"
						:key="encoder.name"
						class="rounded-full border border-solid px-3 py-1 text-sm"
						:class="encoder.available ? 'border-green bg-highlight-green text-green' : 'border-surface-5 text-secondary'"
					>
						{{ encoder.label }}: {{ encoder.available ? 'disponible' : 'no disponible' }}
					</li>
				</ul>
				<div class="flex flex-wrap gap-2">
					<Button :disabled="busyEncoders" @click="reprobe">
						<RefreshCwIcon :class="{ 'animate-spin': busyEncoders }" /> Volver a comprobar
					</Button>
					<Button type="outlined" color="red" @click="removeFfmpeg"><TrashIcon /> Quitar componente</Button>
				</div>
			</template>
		</section>

		<!-- Activación y tecla -->
		<section class="mt-6 flex flex-col gap-4 border-0 border-t border-solid border-surface-4 pt-6">
			<div class="flex items-center justify-between gap-4">
				<div>
					<h3 class="m-0 text-lg font-semibold text-contrast">Activar la grabación</h3>
					<p class="m-0 mt-1 text-secondary">
						La tecla solo funciona mientras juegas a un Minecraft lanzado desde Strata.
					</p>
				</div>
				<span v-tooltip="!ffmpegReady ? 'Instala primero el componente de grabación' : undefined" class="inline-flex shrink-0">
					<Toggle
						id="recording-enabled"
						v-model="current.enabled"
						:disabled="!ffmpegReady"
						aria-label="Activar la grabación"
					/>
				</span>
			</div>

			<div class="flex flex-col gap-2">
				<h3 class="m-0 text-lg font-semibold text-contrast">Tecla de inicio y detención</h3>
				<p class="m-0 text-secondary">
					Pulsa la tecla una vez para empezar y otra para detener. Funciona con Minecraft en primer plano.
				</p>
				<div class="flex flex-wrap items-center gap-2">
					<Button :color="capturing ? 'brand' : undefined" :type="capturing ? 'colored' : undefined" @click="capturing ? stopCapture() : startCapture()">
						<KeyboardIcon />
						{{ capturing ? 'Pulsa una tecla… (Esc para cancelar)' : hotkeyText }}
					</Button>
					<Button v-if="!capturing" type="outlined" @click="startCapture">Cambiar</Button>
					<Button type="outlined" :disabled="hotkeyIsDefault" @click="restoreHotkey"><UndoIcon /> Restaurar ({{ hotkeyLabel(DEFAULT_HOTKEY) }})</Button>
				</div>
				<Admonition v-if="keyError" type="critical" :body="keyError" dismissible @dismiss="keyError = ''" />
				<Admonition
					v-if="conflicts.length"
					type="warning"
					header="Esta tecla ya se usa en Minecraft"
					:body="`Con la grabación activa, esa tecla dejará de llegar al juego. Usada en: ${conflicts.map((c) => `${c.binding} (${c.source})`).join(', ')}. Te recomendamos elegir otra.`"
				/>
				<Admonition
					v-if="status.hotkey.error && current.enabled"
					type="critical"
					header="La tecla no se pudo registrar"
					:body="status.hotkey.error"
				/>
			</div>
		</section>

		<!-- Calidad -->
		<section class="mt-6 flex flex-col gap-4 border-0 border-t border-solid border-surface-4 pt-6">
			<h3 class="m-0 text-lg font-semibold text-contrast">Vídeo</h3>
			<div class="grid gap-4 sm:grid-cols-2">
				<div class="flex flex-col gap-1.5">
					<span class="font-semibold text-contrast">Fotogramas por segundo</span>
					<Combobox v-model="fpsModel" :options="fpsOptions" :show-icon-in-selected="false" />
					<span class="text-sm text-secondary">60 FPS es más fluido pero ocupa más y exige más al equipo.</span>
				</div>
				<div class="flex flex-col gap-1.5">
					<span class="font-semibold text-contrast">Calidad</span>
					<Combobox v-model="qualityModel" :options="qualityOptions" :show-icon-in-selected="false" />
				</div>
				<div class="flex flex-col gap-1.5">
					<span class="font-semibold text-contrast">Resolución</span>
					<Combobox v-model="resolutionModel" :options="resolutionOptions" :show-icon-in-selected="false" />
					<span class="text-sm text-secondary">Reducirla baja el consumo y el tamaño; nunca se amplía.</span>
				</div>
				<div class="flex flex-col gap-1.5">
					<span class="font-semibold text-contrast">Formato</span>
					<Combobox v-model="formatModel" :options="formatOptions" :show-icon-in-selected="false" />
				</div>
				<div class="flex flex-col gap-1.5 sm:col-span-2">
					<span class="font-semibold text-contrast">Codificación</span>
					<Combobox v-model="encoderModel" :options="encoderOptions" :show-icon-in-selected="false" />
					<span class="text-sm text-secondary">
						«Automático» usa la tarjeta gráfica si funciona y, si no, el procesador. {{ encoderSummary }}
					</span>
				</div>
			</div>
		</section>

		<!-- Audio -->
		<section class="mt-6 flex flex-col gap-4 border-0 border-t border-solid border-surface-4 pt-6">
			<h3 class="m-0 text-lg font-semibold text-contrast">Audio</h3>
			<div class="flex items-center justify-between gap-4">
				<div>
					<h4 class="m-0 font-semibold text-contrast">Audio del juego</h4>
					<p class="m-0 mt-1 text-secondary">
						Graba solo el sonido de Minecraft, no el de otras aplicaciones (requiere Windows 10 2004 o posterior).
					</p>
				</div>
				<Toggle id="recording-game-audio" v-model="current.game_audio" aria-label="Grabar el audio del juego" />
			</div>
			<div class="flex items-center justify-between gap-4">
				<div>
					<h4 class="m-0 font-semibold text-contrast">Micrófono</h4>
					<p class="m-0 mt-1 text-secondary">Mezcla tu micrófono predeterminado del sistema con el audio del juego.</p>
				</div>
				<Toggle id="recording-microphone" v-model="current.microphone" aria-label="Grabar el micrófono" />
			</div>
		</section>

		<!-- Destino -->
		<section class="mt-6 flex flex-col gap-3 border-0 border-t border-solid border-surface-4 pt-6">
			<h3 class="m-0 text-lg font-semibold text-contrast">Carpeta de destino</h3>
			<p class="m-0 text-secondary">
				<template v-if="current.folder">
					Los vídeos se guardan en una subcarpeta por instancia dentro de:
					<code class="break-all">{{ current.folder }}</code>
				</template>
				<template v-else>
					Por defecto, cada instancia guarda sus vídeos en su carpeta <code>recordings</code>, junto a las capturas.
				</template>
			</p>
			<div class="flex flex-wrap gap-2">
				<Button @click="chooseFolder"><FolderOpenIcon /> Elegir carpeta</Button>
				<Button v-if="current.folder" type="outlined" @click="current.folder = null"><UndoIcon /> Usar la predeterminada</Button>
			</div>
		</section>

		<!-- Indicador -->
		<section class="mt-6 flex items-center justify-between gap-4 border-0 border-t border-solid border-surface-4 pt-6">
			<div>
				<h3 class="m-0 text-lg font-semibold text-contrast">Indicador de grabación</h3>
				<p class="m-0 mt-1 text-secondary">
					Muestra «REC» con el tiempo en una esquina de la ventana del juego. No sale en el vídeo.
				</p>
			</div>
			<Toggle id="recording-indicator" v-model="current.indicator" aria-label="Mostrar el indicador de grabación" />
		</section>
	</template>
</template>
