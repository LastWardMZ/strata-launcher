// Grabación de partidas, biblioteca de vídeos y recorte (comandos `recording_*` del launcher; solo Windows).
import { convertFileSrc, invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

export type Hotkey = { ctrl: boolean; alt: boolean; shift: boolean; key: string }

export type RecordingSettings = {
	enabled: boolean
	hotkey: Hotkey
	fps: 30 | 60
	quality: 'low' | 'medium' | 'high'
	resolution: 'original' | '1080p' | '720p'
	format: 'mp4' | 'mkv'
	game_audio: boolean
	microphone: boolean
	encoder: 'auto' | 'hardware' | 'software'
	folder: string | null
	indicator: boolean
}

export type EncoderStatus = { name: string; label: string; hardware: boolean; available: boolean }

export type RecordingStatus = {
	supported: boolean
	settings: RecordingSettings
	ffmpeg: { installed: boolean; version: string; installing: boolean; download_mb: number }
	encoders: EncoderStatus[] | null
	encoder_in_use: string | null
	probing_encoders: boolean
	recording: { instance_name: string; elapsed_seconds: number; encoder: string; path: string } | null
	hotkey: { label: string; registered: boolean; error: string | null }
	game_running: boolean
}

export type VideoItem = {
	id: string
	path: string
	file_name: string
	instance_id: string
	instance_name: string
	created_at: string
	size: number
	duration: number | null
	width: number | null
	height: number | null
	has_audio: boolean | null
}

export type RecordingConflict = { source: string; binding: string }

export type RecordingEvent =
	| { type: 'started'; instance_name: string; encoder: string; warnings: string[] }
	| { type: 'saved'; video: VideoItem; reason: string; warnings: string[]; repaired: boolean }
	| { type: 'discarded'; reason: string; detail?: string }
	| { type: 'error'; message: string }
	| { type: 'needs-ffmpeg' }
	| { type: 'game'; running: boolean }
	| { type: 'hotkey'; registered: boolean; error: string | null }
	| { type: 'encoders' }
	| { type: 'ffmpeg'; phase: string; downloaded: number; total: number }
	| {
			type: 'trim'
			job: string
			state: 'progress' | 'done' | 'error' | 'canceled'
			progress?: number
			output?: string
			duration?: number | null
			message?: string
	  }

export const DEFAULT_HOTKEY: Hotkey = { ctrl: false, alt: false, shift: false, key: 'F9' }

export const videoKeys = {
	all: ['videos'] as const,
	list: () => ['videos', 'list'] as const,
}

export const recordingStatusKey = ['recording', 'status'] as const

export const recording_support = () => invoke<boolean>('recording_support')
export const recording_status = () => invoke<RecordingStatus>('recording_status')
export const recording_set_settings = (settings: RecordingSettings) =>
	invoke<RecordingStatus>('recording_set_settings', { settings })
export const recording_install_ffmpeg = () => invoke<RecordingStatus>('recording_install_ffmpeg')
export const recording_remove_ffmpeg = () => invoke<RecordingStatus>('recording_remove_ffmpeg')
export const recording_reprobe_encoders = () => invoke<RecordingStatus>('recording_reprobe_encoders')
export const recording_check_conflicts = (hotkey: Hotkey) =>
	invoke<RecordingConflict[]>('recording_check_conflicts', { hotkey })
export const recording_toggle = () => invoke<void>('recording_toggle')
export const recording_list = () => invoke<VideoItem[]>('recording_list')
export const recording_thumbnail = (path: string) =>
	invoke<string | null>('recording_thumbnail', { path })
export const recording_delete = (path: string) => invoke<void>('recording_delete', { path })
export const recording_reveal = (path: string) => invoke<void>('recording_reveal', { path })
export const recording_allow_playback = (path: string) =>
	invoke<void>('recording_allow_playback', { path })
export const recording_trim_start = (
	path: string,
	start: number,
	end: number,
	precise: boolean,
	gif = false,
	speed = 1,
	mute = false,
) => invoke<string>('recording_trim_start', { path, start, end, precise, gif, speed, mute })
/** Une los vídeos (en este orden) en uno nuevo; el progreso llega como los recortes (`type: 'trim'`). */
export const recording_merge_start = (paths: string[]) => invoke<string>('recording_merge_start', { paths })
export const recording_trim_cancel = (job: string) => invoke<void>('recording_trim_cancel', { job })

/** Texto legible de un error de Tauri (texto u objeto `{ message }`). */
export function recordingError(error: unknown): string {
	if (typeof error === 'string') return error
	if (error instanceof Error) return error.message
	if (error && typeof error === 'object' && 'message' in error) return String(error.message)
	return 'Ha ocurrido un error inesperado.'
}

/** Prepara un vídeo para reproducirlo en el launcher y devuelve su URL. */
export async function playbackUrl(path: string): Promise<string> {
	await recording_allow_playback(path)
	return convertFileSrc(path)
}

export function thumbnailUrl(path: string): string {
	return convertFileSrc(path)
}

export function onRecordingEvent(handler: (event: RecordingEvent) => void) {
	return listen<RecordingEvent>('recording-event', (event) => handler(event.payload))
}

// ------------------------------------------------------------------ formato

/** `mm:ss` o `h:mm:ss` (para duraciones largas). */
export function formatClock(seconds: number | null | undefined, decimals = 0): string {
	if (seconds === null || seconds === undefined || !Number.isFinite(seconds) || seconds < 0) return '--:--'
	const total = Math.floor(seconds)
	const h = Math.floor(total / 3600)
	const m = Math.floor((total % 3600) / 60)
	const s = total % 60
	const fraction = decimals > 0 ? `.${Math.floor((seconds - total) * 10 ** decimals).toString().padStart(decimals, '0')}` : ''
	const mm = h > 0 ? m.toString().padStart(2, '0') : m.toString()
	return `${h > 0 ? `${h}:` : ''}${mm}:${s.toString().padStart(2, '0')}${fraction}`
}

/** Interpreta `m:ss`, `h:mm:ss` o segundos (`35`, `35.5`). Devuelve `null` si no es válido. */
export function parseClock(text: string): number | null {
	const parts = text.trim().replace(',', '.').split(':')
	if (parts.length === 0 || parts.length > 3 || parts.some((p) => p === '' || !/^\d+(\.\d+)?$/.test(p))) return null
	return parts.reduce((total, part) => total * 60 + Number(part), 0)
}

export function formatResolution(video: Pick<VideoItem, 'width' | 'height'>): string {
	return video.width && video.height ? `${video.width}×${video.height}` : ''
}

export function formatFileSize(bytes: number): string {
	if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`
	if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(bytes < 10 * 1024 ** 2 ? 1 : 0)} MB`
	return `${(bytes / 1024 ** 3).toFixed(2)} GB`
}

/** Código de tecla de `KeyboardEvent.code` → etiqueta legible. */
export function keyLabel(code: string): string {
	if (code.startsWith('Key')) return code.slice(3)
	if (code.startsWith('Digit')) return code.slice(5)
	const names: Record<string, string> = {
		PrintScreen: 'Impr Pant',
		Pause: 'Pausa',
		ScrollLock: 'Bloq Despl',
		PageUp: 'Re Pág',
		PageDown: 'Av Pág',
		ArrowLeft: '←',
		ArrowRight: '→',
		ArrowUp: '↑',
		ArrowDown: '↓',
		Backquote: '`',
	}
	return names[code] ?? code.replace('Numpad', 'Num ')
}

export function hotkeyLabel(hotkey: Hotkey): string {
	const parts = []
	if (hotkey.ctrl) parts.push('Ctrl')
	if (hotkey.alt) parts.push('Alt')
	if (hotkey.shift) parts.push('Mayús')
	parts.push(keyLabel(hotkey.key))
	return parts.join(' + ')
}

/** Teclas que el backend admite (las mismas que `vk_from_code`). */
export function isSupportedKey(code: string): boolean {
	return (
		/^F([1-9]|1\d|2[0-4])$/.test(code) ||
		/^Key[A-Z]$/.test(code) ||
		/^Digit\d$/.test(code) ||
		/^Numpad(\d|Multiply|Add|Subtract|Decimal|Divide)$/.test(code) ||
		[
			'Pause',
			'PageUp',
			'PageDown',
			'End',
			'Home',
			'ArrowLeft',
			'ArrowUp',
			'ArrowRight',
			'ArrowDown',
			'PrintScreen',
			'Insert',
			'Delete',
			'ScrollLock',
			'Semicolon',
			'Equal',
			'Comma',
			'Minus',
			'Period',
			'Slash',
			'Backquote',
			'BracketLeft',
			'Backslash',
			'BracketRight',
			'Quote',
		].includes(code)
	)
}

/** Una tecla sola solo es válida si no se usa para escribir (F1–F24, Pausa, Impr Pant, Bloq Despl). */
export function isSafeAlone(code: string): boolean {
	return /^F([1-9]|1\d|2[0-4])$/.test(code) || ['Pause', 'PrintScreen', 'ScrollLock'].includes(code)
}
